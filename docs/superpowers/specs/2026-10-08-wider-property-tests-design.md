# Wider property tests — Design

**Issue:** #386, tracker #383
**Milestone:** v3.0.0, branch `v3`, with a v2.x backport to `main` (test and CI only;
no library change)

## 1. Goal

`tests/property_test.rs` checks query semantics against a reference evaluator,
but only for one-pattern queries over an in-memory database written one fact per
`transact`, and it counts a query error as an empty result. Widen it to joins,
negation, predicates and temporal queries over histories written in batches with
retractions, on both the in-memory and the file-backed database. The sibling
model-based test (#385) covers operation sequences with single-pattern reads;
this test covers query semantics.

## 2. Shared reference model

The temporal reference of `tests/model_based_test.rs` (value pool `V`, the time
grid, `Window`/`Effective`, `valid_at`, and `Model` with `verdict`, `records`,
`commit` and `live`) moves unchanged to `tests/reference_model/mod.rs`. Both test
files use it with `mod reference_model;`. One copy of the subtle rules (#435 one
window per triple, #477 latest statement wins, API-011/API-019 verdicts) keeps
the two tests from drifting apart.

## 3. Histories

A case writes a history of 1–12 operations to a fresh database:

| Op | API |
|---|---|
| `Transact` | one `(transact …)` of 1–8 facts, tx- and fact-level windows from the grid |
| `Retract` | one `(retract …)` of 1–4 triples |
| `WriteTx` | `begin_write`, 1–4 transact/retract statements, `commit` |
| `Checkpoint` | `checkpoint()` (file backend only; a no-op in memory) |

Facts come from the shared pools (4 entities, 3 attributes, 18 values including
refs to the entities), so batches repeat entities and attributes, and retractions
hit live triples. Generated statements are always accepted: an empty or inverted
window is rewritten to start in 2000 and a repeated triple in one statement
keeps one window. Rejections are the model-based test's job; here every write
must succeed.

## 4. Backends

- **Memory:** `Minigraf::in_memory()`.
- **File:** `open_with_options` in a temp dir (`SyncMode::Normal`), with the
  history's `Checkpoint` ops applied, so the queries read a mix of checkpointed
  and pending facts. The queries run once on the open handle, then again after a
  drop and reopen (default options or `page_cache_size(4)`).

## 5. Queries

Each case runs 1–4 generated queries. A query has:

- **Patterns:** 1–3 `[E A V]`. `E` is an entity variable (`?e0`–`?e2`) or an
  entity constant. `A` is an attribute constant, or rarely `?a`. `V` is a value
  variable (`?v0`–`?v2`), an entity variable (a ref join), a pool constant, or
  `_`. The small variable pools make patterns share variables, so most queries
  with two or three patterns are joins. Every pattern after the first shares a
  variable with an earlier one, so there are no cross products.
- **Negation (optional):** `(not [E A V])` over variables already bound, or
  `(not-join [?x] [?x A ?fresh])` with a variable already bound.
- **Predicate (optional):** `[(op ?x c)]` or `[(op ?x ?y)]` with `op` in
  `< <= > >= = !=`, over bound variables.
- **Temporal:** none, `:as-of N`, `:valid-at T`, both, or `:any-valid-time`.
  `N` ranges over 0..=tx count + 1, `T` over the model-based grid.
- **Find:** a non-empty subset of the bound variables.

## 6. Reference evaluator

1. Visible facts: `Model::live(as_of)`, then `valid_at` (skipped for
   `:any-valid-time`).
2. Patterns: a nested-loop join over the visible facts, one binding map per
   combination; `_` matches anything and binds nothing; an entity variable in `V`
   matches `V::Ref`.
3. `not` removes a binding when its pattern matches a visible fact; `not-join`
   when some visible fact matches with the join variable bound.
4. Predicates follow `eval_binop`: `=`/`!=` compare values structurally;
   orderings compare two strings bytewise and numbers as `f64`; any other pair is
   a type error that drops the binding (no query error).
5. Project onto the find variables.

**Comparison.** The engine returns one row per binding, not a set: `[:find ?e
:where [?e :c ?v]]` gives one row per `?v`. When the find covers every variable
and no pattern has `_`, each binding is distinct, so rows are compared as sorted
multisets; a duplicated or dropped fact then shows up. Otherwise rows are
compared as sets, which holds whether the engine keeps bag semantics or moves to
sets.

Results render as in the model-based test: entities as pool indices, so failure
messages carry no `Uuid` (CodeQL rule).

## 7. Errors

Any write, checkpoint, reopen or query error fails the case with its error code.
The three existing properties stay (one fact per `transact`, cheap), with
`if result.is_err() { return vec![]; }` replaced by a failure.

## 8. Cases and CI

New properties read `PROPTEST_CASES` (default 256 in normal CI). The file-backed
property runs a tenth of that, since each case touches the disk. Measured per
case: 3 ms in memory, 57 ms file-backed, 0.7 ms for each one-pattern property.
The nightly count drops from 8,000,000 to 1,000,000: about 1.6 h locally, well
inside the 6-hour limit on a slower runner.

Histories and query constants favour small integers and entities, and drop the
model-based test's hot triple: with the shared generators only 16% of queries
returned rows; with these, about 35% do, over a mean of 13 visible facts.

Mutation checks during development, each caught: `:valid-at` inclusive at
`:valid-to`; `<` evaluated as `<=`; a predicate type error that keeps the row;
the on-disk read skipping retractions, or not hiding a triple's older
transactions; and, in the reference, `not` ignoring `:as-of` and patterns
ignoring their attribute. (Emitting on-disk retractions as assertions is an
equivalent mutant: the executor's net-assert step drops them.) Minimized
failures become named regression tests in the same file.

## 9. v2.x backport

A separate PR to `main` ports the test with a reference limited to v2.x
semantics:

- No v8 model rules: v2 has no API-011/API-019 (#435, #436), so the generator
  never repeats a triple within one transaction and never writes an empty
  window.
- #371 (same-transaction multi-valued facts read back as one value): no
  transaction writes two values of one entity and attribute, and no batched
  retract removes two such values.
- #435 (several valid windows per triple): a triple is never asserted again
  while it is live.
- #406/#414 do not apply: generated finds are always bound.

`verify()` is not called (it does not exist on v2). The PR names these
exclusions.
