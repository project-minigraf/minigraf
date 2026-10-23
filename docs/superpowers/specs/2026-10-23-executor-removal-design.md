# Executor removal — Design

**Issue:** #551 (streaming engine 9/9, part of #473)
**Parent design:** `docs/superpowers/specs/2026-10-11-streaming-engine-design.md` (§9, §10, §12), called "the #473 spec" below
**Milestone:** v3.0.0, branch `v3`. The `.graph` file format stays v8 and the WAL format stays v2.
**Status:** brainstormed 2026-10-23; not implemented

This spec refines §9 of the #473 spec against the code on `v3` at `1096e6a2`. Where the two differ, this one applies. §9 lists every difference.

## 1. Decisions

Settled in the brainstorm on 2026-10-23:

1. **Three PRs** (§8): the engine runs every query; then the executor's query path and the oracle are deleted; then measurement and documentation.
2. **`:as-of` with a timestamp becomes a counter bound when it can and a filter otherwise** (§3.1).
3. **The `test-oracle` Cargo feature becomes `test-hooks`** (§5.3). It still gates the test-only hooks after the oracle is gone.

## 2. What still runs on the executor

`DatalogExecutor::open_query` (`executor.rs:514`) runs a query on the engine when `engine::plan` (or, for a rule query, `engine::plan_rules`) returns a plan and `engine::open` returns a stream; otherwise it runs it on the executor (`execute_query_on_executor`, `execute_query_with_rules`).

A run of the whole suite with `--features test-oracle` on `1096e6a2`, logging every query that reached either executor path, found 48 such queries of 21 shapes. Each falls into one of these causes:

| # | Cause | Where the planner stops | Example (test) |
|---|---|---|---|
| A | `:as-of` with a timestamp | `temporal` (`plan.rs:303`) | `tests/prepared_statements_test.rs`, `rule_query_routing` |
| B | An `or` branch with a nested `or`/`or-join`, a rule invocation, a UDF predicate, or a `not` that reads a variable only an expression of the branch binds | `runs_branch` (`plan.rs:1202`) via `union` (`plan.rs:1061`) | `tests/disjunction_test.rs`, `tests/plan_independence_test.rs:405`, `deep` and `notable` rules |
| C | A rule body with no pattern | `Builder::clause` for `Expr` (`self.source.as_ref()?`) | `one` in `rule_query_routing` |
| D | A constant in a head (the magic-set rewrite writes one, #608) | `rule_plan`'s `arg` (a head argument must be a variable) | `test_608_rule_call_with_unbound_argument` (`(chain ?x :c)`) |
| E | A non-keyword constant in the entity position of a rule invocation | `entity_const` (`plan.rs:1355`) | `(named "alice" ?e)` in `tests/recursive_rules_test.rs:715` |
| F | An invocation of three or more arguments | `calls_as_patterns` (`rule_invocation_to_pattern` errors) | `(reach ?x ?y ?z)` in `rule_query_routing` |

No query in the suite hit the other refusals, but each is reachable:

| # | Cause | Where |
|---|---|---|
| G | `or` branches whose own (hidden) variables differ | `union`, the `own != names` check; the parser rejects every such query tried (PRS-057 counts hidden variables as new) |
| H | An `or-join` whose join variable no earlier clause binds | `union`; the executor reports `INT-031` (`executor.rs:2260`) |
| I | A `not`/`not-join` body holding anything but patterns and expressions (a nested `not`, `or`, or a rule invocation in a plain query's body) | `Builder::anti` |
| J | A `not` before any pattern, with no pattern after (`front` not empty at `finish`) | `Builder::finish` |
| K | A head of no arguments (`INT-022` on the executor), or of more than two, whose arguments after the second the executor ignores (`evaluator.rs:365–390`) | `rule_plan` |
| L | A stratification error | `plan_rules` (`stratify().ok()?`) |
| M | A snapshot whose committed reader has no keys (`key_source` returns `None`) | `engine::open`, `engine::open_rules` |
| N | An `:as-of` / `:valid-at` slot | `temporal`; prepared queries substitute every slot before this, so it is unreachable |

## 3. The engine runs every query (PR 1)

`plan` and `plan_rules` return `Result<Plan>` / `Result<RulesPlan>` instead of `Option`. Every `None` becomes an operator path (A–E, I, J, K with arguments) or a coded error (F, G, H, K without arguments, L, N), and `engine::open`/`open_rules` stop returning `Option` (M). Lookups that cannot fail once the planner has bound a slot (`by_var.get(v)?` after the slot was added) return `INT-` internal errors. With no `None` left, `open_query` has no fallback, which is what makes PR 2 a deletion.

The errors keep today's codes. Each one the executor reports today for F, G, H, K, L and N is raised by the planner, in the order §5.5 of the #473 spec fixes (pseudo-attributes, then the rules, then unknown predicates). A test per cause pins the code.

### 3.1 `:as-of` with a timestamp (A)

`AsOf::Timestamp(t)` keeps the fact versions whose `tx_id <= t` (`storage.rs:905`), then decides net-assert among them. `tx_id` is the wall-clock time of the transaction, `max(now, next_pending_tx_id)` at commit (`db.rs:1624`). It need not rise with `tx_count`: the clock can step back between two opens, and `LogWriter` (#431) copies each record's `tx_id` as it was. So the set of transactions a timestamp keeps is not always a prefix of `tx_count`.

- **Monotonic.** The snapshot is *monotonic* when `tx_id` never decreases as `tx_count` rises, over its committed transactions (DICT `tx_count → tx_id` entries) and then its pending ones.
  - The committed reader computes this once per generation, on the first timestamp query that reads it, in one pass over the DICT tx entries, and keeps it with the reader.
  - The pending store keeps a flag up to date as it appends a transaction (the new `tx_id` is not less than the previous one) and clears it at checkpoint with the store.
  - The snapshot is monotonic when both are and the first pending `tx_id` is not less than the last committed one.
- **Monotonic snapshot:** the timestamp becomes the counter `c`, the largest `tx_count` whose `tx_id <= t` (0 when none), found by binary search: over the pending transactions in memory, and over the committed ones by DICT point lookups. The query then runs exactly as `:as-of c`, a key bound on every index.
- **Otherwise:** scans run with no counter bound and a `tx_id <= t` test on each version before net-assert. A version's `tx_id` comes from its `tx_count` through the DICT (`DictReader::tx_id_of`) for committed keys and from the fact for pending ones.
- Rule queries take the same bound (`RulesPlan::as_of`).

A test covers both paths: a file whose `tx_id`s decrease (built with `LogWriter`) and one whose `tx_id`s rise, each at timestamps before, between and after its transactions, against `tests/reference_model/`.

### 3.2 `or` branches (B, G, H)

A branch is planned as a `:where` is (`clauses`): its patterns and expressions in `optimizer::plan_seeded` order, then its `or`s in text order, then its deferred clauses. So a branch may hold nested `or`/`or-join`s, `not`s and rule invocations (as patterns of their relations, as in a rule body).

- **Clause order inside a branch.** The executor applies a branch's patterns, then its nested `or`s, then its `not`s, then its expressions (`evaluate_branch`, `executor.rs:1868`). The engine applies them as it does at top level, where #625 fixed the same ordering: a `not` or nested `or` that reads a variable only an expression of the branch binds sees that binding. The answers differ only for such branches, where the executor treated the variable as unbound inside the `not` or nested `or`. Example, checked on `1096e6a2`: over `[:x :n 1] [:y :n 2] [:z :n 5]`, `[:find ?e :where [?e :n ?v] (or-join [?e ?v] (and [(+ ?v 1) ?w] (not [?f :n ?w])) [?e :n 5])]` returns `:z` on the executor (the `not` matches any fact, so the first branch keeps no row) and `:y`, `:z` on the engine.
- **UDF predicates in a branch.** Every predicate a query names is resolved when the query opens, as `:find` functions are (#414) and as top-level predicates are already (`check_clauses`). The executor reports an unknown predicate in a branch only when a row reaches it; the engine reports it whether or not one does.
- **Branches whose own variables differ (G).** PRS-057 counts a branch's hidden metadata variables as new variables, so the parser rejects an `or` whose branches' hidden variables differ (checked on `1096e6a2` with one and with two pseudo-attribute branches). If one reaches the planner, it is an `INT-` internal error, with a test that pins the parser's rejection.
- **An `or-join` whose join variable is unbound before it (H)** is `INT-031`, as on the executor.

### 3.3 Bodies with nothing to scan first (C, I, J)

- **No pattern (C).** A body or `:where` whose clauses bind before any pattern starts from `Source::Seed`, the one empty row, as an `or` with nothing before it already does. An expression clause then evaluates once per row of the seed.
- **`not` bodies (I).** `Builder::anti` plans its body with `clauses`, as §3.2 plans a branch, so a body may hold a nested `not`, `or`, or rule invocation.
- **`front` left at `finish` (J).** A query of `not`s and no pattern starts from `Source::Seed`, and the `front` steps run on it.

### 3.4 Rule heads and invocations (D, E, F, K, L)

- **Constant head arguments (D).** `HeadArg` gains `Const(Cell)`. Only the magic-set rewrite writes one (PRS-114 rejects a constant in a rule the user defines), so the seed's entity cell is the same in every row the rule derives.
- **Constants in an invocation's entity position (E).** A relation's entity cells are entities only: a head entity that is not a ref or a keyword fails the query with `QRY-013` when a rule derives it, on the executor (`evaluator.rs:365–377`) and on the engine (`recursive.rs:210`). So a non-entity constant in an invocation's entity position (a string, number, boolean or `nil`) is a term that matches no row, and the query still derives its rules first: `(named "alice" ?e)` over `(named ?n ?e) [?e :name ?n]` fails with `QRY-013`, as `tests/recursive_rules_test.rs:715` expects today, and returns no row when the rule derives nothing.
- **Heads of more than two arguments (K).** The arguments after the second are ignored, as on the executor. A head of no arguments is `INT-022`.
- **F, L** are errors (§3).

### 3.5 Readers without keys (M)

`CommittedReader::key_source` becomes required. `OnDiskReader` is the only production implementation; test readers that return `None` either implement it over their facts or their tests move to an on-disk database.

### 3.6 Proof that nothing falls back

In PR 1, under `cfg(test)` and `test-oracle`, reaching `execute_query_on_executor` or `execute_query_with_rules` other than from the oracle panics. The whole suite, run with `--features test-oracle` in CI, then shows that no query falls back, and every answer has matched the executor once more before PR 2 deletes it.

## 4. Answer changes

v3.0.0 changes these answers compared with v2.x and with v3 before PR 1. Each goes into `CHANGELOG.md`'s migration notes and the docs site (decision `v3-followup-notes`, item 2):

1. A `not` or nested `or` inside an `or` branch sees a variable an expression of the branch binds (§3.2).
2. An unknown predicate in an `or` branch is an error even when no row reaches it (§3.2).

PR 1 runs the oracle with these two exempted, each by a named test that shows the executor's answer and the engine's.

## 5. Deleting the executor (PR 2)

### 5.1 Code removed

- `executor.rs`: the query path: `execute_query_on_executor`, `execute_query_with_rules`, `filter_facts_for_query`, `live_source_facts`, `selective_fact_fetch`, `evaluate_branch`, `apply_or_clauses`, `apply_expr_clauses` and every helper only they call, the oracle hooks (`oracle`, `RUNS`' executor side), and the unit tests of that path. `DatalogExecutor` keeps `transact`, `retract` and `rule` commands and the dispatch to the engine. `eval_expr`, `is_truthy` and `Binders` stay (the engine uses them) and move next to their users where that removes an import from `executor.rs`.
- `matcher.rs`: `PatternMatcher` goes. `edn_to_value` and `edn_to_entity_id` stay (the parser, the statement reader and the planner use them).
- `evaluator.rs`: `RecursiveEvaluator`, `StratifiedEvaluator` and `evaluate_not_join` go. `rule_invocation_to_pattern`, `DEFAULT_MAX_ITERATIONS`, `DEFAULT_MAX_DERIVED_FACTS` and the limit errors (`derived_facts_exceeded`, `iterations_exceeded`) stay.
- `engine/oracle.rs`, and the oracle blocks in `open_query` and `open_rule_query`.
- `FactStorage` methods whose only caller was the executor's query path. The plan lists them, each checked with `cargo build` on all targets and features.

### 5.2 Tests

- Tests of the executor's internals go with them.
- Tests that ran a query through the executor directly (`DatalogExecutor::execute` on a hand-built `FactStorage`) and check an answer move to `Minigraf::in_memory`, so they keep checking the answer on the engine.
- `tests/model_based_test.rs`, `tests/property_test.rs` and `tests/golden_corpus_test.rs` stay: their oracle is `tests/reference_model/` and the stored corpus, not the executor.

### 5.3 The `test-hooks` feature

`test-oracle` is renamed `test-hooks`. It gates `__reorder`, `__tx_chunk_facts`, `__statement_facts_high_water` and `__subplans` (`lib.rs:123–165`), and nothing else.

- `rust.yml` and `smoke.yml` on `v3` pass `--features test-hooks`.
- `crash-kill.yml` is the same file on `main` and `v3` (#687). Its check (`grep -q '^test-oracle = ' Cargo.toml`) becomes a check for `test-hooks`, on both branches as separate PRs. `main`'s `Cargo.toml` defines neither feature, so its run is unchanged.
- The integration tests that named `test-oracle` in `#![cfg(...)]` name `test-hooks`.

### 5.4 Binary size

With the executor gone, `binary-size.yml` on `v3` sets `SIZE_GATE: fail` and `SIZE_LIMIT_BYTES: 1228800` (1,200 KiB), and the temporary note in PHILOSOPHY.md §4 goes (#680; comments on #551). If the release binary is over 1,200 KiB after the deletion, PR 2 reports the size and its largest contributors (`cargo bloat --release --crates` and `-n 30`) before any change to the limit.

## 6. Measurement (PR 3)

On the bench machine, idle, against `v3` before PR 1 (`1096e6a2`) as the baseline:

- The #394 envelope at 100K, 1M and 10M facts (`cargo run --profile envelope --example perf_envelope -- --scales 100k,1m,10m`), base and PR in parallel on different NUMA nodes, swapped each round.
- The #379 point queries must not regress (#473 spec §9).
- `:as-of` with a timestamp: monotonic and non-monotonic fixtures at 1M facts, a point query and an attribute scan each.
- Python round trip: the `minigraf-python` `v3` binding, built against this branch, opens the 1M envelope fixture, runs an attribute-scan query and drains it with `MiniGrafCursor::next_batch(1000)`; wall time and peak RSS go into `docs/BENCHMARKS.md`'s envelope section. The script lives in `examples/` of this repo, or in `minigraf-python`'s `v3` branch if it needs the binding's build; the plan settles which.

Results go into `docs/BENCHMARKS.md` (envelope section) and the PR.

## 7. Acceptance and documentation (PR 3)

- Every unchecked acceptance test of #473 except the `:limit` one (#306): each is matched to the test that covers it (`memory_limit_acceptance_test.rs`, `large_transaction_acceptance_test.rs`, `recursive_engine_test.rs`, `cursor_test.rs`, the fact-log and snapshot tests), and any without one gets one. The #473 checklist is ticked with links.
- `tests/cursor_test.rs` and every binding's test suite pass unchanged (the bindings' `v3` CI, after their git pin moves to the merge).
- Docs: `CLAUDE.md` (module list without the executor and oracle, `test-hooks`, test count), `ROADMAP.md`, `README.md`, `docs/TEST_COVERAGE.md`, `CHANGELOG.md` (with §4's answer changes), and the docs site's `content/architecture.md` (the engine as the only query path), with `@since v3.0.0` where the text differs from v2.x.

## 8. Delivery

Three PRs to `v3`, merge commits. None uses a closing keyword for #473 or #551; #551 is closed by hand after PR 3, and #473 after its remaining item (#306) is tracked there.

1. **The engine runs every query** (§3, §4): `plan`/`plan_rules` return `Result`, causes A–N, the fallback panic (§3.6), the two answer-change tests.
2. **Delete the executor's query path** (§5): code, tests, `test-hooks`, the size gate; the `crash-kill.yml` change also on `main`.
3. **Measure and document** (§6, §7).

## 9. Changes from the #473 spec

1. **The engine had to cover more first.** §9 of the #473 spec deletes the executor "when every clause kind runs on the new engine". Every clause kind does, but not every shape (§2), so PR 1 comes before the deletion.
2. **The feature stays, renamed.** §9 deletes "the oracle feature". It also gates test hooks added after it (#550, #596, #546), so it is renamed `test-hooks` (§5.3).
3. **Two answer changes** (§4). The #473 spec's non-goal "every query returns the same rows, as a bag, as today" does not hold for these two shapes, where the executor's answer depends on its clause order inside a branch or on whether a row reaches a predicate.
