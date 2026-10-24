use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ErrorCode, MinigrafError, err_coded};

// ============================================================================
// Datalog EAV Model (Phase 3+)
// ============================================================================

/// Transaction ID type - milliseconds since UNIX epoch
///
/// We use timestamps as transaction IDs for natural chronological ordering
/// and consistency with bi-temporal valid_time (Phase 4). Millisecond precision
/// is sufficient for Phase 3's single-threaded usage.
pub(crate) type TxId = u64;

/// Get current timestamp as transaction ID (milliseconds since UNIX epoch)
pub(crate) fn tx_id_now() -> TxId {
    #[cfg(all(target_arch = "wasm32", feature = "browser"))]
    {
        // On WASM browser targets, `std::time::SystemTime::now()` panics.
        // Use `js_sys::Date::now()` which returns milliseconds since the Unix
        // epoch as an f64 (same precision as `Date.now()` in JavaScript).
        // It is a whole, non-negative number far below 2^53, and an `f64` to
        // `u64` cast saturates (NaN gives 0), so the cast cannot misbehave.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let ms = js_sys::Date::now() as u64;
        ms
    }
    #[cfg(not(all(target_arch = "wasm32", feature = "browser")))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    }
}

/// A unique identifier for a graph entity (UUID v4).
///
/// In Datalog syntax, entities are written as keywords (`:alice`, `:person/42`)
/// or as UUID literals (`#uuid "550e8400-e29b-41d4-a716-446655440000"`).
/// `EntityId` values appear in [`Value::Ref`] for cross-entity relationships
/// and are returned in query result rows.
pub type EntityId = Uuid;

/// Attribute name - namespace-qualified keywords like ":person/name" or ":friend"
pub(crate) type Attribute = String;

/// All value types that can be stored in a Minigraf fact.
///
/// Values appear in the third position of EAV triples:
/// `[entity attribute value]`, e.g. `[:alice :person/name "Alice"]`.
///
/// # Datalog literals
///
/// | Variant | Datalog syntax | Example |
/// |---------|---------------|---------|
/// | `String` | double-quoted | `"hello"` |
/// | `Integer` | bare integer | `42`, `-7` |
/// | `Float` | decimal | `3.14`, `-0.5` |
/// | `Boolean` | `true` / `false` | `true` |
/// | `Ref` | keyword entity or UUID | `:alice`, `#uuid "550e8400-..."` |
/// | `Keyword` | colon-prefixed | `:status/active`, `:red` |
/// | `Null` | `nil` | `nil` |
/// | `Set` | `#{...}` of the above | `#{:role/admin :role/editor}` |
///
/// New variants may be added in later releases, so a `match` outside this
/// crate needs a wildcard arm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum Value {
    /// A UTF-8 string. Datalog literal: `"hello"`.
    String(String),
    /// A 64-bit signed integer. Datalog literal: `42` or `-7`.
    Integer(i64),
    /// A 64-bit float. Datalog literal: `3.14` or `-0.5`.
    Float(f64),
    /// A boolean. Datalog literal: `true` or `false`.
    Boolean(bool),
    /// A reference to another entity by its [`EntityId`] (UUID).
    /// Datalog literal: `:alice` (keyword short-form) or `#uuid "..."`.
    Ref(EntityId),
    /// A namespaced keyword used for enumerated values and tags.
    /// Datalog literal: `:status/active`, `:red`.
    Keyword(String),
    /// An absent value. Datalog literal: `nil`.
    Null,
    /// A finite set of the values above (never of sets). Datalog literal:
    /// `#{1 2 3}`.
    Set(ValueSet),
}

