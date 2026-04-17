use super::optimizer::IndexHint;
use super::types::{AttributeSpec, EdnValue, Pattern, PseudoAttr};
use crate::graph::FactStorage;
use crate::graph::types::{EntityId, Fact, Value};
use crate::storage::index::Indexes;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Variable bindings for query execution
/// Maps variable names (e.g., "?name") to their bound values
pub type Bindings = HashMap<String, Value>;

enum MatcherStorage {
    Owned(FactStorage),
    Slice(Arc<[Fact]>),
}

/// Pattern matcher that finds facts matching a pattern and produces bindings
pub struct PatternMatcher {
    storage: MatcherStorage,
    /// The `:db/valid-at` value for this query context (Value::Null when not set).
    pub(crate) valid_at_value: Value,
    #[allow(dead_code)]
    /// Indexes for index-guided lookups (Phase 6.2)
    indexes: Arc<Indexes>,
}

impl PatternMatcher {
    pub fn new(storage: FactStorage) -> Self {
        let indexes = storage.pending_indexes_snapshot();
        PatternMatcher {
            storage: MatcherStorage::Owned(storage),
            valid_at_value: Value::Null,
            indexes: Arc::new(indexes),
        }
    }

    /// Constructs a [`PatternMatcher`] over a pre-built, already-filtered slice of
    /// asserted facts. The caller is responsible for ensuring the slice contains
    /// only currently asserted facts (equivalent to `FactStorage::get_asserted_facts()`
    /// at the snapshot moment). No additional filtering is applied at match time.
    pub(crate) fn from_slice(facts: Arc<[Fact]>) -> Self {
        PatternMatcher {
            storage: MatcherStorage::Slice(facts),
            valid_at_value: Value::Null,
            indexes: Arc::new(Indexes::new()),
        }
    }

    /// Constructs a matcher with an explicit `:db/valid-at` binding value.
    /// Used by the executor when the query has a known `valid_at` point.
    pub(crate) fn from_slice_with_valid_at(facts: Arc<[Fact]>, valid_at: Value) -> Self {
        PatternMatcher {
            storage: MatcherStorage::Slice(facts),
            valid_at_value: valid_at,
            indexes: Arc::new(Indexes::new()),
        }
    }

