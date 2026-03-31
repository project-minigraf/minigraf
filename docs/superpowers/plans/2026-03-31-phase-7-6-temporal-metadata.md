# Phase 7.6 Temporal Metadata Bindings Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose `:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`, and `:db/valid-at` as first-class bindable values in Datalog `:where` patterns, unlocking the full four-class taxonomy of temporal queries.

**Architecture:** A new `PseudoAttr` enum and `AttributeSpec` wrapper type replace `EdnValue` in `Pattern.attribute`. The parser detects `:db/*` keywords and constructs `AttributeSpec::Pseudo`; the matcher binds the corresponding fact-metadata field instead of matching a stored attribute name. `:db/valid-at` is query-level: the executor computes it once and passes it into the matcher via a new `from_slice_with_valid_at` constructor.

**Tech Stack:** Rust, existing Minigraf codebase (no new dependencies).

---

## File Map

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `PseudoAttr`, `AttributeSpec`; change `Pattern.attribute` from `EdnValue` to `AttributeSpec` |
| `src/query/datalog/optimizer.rs` | Handle `AttributeSpec` in `selectivity_score` and `select_index` |
| `src/query/datalog/evaluator.rs` | Handle `AttributeSpec` in `substitute_pattern` |
| `src/query/datalog/parser.rs` | Add `parse_query_pattern`; replace 5 `Pattern::from_edn` call sites; update `outer_vars_from_clause` |
| `src/query/datalog/matcher.rs` | Add `valid_at_value` field; add `from_slice_with_valid_at`; branch on `AttributeSpec` in `match_fact_against_pattern` and `apply_bindings_to_pattern` |
| `src/query/datalog/executor.rs` | Add `query_uses_per_fact_pseudo_attr`; add hard-error check; compute `valid_at_value`; pass to matcher; update transact/retract attribute matches |
| `src/db.rs` | Update attribute matches in `materialize_transact` and `materialize_retraction` |
| `tests/temporal_metadata_test.rs` | New: all integration tests |

---

### Task 1: Add `PseudoAttr` and `AttributeSpec` to `types.rs`; migrate all call sites

**Files:**
- Modify: `src/query/datalog/types.rs`
- Modify: `src/query/datalog/optimizer.rs`
- Modify: `src/query/datalog/evaluator.rs`
- Modify: `src/query/datalog/matcher.rs`
- Modify: `src/query/datalog/executor.rs`
- Modify: `src/db.rs`
- Modify: `src/query/datalog/parser.rs` (one line)

- [ ] **Step 1: Write failing unit tests for `PseudoAttr`**

Add inside the `#[cfg(test)]` block at the bottom of `src/query/datalog/types.rs`:

```rust
#[test]
fn test_pseudo_attr_from_keyword_known() {
    assert!(matches!(PseudoAttr::from_keyword(":db/valid-from"), Some(PseudoAttr::ValidFrom)));
    assert!(matches!(PseudoAttr::from_keyword(":db/valid-to"), Some(PseudoAttr::ValidTo)));
    assert!(matches!(PseudoAttr::from_keyword(":db/tx-count"), Some(PseudoAttr::TxCount)));
    assert!(matches!(PseudoAttr::from_keyword(":db/tx-id"), Some(PseudoAttr::TxId)));
    assert!(matches!(PseudoAttr::from_keyword(":db/valid-at"), Some(PseudoAttr::ValidAt)));
}

#[test]
fn test_pseudo_attr_from_keyword_unknown() {
    assert!(PseudoAttr::from_keyword(":person/name").is_none());
    assert!(PseudoAttr::from_keyword(":db/other").is_none());
    assert!(PseudoAttr::from_keyword("").is_none());
}

#[test]
fn test_pseudo_attr_is_per_fact() {
    assert!(PseudoAttr::ValidFrom.is_per_fact());
    assert!(PseudoAttr::ValidTo.is_per_fact());
    assert!(PseudoAttr::TxCount.is_per_fact());
    assert!(PseudoAttr::TxId.is_per_fact());
    assert!(!PseudoAttr::ValidAt.is_per_fact());
}

#[test]
fn test_attribute_spec_real_variant() {
    let spec = AttributeSpec::Real(EdnValue::Keyword(":person/name".to_string()));
    assert!(matches!(spec, AttributeSpec::Real(_)));
}

#[test]
fn test_attribute_spec_pseudo_variant() {
    let spec = AttributeSpec::Pseudo(PseudoAttr::ValidFrom);
    assert!(matches!(spec, AttributeSpec::Pseudo(PseudoAttr::ValidFrom)));
}

#[test]
fn test_pattern_new_wraps_real() {
    let p = Pattern::new(
        EdnValue::Symbol("?e".to_string()),
        EdnValue::Keyword(":person/name".to_string()),
        EdnValue::Symbol("?n".to_string()),
    );
    assert!(matches!(p.attribute, AttributeSpec::Real(_)));
}

#[test]
fn test_pattern_pseudo_wraps_pseudo() {
    let p = Pattern::pseudo(
        EdnValue::Symbol("?e".to_string()),
        PseudoAttr::ValidFrom,
        EdnValue::Symbol("?vf".to_string()),
    );
    assert!(matches!(p.attribute, AttributeSpec::Pseudo(PseudoAttr::ValidFrom)));
}
```

