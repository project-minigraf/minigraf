use crate::graph::FactStorage;
use crate::graph::types::Value;
use crate::query::datalog::executor::{DatalogExecutor, QueryResult};
use crate::query::datalog::functions::FunctionRegistry;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{
    AsOf, AttributeSpec, DatalogCommand, DatalogQuery, EdnValue, Expr, ValidAt, WhereClause,
};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ─── BindValue ────────────────────────────────────────────────────────────────

/// A concrete value supplied to a named bind slot (`$name`) in a `PreparedQuery`.
#[derive(Debug, Clone)]
pub enum BindValue {
    /// Substituted into an entity position: `[$entity :attr ?v]`.
    /// Not accepted in rule invocation argument positions — use `Val(Value::Ref(...))` there.
    Entity(Uuid),
    /// Substituted into a value position `[?e :attr $val]` or an expression literal.
    Val(Value),
    /// Substituted into an `:as-of $tx` slot (monotonic transaction counter).
    TxCount(u64),
    /// Substituted into an `:as-of $tx` slot (wall-clock millis) or `:valid-at $date` slot.
    Timestamp(i64),
    /// Substituted into a `:valid-at $date` slot — disables valid-time filtering.
    AnyValidTime,
}

fn bind_value_type_name(bv: &BindValue) -> &'static str {
    match bv {
        BindValue::Entity(_) => "Entity",
        BindValue::Val(_) => "Val",
        BindValue::TxCount(_) => "TxCount",
        BindValue::Timestamp(_) => "Timestamp",
        BindValue::AnyValidTime => "AnyValidTime",
    }
}

// ─── PreparedQuery ────────────────────────────────────────────────────────────

/// A parsed query template with named bind slots (`$name`).
///
/// Obtain via [`crate::db::Minigraf::prepare`].
/// Execute many times via [`PreparedQuery::execute`].
pub struct PreparedQuery {
    template: DatalogQuery,
    slot_names: Vec<String>,
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
}

impl std::fmt::Debug for PreparedQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedQuery")
            .field("slot_names", &self.slot_names)
            .finish_non_exhaustive()
    }
}

impl PreparedQuery {
    /// Substitute bind values and execute against the current database state.
    ///
    /// Extra bindings not referenced by the query are silently ignored.
    ///
    /// # Errors
    /// - Missing bind value for a slot present in the query.
    /// - Type mismatch (e.g. `Val` supplied for an `:as-of` slot).
    pub fn execute(&self, bindings: &[(&str, BindValue)]) -> Result<QueryResult> {
        let binding_map: HashMap<&str, &BindValue> =
            bindings.iter().map(|(name, val)| (*name, val)).collect();

        for name in &self.slot_names {
            if !binding_map.contains_key(name.as_str()) {
                anyhow::bail!("missing bind value for slot '${}'", name);
            }
        }

        let filled_query = substitute(&self.template, &binding_map)?;

        let executor = DatalogExecutor::new_with_rules_and_functions(
            self.fact_storage.clone(),
            self.rules.clone(),
            self.functions.clone(),
        );
        executor.execute(DatalogCommand::Query(filled_query))
    }
}

// ─── Internal constructor ─────────────────────────────────────────────────────

pub(crate) fn prepare_query(
    query: DatalogQuery,
    fact_storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    functions: Arc<RwLock<FunctionRegistry>>,
) -> Result<PreparedQuery> {
    validate_no_attribute_slots(&query)?;
    let slot_names = collect_slot_names(&query);
    Ok(PreparedQuery {
        template: query,
        slot_names,
        fact_storage,
        rules,
        functions,
    })
}

// ─── Validation ───────────────────────────────────────────────────────────────

fn validate_no_attribute_slots(query: &DatalogQuery) -> Result<()> {
    validate_clauses_no_attr_slots(&query.where_clauses)
}