    fn get_facts(&self) -> Cow<'_, [Fact]> {
        match &self.storage {
            MatcherStorage::Owned(s) => Cow::Owned(s.get_asserted_facts().unwrap_or_default()),
            MatcherStorage::Slice(s) => Cow::Borrowed(s),
        }
    }

    /// Match a single pattern against all facts in storage
    /// Returns a list of bindings, one for each matching fact
    pub fn match_pattern(&self, pattern: &Pattern) -> Vec<Bindings> {
        let mut results = Vec::new();

        // Get all currently asserted facts
        let facts = self.get_facts();

        for fact in &*facts {
            if let Some(bindings) = self.match_fact_against_pattern(fact, pattern) {
                results.push(bindings);
            }
        }

        results
    }

    /// Try to match a single fact against a pattern
    /// Returns Some(bindings) if successful, None otherwise
    fn match_fact_against_pattern(&self, fact: &Fact, pattern: &Pattern) -> Option<Bindings> {
        let mut bindings = HashMap::new();

        // Match entity
        if !self.match_component(&pattern.entity, &Value::Ref(fact.entity), &mut bindings) {
            return None;
        }

        match &pattern.attribute {
            AttributeSpec::Real(attr_edn) => {
                // Match attribute
                if !self.match_component(
                    attr_edn,
                    &Value::Keyword(fact.attribute.clone()),
                    &mut bindings,
                ) {
                    return None;
                }
                // Match value
                if !self.match_component(&pattern.value, &fact.value, &mut bindings) {
                    return None;
                }
                // Store hidden fact-metadata keys so that subsequent pseudo-attr patterns
                // for the same entity can read per-fact temporal/tx metadata without
                // cross-joining against every other fact for the entity.
                // Keys are prefixed with `__f` and namespaced by entity UUID to avoid
                // collisions across entities. They are never referenced in :find, so they
                // are silently filtered out during result extraction.
                let eid = fact.entity.to_string();
                bindings.insert(format!("__fvf_{}", eid), Value::Integer(fact.valid_from));
                bindings.insert(format!("__fvt_{}", eid), Value::Integer(fact.valid_to));
                bindings.insert(
                    format!("__ftc_{}", eid),
                    Value::Integer(fact.tx_count as i64),
                );
                bindings.insert(format!("__fti_{}", eid), Value::Integer(fact.tx_id as i64));
            }
            AttributeSpec::Pseudo(pseudo) => {
                // Pseudo-attribute: skip stored attribute match; bind fact metadata
                // field to the value position variable (or match against a constant).
                let pseudo_value = match pseudo {
                    PseudoAttr::ValidFrom => Value::Integer(fact.valid_from),
                    PseudoAttr::ValidTo => Value::Integer(fact.valid_to),
                    PseudoAttr::TxCount => Value::Integer(fact.tx_count as i64),
                    PseudoAttr::TxId => Value::Integer(fact.tx_id as i64),
                    PseudoAttr::ValidAt => self.valid_at_value.clone(),
                };
                if !self.match_component(&pattern.value, &pseudo_value, &mut bindings) {
                    return None;
                }
            }
        }

        Some(bindings)
    }

    /// Match a pattern component (entity, attribute, or value) against a fact value
    /// Returns true if match succeeds, updating bindings for variables
    fn match_component(
        &self,
        pattern_component: &EdnValue,
        fact_value: &Value,
        bindings: &mut Bindings,
    ) -> bool {
        match pattern_component {
            // Anonymous wildcard `_`: match any value without binding
            EdnValue::Symbol(var) if var == "_" => true,

            // Wildcard variable (starts with ?_): match any value without binding
            EdnValue::Symbol(var) if var.starts_with("?_") => true,

            // Variable: bind it or check consistency
            EdnValue::Symbol(var) if var.starts_with('?') => {
                if let Some(existing) = bindings.get(var) {
                    // Variable already bound, check consistency
                    existing == fact_value
                } else {
                    // Bind the variable
                    bindings.insert(var.clone(), fact_value.clone());
                    true
                }
            }

            // Constant: must match exactly
            EdnValue::Keyword(k) => {
                // Keywords can match either Value::Keyword (for attributes)
                // or Value::Ref (for entities - need to convert keyword to UUID)
                match fact_value {
                    Value::Keyword(fk) => k == fk,
                    Value::Ref(entity_id) => {
                        // Convert keyword to UUID and compare
                        if let Ok(expected_id) = edn_to_entity_id(&EdnValue::Keyword(k.clone())) {
                            expected_id == *entity_id
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }

            EdnValue::String(s) => {
                if let Value::String(fs) = fact_value {
                    s == fs
                } else {
                    false
                }
            }

            EdnValue::Integer(i) => {
                if let Value::Integer(fi) = fact_value {
                    i == fi
                } else {
                    false
                }
            }

            EdnValue::Float(f) => {
                if let Value::Float(ff) = fact_value {
                    (f - ff).abs() < f64::EPSILON
                } else {
                    false
                }
            }

            EdnValue::Boolean(b) => {
                if let Value::Boolean(fb) = fact_value {
                    b == fb
                } else {
                    false
                }
            }

            EdnValue::Uuid(u) => match fact_value {
                Value::Ref(entity_id) => u == entity_id,
                // A keyword stored as a value may represent an entity reference.
                // Convert it to its canonical UUID and compare — symmetric with
                // the EdnValue::Keyword arm above that handles Value::Ref.
                Value::Keyword(k) => {
                    edn_to_entity_id(&EdnValue::Keyword(k.clone())).is_ok_and(|id| u == &id)
                }
                _ => false,
            },

            EdnValue::Nil => matches!(fact_value, Value::Null),

            // Symbols (non-variables) or other types are not supported in patterns
            _ => false,
        }
    }

    /// Match multiple patterns with variable unification
    /// Returns bindings that satisfy all patterns simultaneously
    pub fn match_patterns(&self, patterns: &[Pattern]) -> Vec<Bindings> {
        if patterns.is_empty() {
            return vec![HashMap::new()];
        }

        // Start with the first pattern
        let mut results = self.match_pattern(&patterns[0]);

        // Join with each subsequent pattern
        for pattern in &patterns[1..] {
            results = self.join_with_pattern(results, pattern);
        }

        results
    }

    /// Match multiple patterns with index hints for optimized lookups.
    pub fn match_patterns_with_hints(&self, patterns: &[(Pattern, IndexHint)]) -> Vec<Bindings> {
        if patterns.is_empty() {
            return vec![HashMap::new()];
        }

        // Start with the first pattern
        let mut results = self.match_pattern_with_hint(&patterns[0].0, &patterns[0].1);

        // Join with each subsequent pattern
        for (pattern, _hint) in &patterns[1..] {
            results = self.join_with_pattern(results, pattern);
        }

        results
    }

    /// Match a single pattern with an index hint for optimized lookup.
    fn match_pattern_with_hint(&self, pattern: &Pattern, hint: &IndexHint) -> Vec<Bindings> {
        // Get matching fact references from index
        let fact_refs = self.lookup_with_hint(pattern, hint);

        // If no index lookup possible, fall back to full scan
        if fact_refs.is_empty() {
            return self.match_pattern(pattern);
        }

        // Get all facts and filter by the fact refs from index lookup
        let facts = self.get_facts();

        let mut results = Vec::new();
        for fact in &*facts {
            if let Some(bindings) = self.match_fact_against_pattern(fact, pattern) {
                results.push(bindings);
            }
        }

        results
    }

    /// Look up fact references using the index based on pattern and hint.
    fn lookup_with_hint(
        &self,
        pattern: &Pattern,
        hint: &IndexHint,
    ) -> Vec<crate::storage::index::FactRef> {
        let indexes = &self.indexes;

        match hint {
            IndexHint::Eavt => {
                // If entity is bound, use EAVT entity lookup
                if let EdnValue::Uuid(entity) = &pattern.entity {
                    return indexes.lookup_eavt_entity(*entity);
                }
                // Fall back to full scan
                vec![]
            }
            IndexHint::Aevt => {
                // If attribute is bound, use AEVT attribute lookup
                if let AttributeSpec::Real(EdnValue::Keyword(attr)) = &pattern.attribute {
                    return indexes.lookup_aevt_attr(attr);
                }
                // Fall back to full scan
                vec![]
            }
            IndexHint::Avet => {
                // If attribute and value are bound, use AVET
                let attr_bound = match &pattern.attribute {
                    AttributeSpec::Real(edn) => {
                        if let EdnValue::Keyword(attr) = edn {
                            Some(attr.clone())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                let value_bound = match &pattern.value {
                    EdnValue::Keyword(k) => Some(Value::Keyword(k.clone())),
                    EdnValue::String(s) => Some(Value::String(s.clone())),
                    EdnValue::Integer(i) => Some(Value::Integer(*i)),
                    EdnValue::Float(f) => Some(Value::Float(*f)),
                    EdnValue::Boolean(b) => Some(Value::Boolean(*b)),
                    EdnValue::Uuid(u) => Some(Value::Ref(*u)),
                    _ => None,
                };

                if let (Some(attr), Some(value)) = (attr_bound, value_bound) {
                    return indexes.lookup_avet_attr_value(&attr, &value);
                }
                // Fall back to full scan
                vec![]
            }
            IndexHint::Vaet => {
                // If value is a Ref, use VAET reverse lookup
                if let EdnValue::Uuid(target) = &pattern.value {
                    return indexes.lookup_vaet_ref(*target);
                }
                // Fall back to full scan
                vec![]
            }
        }
    }

    /// Match multiple patterns starting from existing seed bindings.
    ///
    /// For each seed binding, extends it by matching all patterns in sequence.
    /// Returns all extended bindings that satisfy every pattern.
    /// If `seed` is empty, returns empty. If `patterns` is empty, returns `seed` unchanged.
    pub(crate) fn match_patterns_seeded(
        &self,
        patterns: &[Pattern],
        seed: Vec<Bindings>,
    ) -> Vec<Bindings> {
        if seed.is_empty() {
            return vec![];
        }
        if patterns.is_empty() {
            return seed;
        }

        let mut results = seed;
        for pattern in patterns {
            results = self.join_with_pattern(results, pattern);
        }
        results
    }

    /// Join existing bindings with a new pattern
    /// Only keeps bindings that are consistent with the new pattern
    fn join_with_pattern(
        &self,
        existing_bindings: Vec<Bindings>,
        pattern: &Pattern,
    ) -> Vec<Bindings> {
        let mut results = Vec::new();

        for existing in existing_bindings {
            // Try to match the pattern with existing bindings
            let new_matches = self.match_pattern_with_bindings(pattern, &existing);
            results.extend(new_matches);
        }

        results
    }

    /// Match a pattern given existing variable bindings
    /// Returns new bindings that extend the existing ones
    fn match_pattern_with_bindings(&self, pattern: &Pattern, existing: &Bindings) -> Vec<Bindings> {
        // Fast path for pseudo-attr patterns: when a preceding real-attr pattern stored
        // hidden fact-metadata keys (e.g. `__fvf_<uuid>`), use them directly instead of
        // scanning all facts and cross-joining. This ensures pseudo-attr patterns are
        // correlated with the specific fact matched by the preceding real-attr pattern.
        if let AttributeSpec::Pseudo(pseudo) = &pattern.attribute {
            // Resolve the entity component to a UUID using existing bindings
            let resolved_entity = self.apply_binding_to_component(&pattern.entity, existing);
            let entity_uuid_opt: Option<uuid::Uuid> = match &resolved_entity {
                EdnValue::Uuid(u) => Some(*u),
                EdnValue::Keyword(k) => edn_to_entity_id(&EdnValue::Keyword(k.clone())).ok(),
                _ => None,
            };
            if let Some(uuid) = entity_uuid_opt {
                let eid = uuid.to_string();
                let hidden_key = match pseudo {
                    PseudoAttr::ValidFrom => format!("__fvf_{}", eid),
                    PseudoAttr::ValidTo => format!("__fvt_{}", eid),
                    PseudoAttr::TxCount => format!("__ftc_{}", eid),
                    PseudoAttr::TxId => format!("__fti_{}", eid),
                    // ValidAt is a query-level constant (not per-fact); fall through to
                    // the normal scan path so each fact produces one binding (all
                    // identical). Task 5 (executor) will inject the correct value.
                    PseudoAttr::ValidAt => {
                        return self.match_pattern_with_bindings_scan(pattern, existing);
                    }
                };
                if let Some(stored_value) = existing.get(&hidden_key) {
                    let mut new_bindings = existing.clone();
                    if self.match_component(&pattern.value, stored_value, &mut new_bindings) {
                        return vec![new_bindings];
                    }
                    return vec![];
                }
            }
        }

        // Default: scan all facts
        self.match_pattern_with_bindings_scan(pattern, existing)
    }

    /// Default join implementation: iterate all facts and try to match.
    fn match_pattern_with_bindings_scan(
        &self,
        pattern: &Pattern,
        existing: &Bindings,
    ) -> Vec<Bindings> {
        let mut results = Vec::new();

        let facts = self.get_facts();

        for fact in &*facts {
            // Try to match with existing bindings
            let mut new_bindings = existing.clone();

            // Apply existing bindings to pattern before matching
            let resolved_pattern = self.apply_bindings_to_pattern(pattern, existing);

            if let Some(additional_bindings) =
                self.match_fact_against_pattern(fact, &resolved_pattern)
            {
                // Check that additional bindings are consistent with existing.
                // Hidden fact-metadata keys (prefixed `__f`) are always overwritten
                // and are excluded from the consistency check.
                let mut consistent = true;
                for (var, val) in &additional_bindings {
                    if var.starts_with("__f") {
                        // Hidden metadata keys: always overwrite, never conflict-check.
                        continue;
                    }
                    if matches!(existing.get(var), Some(existing_val) if existing_val != val) {
                        consistent = false;
                        break;
                    }
                }

                if consistent {
                    // Merge bindings (hidden keys are overwritten by the new fact's values)
                    new_bindings.extend(additional_bindings);
                    results.push(new_bindings);
                }
            }
        }

        results
    }

    /// Apply existing bindings to a pattern, replacing bound variables with their values
    fn apply_bindings_to_pattern(&self, pattern: &Pattern, bindings: &Bindings) -> Pattern {
        let attribute = match &pattern.attribute {
            AttributeSpec::Real(edn) => {
                AttributeSpec::Real(self.apply_binding_to_component(edn, bindings))
            }
            AttributeSpec::Pseudo(p) => AttributeSpec::Pseudo(p.clone()),
        };
        Pattern {
            entity: self.apply_binding_to_component(&pattern.entity, bindings),
            attribute,
            value: self.apply_binding_to_component(&pattern.value, bindings),
            valid_from: pattern.valid_from,
            valid_to: pattern.valid_to,
        }
    }

    /// Apply bindings to a single pattern component
    fn apply_binding_to_component(&self, component: &EdnValue, bindings: &Bindings) -> EdnValue {
        match component {
            EdnValue::Symbol(var) if var.starts_with('?') => {
                if let Some(value) = bindings.get(var) {
                    // Convert Value to EdnValue
                    self.value_to_edn(value)
                } else {
                    component.clone()
                }
            }
            _ => component.clone(),
        }
    }

    /// Convert a Value to EdnValue for pattern matching
    fn value_to_edn(&self, value: &Value) -> EdnValue {
        match value {
            Value::String(s) => EdnValue::String(s.clone()),
            Value::Integer(i) => EdnValue::Integer(*i),
            Value::Float(f) => EdnValue::Float(*f),
            Value::Boolean(b) => EdnValue::Boolean(*b),
            Value::Ref(entity_id) => EdnValue::Uuid(*entity_id),
            Value::Keyword(k) => EdnValue::Keyword(k.clone()),
            Value::Null => EdnValue::Nil,
        }
    }
}

/// Convert EdnValue to Value for storage
pub fn edn_to_value(edn: &EdnValue) -> Result<Value, String> {
    match edn {
        EdnValue::String(s) => Ok(Value::String(s.clone())),
        EdnValue::Integer(i) => Ok(Value::Integer(*i)),
        EdnValue::Float(f) => Ok(Value::Float(*f)),
        EdnValue::Boolean(b) => Ok(Value::Boolean(*b)),
        EdnValue::Keyword(k) => Ok(Value::Keyword(k.clone())),
        EdnValue::Uuid(u) => Ok(Value::Ref(*u)),
        EdnValue::Nil => Ok(Value::Null),
        EdnValue::Symbol(s) if s.starts_with('?') => {
            Err(format!("Cannot convert unbound variable {} to value", s))
        }
        _ => Err(format!("Cannot convert {:?} to Value", edn)),
    }
}

/// Convert EdnValue to EntityId (must be a keyword or UUID)
pub fn edn_to_entity_id(edn: &EdnValue) -> Result<EntityId, String> {
    match edn {
        EdnValue::Keyword(k) => {
            // Convert keyword to deterministic UUID
            // For now, we'll use a simple hash-based approach
            // In production, you might want a more sophisticated method
            let hash = k.as_bytes();
            // Create a UUID from the keyword string
            // This is deterministic: same keyword always gives same UUID
            if let Ok(uuid) = Uuid::parse_str(k.trim_start_matches(':')) {
                Ok(uuid)
            } else {
                // Generate UUID from keyword name
                Ok(Uuid::new_v5(&Uuid::NAMESPACE_OID, hash))
            }
        }
        EdnValue::Uuid(u) => Ok(*u),
        _ => Err(format!(
            "Expected keyword or UUID for entity, got {:?}",
            edn
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_slice_with_valid_at_field() {
        use crate::graph::types::Value;
        use std::sync::Arc;
        let facts: Arc<[_]> = Arc::from(vec![]);
        let m = PatternMatcher::from_slice_with_valid_at(facts, Value::Integer(12345));
        assert_eq!(m.valid_at_value, Value::Integer(12345));
    }

    #[test]
    fn test_match_simple_pattern() {
        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();

        // Add some facts
        storage
            .transact(
                vec![
                    (
                        alice_id,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice_id, ":person/age".to_string(), Value::Integer(30)),
                ],
                None,
            )
            .unwrap();

        let matcher = PatternMatcher::new(storage);

        // Pattern: [?e :person/name "Alice"]
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        );

        let results = matcher.match_pattern(&pattern);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("?e"), Some(&Value::Ref(alice_id)));
    }

    #[test]
    fn test_match_pattern_with_variable_value() {
        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();

        storage
            .transact(
                vec![(
                    alice_id,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        let matcher = PatternMatcher::new(storage);

        // Pattern: [?e :person/name ?name]
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::Symbol("?name".to_string()),
        );

        let results = matcher.match_pattern(&pattern);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("?name"),
            Some(&Value::String("Alice".to_string()))
        );
    }

    #[test]
    fn test_match_multiple_patterns() {
        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();

        storage
            .transact(
                vec![
                    (
                        alice_id,
                        ":person/name".to_string(),
                        Value::String("Alice".to_string()),
                    ),
                    (alice_id, ":person/age".to_string(), Value::Integer(30)),
                ],
                None,
            )
            .unwrap();

        let matcher = PatternMatcher::new(storage);

        // Patterns: [?e :person/name ?name] [?e :person/age ?age]
        let patterns = vec![
            Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::Symbol("?name".to_string()),
            ),
            Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":person/age".to_string()),
                EdnValue::Symbol("?age".to_string()),
            ),
        ];

        let results = matcher.match_patterns(&patterns);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].get("?name"),
            Some(&Value::String("Alice".to_string()))
        );
        assert_eq!(results[0].get("?age"), Some(&Value::Integer(30)));
    }

    #[test]
    fn test_match_patterns_no_match() {
        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();

        storage
            .transact(
                vec![(
                    alice_id,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        let matcher = PatternMatcher::new(storage);

        // Pattern asks for Bob, but we only have Alice
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Bob".to_string()),
        );

        let results = matcher.match_pattern(&pattern);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_edn_to_value() {
        assert_eq!(
            edn_to_value(&EdnValue::String("test".to_string())).unwrap(),
            Value::String("test".to_string())
        );
        assert_eq!(
            edn_to_value(&EdnValue::Integer(42)).unwrap(),
            Value::Integer(42)
        );
        assert_eq!(
            edn_to_value(&EdnValue::Boolean(true)).unwrap(),
            Value::Boolean(true)
        );

        // Variables should fail
        let result = edn_to_value(&EdnValue::Symbol("?x".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn test_edn_to_entity_id() {
        let uuid = Uuid::new_v4();
        assert_eq!(edn_to_entity_id(&EdnValue::Uuid(uuid)).unwrap(), uuid);

        // Keywords should generate deterministic UUIDs
        let result1 = edn_to_entity_id(&EdnValue::Keyword(":alice".to_string())).unwrap();
        let result2 = edn_to_entity_id(&EdnValue::Keyword(":alice".to_string())).unwrap();
        assert_eq!(result1, result2); // Same keyword = same UUID
    }

    #[test]
    fn test_match_patterns_seeded_with_existing_bindings() {
        use uuid::Uuid;
        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();
        let bob_id = Uuid::new_v4();

        storage
            .transact(
                vec![
                    (alice_id, ":person/age".to_string(), Value::Integer(30)),
                    (bob_id, ":person/age".to_string(), Value::Integer(25)),
                ],
                None,
            )
            .unwrap();

        let matcher = PatternMatcher::new(storage);

        // Seed: ?e is already bound to alice_id
        let seed = vec![{
            let mut m = HashMap::new();
            m.insert("?e".to_string(), Value::Ref(alice_id));
            m
        }];

        // Pattern: [?e :person/age ?age]
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/age".to_string()),
            EdnValue::Symbol("?age".to_string()),
        );

        let results = matcher.match_patterns_seeded(&[pattern], seed);
        // Should find age=30 for alice only (bob is not in seed)
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("?age"), Some(&Value::Integer(30)));
    }

    #[test]
    fn test_match_patterns_seeded_empty_seed_returns_empty() {
        use uuid::Uuid;
        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();
        storage
            .transact(vec![(alice_id, ":a".to_string(), Value::Integer(1))], None)
            .unwrap();
        let matcher = PatternMatcher::new(storage);
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":a".to_string()),
            EdnValue::Symbol("?v".to_string()),
        );
        let results = matcher.match_patterns_seeded(&[pattern], vec![]);
        assert!(results.is_empty());
    }

    #[test]
    fn test_match_patterns_seeded_empty_patterns_returns_seed() {
        let storage = FactStorage::new();
        let matcher = PatternMatcher::new(storage);
        let seed = vec![{
            let mut m = HashMap::new();
            m.insert("?x".to_string(), Value::Integer(42));
            m
        }];
        let results = matcher.match_patterns_seeded(&[], seed.clone());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].get("?x"), Some(&Value::Integer(42)));
    }

    #[test]
    fn test_from_slice_matches_same_as_owned() {
        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice,
                    ":person/name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();

        // Build owned matcher the existing way
        let owned_matcher = PatternMatcher::new(storage.clone());

        // Build slice matcher via from_slice
        let facts: Arc<[Fact]> = Arc::from(storage.get_asserted_facts().unwrap());
        let slice_matcher = PatternMatcher::from_slice(facts);

        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::Symbol("?name".to_string()),
        );

        let owned_results = owned_matcher.match_pattern(&pattern);
        let slice_results = slice_matcher.match_pattern(&pattern);

        assert_eq!(
            owned_results.len(),
            slice_results.len(),
            "result count mismatch"
        );
        assert_eq!(
            owned_results[0].get("?name"),
            slice_results[0].get("?name"),
            "bound value mismatch"
        );
    }

    #[test]
    fn test_from_slice_empty() {
        let empty: Arc<[Fact]> = Arc::from(vec![]);
        let matcher = PatternMatcher::from_slice(empty);
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":any".to_string()),
            EdnValue::Symbol("?v".to_string()),
        );
        let results = matcher.match_pattern(&pattern);
        assert!(results.is_empty(), "empty slice should produce no results");
    }

    #[test]
    fn test_from_slice_respects_caller_prefiltering() {
        // The Slice arm applies no internal filtering — the caller is responsible.
        // This test verifies that if the caller correctly excludes retracted facts
        // before building the slice, the matcher respects that.
        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice,
                    ":name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();
        storage
            .retract(vec![(
                alice,
                ":name".to_string(),
                Value::String("Alice".to_string()),
            )])
            .unwrap();

        // net_asserted_facts() returns the true net state: for each (entity, attribute, value)
        // triple, the most recent record wins; retractions exclude the triple entirely.
        let asserted: Arc<[Fact]> = Arc::from(crate::graph::storage::net_asserted_facts(
            storage.get_all_facts().unwrap(),
        ));
        let matcher = PatternMatcher::from_slice(asserted);

        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":name".to_string()),
            EdnValue::Symbol("?v".to_string()),
        );
        let results = matcher.match_pattern(&pattern);
        assert!(
            results.is_empty(),
            "retracted fact should not appear in slice-based matcher"
        );
    }

    #[test]
    fn test_from_slice_no_internal_filtering() {
        // from_slice does NO filtering — the caller must pre-filter.
        // If the caller passes a raw slice that includes a retracted fact,
        // the matcher will return it. This is intentional by design.
        let storage = FactStorage::new();
        let alice = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice,
                    ":name".to_string(),
                    Value::String("Alice".to_string()),
                )],
                None,
            )
            .unwrap();
        storage
            .retract(vec![(
                alice,
                ":name".to_string(),
                Value::String("Alice".to_string()),
            )])
            .unwrap();

        // Deliberately build a raw, unfiltered slice (all facts, including the retraction)
        // This simulates a caller mistake — passing unfiltered facts to from_slice.
        let all_facts: Arc<[Fact]> = Arc::from(storage.get_all_facts().unwrap());
        let matcher = PatternMatcher::from_slice(all_facts);

        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":name".to_string()),
            EdnValue::Symbol("?v".to_string()),
        );
        let results = matcher.match_pattern(&pattern);

        // The matcher returns ALL facts in the slice — both the assert and retract records.
        // Caller must pre-filter; from_slice does not filter internally.
        assert!(
            !results.is_empty(),
            "from_slice does not filter internally — raw slice passes through unchanged"
        );
    }

    #[test]
    fn test_pseudo_attr_valid_from_join() {
        // Simulates the time_interval_entire_interval test pattern:
        // [?e :item/label _] [?e :db/valid-from ?vf]
        use crate::graph::storage::net_asserted_facts;
        use crate::graph::types::{Fact, Value};
        use crate::query::datalog::types::{AttributeSpec, PseudoAttr};
        use uuid::Uuid;

        let storage = crate::graph::FactStorage::new();
        let e1 = Uuid::new_v4();

        // Transact e1 with explicit valid-from = 1577836800000
        let opt =
            crate::graph::types::TransactOptions::new(Some(1577836800000), Some(1735689600000));
        storage
            .transact_batch(
                vec![(
                    e1,
                    ":item/label".to_string(),
                    Value::String("A".to_string()),
                    None,
                )],
                Some(opt),
            )
            .unwrap();

        // Get all facts
        let all_facts: Arc<[Fact]> =
            Arc::from(net_asserted_facts(storage.get_all_facts().unwrap()));
        let matcher = PatternMatcher::from_slice(all_facts.clone());

        // First pattern: [?e :item/label _]
        let p1 = Pattern {
            entity: EdnValue::Symbol("?e".to_string()),
            attribute: AttributeSpec::Real(EdnValue::Keyword(":item/label".to_string())),
            value: EdnValue::Symbol("_".to_string()),
            valid_from: None,
            valid_to: None,
        };
        let r1 = matcher.match_pattern(&p1);
        assert_eq!(r1.len(), 1, "first pattern should bind ?e");

        // Second pattern: [?e :db/valid-from ?vf]
        let p2 = Pattern {
            entity: EdnValue::Symbol("?e".to_string()),
            attribute: AttributeSpec::Pseudo(PseudoAttr::ValidFrom),
            value: EdnValue::Symbol("?vf".to_string()),
            valid_from: None,
            valid_to: None,
        };
        let r2 = matcher.match_patterns_seeded(&[p2], r1);
        assert_eq!(r2.len(), 1, "second pattern should bind ?vf");
        assert_eq!(r2[0].get("?vf"), Some(&Value::Integer(1577836800000)));
    }

    #[test]
    fn test_pseudo_attr_valid_to_tx_count_tx_id_scan_path() {
        // Exercises matcher.rs lines 120-122: ValidTo/TxCount/TxId arms in
        // match_fact_against_pattern (scan path — no hidden keys seeded).
        use crate::graph::storage::net_asserted_facts;
        use crate::graph::types::Value as GValue;
        use std::sync::Arc;

        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice_id,
                    ":item/label".to_string(),
                    GValue::String("z".to_string()),
                )],
                None,
            )
            .unwrap();

        let all_facts: Arc<[Fact]> =
            Arc::from(net_asserted_facts(storage.get_all_facts().unwrap()));
        let matcher = PatternMatcher::from_slice(all_facts);

        // Each test uses a UUID entity + wildcard value so the entity check passes
        // and no hidden key is seeded → falls through to scan → hits line 120/121/122.

        // Line 120: ValidTo
        let p_vt = Pattern {
            entity: EdnValue::Uuid(alice_id),
            attribute: AttributeSpec::Pseudo(PseudoAttr::ValidTo),
            value: EdnValue::Symbol("?vt".to_string()),
            valid_from: None,
            valid_to: None,
        };
        let r_vt = matcher.match_pattern(&p_vt);
        assert_eq!(r_vt.len(), 1, "ValidTo scan should bind one result");

        // Line 121: TxCount
        let p_tc = Pattern {
            entity: EdnValue::Uuid(alice_id),
            attribute: AttributeSpec::Pseudo(PseudoAttr::TxCount),
            value: EdnValue::Symbol("?tc".to_string()),
            valid_from: None,
            valid_to: None,
        };
        let r_tc = matcher.match_pattern(&p_tc);
        assert_eq!(r_tc.len(), 1, "TxCount scan should bind one result");

        // Line 122: TxId
        let p_ti = Pattern {
            entity: EdnValue::Uuid(alice_id),
            attribute: AttributeSpec::Pseudo(PseudoAttr::TxId),
            value: EdnValue::Symbol("?ti".to_string()),
            valid_from: None,
            valid_to: None,
        };
        let r_ti = matcher.match_pattern(&p_ti);
        assert_eq!(r_ti.len(), 1, "TxId scan should bind one result");
    }

    #[test]
    fn test_pseudo_attr_entity_non_uuid_non_keyword_falls_through() {
        // Exercises matcher.rs line 302: `_ => None` when resolved entity is
        // neither Uuid nor Keyword — falls through to scan path.
        use crate::graph::storage::net_asserted_facts;
        use crate::graph::types::Value as GValue;
        use std::sync::Arc;

        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice_id,
                    ":item/label".to_string(),
                    GValue::String("x".to_string()),
                )],
                None,
            )
            .unwrap();

        let all_facts: Arc<[Fact]> =
            Arc::from(net_asserted_facts(storage.get_all_facts().unwrap()));
        let matcher = PatternMatcher::from_slice(all_facts);

        // Entity is an Integer — neither Uuid nor Keyword → `_ => None` path
        let pattern = Pattern {
            entity: EdnValue::Integer(99),
            attribute: AttributeSpec::Pseudo(PseudoAttr::ValidFrom),
            value: EdnValue::Symbol("?vf".to_string()),
            valid_from: None,
            valid_to: None,
        };
        // Falls through to scan; Integer entity won't match any stored UUID → 0 results
        let results = matcher.match_pattern(&pattern);
        assert_eq!(
            results.len(),
            0,
            "non-uuid/keyword entity should yield no matches"
        );
    }

    #[test]
    fn test_pseudo_attr_hidden_key_value_mismatch_returns_empty() {
        // Exercises matcher.rs line 323: `return vec![]` when the stored hidden-key
        // value doesn't match the pattern value component.
        use crate::graph::storage::net_asserted_facts;
        use crate::graph::types::Value as GValue;
        use std::sync::Arc;

        let storage = FactStorage::new();
        let alice_id = Uuid::new_v4();
        storage
            .transact(
                vec![(
                    alice_id,
                    ":item/label".to_string(),
                    GValue::String("y".to_string()),
                )],
                None,
            )
            .unwrap();

        let all_facts: Arc<[Fact]> =
            Arc::from(net_asserted_facts(storage.get_all_facts().unwrap()));
        let matcher = PatternMatcher::from_slice(all_facts.clone());

        // Seed bindings with the real-attr pattern so hidden keys are populated
        let p1 = Pattern {
            entity: EdnValue::Symbol("?e".to_string()),
            attribute: AttributeSpec::Real(EdnValue::Keyword(":item/label".to_string())),
            value: EdnValue::Symbol("_".to_string()),
            valid_from: None,
            valid_to: None,
        };
        let seeded = matcher.match_pattern(&p1);
        assert_eq!(seeded.len(), 1, "seed should match one fact");

        // Ask for :db/valid-from with an impossible constant value (-999)
        // The hidden key exists but -999 won't match the real valid_from → vec![]
        let p2 = Pattern {
            entity: EdnValue::Symbol("?e".to_string()),
            attribute: AttributeSpec::Pseudo(PseudoAttr::ValidFrom),
            value: EdnValue::Integer(-999),
            valid_from: None,
            valid_to: None,
        };
        let results = matcher.match_patterns_seeded(&[p2], seeded);
        assert_eq!(
            results.len(),
            0,
            "mismatched constant should return no bindings"
        );
    }
}