impl Eq for Value {}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Value::String(s) => s.hash(state),
            Value::Integer(i) => i.hash(state),
            Value::Float(f) => {
                if f.is_nan() {
                    0_u8.hash(state);
                } else if *f == 0.0 {
                    // `-0.0 == 0.0`, so both must hash alike (#545).
                    2_u8.hash(state);
                    0_u64.hash(state);
                } else if f.is_sign_negative() {
                    1_u8.hash(state);
                    (-f).to_bits().hash(state);
                } else {
                    2_u8.hash(state);
                    f.to_bits().hash(state);
                }
            }
            Value::Boolean(b) => b.hash(state),
            Value::Ref(r) => r.hash(state),
            Value::Keyword(k) => k.hash(state),
            Value::Null => {}
            Value::Set(s) => s.hash(state),
        }
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Ordering for `Value` variants.
///
/// # NaN Semantics
/// - `NaN` is ordered as **Greater** than all other float values (including positive infinity)
/// - `NaN` equals `NaN` (two NaN values are considered equal)
/// - When comparing a float against a non-float, the float's NaN status determines ordering
///
/// # Cross-Variant Ordering
/// Values of different variants are ordered by discriminant:
/// - String (0) < Integer (1) < Float (2) < Boolean (3) < Ref (4) < Keyword (5) < Null (6) < Set (7)
///
/// Two sets compare by their elements in this order, lexicographically; a
/// set that is a proper prefix of another is less.
impl Ord for Value {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Simple variant ordering using match
        match (self, other) {
            // Same variant - compare inner values
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                if a.is_nan() && b.is_nan() {
                    std::cmp::Ordering::Equal
                } else if a.is_nan() {
                    std::cmp::Ordering::Greater
                } else if b.is_nan() {
                    std::cmp::Ordering::Less
                } else {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                }
            }
            (Value::Boolean(a), Value::Boolean(b)) => a.cmp(b),
            (Value::Ref(a), Value::Ref(b)) => a.cmp(b),
            (Value::Keyword(a), Value::Keyword(b)) => a.cmp(b),
            (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
            (Value::Set(a), Value::Set(b)) => a.cmp(b),
            // Different variants — order by a stable integer discriminant so
            // the total order is deterministic and does not depend on stack
            // addresses (the previous implementation used pointer values here,
            // which are non-deterministic and violate Ord's contract).
            _ => {
                fn discriminant(v: &Value) -> u8 {
                    match v {
                        Value::String(_) => 0,
                        Value::Integer(_) => 1,
                        Value::Float(_) => 2,
                        Value::Boolean(_) => 3,
                        Value::Ref(_) => 4,
                        Value::Keyword(_) => 5,
                        Value::Null => 6,
                        Value::Set(_) => 7,
                    }
                }
                discriminant(self).cmp(&discriminant(other))
            }
        }
    }
}

impl Value {
    /// Extract string value if this is a String variant
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Extract integer value if this is an Integer variant
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Extract float value if this is a Float variant
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Extract boolean value if this is a Boolean variant
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract entity reference if this is a Ref variant
    pub fn as_ref(&self) -> Option<EntityId> {
        match self {
            Value::Ref(id) => Some(*id),
            _ => None,
        }
    }

    /// Extract keyword if this is a Keyword variant
    pub fn as_keyword(&self) -> Option<&str> {
        match self {
            Value::Keyword(k) => Some(k),
            _ => None,
        }
    }

    /// Check if this value is Null
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Extract the set if this is a Set variant
    pub fn as_set(&self) -> Option<&ValueSet> {
        match self {
            Value::Set(s) => Some(s),
            _ => None,
        }
    }
}

/// A finite set of values, none of them a set (#316).
///
/// Two elements are the same element when [`Value`]'s `Ord` says they are
/// equal: `-0.0` and `0.0` are one element (`-0.0` is kept), every NaN is
/// one element, a keyword never equals a ref, and `1` never equals `1.0`.
/// Iteration is in `Value`'s order.
#[derive(Clone, Default, PartialOrd, Ord)]
pub struct ValueSet(BTreeSet<Value>);

impl ValueSet {
    /// The set of `values`, duplicates removed. A set among them is API-021.
    pub fn new(values: impl IntoIterator<Item = Value>) -> Result<Self, MinigrafError> {
        let mut set = BTreeSet::new();
        for v in values {
            if matches!(v, Value::Set(_)) {
                return Err(MinigrafError::from(err_coded!(ErrorCode::Api021)));
            }
            // `0.0` and `-0.0` are one element: keep `-0.0`, the least of the
            // two by `value_order`, whichever came first.
            let replace = match (&v, set.get(&v)) {
                (Value::Float(new), Some(Value::Float(old))) => {
                    *new == 0.0 && new.is_sign_negative() && !old.is_sign_negative()
                }
                (_, Some(_)) => false,
                (_, None) => true,
            };
            if replace {
                set.replace(v);
            }
        }
        Ok(ValueSet(set))
    }

