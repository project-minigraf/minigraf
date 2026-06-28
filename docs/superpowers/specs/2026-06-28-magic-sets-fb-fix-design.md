# Magic Sets `fb` Adornment Fix — v1.2.1

**Date:** 2026-06-28
**Issues:** #297, #298
**Release:** v1.2.1 (patch)

## Problem

Two regressions introduced in v1.2.0 (magic sets rewriting, #289):

### #298 — Silent empty results for value-position keyword binding

Query: `(query [:find ?emp :where (reports-to ?emp :alice)])`

Adornment: `['f', 'b']` — arg0 free, arg1 (keyword `:alice`) bound.

Current behaviour: the seed fact is encoded as `(random_uuid, :__magic_reports-to_fb, Keyword(":alice"))` using an ephemeral random UUID as the entity. The magic guard `(__magic_reports-to_fb ?mgr)` is a 1-arg pattern, so `?mgr` binds to the entity position — the random UUID — rather than to `:alice`. The rule body `[?emp :employee/manager ?mgr]` then looks for facts with value `= Ref(random_uuid)`, which never matches stored `Keyword(":alice")` values → empty results.

### #297 — "Unbound variable in rule head: ?mid" for intermediate variables

Query: `(query [:find ?y :where (reach :a ?y)])` with rule containing intermediate variable `?mid`.

Adornment: `['b', 'f']`. The exact failure path requires reproducing via test; believed to be related to keyword coercion in the same seed/guard mechanism. Test 2 (below) will confirm whether the `fb` fix resolves it as a side-effect, or whether a separate fix is needed.

## Root Cause

For `fb` adornments, `build_seed_facts` uses `Uuid::new_v4()` (ephemeral random) as the seed entity and stores the bound keyword in the VALUE position. `inject_magic_guard` emits a 1-arg guard, which matches the seed fact's ENTITY position. The variable therefore binds to the random UUID, not to the keyword value.

## Fix — Approach 2: Sentinel entity + 2-arg guard

### Sentinel entity

A new helper:

```rust
fn sentinel_entity(magic_attr: &str) -> EntityId {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, magic_attr.as_bytes())
}
```

- Deterministic: same magic attribute → same UUID across runs and processes.
- Unique per magic predicate: the attribute string `":__magic_pred_fb"` is distinct per predicate.
- No collision risk with user data: magic attribute names use the `__magic_` prefix by convention.

### Seed fact (fb case)

Old: `(Uuid::new_v4(), magic_attr, edn_to_value(arg1))`

New: `(sentinel_entity(&magic_attr), magic_attr, edn_to_value(arg1))`

The sentinel is now the entity; the keyword value stays in the VALUE position.

### Magic guard (fb case)

For `fb` adornments (bound position > 0), `inject_magic_guard` emits a **2-arg RuleInvocation**:

```
args: [EdnValue::Uuid(sentinel_entity(&magic_attr)), bound_head_var]
```

`rule_invocation_to_pattern` already handles 2-arg invocations:

```
Pattern { entity: Uuid(sentinel), attr: magic_attr, value: bound_head_var }
```

When matched against the seed fact:
- entity `Uuid(sentinel) == sentinel` ✓
- value binds `?mgr = Keyword(":alice")` ✓ (correct type preserved)

### Propagation rules (fb case)

`build_propagation_rules` uses the same 2-arg guard format when the outer adornment starts with `'f'`.

### bf path

Unchanged. The existing 1-arg guard and `edn_to_entity_id(arg0)` entity encoding remain for `bf` adornments.

## Scope of changes

All changes in `src/query/datalog/magic_sets.rs`:

| Function | Change |
|---|---|
| `build_seed_facts` | For `fb`: replace `Uuid::new_v4()` with `sentinel_entity(&magic_attr)` |
| `inject_magic_guard` | For `fb` (bound position > 0): emit 2-arg guard `[Uuid(sentinel), bound_head_var]` |
| `build_propagation_rules` | For `fb`: use same 2-arg guard format |
| `sentinel_entity` | New private helper |

No changes to `evaluator.rs`, `matcher.rs`, `executor.rs`, or `parser.rs`.

## Tests

### Integration tests (added to `tests/magic_sets_test.rs`)

**Test 1 — #298 (value-position keyword binding, non-recursive):**

```
transact [[:bob :employee/manager :alice]]
transact [[:carol :employee/manager :alice]]
rule (reports-to ?emp ?mgr) :- [?emp :employee/manager ?mgr]
query [:find ?emp :where (reports-to ?emp :alice)]
→ must return keyword entities for :bob and :carol
```

**Test 2 — #297 (entity-position keyword, intermediate variable, recursive):**

```
transact [[:a :edge :b]]
transact [[:b :edge :c]]
rule (reach ?x ?y) :- [?x :edge ?y]
rule (reach ?x ?y) :- [?x :edge ?mid] (reach ?mid ?y)
query [:find ?y :where (reach :a ?y)]
→ must return keyword entities for :b and :c
```

If Test 2 does not pass after the `fb` fix, diagnose from the actual test output (the exact error is not reproducible from static analysis alone).

### Unit tests (added to `magic_sets.rs`)

- `test_fb_seed_uses_sentinel_entity` — same keyword arg → same entity UUID on two calls (not random)
- `test_fb_guard_is_two_arg` — `inject_magic_guard` emits 2-arg args for `['f', 'b']` adornment
- `test_fb_sentinel_matches_seed` — sentinel UUID in guard args equals sentinel UUID in seed entity

### Existing tests

All existing magic sets tests (`test_literal_arg_is_bound`, `test_bound_start_transitive_closure`, etc.) must continue to pass — the `bf` path is untouched.

## Edge cases

- **`edn_to_value` fails for fb arg1:** skip seed generation for that predicate (same as today).
- **`bf` adornment:** completely unaffected — separate code path.
- **Fully-bound `bb` adornment:** handled by existing fast-path that skips magic sets when `!any_free`.
- **All-free adornment:** returns `None` from `rewrite()` as before.
