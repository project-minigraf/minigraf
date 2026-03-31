# Phase 7.6 Design: Temporal Metadata Bindings + Range Queries

**Date**: 2026-03-31
**Phase**: 7.6
**Status**: Design approved

---

## Goal

Expose `valid_from`, `valid_to`, `tx_count`, `tx_id`, and the query-level `valid_at` point as first-class bindable values in Datalog `:where` clauses, unlocking the full four-class taxonomy of temporal queries.

---

## Background — The Four Temporal Query Classes

| Class | Description | Minigraf before 7.6 |
|---|---|---|
| **Point-in-Time** | Snapshot of state at a specific moment | ✅ `:as-of` / `:valid-at` |
| **Time Interval** | Facts alive at any point during [T1, T2] | ⚠️ `:any-valid-time` only (no range predicate) |
| **Time-Point Lookup** | Given objects + criteria, find *when* those states existed | ❌ temporal metadata not queryable |
| **Time-Interval Lookup** | Find interval(s) where object states matched criteria | ❌ temporal metadata not queryable |

Root gap: `valid_from`, `valid_to`, and `tx_count` are stored per-fact but invisible to the Datalog engine.

---

## Pseudo-Attributes

Five built-in read-only pseudo-attributes, recognised in the attribute position of `:where` patterns:

| Keyword | Binds | Source |
|---|---|---|
| `:db/valid-from` | `Value::Integer(fact.valid_from)` | Per-fact (ms since epoch) |
| `:db/valid-to` | `Value::Integer(fact.valid_to)` | Per-fact (ms since epoch; `i64::MAX` = forever) |
| `:db/tx-count` | `Value::Integer(fact.tx_count as i64)` | Per-fact (monotonic counter) |
| `:db/tx-id` | `Value::Integer(fact.tx_id as i64)` | Per-fact (ms since epoch) |
| `:db/valid-at` | `Value::Integer(t)` / `Value::Null` | Query-level constant (see below) |

**`:db/valid-at` binding semantics**:
- `:valid-at <timestamp>` → `Value::Integer(t)`
- No `:valid-at` (default = now) → `Value::Integer(now)`
- `:any-valid-time` → `Value::Null`

---

## Syntax Examples

```datalog
;; Time Interval — facts alive at any point during [T1, T2]
(query [:find ?e ?name
        :any-valid-time
        :where [?e :person/name ?name]
               [?e :db/valid-from ?vf]
               [?e :db/valid-to ?vt]
               [(<= ?vf 1704067200000)]
               [(>= ?vt 1696118400000)]])

;; Time-Point Lookup — find all moments when Alice's salary exceeded 100k
(query [:find ?vf
        :any-valid-time
        :where [:alice :person/salary ?s]
               [:alice :db/valid-from ?vf]
               [(> ?s 100000)]])

;; Time-Interval Lookup — find intervals when Alice was employed
(query [:find ?vf ?vt
        :any-valid-time
        :where [:alice :employment/status :employed]
               [:alice :db/valid-from ?vf]
               [:alice :db/valid-to ?vt]])

;; Same-tx join — two entities written in the same transaction
(query [:find ?e1 ?e2
        :any-valid-time
        :where [?e1 :db/tx-id ?tx]
               [?e2 :db/tx-id ?tx]
               [(not= ?e1 ?e2)]])

;; Query-level valid-at binding
(query [:find ?e ?vat
        :valid-at 1704067200000
        :where [?e :person/name _]
               [?e :db/valid-at ?vat]])
```

---

## Architecture

### Approach: Type-safe `AttributeSpec` in the AST (Approach A)

Pseudo-attributes are a compile-time-distinct concept in the AST. `Pattern.attribute` changes from `EdnValue` to `AttributeSpec`, giving exhaustive match coverage everywhere the attribute position is handled.

---

## Component Design

### 1. `src/query/datalog/types.rs`

**New types**:

```rust
/// The five built-in pseudo-attributes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PseudoAttr {
    ValidFrom,  // :db/valid-from
    ValidTo,    // :db/valid-to
    TxCount,    // :db/tx-count
    TxId,       // :db/tx-id
    ValidAt,    // :db/valid-at
}

impl PseudoAttr {
    /// Returns `Some(variant)` if `k` is a reserved `:db/*` keyword.
    pub fn from_keyword(k: &str) -> Option<Self>;
    /// Returns the canonical keyword string.
    pub fn as_keyword(&self) -> &'static str;
    /// True for the four per-fact pseudo-attributes (all except ValidAt).
    pub fn is_per_fact(&self) -> bool;
}

/// Attribute position in a Pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeSpec {
    Real(EdnValue),
    Pseudo(PseudoAttr),
}
```

**`Pattern.attribute`** changes from `EdnValue` to `AttributeSpec`.

Constructors updated:
- `Pattern::new` accepts `attribute: AttributeSpec`
- `Pattern::with_valid_time` accepts `attribute: AttributeSpec`
- `Pattern::from_edn` detects `:db/*` keywords and constructs `AttributeSpec::Pseudo`
- A convenience `Pattern::real(entity, attribute: EdnValue, value)` constructor wraps `AttributeSpec::Real` for the common case. All existing `Pattern::new(e, a, v)` call sites are updated to `Pattern::real(e, a, v)` — a mechanical substitution with no semantic change.

---

### 2. `src/query/datalog/parser.rs`

When parsing the attribute element of a pattern vector:

1. If `EdnValue::Keyword(k)` and `PseudoAttr::from_keyword(k)` returns `Some(p)` → `AttributeSpec::Pseudo(p)`
2. Otherwise → `AttributeSpec::Real(edn_value)`

**Parse-time validation** (hard errors):

- `:db/*` keyword in **entity** position → `"pseudo-attribute :db/... is not valid in entity position"`
- `:db/*` keyword in **value** position → `"pseudo-attribute :db/... is not valid in value position"`

Applies everywhere a `Pattern` can appear: `:where` clauses, rule bodies, `not`, `not-join`, `or`, `or-join`.

**No parse-time restriction** on missing `:any-valid-time` — this is a runtime check (see executor).

---

### 3. `src/query/datalog/matcher.rs`

`match_fact_against_pattern` branches on `pattern.attribute`:

```
AttributeSpec::Real(edn) → existing logic (match stored attribute string)
AttributeSpec::Pseudo(p) → skip attribute match; bind fact metadata to value variable
```

Per-fact pseudo-attribute value resolution:

```
ValidFrom → Value::Integer(fact.valid_from)
ValidTo   → Value::Integer(fact.valid_to)
TxCount   → Value::Integer(fact.tx_count as i64)
TxId      → Value::Integer(fact.tx_id as i64)
ValidAt   → not handled here (query-level; see executor)
```

The entity position participates in matching normally — `[?e :db/valid-from ?vf]` iterates all facts but constrains on entity.

---

### 4. `src/query/datalog/executor.rs`

**`:db/valid-at` injection** (query-level constant, not per-fact):

At the start of `execute_query` and `execute_query_with_rules`, compute:

```rust
let valid_at_value = match &query.valid_at {
    Some(ValidAt::Timestamp(t)) => Value::Integer(*t),
    Some(ValidAt::AnyValidTime) => Value::Null,
    None => Value::Integer(now),
};
```

If any where clause contains a `AttributeSpec::Pseudo(PseudoAttr::ValidAt)` pattern with a variable in value position, inject `valid_at_value` as a pre-seeded binding into every initial binding row before `match_patterns` runs. The existing join machinery then handles it as a bound variable.

**Hard error for missing `:any-valid-time`**:

After `filter_facts_for_query`, if:
- The query contains any pattern with a per-fact pseudo-attribute (`ValidFrom`, `ValidTo`, `TxCount`, `TxId`), **and**
- `query.valid_at != Some(ValidAt::AnyValidTime)`

Return `Err("temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and :db/tx-id require :any-valid-time; add :any-valid-time to your query")`.

`:db/valid-at` is **exempt** from this restriction.

---

### 5. `src/query/datalog/optimizer.rs`

When scoring pattern selectivity, `AttributeSpec::Pseudo(_)` patterns receive the lowest selectivity score (equivalent to `IndexHint::FullScan`) and are never reordered ahead of real-attribute patterns.

One additional match arm in the selectivity scoring function — no other changes.

---

## Error Handling

| Scenario | Error type | Message |
|---|---|---|
| `:db/*` in entity position | Parse error | `"pseudo-attribute :db/... is not valid in entity position"` |
| `:db/*` in value position | Parse error | `"pseudo-attribute :db/... is not valid in value position"` |
| Per-fact pseudo-attr without `:any-valid-time` | Runtime `Err` | `"temporal pseudo-attributes ... require :any-valid-time"` |
| Unknown `:db/*` keyword | Parse error (unrecognised keyword, treated as real attr — no special error needed) | — |

---

## Testing

### Integration tests (`tests/temporal_metadata_test.rs`)

**Temporal query classes**:
- Time Interval: facts alive at any point during [T1, T2] (`valid_from <= T2 AND valid_to >= T1`)
- Time Interval (strict): facts alive for the entire interval (`valid_from <= T1 AND valid_to >= T2`)
- Time-Point Lookup: find historic moments when entity-attribute value matched a threshold
- Time-Interval Lookup: enumerate all validity intervals for an entity-attribute pair

**Tx-time correlation**:
- Bind `:db/tx-count` and join with `:as-of` counter
- Bind `:db/tx-id` across two entities written in the same transaction (same-tx join)

**`:db/valid-at`**:
- Bind with explicit `:valid-at <timestamp>` → `Value::Integer(t)`
- Bind with default (no `:valid-at`) → `Value::Integer(now)` (approximate check: value > 0)
- Bind with `:any-valid-time` → `Value::Null`

### Parse-error tests (unit, in `parser.rs`)

- `:db/valid-from` in entity position → error
- `:db/valid-from` in value position → error
- (representative; covers all five pseudo-attributes by the same code path)

### Runtime hard-error tests (integration)

- `:db/valid-from` without `:any-valid-time` → `Err`
- `:db/valid-to` without `:any-valid-time` → `Err`
- `:db/tx-count` without `:any-valid-time` → `Err`
- `:db/tx-id` without `:any-valid-time` → `Err`
- `:db/valid-at` without `:any-valid-time` → succeeds (no restriction)

---

## Files Changed

| File | Change |
|---|---|
| `src/query/datalog/types.rs` | Add `PseudoAttr`, `AttributeSpec`; change `Pattern.attribute`; add `Pattern::real` convenience constructor |
| `src/query/datalog/parser.rs` | Detect pseudo-attributes at parse time; validate entity/value positions |
| `src/query/datalog/matcher.rs` | Branch on `AttributeSpec` in `match_fact_against_pattern` |
| `src/query/datalog/executor.rs` | Inject `:db/valid-at` constant; hard-error check for missing `:any-valid-time` |
| `src/query/datalog/optimizer.rs` | `AttributeSpec::Pseudo` → `IndexHint::FullScan` in selectivity scoring |
| `tests/temporal_metadata_test.rs` | New integration test file |

---

## What Does Not Change

- `WhereClause` variants — no new variant needed
- `DatalogQuery` fields — `valid_at` and `as_of` unchanged
- The `:any-valid-time` / `:valid-at` / `:as-of` parser syntax — unchanged
- All existing tests — `Pattern::real` convenience constructor keeps existing callers unaffected
- File format — pseudo-attributes are query-only, never stored