    /// The elements, in `Value`'s order.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.0.iter()
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the set has no elements.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// True when `v` is an element.
    pub fn contains(&self, v: &Value) -> bool {
        self.0.contains(v)
    }
}

impl<'a> IntoIterator for &'a ValueSet {
    type Item = &'a Value;
    type IntoIter = std::collections::btree_set::Iter<'a, Value>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Equal when every element is equal under `Value`'s `Ord`, so a set holding
/// a NaN equals itself.
impl PartialEq for ValueSet {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for ValueSet {}

impl std::hash::Hash for ValueSet {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.len().hash(state);
        for v in &self.0 {
            v.hash(state);
        }
    }
}

impl std::fmt::Debug for ValueSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("#")?;
        f.debug_set().entries(self.0.iter()).finish()
    }
}

impl Serialize for ValueSet {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter())
    }
}

/// Read through [`ValueSet::new`], so the order is restored and a nested set
/// is an error, whatever the input.
impl<'de> Deserialize<'de> for ValueSet {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let values = Vec::<Value>::deserialize(deserializer)?;
        ValueSet::new(values).map_err(serde::de::Error::custom)
    }
}

/// Sentinel value for open-ended valid time (a fact is valid "forever").
/// Used as the default `valid_to` when no end time is specified.
pub(crate) const VALID_TIME_FOREVER: i64 = i64::MAX;

/// A Datalog fact: (Entity, Attribute, Value) triple with transaction metadata
///
/// This is the core data structure for Phase 3+. Facts are immutable and versioned
/// by transaction ID. Facts are never deleted, only retracted (asserted=false).
///
/// Phase 4 adds bi-temporal fields:
/// - `tx_count`: monotonically incrementing batch counter within a transaction
/// - `valid_from`: when the fact became valid in the real world (millis since epoch)
/// - `valid_to`: when the fact stopped being valid (`VALID_TIME_FOREVER` = open-ended)
///
/// # Examples
/// ```ignore
/// use uuid::Uuid;
/// use crate::graph::types::{Fact, Value};
///
/// // Fact: Alice's name is "Alice"
/// let alice_id = Uuid::new_v4();
/// let fact = Fact::new(
///     alice_id,
///     ":person/name".to_string(),
///     Value::String("Alice".to_string()),
///     1, // transaction ID
/// );
///
/// // Fact: Alice is friends with Bob (reference)
/// let bob_id = Uuid::new_v4();
/// let friendship = Fact::new(
///     alice_id,
///     ":friend".to_string(),
///     Value::Ref(bob_id),
///     2,
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Fact {
    /// The entity this fact is about
    pub(crate) entity: EntityId,
    /// The attribute/property name (namespace-qualified, e.g., ":person/name")
    pub(crate) attribute: Attribute,
    /// The value of this attribute
    pub(crate) value: Value,
    /// Transaction ID that asserted or retracted this fact
    pub(crate) tx_id: TxId,
    /// Monotonically incrementing batch counter within a transaction (Phase 4)
    pub(crate) tx_count: u64,
    /// Valid-time start: when the fact became valid in the real world (millis since epoch).
    /// Defaults to `tx_id as i64` (wall-clock time of the transaction).
    pub(crate) valid_from: i64,
    /// Valid-time end: when the fact stopped being valid (millis since epoch).
    /// `VALID_TIME_FOREVER` means the fact is open-ended (still valid).
    pub(crate) valid_to: i64,
    /// True if this fact is asserted, false if retracted.
    /// Retractions are used instead of deletions to maintain history.
    pub(crate) asserted: bool,
}

impl Fact {
    /// Create a new asserted fact with default valid time (valid_from=tx_id, valid_to=FOREVER).
    #[cfg(test)]
    pub(crate) fn new(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: i64::try_from(tx_id).unwrap_or(i64::MAX),
            valid_to: VALID_TIME_FOREVER,
            asserted: true,
        }
    }

    /// Create an asserted fact with explicit valid time and tx_count.
    pub(crate) fn with_valid_time(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
        tx_count: u64,
        valid_from: i64,
        valid_to: i64,
    ) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count,
            valid_from,
            valid_to,
            asserted: true,
        }
    }

    /// Create a retraction with default valid time.
    pub(crate) fn retract(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
    ) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: i64::try_from(tx_id).unwrap_or(i64::MAX),
            valid_to: VALID_TIME_FOREVER,
            asserted: false,
        }
    }

    /// Check if this is an assertion (not a retraction)
    #[cfg(test)]
    pub(crate) fn is_asserted(&self) -> bool {
        self.asserted
    }
}