fn validate_clauses_no_attr_slots(clauses: &[WhereClause]) -> Result<()> {
    for clause in clauses {
        match clause {
            WhereClause::Pattern(p) => {
                if let AttributeSpec::Real(EdnValue::BindSlot(name)) = &p.attribute {
                    anyhow::bail!(
                        "bind slot '${name}' is not permitted in attribute position; \
                         the query optimizer selects an index based on the attribute at \
                         prepare time and cannot handle a parameterised attribute"
                    );
                }
            }
            WhereClause::Not(inner) => validate_clauses_no_attr_slots(inner)?,
            WhereClause::NotJoin { clauses: inner, .. } => validate_clauses_no_attr_slots(inner)?,
            WhereClause::Or(branches) => {
                for b in branches {
                    validate_clauses_no_attr_slots(b)?;
                }
            }
            WhereClause::OrJoin { branches, .. } => {
                for b in branches {
                    validate_clauses_no_attr_slots(b)?;
                }
            }
            WhereClause::Expr { .. } | WhereClause::RuleInvocation { .. } => {}
        }
    }
    Ok(())
}

// ─── Slot collection ──────────────────────────────────────────────────────────

fn collect_slot_names(query: &DatalogQuery) -> Vec<String> {
    let mut names: HashSet<String> = HashSet::new();
    if let Some(AsOf::Slot(name)) = &query.as_of {
        names.insert(name.clone());
    }
    if let Some(ValidAt::Slot(name)) = &query.valid_at {
        names.insert(name.clone());
    }
    collect_slots_from_clauses(&query.where_clauses, &mut names);
    let mut result: Vec<String> = names.into_iter().collect();
    result.sort();
    result
}

fn collect_slots_from_clauses(clauses: &[WhereClause], names: &mut HashSet<String>) {
    for clause in clauses {
        match clause {
            WhereClause::Pattern(p) => {
                if let EdnValue::BindSlot(name) = &p.entity {
                    names.insert(name.clone());
                }
                if let EdnValue::BindSlot(name) = &p.value {
                    names.insert(name.clone());
                }
            }
            WhereClause::Not(inner) => collect_slots_from_clauses(inner, names),
            WhereClause::NotJoin { clauses: inner, .. } => collect_slots_from_clauses(inner, names),
            WhereClause::Or(branches) => {
                for b in branches {
                    collect_slots_from_clauses(b, names);
                }
            }
            WhereClause::OrJoin { branches, .. } => {
                for b in branches {
                    collect_slots_from_clauses(b, names);
                }
            }
            WhereClause::Expr { expr, .. } => collect_slots_from_expr(expr, names),
            WhereClause::RuleInvocation { args, .. } => {
                for arg in args {
                    if let EdnValue::BindSlot(name) = arg {
                        names.insert(name.clone());
                    }
                }
            }
        }
    }
}

fn collect_slots_from_expr(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::Slot(name) => {
            names.insert(name.clone());
        }
        Expr::BinOp(_, l, r) => {
            collect_slots_from_expr(l, names);
            collect_slots_from_expr(r, names);
        }
        Expr::UnaryOp(_, arg) => collect_slots_from_expr(arg, names),
        Expr::Var(_) | Expr::Lit(_) => {}
    }
}

// ─── Substitution ─────────────────────────────────────────────────────────────

fn substitute(
    template: &DatalogQuery,
    bindings: &HashMap<&str, &BindValue>,
) -> Result<DatalogQuery> {
    let mut query = template.clone();

    if let Some(AsOf::Slot(name)) = query.as_of.take() {
        query.as_of = Some(resolve_as_of_slot(&name, bindings)?);
    }

    if let Some(ValidAt::Slot(name)) = query.valid_at.take() {
        query.valid_at = Some(resolve_valid_at_slot(&name, bindings)?);
    }

    for clause in &mut query.where_clauses {
        substitute_where_clause(clause, bindings)?;
    }

    Ok(query)
}