- [ ] **Step 2: Run tests — expect compile error (types don't exist yet)**

```bash
cargo test -q 2>&1 | head -30
```
Expected: compile error mentioning `PseudoAttr`, `AttributeSpec`.

- [ ] **Step 3: Add `PseudoAttr` and `AttributeSpec` to `types.rs`**

Insert before the `Pattern` struct definition in `src/query/datalog/types.rs`:

```rust
/// Built-in pseudo-attributes — reserved `:db/*` keywords that bind fact metadata
/// rather than stored attribute values. Never stored in the fact database.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PseudoAttr {
    ValidFrom, // :db/valid-from → Value::Integer(fact.valid_from)
    ValidTo,   // :db/valid-to   → Value::Integer(fact.valid_to)
    TxCount,   // :db/tx-count   → Value::Integer(fact.tx_count as i64)
    TxId,      // :db/tx-id      → Value::Integer(fact.tx_id as i64)
    ValidAt,   // :db/valid-at   → query-level constant (Value::Integer or Value::Null)
}

impl PseudoAttr {
    /// Returns `Some(variant)` if `k` is a reserved `:db/*` pseudo-attribute keyword.
    pub fn from_keyword(k: &str) -> Option<Self> {
        match k {
            ":db/valid-from" => Some(PseudoAttr::ValidFrom),
            ":db/valid-to"   => Some(PseudoAttr::ValidTo),
            ":db/tx-count"   => Some(PseudoAttr::TxCount),
            ":db/tx-id"      => Some(PseudoAttr::TxId),
            ":db/valid-at"   => Some(PseudoAttr::ValidAt),
            _                => None,
        }
    }

    /// Returns the canonical keyword string for this pseudo-attribute.
    pub fn as_keyword(&self) -> &'static str {
        match self {
            PseudoAttr::ValidFrom => ":db/valid-from",
            PseudoAttr::ValidTo   => ":db/valid-to",
            PseudoAttr::TxCount   => ":db/tx-count",
            PseudoAttr::TxId      => ":db/tx-id",
            PseudoAttr::ValidAt   => ":db/valid-at",
        }
    }

    /// True for the four per-fact pseudo-attributes (all except `ValidAt`).
    /// Per-fact pseudo-attrs require `:any-valid-time` in the query.
    pub fn is_per_fact(&self) -> bool {
        !matches!(self, PseudoAttr::ValidAt)
    }
}

/// Attribute position in a `Pattern` — either a real stored attribute keyword
/// or a built-in pseudo-attribute that binds fact metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeSpec {
    Real(EdnValue),
    Pseudo(PseudoAttr),
}
```

- [ ] **Step 4: Change `Pattern.attribute` from `EdnValue` to `AttributeSpec`, update constructors**

Replace the `Pattern` struct and its `impl` block in `types.rs`. Change only the `attribute` field type and the relevant constructors:

```rust
pub struct Pattern {
    pub entity: EdnValue,
    pub attribute: AttributeSpec,  // was: EdnValue
    pub value: EdnValue,
    pub valid_from: Option<i64>,
    pub valid_to: Option<i64>,
}

impl Pattern {
    /// Create a pattern with a real (stored) attribute. The primary constructor.
    pub fn new(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Real(attribute),
            value,
            valid_from: None,
            valid_to: None,
        }
    }

    /// Create a pattern with a pseudo-attribute (`:db/*`).
    pub fn pseudo(entity: EdnValue, pseudo: PseudoAttr, value: EdnValue) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Pseudo(pseudo),
            value,
            valid_from: None,
            valid_to: None,
        }
    }

    pub fn with_valid_time(
        entity: EdnValue,
        attribute: EdnValue,
        value: EdnValue,
        valid_from: Option<i64>,
        valid_to: Option<i64>,
    ) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Real(attribute),
            value,
            valid_from,
            valid_to,
        }
    }

    pub fn from_edn(vector: &[EdnValue]) -> Result<Self, String> {
        if vector.len() != 3 {
            return Err(format!(
                "Pattern must have exactly 3 elements (E A V), got {}",
                vector.len()
            ));
        }
        Ok(Pattern {
            entity: vector[0].clone(),
            attribute: AttributeSpec::Real(vector[1].clone()),
            value: vector[2].clone(),
            valid_from: None,
            valid_to: None,
        })
    }
    // keep remaining methods (from_edn_fact, as_variable, etc.) unchanged
}
```

- [ ] **Step 5: Fix `types.rs` tests that access `pattern.attribute` directly**

In the `#[cfg(test)]` block of `types.rs`, update:

```rust
// Line ~583: was: assert!(pattern.attribute.is_keyword());
assert!(matches!(pattern.attribute, AttributeSpec::Real(EdnValue::Keyword(_))));

// Line ~597: was: assert_eq!(pattern.attribute, EdnValue::Keyword(":person/name".to_string()));
assert_eq!(
    pattern.attribute,
    AttributeSpec::Real(EdnValue::Keyword(":person/name".to_string()))
);
```

- [ ] **Step 6: Fix `optimizer.rs`**

Add import at top: `use crate::query::datalog::types::AttributeSpec;`

Add helper function before `selectivity_score`:

```rust
/// True if the attribute is a real (stored) bound keyword — i.e., can drive index selection.
/// Pseudo-attributes are never variable but also never drive stored indexes.
fn attr_is_index_bound(a: &AttributeSpec) -> bool {
    match a {
        AttributeSpec::Real(edn) => !is_variable(edn),
        AttributeSpec::Pseudo(_) => false,
    }
}
```

Replace `let a = !is_variable(&p.attribute);` in `selectivity_score` with:
```rust
let a = attr_is_index_bound(&p.attribute);
```

Replace `let a_bound = !is_variable(&p.attribute);` in `select_index` with:
```rust
let a_bound = attr_is_index_bound(&p.attribute);
```

In the test `make_pattern` helper, change struct literal to constructor call:
```rust
fn make_pattern(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Pattern {
    Pattern::new(entity, attribute, value)
}
```

- [ ] **Step 7: Fix `evaluator.rs` — `substitute_pattern`**

Add import: `use crate::query::datalog::types::AttributeSpec;`

Replace the body of `substitute_pattern` (around line 358):

```rust
pub fn substitute_pattern(pattern: &Pattern, binding: &Bindings) -> Pattern {
    let attribute = match &pattern.attribute {
        AttributeSpec::Real(edn) => AttributeSpec::Real(substitute_value(edn, binding)),
        AttributeSpec::Pseudo(p) => AttributeSpec::Pseudo(p.clone()),
    };
    Pattern {
        entity: substitute_value(&pattern.entity, binding),
        attribute,
        value: substitute_value(&pattern.value, binding),
        valid_from: pattern.valid_from,
        valid_to: pattern.valid_to,
    }
}
```

- [ ] **Step 8: Fix `matcher.rs` — `apply_bindings_to_pattern`**

Add import: `use crate::query::datalog::types::{AttributeSpec, PseudoAttr};`

Replace `apply_bindings_to_pattern` body:

```rust
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
```

Note: The `match_fact_against_pattern` method still accesses `&pattern.attribute` — it currently passes it to `match_component` which expects `&EdnValue`. This will fail to compile. Leave a `// TODO Task 4` comment there for now and temporarily work around it by adding a stub that compiles:

```rust
fn match_fact_against_pattern(&self, fact: &Fact, pattern: &Pattern) -> Option<Bindings> {
    let mut bindings = HashMap::new();

    if !self.match_component(&pattern.entity, &Value::Ref(fact.entity), &mut bindings) {
        return None;
    }

    // TODO Task 4: handle AttributeSpec::Pseudo
    match &pattern.attribute {
        AttributeSpec::Real(attr_edn) => {
            if !self.match_component(attr_edn, &Value::Keyword(fact.attribute.clone()), &mut bindings) {
                return None;
            }
        }
        AttributeSpec::Pseudo(_) => {
            // placeholder — full binding in Task 4
        }
    }

    if !self.match_component(&pattern.value, &fact.value, &mut bindings) {
        return None;
    }

    Some(bindings)
}
```

- [ ] **Step 9: Fix `executor.rs` — transact/retract attribute matches**

Add import at top of `executor.rs`: `use crate::query::datalog::types::AttributeSpec;`

In `execute_transact` (around line 79), replace:
```rust
let attribute = match &pattern.attribute {
    EdnValue::Keyword(k) => k.clone(),
    _ => return Err(anyhow!("Attribute must be a keyword")),
};
```
with:
```rust
let attribute = match &pattern.attribute {
    AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
    AttributeSpec::Real(_) => return Err(anyhow!("Attribute must be a keyword")),
    AttributeSpec::Pseudo(_) => return Err(anyhow!("Cannot transact a pseudo-attribute")),
};
```

In `execute_retract` (around line 112), replace the identical pattern with the same fix.

- [ ] **Step 10: Fix `db.rs` — attribute matches in `materialize_transact` and `materialize_retraction`**

Add import near top of `db.rs` (or inline with full path):
```rust
use crate::query::datalog::types::AttributeSpec;
```

In `materialize_transact` (around line 574), replace:
```rust
let attr = match &pattern.attribute {
    EdnValue::Keyword(k) => k.clone(),
    _ => anyhow::bail!("attribute must be a keyword"),
};
```
with:
```rust
let attr = match &pattern.attribute {
    AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
    AttributeSpec::Real(_) => anyhow::bail!("attribute must be a keyword"),
    AttributeSpec::Pseudo(_) => anyhow::bail!("cannot transact a pseudo-attribute"),
};
```

Apply the same fix in `materialize_retraction` (around line 610).

- [ ] **Step 11: Fix `parser.rs` — `outer_vars_from_clause`**

Add import: `use crate::query::datalog::types::AttributeSpec;`

In `outer_vars_from_clause`, replace the `WhereClause::Pattern(p)` arm (around line 1109):

```rust
WhereClause::Pattern(p) => {
    let mut vars = Vec::new();
    if let Some(name) = p.entity.as_variable()
        && !name.starts_with("?_")
    {
        vars.push(name.to_string());
    }
    // attribute: only Real attributes can be variables (Pseudo are never variables)
    if let AttributeSpec::Real(attr_edn) = &p.attribute {
        if let Some(name) = attr_edn.as_variable()
            && !name.starts_with("?_")
        {
            vars.push(name.to_string());
        }
    }
    if let Some(name) = p.value.as_variable()
        && !name.starts_with("?_")
    {
        vars.push(name.to_string());
    }
    vars
}
```

Also update the parser test at line ~1475 that checks `patterns[0].attribute`:
```rust
assert_eq!(
    patterns[0].attribute,
    AttributeSpec::Real(EdnValue::Keyword(":person/name".to_string()))
);
```

- [ ] **Step 12: Run `cargo test` — all existing tests must pass**

```bash
cargo test 2>&1 | tail -20
```
Expected: all 617 existing tests pass. Fix any remaining compile errors before continuing.

- [ ] **Step 13: Commit**

```bash
git add src/query/datalog/types.rs src/query/datalog/optimizer.rs \
        src/query/datalog/evaluator.rs src/query/datalog/matcher.rs \
        src/query/datalog/executor.rs src/db.rs src/query/datalog/parser.rs
git commit -m "$(cat <<'EOF'
refactor: add PseudoAttr/AttributeSpec types, migrate Pattern.attribute

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Write failing integration tests

**Files:**
- Create: `tests/temporal_metadata_test.rs`

- [ ] **Step 1: Create the test file with all integration tests**

```rust
//! Phase 7.6 integration tests — temporal metadata pseudo-attribute bindings.

use minigraf::{Minigraf, OpenOptions, QueryResult, Value};

fn db() -> Minigraf {
    OpenOptions::new().open_memory().unwrap()
}

fn results(r: &QueryResult) -> &Vec<Vec<Value>> {
    match r {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected QueryResults, got {:?}", r),
    }
}

// ─── Time Interval Tests ─────────────────────────────────────────────────────

/// Time Interval — find facts alive at any point during interval [T1, T2].
/// Condition: valid_from <= T2 AND valid_to >= T1.
/// "2023-01-01" = 1672531200000 ms, "2024-01-01" = 1704067200000 ms.
#[test]
fn time_interval_any_point_during() {
    let db = db();
    // e1: valid 2022-01-01 to 2023-07-01 → overlaps [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2022-01-01" :valid-to "2023-07-01"} [[:e1 :item/label "A"]])"#).unwrap();
    // e2: valid 2023-07-01 onwards → overlaps [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2023-07-01"} [[:e2 :item/label "B"]])"#).unwrap();
    // e3: valid 2015-01-01 to 2020-01-01 → does NOT overlap [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2015-01-01" :valid-to "2020-01-01"} [[:e3 :item/label "C"]])"#).unwrap();

    // T1 = 2023-01-01 = 1672531200000, T2 = 2024-01-01 = 1704067200000
    let r = db.execute(r#"
        (query [:find ?e
                :any-valid-time
                :where [?e :item/label _]
                       [?e :db/valid-from ?vf]
                       [?e :db/valid-to   ?vt]
                       [(<= ?vf 1704067200000)]
                       [(>= ?vt 1672531200000)]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 2, "e1 and e2 overlap [2023, 2024]; e3 does not");
}

/// Time Interval (strict) — facts alive for the *entire* interval [T1, T2].
/// Condition: valid_from <= T1 AND valid_to >= T2.
#[test]
fn time_interval_entire_interval() {
    let db = db();
    // e1: valid 2020-01-01 to 2025-01-01 → covers entire [2023-01-01, 2024-01-01]
    db.execute(r#"(transact {:valid-from "2020-01-01" :valid-to "2025-01-01"} [[:e1 :item/label "A"]])"#).unwrap();
    // e2: valid 2023-07-01 onwards → does NOT cover T1 = 2023-01-01
    db.execute(r#"(transact {:valid-from "2023-07-01"} [[:e2 :item/label "B"]])"#).unwrap();

    // T1 = 1672531200000, T2 = 1704067200000, T_end_2025 = 1735689600000
    let r = db.execute(r#"
        (query [:find ?e
                :any-valid-time
                :where [?e :item/label _]
                       [?e :db/valid-from ?vf]
                       [?e :db/valid-to   ?vt]
                       [(<= ?vf 1672531200000)]
                       [(>= ?vt 1704067200000)]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1, "only e1 covers the entire interval");
}

// ─── Time-Point Lookup ───────────────────────────────────────────────────────

/// Time-Point Lookup — find all valid_from timestamps when Alice's salary exceeded 50000.
#[test]
fn time_point_lookup_salary_threshold() {
    let db = db();
    // salary 100000, valid 2023-01-01 to 2024-01-01
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2024-01-01"} [[:alice :person/salary 100000]])"#).unwrap();
    // salary 30000, valid 2024-01-01 onwards
    db.execute(r#"(transact {:valid-from "2024-01-01"} [[:alice :person/salary 30000]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vf
                :any-valid-time
                :where [:alice :person/salary ?s]
                       [:alice :db/valid-from ?vf]
                       [(> ?s 50000)]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1, "only the 2023 salary entry exceeds 50000");
    assert_eq!(rows[0][0], Value::Integer(1672531200000), "valid-from = 2023-01-01");
}

// ─── Time-Interval Lookup ────────────────────────────────────────────────────

/// Time-Interval Lookup — enumerate all validity intervals for Alice's employment status.
#[test]
fn time_interval_lookup_employment_status() {
    let db = db();
    db.execute(r#"(transact {:valid-from "2022-01-01" :valid-to "2023-01-01"} [[:alice :employment/status :probation]])"#).unwrap();
    db.execute(r#"(transact {:valid-from "2023-01-01" :valid-to "2025-01-01"} [[:alice :employment/status :permanent]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vf ?vt
                :any-valid-time
                :where [:alice :employment/status _]
                       [:alice :db/valid-from ?vf]
                       [:alice :db/valid-to   ?vt]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 2, "two distinct employment intervals");
}

// ─── Tx-time Correlation ─────────────────────────────────────────────────────

/// Bind :db/tx-count and verify it matches :as-of counter semantics.
#[test]
fn tx_count_binding() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap(); // tx_count = 1
    db.execute(r#"(transact [[:bob :person/name "Bob"]])"#).unwrap();   // tx_count = 2

    // Query with :any-valid-time: bind tx_count for all name facts
    let r = db.execute(r#"
        (query [:find ?e ?tc
                :any-valid-time
                :where [?e :person/name _]
                       [?e :db/tx-count ?tc]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 2);

    // The tx_counts must be 1 and 2 (in any order)
    let mut counts: Vec<i64> = rows.iter()
        .map(|r| match r[1] { Value::Integer(n) => n, _ => panic!("expected Integer") })
        .collect();
    counts.sort();
    assert_eq!(counts, vec![1, 2]);
}

/// Bind :db/tx-id across two entities written in the same transaction — same tx-id.
#[test]
fn tx_id_same_transaction_join() {
    let db = db();
    // Alice and Bob written in the same transaction → same tx_id
    db.execute(r#"(transact [[:alice :person/name "Alice"] [:bob :person/name "Bob"]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?e1 ?e2
                :any-valid-time
                :where [?e1 :person/name _]
                       [?e2 :person/name _]
                       [?e1 :db/tx-id ?tx]
                       [?e2 :db/tx-id ?tx]])
    "#).unwrap();
    let rows = results(&r);
    // Both share the same tx-id: (alice, alice), (alice, bob), (bob, alice), (bob, bob)
    assert_eq!(rows.len(), 4, "cross-join of 2 entities with same tx-id = 4 rows");
}

// ─── :db/valid-at Tests ──────────────────────────────────────────────────────

/// :db/valid-at binds the effective query timestamp when :valid-at is explicit.
#[test]
fn valid_at_explicit_timestamp() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();

    // 2023-01-01 = 1672531200000
    let r = db.execute(r#"
        (query [:find ?vat
                :valid-at "2023-01-01"
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::Integer(1672531200000));
}

/// :db/valid-at binds the current time when no :valid-at is specified.
#[test]
fn valid_at_default_is_now() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vat
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1);
    // The value should be a positive ms timestamp (approximately now)
    match rows[0][0] {
        Value::Integer(n) => assert!(n > 0, "valid-at default should be a positive timestamp"),
        _ => panic!("expected Integer for :db/valid-at default"),
    }
}

/// :db/valid-at binds Value::Null when :any-valid-time is used.
#[test]
fn valid_at_any_valid_time_is_null() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();

    let r = db.execute(r#"
        (query [:find ?vat
                :any-valid-time
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#).unwrap();
    let rows = results(&r);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], Value::Null);
}

// ─── Parse-error Tests ───────────────────────────────────────────────────────

/// :db/* in entity position is a parse error.
#[test]
fn parse_error_pseudo_attr_in_entity_position() {
    let db = db();
    let result = db.execute(r#"
        (query [:find ?v
                :any-valid-time
                :where [:db/valid-from :person/name ?v]])
    "#);
    assert!(result.is_err(), "pseudo-attribute in entity position must be a parse error");
}

/// :db/* in value position is a parse error.
#[test]
fn parse_error_pseudo_attr_in_value_position() {
    let db = db();
    let result = db.execute(r#"
        (query [:find ?e
                :any-valid-time
                :where [?e :person/name :db/valid-from]])
    "#);
    assert!(result.is_err(), "pseudo-attribute in value position must be a parse error");
}

// ─── Runtime Hard-error Tests ────────────────────────────────────────────────

/// :db/valid-from without :any-valid-time is a runtime error.
#[test]
fn runtime_error_valid_from_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?vf
                :where [:alice :person/name _]
                       [:alice :db/valid-from ?vf]])
    "#);
    assert!(result.is_err(), ":db/valid-from requires :any-valid-time");
}

/// :db/valid-to without :any-valid-time is a runtime error.
#[test]
fn runtime_error_valid_to_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?vt
                :where [:alice :person/name _]
                       [:alice :db/valid-to ?vt]])
    "#);
    assert!(result.is_err(), ":db/valid-to requires :any-valid-time");
}

/// :db/tx-count without :any-valid-time is a runtime error.
#[test]
fn runtime_error_tx_count_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?tc
                :where [:alice :person/name _]
                       [:alice :db/tx-count ?tc]])
    "#);
    assert!(result.is_err(), ":db/tx-count requires :any-valid-time");
}

/// :db/tx-id without :any-valid-time is a runtime error.
#[test]
fn runtime_error_tx_id_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?ti
                :where [:alice :person/name _]
                       [:alice :db/tx-id ?ti]])
    "#);
    assert!(result.is_err(), ":db/tx-id requires :any-valid-time");
}

/// :db/valid-at without :any-valid-time succeeds (no restriction on valid-at).
#[test]
fn valid_at_succeeds_without_any_valid_time() {
    let db = db();
    db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
    let result = db.execute(r#"
        (query [:find ?vat
                :where [:alice :person/name _]
                       [:alice :db/valid-at ?vat]])
    "#);
    assert!(result.is_ok(), ":db/valid-at must not require :any-valid-time");
}
```

- [ ] **Step 2: Run tests — expect failures**

```bash
cargo test --test temporal_metadata_test 2>&1 | tail -30
```
Expected: all tests in the file either fail (parse error returned when success expected, or success when error expected) because the parser doesn't recognise `:db/*` yet.

---

### Task 3: Parser — add `parse_query_pattern`, replace call sites

**Files:**
- Modify: `src/query/datalog/parser.rs`

- [ ] **Step 1: Write failing parse-error unit test in `parser.rs`**

Add inside the `#[cfg(test)]` block of `parser.rs`:

```rust
#[test]
fn test_parse_pseudo_attr_in_where_clause() {
    let cmd = parse_datalog_command(
        "(query [:find ?vf :any-valid-time :where [?e :person/name _] [?e :db/valid-from ?vf]])"
    ).unwrap();
    match cmd {
        DatalogCommand::Query(q) => {
            let patterns = q.get_patterns();
            // The second pattern should have a Pseudo attribute
            assert!(matches!(
                patterns.iter().find(|p| matches!(p.attribute, AttributeSpec::Pseudo(_))),
                Some(_)
            ), "expected a Pseudo attribute pattern");
        }
        _ => panic!("expected Query"),
    }
}

#[test]
fn test_parse_error_pseudo_attr_entity_position() {
    let result = parse_datalog_command(
        "(query [:find ?v :any-valid-time :where [:db/valid-from :person/name ?v]])"
    );
    assert!(result.is_err(), "pseudo-attr in entity position should error");
}

#[test]
fn test_parse_error_pseudo_attr_value_position() {
    let result = parse_datalog_command(
        "(query [:find ?e :any-valid-time :where [?e :person/name :db/valid-from]])"
    );
    assert!(result.is_err(), "pseudo-attr in value position should error");
}
```

- [ ] **Step 2: Run failing tests**

```bash
cargo test -p minigraf test_parse_pseudo_attr test_parse_error_pseudo_attr 2>&1 | tail -20
```
Expected: `test_parse_pseudo_attr_in_where_clause` fails (attribute is parsed as Real, not Pseudo). Parse-error tests may pass or fail depending on whether the keyword just passes through.

- [ ] **Step 3: Add `parse_query_pattern` function and update imports**

Add `use crate::query::datalog::types::{AttributeSpec, PseudoAttr};` to the imports in `parser.rs`.

Add the following function (place it near the existing `Pattern::from_edn` call sites, e.g. after `parse_or_branch_item`):

```rust
/// Parse a where-clause pattern vector with pseudo-attribute detection.
///
/// Detects `:db/*` keywords in the attribute position and wraps them in
/// `AttributeSpec::Pseudo`. Rejects `:db/*` keywords in entity or value positions.
/// Falls through to `Pattern::from_edn` for regular patterns.
fn parse_query_pattern(vec: &[EdnValue]) -> Result<Pattern, String> {
    if vec.len() != 3 {
        return Err(format!(
            "Pattern must have exactly 3 elements (E A V), got {}",
            vec.len()
        ));
    }

    // Reject :db/* in entity position
    if let EdnValue::Keyword(k) = &vec[0] {
        if PseudoAttr::from_keyword(k).is_some() {
            return Err(format!(
                "pseudo-attribute {} is not valid in entity position",
                k
            ));
        }
    }

    // Reject :db/* in value position
    if let EdnValue::Keyword(k) = &vec[2] {
        if PseudoAttr::from_keyword(k).is_some() {
            return Err(format!(
                "pseudo-attribute {} is not valid in value position",
                k
            ));
        }
    }

    // Detect pseudo-attribute in attribute position
    if let EdnValue::Keyword(k) = &vec[1] {
        if let Some(pseudo) = PseudoAttr::from_keyword(k) {
            return Ok(Pattern::pseudo(vec[0].clone(), pseudo, vec[2].clone()));
        }
    }

    Pattern::from_edn(vec)
}
```

- [ ] **Step 4: Replace `Pattern::from_edn(vec)?` in query where-clause contexts**

There are **5** call sites. Replace each with `parse_query_pattern(vec)?`:

1. `src/query/datalog/parser.rs` around line 571 (`:where` clause main loop):
   ```rust
   // was: let pattern = Pattern::from_edn(pattern_vec)?;
   let pattern = parse_query_pattern(pattern_vec)?;
   ```

2. Around line 916 (`not` body parsing):
   ```rust
   // was: let pattern = Pattern::from_edn(vec)?;
   let pattern = parse_query_pattern(vec)?;
   ```

3. Around line 977 (`not-join` body parsing):
   ```rust
   // was: let pattern = Pattern::from_edn(vec)?;
   let pattern = parse_query_pattern(vec)?;
   ```

4. Around line 1095 (`parse_or_branch_item`):
   ```rust
   // was: Ok(WhereClause::Pattern(Pattern::from_edn(vec)?))
   Ok(WhereClause::Pattern(parse_query_pattern(vec)?))
   ```

5. Around line 1356 (rule body parsing):
   ```rust
   // was: let pattern = Pattern::from_edn(vec)?;
   let pattern = parse_query_pattern(vec)?;
   ```

Do NOT change line 768 (transact) or line 692 (`Pattern::with_valid_time`) — those remain `Pattern::from_edn` / `Pattern::with_valid_time`.

- [ ] **Step 5: Run parse-related tests**

```bash
cargo test -p minigraf test_parse 2>&1 | tail -20
```
Expected: all existing parse tests pass; the three new parse tests should now pass.

- [ ] **Step 6: Run integration tests — parse-error cases should now pass**

```bash
cargo test --test temporal_metadata_test parse_error 2>&1 | tail -20
```
Expected: `parse_error_pseudo_attr_in_entity_position` and `parse_error_pseudo_attr_in_value_position` pass. Remaining tests still fail (matcher/executor not implemented).

- [ ] **Step 7: Commit**

```bash
git add src/query/datalog/parser.rs
git commit -m "$(cat <<'EOF'
feat: parser — detect pseudo-attributes in query where-clause patterns

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 4: Matcher — bind pseudo-attribute fields from fact metadata

**Files:**
- Modify: `src/query/datalog/matcher.rs`

- [ ] **Step 1: Add `valid_at_value` field to `PatternMatcher` and new constructor**

In `matcher.rs`, update the struct and constructors:

```rust
pub struct PatternMatcher {
    storage: MatcherStorage,
    /// Value to bind for `:db/valid-at` pseudo-attribute.
    /// `Value::Null` = `:any-valid-time`; `Value::Integer(t)` = specific point.
    /// Defaults to `Value::Null` for matchers created outside of a query context.
    valid_at_value: Value,
}

impl PatternMatcher {
    pub fn new(storage: FactStorage) -> Self {
        PatternMatcher {
            storage: MatcherStorage::Owned(storage),
            valid_at_value: Value::Null,
        }
    }

    pub(crate) fn from_slice(facts: Arc<[Fact]>) -> Self {
        PatternMatcher {
            storage: MatcherStorage::Slice(facts),
            valid_at_value: Value::Null,
        }
    }

    /// Constructs a matcher with an explicit `:db/valid-at` binding value.
    /// Used by the executor when the query has a known `valid_at` point.
    pub(crate) fn from_slice_with_valid_at(facts: Arc<[Fact]>, valid_at: Value) -> Self {
        PatternMatcher {
            storage: MatcherStorage::Slice(facts),
            valid_at_value: valid_at,
        }
    }
    // ... rest of impl unchanged
}
```

- [ ] **Step 2: Complete `match_fact_against_pattern` for `AttributeSpec::Pseudo`**

Replace the stub from Task 1 Step 8 with the full implementation:

```rust
fn match_fact_against_pattern(&self, fact: &Fact, pattern: &Pattern) -> Option<Bindings> {
    let mut bindings = HashMap::new();

    // Match entity (unchanged)
    if !self.match_component(&pattern.entity, &Value::Ref(fact.entity), &mut bindings) {
        return None;
    }

    match &pattern.attribute {
        AttributeSpec::Real(attr_edn) => {
            // Real attribute: match stored attribute name, then match value
            if !self.match_component(
                attr_edn,
                &Value::Keyword(fact.attribute.clone()),
                &mut bindings,
            ) {
                return None;
            }
            if !self.match_component(&pattern.value, &fact.value, &mut bindings) {
                return None;
            }
        }
        AttributeSpec::Pseudo(pseudo) => {
            // Pseudo-attribute: skip stored attribute match; bind fact metadata
            // field to the value position variable (or match against a constant).
            let pseudo_value = match pseudo {
                PseudoAttr::ValidFrom => Value::Integer(fact.valid_from),
                PseudoAttr::ValidTo   => Value::Integer(fact.valid_to),
                PseudoAttr::TxCount   => Value::Integer(fact.tx_count as i64),
                PseudoAttr::TxId      => Value::Integer(fact.tx_id as i64),
                PseudoAttr::ValidAt   => self.valid_at_value.clone(),
            };
            if !self.match_component(&pattern.value, &pseudo_value, &mut bindings) {
                return None;
            }
        }
    }

    Some(bindings)
}
```

- [ ] **Step 3: Run integration tests — per-fact pseudo-attr tests should pass**

```bash
cargo test --test temporal_metadata_test 2>&1 | tail -30
```
Expected: `time_interval_*`, `time_point_lookup_*`, `time_interval_lookup_*`, `tx_count_binding`, `tx_id_same_transaction_join` now pass. The `:db/valid-at` tests and runtime hard-error tests still fail.

- [ ] **Step 4: Commit**

```bash
git add src/query/datalog/matcher.rs
git commit -m "$(cat <<'EOF'
feat: matcher — bind pseudo-attribute fields from fact metadata

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 5: Executor — hard-error guard + `:db/valid-at` injection

**Files:**
- Modify: `src/query/datalog/executor.rs`

- [ ] **Step 1: Add `query_uses_per_fact_pseudo_attr` helper**

Add this free function near the top of `executor.rs` (after the imports):

```rust
/// Returns true if any where clause (at any depth) contains a per-fact
/// pseudo-attribute pattern (ValidFrom / ValidTo / TxCount / TxId).
/// Used to enforce the `:any-valid-time` requirement.
fn query_uses_per_fact_pseudo_attr(query: &DatalogQuery) -> bool {
    fn check_clauses(clauses: &[WhereClause]) -> bool {
        clauses.iter().any(|c| match c {
            WhereClause::Pattern(p) => matches!(
                &p.attribute,
                AttributeSpec::Pseudo(pa) if pa.is_per_fact()
            ),
            WhereClause::Not(inner) | WhereClause::NotJoin { clauses: inner, .. } => {
                check_clauses(inner)
            }
            WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
                branches.iter().any(|b| check_clauses(b))
            }
            _ => false,
        })
    }
    check_clauses(&query.where_clauses)
}
```

Make sure `AttributeSpec` and `PseudoAttr` are imported (they should be from Task 1).

- [ ] **Step 2: Add hard-error check and `valid_at_value` computation in `execute_query`**

At the start of the `execute_query` method body, immediately after computing `now`, add:

```rust
// Compute the query-level valid_at value for :db/valid-at pseudo-attribute binding.
let valid_at_value = match &query.valid_at {
    Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
    Some(ValidAt::AnyValidTime) => Value::Null,
    None => Value::Integer(now as i64),
};

// Hard-error: per-fact pseudo-attrs require :any-valid-time to suppress
// the valid-time filter before pattern matching.
if query_uses_per_fact_pseudo_attr(&query)
    && !matches!(query.valid_at, Some(ValidAt::AnyValidTime))
{
    return Err(anyhow!(
        "temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and \
         :db/tx-id require :any-valid-time; add :any-valid-time to your query"
    ));
}
```

- [ ] **Step 3: Pass `valid_at_value` to `PatternMatcher` in `execute_query`**

Replace (around line 185):
```rust
let matcher = PatternMatcher::from_slice(filtered_facts.clone());
```
with:
```rust
let matcher = PatternMatcher::from_slice_with_valid_at(filtered_facts.clone(), valid_at_value.clone());
```

Also update the `not_body_matches` call site (around line 560) and the internal `PatternMatcher` inside `apply_or_clauses` (around line 560 of executor.rs — it calls `PatternMatcher::from_slice(storage.clone())`) to pass `valid_at_value`. Add `valid_at: Value` as a parameter to `apply_or_clauses` and thread it through to the internal matcher.

In `not_body_matches`, add `valid_at_value: Value` as a parameter and pass it to the `PatternMatcher::from_slice_with_valid_at` call. Update all callers of `not_body_matches` to pass the `valid_at_value`.

Actually `not_body_matches` is a free function — simpler to give it a `valid_at: Value` parameter:

```rust
fn not_body_matches(not_body: &[WhereClause], outer: &Binding, storage: Arc<[Fact]>, valid_at: Value) -> bool {
    // ...
    let matcher = crate::query::datalog::matcher::PatternMatcher::from_slice_with_valid_at(storage.clone(), valid_at);
    // ...
}
```

Update the call sites of `not_body_matches` in `execute_query` to pass `valid_at_value.clone()`.

- [ ] **Step 4: Add hard-error check and `valid_at_value` in `execute_query_with_rules`**

Apply the same pattern as Step 2 to `execute_query_with_rules`:

```rust
let valid_at_value = match &query.valid_at {
    Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
    Some(ValidAt::AnyValidTime) => Value::Null,
    None => Value::Integer(now as i64),
};

if query_uses_per_fact_pseudo_attr(&query)
    && !matches!(query.valid_at, Some(ValidAt::AnyValidTime))
{
    return Err(anyhow!(
        "temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and \
         :db/tx-id require :any-valid-time; add :any-valid-time to your query"
    ));
}
```

And update the matcher call (around line 346):
```rust
let matcher = PatternMatcher::from_slice_with_valid_at(derived_facts.clone(), valid_at_value.clone());
```

- [ ] **Step 5: Run all tests**

```bash
cargo test 2>&1 | tail -30
```
Expected: all 617 existing tests plus all new `temporal_metadata_test` tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "$(cat <<'EOF'
feat: executor — hard-error guard for missing :any-valid-time, :db/valid-at injection

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

---

### Task 6: Documentation sync

**Files:**
- Modify: `ROADMAP.md`
- Modify: `CHANGELOG.md`
- Modify: `CLAUDE.md`
- Modify: `TEST_COVERAGE.md`

- [ ] **Step 1: Get final test count**

```bash
cargo test 2>&1 | grep "test result"
```
Record the new total (617 + count of new temporal_metadata_test tests).

- [ ] **Step 2: Update `ROADMAP.md`**

In the Phase 7 sub-phases list, mark 7.6 complete:
```
- **7.6** ✅ Temporal Metadata Bindings + Range Queries
```

Add a `### 7.6 Temporal Metadata Bindings + Range Queries ✅ COMPLETE` section with status, what was built, and the new test count.

- [ ] **Step 3: Update `CHANGELOG.md`**

Add a new version entry (bump minor version — check current version in `Cargo.toml` first: `grep '^version' Cargo.toml`). Document: `PseudoAttr`, `AttributeSpec`, five pseudo-attributes, parse/runtime errors, new integration test file.

- [ ] **Step 4: Update `CLAUDE.md`**

Update the test count line (e.g., `617 tests passing` → new count). Update the Architecture section if any new file was added (no new files in this phase — only `tests/temporal_metadata_test.rs` which is already noted in TEST_COVERAGE.md convention).

- [ ] **Step 5: Update `TEST_COVERAGE.md`**

Add `tests/temporal_metadata_test.rs` to the integration test table with its test count. Update the total.

- [ ] **Step 6: Bump version in `Cargo.toml`**

```bash
grep '^version' Cargo.toml
```
Increment the patch or minor version as appropriate.

- [ ] **Step 7: Commit docs and version bump**

```bash
git add ROADMAP.md CHANGELOG.md CLAUDE.md TEST_COVERAGE.md Cargo.toml Cargo.lock
git commit -m "$(cat <<'EOF'
docs: sync docs for Phase 7.6 completion — temporal metadata bindings

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 8: Tag the release**

```bash
git tag -a v<new-version> -m "Phase 7.6 complete — temporal metadata bindings + range queries"
git push origin v<new-version>
```