#[cfg(test)]
impl Fact {
    /// Create a fact with explicit asserted flag and default valid time.
    /// Used only in tests.
    #[allow(dead_code)]
    pub(crate) fn with_asserted(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
        asserted: bool,
    ) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: i64::try_from(tx_id).unwrap_or(i64::MAX),
            valid_to: VALID_TIME_FOREVER,
            asserted,
        }
    }

    /// Check if this is a retraction. Used only in tests.
    pub(crate) fn is_retracted(&self) -> bool {
        !self.asserted
    }
}

/// Options for controlling valid time on a transact/retract call.
///
/// When `valid_from` is `None`, defaults to the transaction timestamp.
/// When `valid_to` is `None`, defaults to `VALID_TIME_FOREVER` (open-ended).
#[derive(Debug, Clone, Default)]
pub(crate) struct TransactOptions {
    /// Override the valid-time start (millis since epoch). `None` = use tx timestamp.
    pub(crate) valid_from: Option<i64>,
    /// Override the valid-time end (millis since epoch). `None` = open-ended (FOREVER).
    pub(crate) valid_to: Option<i64>,
}

impl TransactOptions {
    pub(crate) fn new(valid_from: Option<i64>, valid_to: Option<i64>) -> Self {
        TransactOptions {
            valid_from,
            valid_to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a TxId from a SystemTime. Test-only helper.
    fn tx_id_from_system_time(time: std::time::SystemTime) -> TxId {
        u64::try_from(
            time.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX)
    }

    #[test]
    fn test_tx_id_timestamp() {
        // Test tx_id_now() returns monotonically increasing values
        let tx1 = tx_id_now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let tx2 = tx_id_now();

        // tx2 should be after tx1 (monotonicity is what matters)
        assert!(
            tx2 > tx1,
            "Transaction IDs should be chronologically ordered"
        );

        // Test that tx_id_from_system_time produces valid tx_id
        use std::time::SystemTime;
        let now = SystemTime::now();
        let tx_id = tx_id_from_system_time(now);
        // tx_id should be positive (valid timestamp)
        assert!(
            tx_id > 0,
            "tx_id_from_system_time should produce positive value"
        );
    }

    #[test]
    fn test_tx_id_ordering() {
        // TxIds created sequentially should be ordered (monotonicity is what matters, not exact timing)
        let mut tx_ids = vec![];
        for _ in 0..5 {
            tx_ids.push(tx_id_now());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        // Verify chronological order
        for i in 1..tx_ids.len() {
            assert!(
                tx_ids[i] > tx_ids[i - 1],
                "TxIds should be strictly increasing"
            );
        }
    }

    #[test]
    fn test_value_creation_and_accessors() {
        // String value
        let string_val = Value::String("Alice".to_string());
        assert_eq!(string_val.as_string(), Some("Alice"));
        assert_eq!(string_val.as_integer(), None);
        assert!(!string_val.is_null());

        // Integer value
        let int_val = Value::Integer(42);
        assert_eq!(int_val.as_integer(), Some(42));
        assert_eq!(int_val.as_string(), None);

        // Float value
        let float_val = Value::Float(4.5);
        assert_eq!(float_val.as_float(), Some(4.5));

        // Boolean value
        let bool_val = Value::Boolean(true);
        assert_eq!(bool_val.as_boolean(), Some(true));

        // Reference value
        let ref_id = Uuid::new_v4();
        let ref_val = Value::Ref(ref_id);
        assert_eq!(ref_val.as_ref(), Some(ref_id));
        assert_eq!(ref_val.as_string(), None);

        // Keyword value
        let keyword_val = Value::Keyword(":person".to_string());
        assert_eq!(keyword_val.as_keyword(), Some(":person"));

        // Null value
        let null_val = Value::Null;
        assert!(null_val.is_null());
        assert_eq!(null_val.as_string(), None);
    }

    #[test]
    fn test_fact_creation() {
        let entity = Uuid::new_v4();
        let fact = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        assert_eq!(fact.entity, entity);
        assert_eq!(fact.attribute, ":person/name");
        assert_eq!(fact.value, Value::String("Alice".to_string()));
        assert_eq!(fact.tx_id, 1);
        assert!(fact.is_asserted());
        assert!(!fact.is_retracted());
    }

    #[test]
    fn test_fact_retraction() {
        let entity = Uuid::new_v4();
        let fact = Fact::retract(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            2,
        );

        assert_eq!(fact.entity, entity);
        assert_eq!(fact.attribute, ":person/name");
        assert_eq!(fact.tx_id, 2);
        assert!(!fact.is_asserted());
        assert!(fact.is_retracted());
    }

    #[test]
    fn test_fact_with_ref_value() {
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // Fact: Alice is friends with Bob
        let friendship = Fact::new(alice, ":friend".to_string(), Value::Ref(bob), 1);

        assert_eq!(friendship.entity, alice);
        assert_eq!(friendship.attribute, ":friend");
        assert_eq!(friendship.value.as_ref(), Some(bob));
        assert!(friendship.is_asserted());
    }

    #[test]
    fn test_fact_equality() {
        let entity = Uuid::new_v4();

        let fact1 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        let fact2 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        assert_eq!(fact1, fact2);

        // Different transaction ID = different fact
        let fact3 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            2,
        );

        assert_ne!(fact1, fact3);

        // Different tx_count = different fact
        let fact4 = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
            99, // tx_count differs from fact1 (which has tx_count=0)
            1,
            VALID_TIME_FOREVER,
        );

        assert_ne!(fact1, fact4);

        // Different valid_from = different fact
        let fact5 = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
            0,
            9999, // valid_from differs from fact1 (which has valid_from=tx_id=1)
            VALID_TIME_FOREVER,
        );

        assert_ne!(fact1, fact5);

        // Different valid_to = different fact
        let fact6 = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
            0,
            1,
            12345, // valid_to differs from fact1 (which has valid_to=VALID_TIME_FOREVER)
        );