fn substitute_where_clause(
    clause: &mut WhereClause,
    bindings: &HashMap<&str, &BindValue>,
) -> Result<()> {
    match clause {
        WhereClause::Pattern(p) => substitute_pattern(p, bindings),
        WhereClause::Not(clauses) => {
            for c in clauses {
                substitute_where_clause(c, bindings)?;
            }
            Ok(())
        }
        WhereClause::NotJoin { clauses, .. } => {
            for c in clauses {
                substitute_where_clause(c, bindings)?;
            }
            Ok(())
        }
        WhereClause::Or(branches) => {
            for branch in branches {
                for c in branch {
                    substitute_where_clause(c, bindings)?;
                }
            }
            Ok(())
        }
        WhereClause::OrJoin { branches, .. } => {
            for branch in branches {
                for c in branch {
                    substitute_where_clause(c, bindings)?;
                }
            }
            Ok(())
        }
        WhereClause::Expr { expr, .. } => substitute_expr(expr, bindings),
        WhereClause::RuleInvocation { args, .. } => {
            for arg in args {
                substitute_edn_value(arg, bindings)?;
            }
            Ok(())
        }
    }
}

fn substitute_pattern(
    p: &mut crate::query::datalog::types::Pattern,
    bindings: &HashMap<&str, &BindValue>,
) -> Result<()> {
    if let EdnValue::BindSlot(name) = &p.entity {
        let name = name.clone();
        p.entity = resolve_entity_slot(&name, bindings)?;
    }
    if let EdnValue::BindSlot(name) = &p.value {
        let name = name.clone();
        p.value = resolve_value_slot(&name, bindings)?;
    }
    Ok(())
}

fn substitute_expr(expr: &mut Expr, bindings: &HashMap<&str, &BindValue>) -> Result<()> {
    match expr {
        Expr::Slot(name) => {
            let name = name.clone();
            *expr = Expr::Lit(resolve_val_slot(&name, bindings)?);
            Ok(())
        }
        Expr::BinOp(_, lhs, rhs) => {
            substitute_expr(lhs, bindings)?;
            substitute_expr(rhs, bindings)
        }
        Expr::UnaryOp(_, arg) => substitute_expr(arg, bindings),
        Expr::Var(_) | Expr::Lit(_) => Ok(()),
    }
}

fn substitute_edn_value(val: &mut EdnValue, bindings: &HashMap<&str, &BindValue>) -> Result<()> {
    if let EdnValue::BindSlot(name) = val {
        let name = name.clone();
        *val = resolve_value_slot(&name, bindings)?;
    }
    Ok(())
}

// ─── Slot resolvers ───────────────────────────────────────────────────────────

