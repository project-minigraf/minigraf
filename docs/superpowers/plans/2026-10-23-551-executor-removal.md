# Executor removal (#551) — Plan

Spec: `docs/superpowers/specs/2026-10-23-executor-removal-design.md` (merged as #714),
called "the spec" below. PRs target `v3`, merge commits. None uses a closing keyword for #473 or
#551; #551 closes by hand after PR 3.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–13: the engine runs every query (spec §3, §4) | `feat/551-engine-everywhere` |
| 2 | delete the executor's query path and the oracle, `test-hooks`, size gate (spec §5) | planned after PR 1 |
| 3 | measurement, acceptance, docs (spec §6, §7) | planned after PR 2 |

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