        assert_ne!(fact1, fact6);
    }

    #[test]
    fn test_value_types() {
        let values = vec![
            Value::String("test".to_string()),
            Value::Integer(42),
            Value::Float(4.5),
            Value::Boolean(true),
            Value::Ref(Uuid::new_v4()),
            Value::Keyword(":status/active".to_string()),
            Value::Null,
        ];

        // All values should serialize/deserialize correctly
        for value in values {
            let serialized = serde_json::to_string(&value).unwrap();
            let deserialized: Value = serde_json::from_str(&serialized).unwrap();
            assert_eq!(value, deserialized);
        }
    }

    #[test]
    fn test_fact_has_valid_time_fields() {
        let entity = Uuid::new_v4();
        let fact = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1000,
        );
        // Defaults: valid_from = tx_id as i64, valid_to = FOREVER, tx_count = 0
        assert_eq!(fact.valid_from, 1000_i64);
        assert_eq!(fact.valid_to, VALID_TIME_FOREVER);
        assert_eq!(fact.tx_count, 0);
    }

    #[test]
    fn test_fact_with_explicit_valid_time() {
        let entity = Uuid::new_v4();
        let fact = Fact::with_valid_time(
            entity,
            ":employment/status".to_string(),
            Value::Keyword(":active".to_string()),
            1000,
            1,
            1672531200000_i64, // 2023-01-01
            1685577600000_i64, // 2023-06-01
        );
        assert_eq!(fact.valid_from, 1672531200000_i64);
        assert_eq!(fact.valid_to, 1685577600000_i64);
        assert_eq!(fact.tx_count, 1);
    }

    #[test]
    fn test_valid_time_forever_constant() {
        assert_eq!(VALID_TIME_FOREVER, i64::MAX);
    }

    #[test]
    fn test_transact_options_defaults() {
        let opts = TransactOptions::default();
        assert!(opts.valid_from.is_none());
        assert!(opts.valid_to.is_none());
    }

    // Helper used by the cross-variant Ord tests: moves values into a fresh
    // stack frame so the two parameters are always at distinct, consistent
    // addresses independent of the call site.
    fn cmp_values(a: Value, b: Value) -> std::cmp::Ordering {
        a.cmp(&b)
    }

    #[test]
    fn value_ord_cross_variant_is_antisymmetric() {
        // The pointer-address bug causes both cmp_values(A, B) and
        // cmp_values(B, A) to return the same ordering (whichever parameter
        // slot has the lower address "wins" in both calls), violating the
        // antisymmetry requirement for Ord: cmp(a,b) must equal reverse(cmp(b,a)).
        let pairs: &[(Value, Value)] = &[
            (Value::String("hello".into()), Value::Integer(42)),
            (Value::Integer(1), Value::Float(1.0)),
            (Value::Boolean(true), Value::Null),
            (Value::Keyword(":k".into()), Value::String("x".into())),
        ];
        for (a, b) in pairs {
            let forward = cmp_values(a.clone(), b.clone());
            let backward = cmp_values(b.clone(), a.clone());
            assert_eq!(
                forward,
                backward.reverse(),
                "Value::Ord cross-variant comparison must be antisymmetric"
            );
            assert_ne!(
                forward,
                std::cmp::Ordering::Equal,
                "Values of different types must not compare as Equal"
            );
        }
    }

    #[test]
    fn value_ord_cross_variant_is_stable() {
        // Ordering between two fixed variant types must be stable: it should
        // not depend on what the inner value is, only on which variant it is.
        // With the pointer-address bug every allocation is at a different
        // address, so this constraint can be violated non-deterministically.
        let string_lt_integer = cmp_values(Value::String("a".into()), Value::Integer(0));
        for s in ["", "z", "hello world"] {
            for i in [i64::MIN, 0, i64::MAX] {
                let result = cmp_values(Value::String(s.into()), Value::Integer(i));
                assert_eq!(
                    result, string_lt_integer,
                    "Cross-variant ordering must depend only on the variant, not the inner value"
                );
            }
        }
    }

    /// Equal values hash alike (#545): `-0.0 == 0.0`.
    #[test]
    fn equal_floats_hash_alike() {
        use std::hash::{BuildHasher, RandomState};
        let h = RandomState::new();
        assert_eq!(
            h.hash_one(Value::Float(-0.0)),
            h.hash_one(Value::Float(0.0)),
            "-0.0 and 0.0"
        );
        assert_ne!(
            h.hash_one(Value::Float(-1.0)),
            h.hash_one(Value::Float(1.0)),
            "-1.0 and 1.0"
        );
    }

    // ── ValueSet (#316) ──────────────────────────────────────────────────────

    fn set(values: Vec<Value>) -> ValueSet {
        ValueSet::new(values).unwrap()
    }

    #[test]
    fn value_set_removes_duplicates() {
        let s = set(vec![
            Value::Integer(1),
            Value::Integer(1),
            Value::Keyword(":a".into()),
            Value::Keyword(":a".into()),
        ]);
        assert_eq!(s.len(), 2, "two distinct elements");
        assert!(s.contains(&Value::Integer(1)), "holds 1");
        assert!(!s.contains(&Value::Float(1.0)), "1.0 is not 1");
    }

    #[test]
    fn value_set_keeps_negative_zero_whatever_the_order() {
        for values in [
            vec![Value::Float(0.0), Value::Float(-0.0)],
            vec![Value::Float(-0.0), Value::Float(0.0)],
        ] {
            let s = set(values);
            assert_eq!(s.len(), 1, "one zero");
            let Some(Value::Float(z)) = s.iter().next() else {
                panic!("a float element");
            };
            assert!(z.is_sign_negative(), "-0.0 is kept");
        }
    }

    #[test]
    fn value_set_holds_one_nan() {
        let s = set(vec![
            Value::Float(f64::NAN),
            Value::Float(-f64::NAN),
            Value::Float(f64::NAN),
        ]);
        assert_eq!(s.len(), 1, "one NaN");
        assert!(s.contains(&Value::Float(f64::NAN)), "NaN is a member");
        assert_eq!(s, s.clone(), "a set holding NaN equals itself");
    }

    #[test]
    fn value_set_rejects_a_set_element() {
        let inner = Value::Set(ValueSet::default());
        let err = ValueSet::new(vec![Value::Integer(1), inner]).unwrap_err();
        assert_eq!(err.code(), "API-021");
    }

    #[test]
    fn value_set_keyword_never_equals_ref() {
        let u = Uuid::new_v5(&Uuid::NAMESPACE_OID, b":a");
        let s = set(vec![Value::Keyword(":a".into())]);
        assert!(!s.contains(&Value::Ref(u)), "a ref is not the keyword");
    }

    #[test]
    fn value_set_iterates_in_value_order() {
        let s = set(vec![
            Value::Null,
            Value::Integer(2),
            Value::String("b".into()),
            Value::Integer(1),
        ]);
        let got: Vec<Value> = s.iter().cloned().collect();
        let mut want = got.clone();
        want.sort();
        assert!(got == want, "canonical order");
        assert!(
            matches!(got.first(), Some(Value::String(_))),
            "strings first"
        );
    }

    #[test]
    fn sets_order_after_every_scalar_and_by_elements() {
        let empty = Value::Set(ValueSet::default());
        for v in [
            Value::String("z".into()),
            Value::Integer(i64::MAX),
            Value::Null,
            Value::Keyword(":z".into()),
        ] {
            assert_eq!(v.cmp(&empty), std::cmp::Ordering::Less, "scalar before set");
        }
        let a = Value::Set(set(vec![Value::Integer(1)]));
        let ab = Value::Set(set(vec![Value::Integer(1), Value::Integer(2)]));
        let b = Value::Set(set(vec![Value::Integer(2)]));
        assert!(empty < a && a < ab && ab < b, "lexicographic, prefix first");
    }

    #[test]
    fn value_set_serde_round_trip_and_validation() {
        let s = Value::Set(set(vec![
            Value::Integer(3),
            Value::String("x".into()),
            Value::Keyword(":k".into()),
        ]));
        let bytes = postcard::to_allocvec(&s).unwrap();
        let back: Value = postcard::from_bytes(&bytes).unwrap();
        assert!(back == s, "round trip");

        // Out of order and with a duplicate: restored by `ValueSet::new`.
        let raw = vec![Value::Integer(3), Value::Integer(1), Value::Integer(3)];
        let mut bytes = vec![7u8]; // variant index of `Value::Set`
        bytes.extend(postcard::to_allocvec(&raw).unwrap());
        let back: Value = postcard::from_bytes(&bytes).unwrap();
        assert!(
            back == Value::Set(set(vec![Value::Integer(1), Value::Integer(3)])),
            "canonical after decode"
        );

        // A nested set is refused.
        let nested = vec![Value::Set(ValueSet::default())];
        let mut bytes = vec![7u8];
        bytes.extend(postcard::to_allocvec(&nested).unwrap());
        assert!(postcard::from_bytes::<Value>(&bytes).is_err(), "nested set");
    }

    /// The serde variant indices of the variants before `Set` are unchanged,
    /// so WAL entries written before #316 decode as before.
    #[test]
    fn value_serde_encoding_of_old_variants_is_unchanged() {
        let cases: [(Value, &[u8]); 4] = [
            (Value::String("a".into()), &[0, 1, b'a']),
            (Value::Integer(1), &[1, 2]),
            (Value::Keyword(":a".into()), &[5, 2, b':', b'a']),
            (Value::Null, &[6]),
        ];
        for (v, want) in cases {
            assert_eq!(
                postcard::to_allocvec(&v).unwrap(),
                want.to_vec(),
                "encoding"
            );
        }
    }

    proptest::proptest! {
        #[test]
        fn value_set_ord_hash_and_eq_agree(
            xs in proptest::collection::vec(scalar(), 0..6),
            ys in proptest::collection::vec(scalar(), 0..6),
        ) {
            use std::hash::{BuildHasher, RandomState};
            let h = RandomState::new();
            let a = set(xs.clone());
            let b = set(ys);
            let mut rev = xs;
            rev.reverse();
            let a2 = set(rev);
            proptest::prop_assert!(a == a2, "insertion order does not matter");
            proptest::prop_assert_eq!(h.hash_one(&a), h.hash_one(&a2));
            proptest::prop_assert_eq!(a.cmp(&b), b.cmp(&a).reverse());
            if a == b {
                proptest::prop_assert_eq!(h.hash_one(&a), h.hash_one(&b));
            }
        }
    }

    fn scalar() -> impl proptest::strategy::Strategy<Value = Value> {
        use proptest::prelude::*;
        prop_oneof![
            (-3i64..3).prop_map(Value::Integer),
            prop_oneof![Just(0.0), Just(-0.0), Just(f64::NAN), Just(1.5)].prop_map(Value::Float),
            "[ab]{0,2}".prop_map(Value::String),
            "[ab]{1,2}".prop_map(|s| Value::Keyword(format!(":{s}"))),
            any::<bool>().prop_map(Value::Boolean),
            Just(Value::Null),
        ]
    }
}