fn resolve_entity_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<EdnValue> {
    match bindings.get(name) {
        Some(BindValue::Entity(u)) => Ok(EdnValue::Uuid(*u)),
        Some(other) => anyhow::bail!(
            "slot '${name}' in entity position requires Entity, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_value_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<EdnValue> {
    match bindings.get(name) {
        Some(BindValue::Val(v)) => Ok(value_to_edn(v)),
        Some(other) => anyhow::bail!(
            "slot '${name}' in value position requires Val, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_val_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<Value> {
    match bindings.get(name) {
        Some(BindValue::Val(v)) => Ok(v.clone()),
        Some(other) => anyhow::bail!(
            "slot '${name}' in expression position requires Val, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_as_of_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<AsOf> {
    match bindings.get(name) {
        Some(BindValue::TxCount(n)) => Ok(AsOf::Counter(*n)),
        Some(BindValue::Timestamp(t)) => Ok(AsOf::Timestamp(*t)),
        Some(other) => anyhow::bail!(
            "slot '${name}' in :as-of position requires TxCount or Timestamp, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn resolve_valid_at_slot(name: &str, bindings: &HashMap<&str, &BindValue>) -> Result<ValidAt> {
    match bindings.get(name) {
        Some(BindValue::Timestamp(t)) => Ok(ValidAt::Timestamp(*t)),
        Some(BindValue::AnyValidTime) => Ok(ValidAt::AnyValidTime),
        Some(other) => anyhow::bail!(
            "slot '${name}' in :valid-at position requires Timestamp or AnyValidTime, got {}",
            bind_value_type_name(other)
        ),
        None => anyhow::bail!("missing bind value for slot '${name}'"),
    }
}

fn value_to_edn(v: &Value) -> EdnValue {
    match v {
        Value::String(s) => EdnValue::String(s.clone()),
        Value::Integer(i) => EdnValue::Integer(*i),
        Value::Float(f) => EdnValue::Float(*f),
        Value::Boolean(b) => EdnValue::Boolean(*b),
        Value::Keyword(k) => EdnValue::Keyword(k.clone()),
        Value::Ref(u) => EdnValue::Uuid(*u),
        Value::Null => EdnValue::Nil,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::datalog::types::{BinOp, FindSpec, Pattern};

    fn make_query_entity_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?name".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::BindSlot("entity".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::Symbol("?name".to_string()),
            ))],
        )
    }

    fn make_query_value_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?e".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::BindSlot("name".to_string()),
            ))],
        )
    }

    fn make_query_attr_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::BindSlot("attr".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        )
    }

    fn make_query_as_of_slot() -> DatalogQuery {
        let mut q = DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":score".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        );
        q.as_of = Some(AsOf::Slot("tx".to_string()));
        q
    }

    fn make_query_valid_at_slot() -> DatalogQuery {
        let mut q = DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":score".to_string()),
                EdnValue::Symbol("?v".to_string()),
            ))],
        );
        q.valid_at = Some(ValidAt::Slot("date".to_string()));
        q
    }

    fn make_query_expr_slot() -> DatalogQuery {
        DatalogQuery::new(
            vec![FindSpec::Variable("?v".to_string())],
            vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Keyword(":score".to_string()),
                    EdnValue::Symbol("?v".to_string()),
                )),
                WhereClause::Expr {
                    expr: Expr::BinOp(
                        BinOp::Gte,
                        Box::new(Expr::Var("?v".to_string())),
                        Box::new(Expr::Slot("threshold".to_string())),
                    ),
                    binding: None,
                },
            ],
        )
    }

    #[test]
    fn test_validate_rejects_attribute_slot() {
        let q = make_query_attr_slot();
        let result = validate_no_attribute_slots(&q);
        assert!(result.is_err(), "expected error for attribute slot");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("attribute position")
        );
    }

    #[test]
    fn test_validate_accepts_entity_slot() {
        let q = make_query_entity_slot();
        assert!(validate_no_attribute_slots(&q).is_ok());
    }

    #[test]
    fn test_validate_accepts_value_slot() {
        let q = make_query_value_slot();
        assert!(validate_no_attribute_slots(&q).is_ok());
    }

    #[test]
    fn test_collect_entity_slot_name() {
        let q = make_query_entity_slot();
        let names = collect_slot_names(&q);
        assert_eq!(names, vec!["entity"]);
    }

    #[test]
    fn test_collect_value_slot_name() {
        let q = make_query_value_slot();
        let names = collect_slot_names(&q);
        assert_eq!(names, vec!["name"]);
    }

    #[test]
    fn test_collect_as_of_slot_name() {
        let q = make_query_as_of_slot();
        let names = collect_slot_names(&q);
        assert!(names.contains(&"tx".to_string()));
    }

    #[test]
    fn test_collect_valid_at_slot_name() {
        let q = make_query_valid_at_slot();
        let names = collect_slot_names(&q);
        assert!(names.contains(&"date".to_string()));
    }

    #[test]
    fn test_collect_expr_slot_name() {
        let q = make_query_expr_slot();
        let names = collect_slot_names(&q);
        assert!(names.contains(&"threshold".to_string()));
    }

    #[test]
    fn test_collect_deduplicates() {
        let q = DatalogQuery::new(
            vec![
                FindSpec::Variable("?name".to_string()),
                FindSpec::Variable("?age".to_string()),
            ],
            vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::BindSlot("entity".to_string()),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::Symbol("?name".to_string()),
                )),
                WhereClause::Pattern(Pattern::new(
                    EdnValue::BindSlot("entity".to_string()),
                    EdnValue::Keyword(":person/age".to_string()),
                    EdnValue::Symbol("?age".to_string()),
                )),
            ],
        );
        let names = collect_slot_names(&q);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "entity");
    }

    #[test]
    fn test_substitute_entity_slot() {
        let q = make_query_entity_slot();
        let uuid = Uuid::new_v4();
        let bv = BindValue::Entity(uuid);
        let bindings: HashMap<&str, &BindValue> = [("entity", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        match &filled.where_clauses[0] {
            WhereClause::Pattern(p) => {
                assert_eq!(p.entity, EdnValue::Uuid(uuid));
            }
            _ => panic!("expected Pattern"),
        }
    }

    #[test]
    fn test_substitute_value_slot() {
        let q = make_query_value_slot();
        let bv = BindValue::Val(Value::String("Alice".to_string()));
        let bindings: HashMap<&str, &BindValue> = [("name", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        match &filled.where_clauses[0] {
            WhereClause::Pattern(p) => {
                assert_eq!(p.value, EdnValue::String("Alice".to_string()));
            }
            _ => panic!("expected Pattern"),
        }
    }

    #[test]
    fn test_substitute_as_of_counter() {
        let q = make_query_as_of_slot();
        let bv = BindValue::TxCount(42);
        let bindings: HashMap<&str, &BindValue> = [("tx", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(filled.as_of, Some(AsOf::Counter(42))));
    }

    #[test]
    fn test_substitute_as_of_timestamp() {
        let q = make_query_as_of_slot();
        let bv = BindValue::Timestamp(1_685_577_600_000);
        let bindings: HashMap<&str, &BindValue> = [("tx", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(
            filled.as_of,
            Some(AsOf::Timestamp(1_685_577_600_000))
        ));
    }

    #[test]
    fn test_substitute_valid_at_timestamp() {
        let q = make_query_valid_at_slot();
        let bv = BindValue::Timestamp(1_685_577_600_000);
        let bindings: HashMap<&str, &BindValue> = [("date", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(
            filled.valid_at,
            Some(ValidAt::Timestamp(1_685_577_600_000))
        ));
    }

    #[test]
    fn test_substitute_valid_at_any() {
        let q = make_query_valid_at_slot();
        let bv = BindValue::AnyValidTime;
        let bindings: HashMap<&str, &BindValue> = [("date", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        assert!(matches!(filled.valid_at, Some(ValidAt::AnyValidTime)));
    }

    #[test]
    fn test_substitute_expr_slot() {
        let q = make_query_expr_slot();
        let bv = BindValue::Val(Value::Integer(50));
        let bindings: HashMap<&str, &BindValue> = [("threshold", &bv)].into();
        let filled = substitute(&q, &bindings).unwrap();
        match &filled.where_clauses[1] {
            WhereClause::Expr {
                expr: Expr::BinOp(_, _, rhs),
                ..
            } => {
                assert!(matches!(rhs.as_ref(), Expr::Lit(Value::Integer(50))));
            }
            _ => panic!("expected BinOp Expr clause"),
        }
    }

    #[test]
    fn test_substitute_type_mismatch_entity_gets_val() {
        let q = make_query_entity_slot();
        let bv = BindValue::Val(Value::String("Alice".to_string()));
        let bindings: HashMap<&str, &BindValue> = [("entity", &bv)].into();
        let result = substitute(&q, &bindings);
        assert!(result.is_err(), "expected type mismatch error");
        assert!(result.unwrap_err().to_string().contains("entity position"));
    }

    #[test]
    fn test_substitute_type_mismatch_as_of_gets_val() {
        let q = make_query_as_of_slot();
        let bv = BindValue::Val(Value::Integer(42));
        let bindings: HashMap<&str, &BindValue> = [("tx", &bv)].into();
        let result = substitute(&q, &bindings);
        assert!(result.is_err(), "expected type mismatch error");
        assert!(result.unwrap_err().to_string().contains(":as-of position"));
    }

    #[test]
    fn test_substitute_type_mismatch_valid_at_gets_tx_count() {
        let q = make_query_valid_at_slot();
        let bv = BindValue::TxCount(5);
        let bindings: HashMap<&str, &BindValue> = [("date", &bv)].into();
        let result = substitute(&q, &bindings);
        assert!(result.is_err(), "expected type mismatch error");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains(":valid-at position")
        );
    }
}
