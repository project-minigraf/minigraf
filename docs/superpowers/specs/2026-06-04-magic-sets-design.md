# Magic Sets Rewriting for Demand-Driven Recursive Rule Evaluation

**Issue:** #289  
**Date:** 2026-06-04  
**Status:** Approved

## Problem

The semi-naive bottom-up evaluator in `RecursiveEvaluator` / `StratifiedEvaluator` computes the complete fixed-point closure of all recursive rules before any query-level filtering is applied. For a transitive-closure rule over a large graph (e.g., a 986-commit git history), a point query like `(ancestor "abc123" ?d)` derives ~500K ancestor pairs even though only O(N) derivations along the relevant path are needed.

Magic sets rewriting (Beeri & Ramakrishnan 1991) is a compile-time rule transformation that pushes the *bound* variables of the top-level query back into the recursive rules as "magic predicates," restricting derivation to facts reachable from the actual query bindings.

## Scope

- Applies to **positive-only recursive rules** (the `RecursiveEvaluator` path). Mixed rules containing `not`/`not-join` are left untransformed and continue to use normal semi-naive evaluation. See the roadmap note in §8.
- Handles **mutual recursion** across an SCC — adornment propagates transitively through the dependency graph.
- **Always on** — no query option or configuration knob. When no bound args exist (all-free query), `rewrite()` returns `None` and evaluation proceeds unchanged with zero overhead.
- **Zero user-visible API changes** — transformation is internal to the query engine.
- **No file format changes** — magic predicates exist only in memory during query execution and are never persisted.

## Module Structure

New file: `src/query/datalog/magic_sets.rs`

Public entry point:

```rust
pub(crate) fn rewrite(
    query: &DatalogQuery,
    registry: &RuleRegistry,
) -> Option<(RuleRegistry, Vec<(EntityId, String, Value)>)>
```

Returns `None` when all rule invocations in the query have all-free adornment (no bound args → no benefit). Otherwise returns:
- A rewritten `RuleRegistry` containing adorned rules and magic-propagation rules.
- A `Vec` of seed facts `(entity, attribute, value)` to be loaded into `filtered_storage` before evaluation.

The module is `pub(crate)`. Exposed via `mod magic_sets;` in `src/query/datalog/mod.rs`.

## Magic Predicate Naming

Magic predicate attributes use the format `__magic_<predname>_<adornment>`, e.g., `__magic_ancestor_bf`. The `__` prefix ensures no collision with user attributes (which conventionally start with `:`). When stored as derived facts by the evaluator the attribute becomes `:__magic_ancestor_bf`.

Adornment strings use `b` (bound) and `f` (free) per arg position, left-to-right. Examples: `bf`, `bb`, `fb`, `b` (1-arg).

## Adornment Algorithm

A single left-to-right pass over the query's where clauses tracks a `HashSet<String>` of grounded variables:

1. For each `WhereClause::Pattern`: after processing it, add any variable that appears in the entity or value position where at least one sibling position is already concrete or grounded to the grounded set.
2. For each `WhereClause::RuleInvocation { predicate, args }`: classify each arg:
   - Non-variable `EdnValue` (literal UUID, string, integer, keyword, etc.) → `b`
   - Variable `?x` in the grounded set → `b`
   - Variable `?x` not in the grounded set → `f`
3. If all positions are `f`, skip this invocation (no magic treatment).

For mutual recursion across an SCC: if predicate `p` is adorned and its rules invoke SCC-peer `q`, the adornment propagates to `q` using the bound positions reachable at the point of the `q` invocation in `p`'s rules.

## Rule Transformation

For each positive rule (no `not`/`not-join`) defining a predicate `p` with adornment `ad`:

### 1. Magic guard

Prepend a `WhereClause::RuleInvocation` for `__magic_p_ad` as the first body clause, using the head's bound-position args:

```
// Before:
(ancestor ?a ?c) :- [?a :commit/parent ?b] (ancestor ?b ?c)

// After (adornment bf — arg0 bound):
(ancestor ?a ?c) :- (__magic_ancestor_bf ?a) [?a :commit/parent ?b] (ancestor ?b ?c)
```

### 2. Magic propagation rules

For each recursive `RuleInvocation` in the rule body calling `p` (or an SCC peer), emit a new rule propagating the magic predicate to the recursive call's bound-position arg. The new rule body is: the magic guard + all non-recursive body clauses preceding the recursive call:

```
(__magic_ancestor_bf ?b) :- (__magic_ancestor_bf ?a) [?a :commit/parent ?b]
```

### 3. Seed facts

For each top-level rule invocation in the query with at least one bound arg, emit one seed fact. Encoding depends on which arg is bound:

- **arg0 bound (`bf` or `bb`):** entity = `edn_to_entity_id(bound_arg)` (handles both UUID literals and keyword aliases like `:alice` via `Uuid::new_v5`); attribute = `format!(":__magic_{}_{}", predname, adornment)`; value = `Value::Boolean(true)` sentinel. The 1-arg magic guard pattern `[?a :__magic_p_ad _]` then binds `?a` to the correct entity. This is the dominant use case (point queries by start node).
- **arg1 bound only (`fb`):** entity = `Uuid::new_v4()` (ephemeral carrier, never persisted); attribute = same format; value = `edn_to_value(bound_arg)`. The magic guard becomes a 2-arg match `[?_ :__magic_p_fb ?b]`. Less common; implementer may treat `fb` as unadorned (all-free fallback) in the first iteration if the encoding complexity is not worth it.

Seed facts are loaded into `filtered_storage` before `StratifiedEvaluator` runs, making the demand signal available on iteration 1.

## Integration Point

In `executor.rs`, `execute_query_with_rules`, after `filtered_storage` is populated and before `StratifiedEvaluator` is constructed:

```rust
let rewritten = {
    let reg = self.rules.read().map_err(|_| anyhow!("rule registry lock poisoned"))?;
    magic_sets::rewrite(&query, &reg)
};

let (eval_rules, seed_facts) = match rewritten {
    Some((rewritten_registry, seeds)) => (Arc::new(RwLock::new(rewritten_registry)), seeds),
    None => (self.rules.clone(), vec![]),
};

for (entity, attribute, value) in seed_facts {
    filtered_storage.load_fact(Fact::new(entity, attribute, value, 0))?;
}

let evaluator = StratifiedEvaluator::new(
    filtered_storage,
    eval_rules,
    self.functions.clone(),
    1000,
    effective_max_derived,
    effective_max_results,
);
```

The post-evaluator result-processing code (pattern matching, result projection, ~100 lines) is unchanged — it operates on `derived_storage` from `evaluator.evaluate(&predicates)?` regardless of which path was taken.

To avoid duplicating the post-evaluator code, a small refactor extracts evaluator construction into a local variable before the result-processing section. No logic changes.

## Testing

### Unit tests — `magic_sets.rs` inline `#[cfg(test)]`

Test the transformation in isolation (no evaluator):

- Literal arg → classified `b`; free variable → classified `f`
- Variable grounded by preceding pattern → classified `b`
- All-free adornment → `rewrite()` returns `None`
- Seed facts generated with correct attribute and value for each bound position
- Magic guard rule prepended to rewritten rules
- Magic propagation rules emitted correctly
- SCC peer adornment propagates (mutual recursion case)
- Mixed rules (containing `not`/`not-join`) are left untouched

### Integration tests — `tests/magic_sets_test.rs`

End-to-end via `Minigraf::open` / `db.execute()`, asserting **result correctness** (not internal fact counts):

- Transitive closure with bound start: `(ancestor "abc123" ?d)` returns exactly the correct reachable set
- Transitive closure with all-free: `(ancestor ?a ?b)` returns full closure correctly (magic sets skipped)
- Mutual recursion: `even`/`odd` with bound seed returns only reachable values
- Multi-hop graph: 3+ levels of recursion with bound start produces correct results
- Query result identity: results with magic sets = results without magic sets for same inputs

The correctness invariant — magic sets must never change query results, only reduce intermediate derivations — is the sole test criterion. Internal derived-fact counts are not asserted (too brittle; subject to evaluation order).

## Roadmap Note for Negation

Magic sets rewriting is not applied to mixed rules containing `not`/`not-join`. These continue to use normal semi-naive evaluation. A note will be added to `ROADMAP.md` (following the style of §9.5 branching) marking this as a known limitation: if profiling shows that negation-heavy recursive rules are a bottleneck in practice, the adornment algorithm can be extended per Beeri & Ramakrishnan §5 (magic sets with stratified negation). No issue is created now — this is exploratory.

## Files Changed

| File | Change |
|---|---|
| `src/query/datalog/magic_sets.rs` | New module — adornment, transformation, `rewrite()` |
| `src/query/datalog/mod.rs` | Add `pub(crate) mod magic_sets;` |
| `src/query/datalog/executor.rs` | Wire `magic_sets::rewrite()` into `execute_query_with_rules` |
| `tests/magic_sets_test.rs` | New integration test file |
| `ROADMAP.md` | Add negation limitation note |

## References

- Beeri, C. & Ramakrishnan, R. (1991). *On the power of magic.* Journal of Logic Programming.
- `src/query/datalog/evaluator.rs` — `RecursiveEvaluator`, `StratifiedEvaluator`
- `src/query/datalog/executor.rs` — `execute_query_with_rules` (integration point)
- `src/query/datalog/optimizer.rs` — precedent for a pure transformation module
- `src/query/datalog/stratification.rs` — `DependencyGraph` (SCC structure reused for mutual recursion)
- Issue #288 — per-query limits (workaround for the symptom this design addresses at the root)
