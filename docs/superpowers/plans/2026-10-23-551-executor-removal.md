# Executor removal (#551) — Plan

Spec: `docs/superpowers/specs/2026-10-23-executor-removal-design.md` (merged as #714),
called "the spec" below. PRs target `v3`, merge commits. None uses a closing keyword for #473 or
#551; #551 closes by hand after PR 3.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–13: the engine runs every query (spec §3, §4) | `feat/551-engine-everywhere` |
| 2 | 14–21: delete the executor's query path and the oracle, `test-hooks`, size gate (spec §5) | `feat/551-delete-executor` |
| 3 | 22–26: measurement, acceptance, docs (spec §6, §7) | `feat/551-measure-docs` |

Each PR leaves `v3` with no unused code outside `cfg(test)` and the oracle's `cfg`, and updates
the test counts. PR 1 changes `CLAUDE.md`'s module notes and the test counts only; `CHANGELOG.md`
and the docs site, with the two answer changes of spec §4, come in PR 3 (spec §7).

The oracle stays on through PR 1: every query each step moves to the engine is also run by the
executor, and the answers must agree (`engine::oracle::check`). So each step's tests first show
the executor's answer for the shape, through the oracle, and the step then keeps it.

## Decisions not fixed by the spec (PR 1)

1. **No fallback by construction (replaces spec §3.6's panic).** `plan`, `plan_rules`,
   `engine::open` and `engine::open_rules` return `Result`. Until step 12 their error is a
   `Refusal`: `Unsupported(Cause)` (the executor still runs the query) or `Fail(anyhow::Error)`
   (the query fails with that error). Each step deletes the `Cause` variants it covers. Step 12
   deletes `Unsupported` and `Refusal` with it, and with them the two fallback calls in
   `open_query` (`executor.rs:532` to `execute_query_with_rules`, `executor.rs:577` to
   `execute_query_on_executor`): `open_query` and `open_rule_query` then have no executor
   branch, the oracle blocks are the only callers left, and `execute_query_on_executor`, `execute_query_with_rules` and everything
   only they reach compile under `cfg(any(test, feature = "test-oracle"))`, as the oracle. That is
   stronger than a panic in tests: no build can fall back. The release binary drops the
   executor's query path in PR 1. PR 1 reports its size; the gate comes back in PR 2 as the spec
   says (§5.4).
2. **Planner invariants are `INT-032`** ("query executor internal error", a catch-all for query
   invariants not expected to be reachable): a slot missing after the planner added it, a
   pseudo-attribute without a resolved source (the parser's PRS-081 runs first), a bare symbol
   that is not a variable, a rule invocation left in a plain query's clauses, G (spec §3.2).
3. **`TxBound` replaces `as_of: Option<u64>` in the read path.** In `storage`:
   `enum TxBound { All, Upto(u64), Timestamp { t: u64, committed: Arc<[(u64, u64)]> } }`, the
   versions a read keeps: every one; those with `tx_count <= n`; or, for committed keys, those
   whose `tx_count` is in one of the sorted, disjoint, inclusive ranges `committed`, and for
   pending and staged facts those with `tx_id <= t`. It replaces `as_of` in
   `KeySource::live_keys`, `OnDiskReader::live_walk` (`tx > limit` becomes
   `!bound.keeps_key(tx)`), `FactStorage::pending_live`/`pending_records`/`pending_facts_of_scan`
   (`bound.keeps_fact(f)`) and the engine's `ScanSpec`. `CommittedReader::live_facts` and the fact log keep
   `Option<u64>` (no timestamp reaches them).
4. **Committed keys are filtered by `tx_count` ranges (deviation 1).** A committed key holds a
   `tx_count`, not a `tx_id`, and the DICT holds one `tx_id` per `tx_count`: a checkpoint or
   migration that meets a second one fails with `STG-037` (`dict.rs:358`), and `LogWriter`
   rejects one (`log_writer.rs:154`). So for committed keys "keep a version whose `tx_id <= t`"
   is "keep a version whose `tx_count` is in the set of transactions with `tx_id <= t`", and
   that set, as ranges, is `committed`; net-assert runs on keys as it does for `Upto`, with no
   DICT lookup per version. Pending and staged facts carry their `tx_id` and are filtered on it
   directly, because nothing guarantees one `tx_id` per `tx_count` there: a read-only open of a
   v7 file loads its facts as pending without the `STG-037` check
   (`persistent_facts.rs:158`), and WAL replay appends each entry's facts as written.
5. **Where the bound is resolved.** The plan keeps the query's `:as-of` unresolved
   (`AsOfSpec { None, Counter(u64), Timestamp(i64) }`, on `Plan` and `RulesPlan`).
   `engine::open`/`open_rules` resolve it once against the snapshot (`Snapshot::tx_bound`) and
   put the `TxBound` in `Ctx`; every scan of the query, its bodies and its rules read it there.
   A negative timestamp keeps nothing (`Upto(0)`), as `u64::try_from(t).unwrap_or(0)` does on the
   executor (`storage.rs:905`).
6. **`Snapshot::tx_bound(t)`.**
   - Committed: `KeySource` gains `tx_ids_monotonic() -> Result<bool>` (one walk of the DICT
     `0x05` prefix, cached in a `OnceLock<bool>` on `OnDiskReader`, so once per reader, i.e. per
     generation or per large transaction's working trees), `last_tx_at(t) -> Result<u64>`
     (largest `tx_count` with `tx_id <= t`, 0 if none; binary search over `1..=last_tx` with a
     `LeafCursor` seek to the first DICT tx key at or above the probe, since `tx_count`s may have
     gaps) and `tx_runs(t, out)` (the walk, appending ranges).
   - Pending: `PendingStore` keeps `first_irregular: usize`, the first position whose fact's
     `tx_id` is below the previous fact's, or differs from it within the same `tx_count`
     (`usize::MAX` if none), updated by `push` and `extend`. The snapshot's pending prefix is
     monotonic when `pending_len <= first_irregular`; its staged facts (one `tx_id`, one
     `tx_count`) when that `tx_id` is not below the prefix's last.
   - Monotonic snapshot (committed, pending, staged, and each part's first `tx_id` not below the
     previous part's last): `Upto(c)`, `c` the largest `tx_count` with `tx_id <= t` over the
     three, by binary search over the pending prefix (facts are in `tx_count` order).
   - Otherwise `Timestamp { t, committed }`, `committed` from the DICT walk. Its ranges are
     charged to the query's `Meter` (#549); a file that alternates on every transaction can need
     one range per transaction.
7. **`Term::Never`.** A constant no cell can equal: an entity constant that is not a keyword or
   UUID, an attribute constant that is not a keyword, a value constant of a kind `value_const`
   does not take (vector, list, map). A scan with a `Never` term reads nothing. Step 9's tests
   first show the executor's answer for each; any kind on which it raises an error instead gets
   that error.
8. **Branches and `not` bodies are planned by `clauses`.** `runs_branch` goes, and
   `Builder::anti` stops filtering clause kinds. `sub`'s correlated builder already starts from
   `Source::Seed`; its hashed builder starts from `Source::Seed` when the body's first clause to
   run is not a pattern. `closed` (whether a body is hashable) also counts the outer variables a
   nested `or`/`or-join` reads (`optimizer::clause_ref_vars`), as it counts a nested `not-join`'s.
9. **Unknown predicates are checked at every depth, rule bodies included.**
   `check_udf_predicates` walks every clause list of the query, and for a rule query every
   evaluated rule's body, and raises `QRY-007` for the first unknown name in text order (query
   first, then rules in `plan_rules`' predicate order). The spec says "every predicate a query
   names"; a rule body's predicates are named by the query that evaluates it.
10. **Oracle exemption.** `engine::oracle::exempt()` returns a guard (`cfg(test)`/`test-oracle`)
    under which `check` is skipped on this thread. Only the two answer-change tests (step 6) use
    it.
11. **Head constants.** `HeadArg::Const(EdnValue)`: the entity position takes the cell
    `recursive.rs`'s `entity` gives the constant (a keyword's or UUID's entity, else `QRY-013`),
    the value position `Cell::of_value`. Resolved once per rule run, not per row.
12. **Readers without keys (M) are `INT-049`** at open, not a required `key_source` (deviation
    2): `MockReader` (`storage.rs:1932`) is the only one, and its tests run no query, so the
    trait stays as it is.

## Deviations from the spec (PR 1)

1. §3.1's per-version `tx_id` filter is a `tx_count` range filter for committed keys
   (decision 4); same versions kept, since the DICT holds one `tx_id` per `tx_count`. Pending
   facts keep the per-version filter. Monotonic pending facts also need one `tx_id` per
   `tx_count` (`first_irregular`).
2. §3.5's required `key_source` is an `INT-049` at open (decision 12).
3. §3.6's test-time panic is structural: no fallback path exists after step 12 (decision 1).
4. A third answer change (decision 9, found in step 6): an unknown predicate in a `not` or
   `not-join` body is `QRY-007` at open. The executor evaluated it per body row, where it
   failed, so the `not` kept every outer row. Spec §3.2's "every predicate a query names" covers
   it, but §4 does not list it; PR 3's CHANGELOG lists it with the other two.
5. Decision 6 in the code: `first_irregular` also marks a falling `tx_count` (a read-only v7
   open's pending facts need not be in `tx_count` order), and `KeySource::tx_order` returns
   whether the DICT's `tx_id`s rise together with the newest `(tx_count, tx_id)`, which
   `last_tx_at` searches up to instead of the meta's counter.

## PR 1 steps

1. **Plan.** This file.
2. **`Refusal`.** `plan`/`plan_rules`/`open`/`open_rules` and the builder functions return
   `Result<_, Refusal>`; every `None` today becomes `Unsupported(cause)` with the spec §2 letter,
   lookups that cannot fail become `Fail(INT-032)`. `open_query` falls back on `Unsupported`
   only. No answer changes: the whole suite and the oracle pass unchanged.
3. **`TxBound` in storage** (decisions 3, 6): the type, `live_walk`/`live_keys`,
   `pending_live`, `first_irregular`, `tx_ids_monotonic`/`last_tx_at`/`tx_runs`,
   `Snapshot::tx_bound`. Unit tests: `first_irregular` under `push`/`extend`/`of`, including two
   `tx_id`s in one `tx_count` (a read-only v7 open); `last_tx_at` with
   gaps, before the first and after the last transaction; `tx_runs` on alternating `tx_id`s;
   each against a scan of all facts with the executor's filter.
4. **`:as-of` timestamps on the engine (A)** (decision 5): `AsOfSpec`, `Ctx`'s bound, rule
   queries too. Tests: a database whose `tx_id`s rise and a `LogWriter`-built one whose `tx_id`s
   fall and rise, at timestamps before, between, at and after transactions, with committed,
   pending and staged facts, plain and rule queries, point and attribute scans; the counter
   path is taken only on the monotonic one (operator counters); `:as-of` with a negative
   timestamp.
5. **Branches (B)** (decision 8): nested `or`/`or-join` in a branch, `not` reading an
   expression's variable, rule invocations in a branch of a rule body (`deep`). Tests:
   `tests/plan_independence_test.rs`'s nested shapes, `tests/disjunction_test.rs`, under each
   `__subplans` setting.
6. **Answer changes** (spec §4; decisions 9, 10): eager `QRY-007` at every depth; the two
   exempted tests, each asserting the executor's answer (through `oracle`) and the engine's.
   UDF predicates in branches (`notable`) then run on the engine.
7. **No pattern first (C, J):** `Source::Seed` for an expression before any pattern and for a
   `front` left at `finish`. Tests: `one` from `rule_query_routing`, a query of `not`s only, an
   expression-only `or` branch.
8. **`not` bodies (I)** (decision 8): nested `not`, `or` and `not-join` in a `not` body, in plain
   and rule queries, hashed and correlated.
9. **Constants (D, E)** (decisions 7, 11): `HeadArg::Const`, `Term::Never`. Tests:
   `test_608_rule_call_with_unbound_argument` on the engine, `(named "alice" ?e)` still
   `QRY-013`, one per `Never` kind.
10. **Errors (F, G, H, K, L, N):** `INT-028` for an invocation of three or more arguments (the
    code `rule_invocation_to_pattern` returns; the test pins what the executor reports today,
    top level and in a branch); `INT-031` for an unbound `or-join` variable; `INT-022` for a head
    without arguments, extra head arguments ignored; the stratification error (`INT-054`)
    unchanged; `INT-025` for a `:valid-at` slot and `INT-032` for an `:as-of` slot; `INT-032`
    for G with a test that the parser rejects it first (PRS-057). Error order: pseudo-attributes,
    rules, unknown predicates (#473 spec §5.5), one test with all three.
11. **Readers without keys (M)** (decision 12).
12. **No fallback** (decision 1): `Refusal` and `Unsupported` go; the executor's query path moves
    under the oracle's `cfg`. `rule_query_routing` and every routing test assert that every
    query runs on the engine (`RUNS`). Full suite with `--features test-oracle`, clippy over all
    targets and features, both wasm targets (`wasm.yml`'s commands), on the bench machine.
13. **Docs and size:** `CLAUDE.md` module notes (engine runs every query, executor path is the
    oracle), test counts in `CLAUDE.md` and `docs/TEST_COVERAGE.md`; the release binary's size in
    the PR description.

## Decisions not fixed by the spec (PR 2)

PR 1 left everything PR 2 deletes under `cfg(any(test, feature = "test-oracle"))`: the release
binary already lacks it (1,140 KiB), so PR 2 is a deletion with no answer change, and every item
under that `cfg` is classified as one of:

- **oracle**: reached only from the oracle (`engine/oracle.rs`, `DatalogExecutor::oracle`, the
  executor's query path, `PatternMatcher`, the two evaluators, `evaluate_not_join`). Deleted.
- **test helper**: also used by `cfg(test)` code outside the oracle (for example
  `FactStorage::get_all_facts` in `persistent_facts.rs`' tests, `get_asserted_facts` in
  `db.rs`' tests, `Fact::new` in `wal.rs`' tests). Its `cfg` becomes `cfg(test)`. A helper used
  only by its own unit tests is deleted with them.
- **test hook**: the thread-local switches behind `__reorder`, `__subplans`,
  `__tx_chunk_facts` and `__statement_facts_high_water`. Their `cfg` becomes
  `cfg(any(test, feature = "test-hooks"))`.

13. **Unit tests that query through `DatalogExecutor::execute` stay where they are (deviation 6).**
    After PR 1, `execute` runs a query on the engine (`execute_query` → `open_query`), so these
    tests already check the engine's answer; moving them to `Minigraf::in_memory` (spec §5.2)
    would change only their setup. Tests that call a deleted function
    (`execute_query_on_executor`, `execute_query_with_rules`, `filter_facts_for_query`,
    `apply_post_processing`, `apply_expr_clauses`, `evaluate_branch`, `branch_outer_reads`,
    `apply_window_functions`, `selective_fact_fetch` and the like) are deleted, except that a
    test whose query and expected answer are not already checked on the engine elsewhere is
    rewritten to run that query through `execute` and keep its expected answer. The PR lists
    each deleted test and the engine test that covers its shape.
14. **The two answer-change tests keep only the engine's answer.** Each asserted the executor's
    answer through the oracle and the engine's; the executor's answer becomes a comment, with
    the spec §4 item it documents. `oracle::exempt` goes with the oracle.
15. **`RUNS` stays** (`cfg(test)`): the routing tests assert with it that a query ran on the
    engine and how often, which still guards against a second path coming back.
16. **CI.** `rust.yml`'s `oracle` job becomes `test-hooks` (`cargo test --features test-hooks`):
    `plan_independence_test` and the `__subplans`/`__tx_chunk_facts` variants of the model,
    property and crash tests still need the feature. `smoke.yml` passes `--features test-hooks`.
    `crash-kill.yml` checks `^test-hooks = ` on `v3` and, in a separate PR, on `main`, so the
    file stays the same on both (#687); `main` defines neither feature, so its run is unchanged.
17. **Coverage floors.** Deleting tested code changes the per-file and whole-crate ratios. If a
    floor in the coverage workflow fails, the PR reports the new figure; a floor is lowered only
    for a file whose remaining code is no less covered than before (checked line by line).

18. **Frozen answers (found in step 15).** `query::engine::tests`, `tests/recursive_engine_test.rs`
    and `tests/plan_independence_test.rs` checked their answers through the oracle only (the
    engine unit tests' `rows()` returns a row count). Before the oracle goes, each of their
    queries' answers is recorded, on the commit before step 15 with the oracle on, in
    `tests/answers/<suite>.txt`: one line per test, query digest and answer (row count and a
    digest of the rows as a bag). After it, each answer must be one of its query's recorded
    ones in that test. Integers within a week of now (wall-clock `tx-id`s, default
    `valid-from`s, a query's own `valid-at`) compare as "now". `MINIGRAF_BLESS_ANSWERS=1`
    records instead; a changed file is a changed answer, justified in review.

## Deviations from the spec (PR 2)

6. §5.2's move of executor-level query tests to `Minigraf::in_memory` is not needed: they run
   on the engine already (decision 13).
7. §5.2 assumed only the model-based, property and golden suites had their own oracle; three
   more suites relied on the executor, and keep its answers frozen (decision 18).
8. §5.4's `SIZE_GATE: fail` is the gate without `SIZE_GATE`: the report-only branch goes with
   it, so `binary-size.yml` differs from `main`'s only in its branch filters.

## PR 2 steps

Each step builds and passes clippy on all targets and features (`cargo clippy --all-targets`,
with `--features test-oracle`/`test-hooks`, `--no-default-features`, `wasm32-unknown-unknown`
with `browser`, `wasm32-wasip1`) and leaves the suite passing with and without the feature.

14. **Plan.** This section.
15. **The oracle goes** (decisions 14, 15): `engine/oracle.rs`, the `check` blocks in
    `open_query` and `open_rule_query`, `DatalogExecutor::oracle`; the two answer-change tests
    keep the engine's answer. The `engine/mod.rs` docs say the engine is the only query path.
16. **The executor's query path goes** (decision 13): every `oracle` item in `executor.rs`
    (`execute_query_on_executor`, `execute_query_with_rules`, `filter_facts_for_query`,
    `live_source_facts`, `selective_fact_fetch`, `evaluate_branch`, `apply_or_clauses`,
    `apply_expr_clauses`, `apply_post_processing` and every helper only they call), the imports
    only they use, and their unit tests (`selective_lookup_tests`, `pushdown_tests` and the
    rest, per decision 13). `eval_expr`, `is_truthy` and `Binders` stay; where the engine is
    their only user they move to `query/engine/` (spec §5.1) in the same step.
17. **`PatternMatcher` and the evaluators go:** `matcher.rs` keeps `edn_to_value` and
    `edn_to_entity_id` and their tests; `evaluator.rs` keeps `rule_invocation_to_pattern`,
    `DEFAULT_MAX_ITERATIONS`, `DEFAULT_MAX_DERIVED_FACTS`, `derived_facts_exceeded`,
    `iterations_exceeded` and their tests. Comments that name the deleted types
    (`tests/error_codes_qry_test.rs`, `tests/not_join_test.rs`, `benches/minigraf_bench.rs`,
    `stratification.rs`, `prepared.rs`, `plan.rs`, `db.rs`, `browser/mod.rs`) name what runs the
    query now.
18. **Storage and type helpers** (classification above): in `graph/storage.rs`
    (`all_facts`, `facts_as_of`, `live_facts_of_scans`, `get_all_facts`, `get_asserted_facts`,
    `filter_facts_as_of`, `load_fact`/`restore_tx_counter`'s `cfg`), `storage/mod.rs`
    (`CommittedReader::all_facts`, `facts_for_entity`, `facts_for_entity_attribute`,
    `facts_for_attribute`, `live_facts`; `TxBound::of`), `storage/reader.rs` (their
    implementations, `scan`, `live_scan`), `storage/dict.rs` (`seed_entity`, `seed_ident`),
    `graph/types.rs` (`Fact::new`, `is_asserted`), `datalog/types.rs` (`FindSpec::var`,
    `get_top_level_rule_invocations`), `optimizer.rs` (`plan`), `aggregate.rs` imports. After
    this step no `test-oracle` `cfg` remains outside the hooks.
19. **`test-oracle` → `test-hooks`** (decision 16): `Cargo.toml`, `lib.rs`'s four hooks, the
    hook `cfg`s in `optimizer.rs` and `parser/statement.rs`, the integration tests' `cfg`s and
    doc comments (`crash_kill_test`, `model_based_test`, `property_test`, `integrity_test`,
    `plan_independence_test`, `large_transaction_acceptance_test`, `memory_limit_acceptance_test`,
    `recursive_engine_test`, `aggregate_window_rules_test`), `rust.yml`, `smoke.yml`,
    `crash-kill.yml`; the unreleased `CHANGELOG.md` entries that name `test-oracle` (#596, #546).
    A separate PR to `main` changes `crash-kill.yml`'s check; `git diff main v3 --
    .github/workflows/crash-kill.yml` is empty after both.
20. **Size gate** (spec §5.4): `binary-size.yml` on `v3` sets `SIZE_GATE: fail` and
    `SIZE_LIMIT_BYTES: 1228800`, its temporary comment goes, and PHILOSOPHY.md §4's v3 note goes
    (#680). If the release binary is over 1,200 KiB, stop and report `cargo bloat --release
    --crates` and `-n 30` before any change to the limit.
21. **Docs, counts, verification:** `CLAUDE.md` module notes (`executor.rs` without its query
    path, `matcher.rs`, `evaluator.rs`, `engine/oracle.rs` gone, `test-hooks`), test counts in
    `CLAUDE.md` and `docs/TEST_COVERAGE.md` (its oracle notes too); the full suite plain and with
    `--features test-hooks`, the wasm targets and the coverage workflow's commands on the bench
    machine; the release binary's size and the deleted-test list (decision 13) in the PR
    description. `CHANGELOG.md`'s migration notes and the docs site stay for PR 3.

## Decisions not fixed by the spec (PR 3)

19. **Baseline build.** The A/B runs `1096e6a2` (`v3` before PR 1) against this branch. The
    baseline gets this branch's `examples/perf_envelope.rs` (the new shapes of decision 20), as
    earlier A/Bs added their shape to the base (`234cc3e`, `58c19dd`); nothing else changes on it.
    The two builds run in parallel, each pinned to its own NUMA node (`numactl -N -m`), swapped
    for the second round; 100K, 1M and 10M, with the default `--budget-secs` and `--cap-secs`.
20. **`:as-of` timestamp shapes (spec §6).** The envelope fixture's `tx_id`s rise with
    `tx_count`, so it is the monotonic fixture. Two shapes are added: `as_of_ts` (the `as_of`
    point query with `:as-of "<ISO time of tx_half>"`) and `as_of_ts_scan`
    (`[:find ?e ?a :as-of "<…>" :where [?e :age ?a]]`, rows checked). The non-monotonic fixture
    is a copy of the 1M fixture rewritten with `LogWriter`: same records, each transaction's
    `tx_id` taken from a fixed permutation of the source's `tx_id`s (so they fall and rise), built
    by a new `perf_envelope` subcommand (`irregular <src> <dst>`) at 1M only. Both shapes run on
    both fixtures; the report adds the irregular fixture as a fourth column at 1M. The ISO time is
    the `tx_id` of transaction `tx_half` in the fixture being measured, read at measure time with
    `fact_log().tx_range(tx_half..=tx_half)` (the fixture metadata holds only `tx_half`, so cached
    fixtures need no rebuild), and it names a time inside each fixture's history.
21. **Python round trip.** `minigraf-python`'s `v3` branch already has `bench/envelope.py`
    (`db.query`, `next_batch(1000)`, `json.loads` per batch, one process per shape). The binding is
    built with `maturin develop --release` against this branch through a Cargo `[patch]` on its
    git dependency, without committing the patch; Python 3 on the bench machine. If the binding
    does not build against `v3` today, the fix is a `minigraf-python` `v3` PR (pin bump), not this
    PR. Measured on both builds: the 1M `attribute_scan` (166,666 rows) and `count_grouped`,
    wall-time p50 and peak RSS, plus the full shape list at 1M on this branch only for the table.
22. **Acceptance mapping (spec §7).** Each #473 item and its test:
    - large-state design: `docs/superpowers/specs/2026-10-11-streaming-engine-design.md`
      (QRY-012 cap, pin + quarantine, a large transaction commits as a checkpoint);
    - `count` over 10M facts in constant memory: no test today. A new ignored test in
      `memory_limit_acceptance_test.rs`, `count_holds_no_rows`, runs
      `[:find (count ?v) :where [?e ?a ?v]]` over 1M facts with `query_memory_limit` at 64 KiB
      and gets the count: the meter bounds what the query holds whatever the fact count (#549),
      so 1M is enough for the bound, and the envelope's 10M `count` row records the RSS;
    - grouped `count` with 1M groups, and the cap: `grouped_count_1m_groups`;
    - 5M derived facts and `max_derived_facts`: `recursive_acceptance_test::reach_5m`;
    - closing a cursor after the first batch, with operator counters:
      `cursor::tests::first_batch_then_close` (in memory and on a file) and
      `tests/cursor_test.rs::close_after_first_batch`;
    - 1M-fact `transact` holding one chunk:
      `large_transaction_acceptance_test::one_million_fact_transact_holds_one_chunk`;
    - Python round trip: decision 21, recorded in `docs/BENCHMARKS.md`;
    - `tests/cursor_test.rs` unchanged: since #432 it gained tests (#543) and one test moved
      from the deprecated `max_results` to `max_derived_facts` (#553), which the alias still
      accepts; the PR states both. Binding suites: each binding's `v3` CI after its pin moves to
      this PR's merge (post-merge step 26).
    The `:limit` item stays open for #306. The #473 checklist is ticked with a link per item
    after merge, by hand.
23. **CHANGELOG.** The eleven `Internal` entries "… run on the streaming engine" (#544 ×3,
    #545 ×2, #546 ×2, #547 ×2, #548, #549 build side) become one `Added` entry, **"Every query runs on the streaming engine"** (#473, #551), that says what
    a user sees (batches of 1,024, ids turned into values only for returned rows, bounded memory
    per operator and `QRY-012`, cursors that stream, the executor and `PatternMatcher` gone) and
    keeps each part's issue number and the figures that matter; the per-shape operator detail
    moves to `docs/BENCHMARKS.md` and the docs site where it is not already there. #379 stays
    its own entry. Three new `Breaking changes` entries for the answer changes: spec §4.1, §4.2
    and PR 1 deviation 4 (unknown predicate in a `not`/`not-join` body is `QRY-007` at open).
    v2.x has no migration-notes section; the `Breaking changes` list is it.
24. **Docs site.** `content/architecture.md`: the module tree and the "Execute" step name the
    engine as the only query path, `matcher.rs` goes, and the "Streaming engine" section's
    "same rows as the executor" paragraph changes to the frozen answers, each under
    `@since v3.0.0` with the v2.x text under `@until v3.0.0`. `content/datalog-reference.md`
    gets the three answer changes where it documents `or`, `not` and predicates. A
    `minigraf-docs` PR, merged after this one.
25. **Not in PR 3:** the v2.x-vs-v3 audit of the playground and docs queries (follow-up
    `v3-followup-notes` item 2) runs after this PR merges, since it needs the final v3 answers;
    it may add `Breaking changes` entries in its own PR.

## PR 3 steps

22. **Plan.** This section.
23. **Envelope shapes and the irregular fixture** (decision 20), with a test that the
    `irregular` subcommand's copy has the source's facts and a falling `tx_id` (the example
    builds under `cargo test --examples`). Then the measurements on the bench machine (decisions
    19, 21): results into `docs/BENCHMARKS.md`, a "Executor removal (#551)" subsection in the
    envelope section (A/B table, `:as-of` timestamp table, notes), the "What is supported" and
    the 10M rows of the first tables updated where the engine changed them, and the Python table
    re-measured with a peak-RSS column per shape. Any shape more than 5% slower on the branch
    than the baseline at any scale is investigated and reported before the PR.
24. **Acceptance test** `count_holds_no_rows` (decision 22), run with the others in
    `memory_limit_acceptance_test.rs` on the bench machine; `smoke.yml` already runs the file.
25. **Docs:** `CHANGELOG.md` (decision 23), `ROADMAP.md` (#551 done, #473 open for #306),
    `README.md` (any executor wording), `CLAUDE.md` and `docs/TEST_COVERAGE.md` counts; the
    docs-site PR (decision 24). Full suite plain and with `--features test-hooks` on the bench
    machine; Codex review.
26. **After merge** (by hand, not in the PR): #473 checklist ticked with links; #551 closed;
    `minigraf-python` and the other bindings' `v3` pins moved to the merge, CI green (spec §7);
    decision 25's audit.
