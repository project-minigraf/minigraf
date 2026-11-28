# Spec — 008-incoming-ref-lookup (#924): bounded typed incoming-reference lookup in v3 write transactions

**Mission Type:** code (plus API docs in three repos)

**Base branches:** `project-minigraf/minigraf` **`v3`**; `project-minigraf/minigraf-java` **`v3`**; `project-minigraf/minigraf-docs` `main`. No `main` (v2.x) backport: this is a new feature, not a data-integrity fix.

**Deliverables (PRs):**
1. Core PR on `v3` — API, implementation, tests, in-repo docs, a copy of this spec at `docs/superpowers/specs/2026-11-28-924-incoming-ref-lookup-design.md`.
2. minigraf-java PR on `v3` — binding method, tests, README, core pin bumped to the core PR's merge commit (opened or finalised after PR 1 merges).
3. minigraf-docs PR — v3-scoped docs (`<!-- @since v3.0.0 -->`).

## 1. User Intent

> #924: add a bounded typed incoming-reference lookup on the v3 write transaction. v3.0.0 is required for the LDBC SNB benchmark (JNI DEL7). Cover the small-write pending path, the large-write working tree, the durable VAET path and WAL-recovered data. It needs two budgets: one on the links returned and one on the records examined, with an explicit exhausted outcome. Include the Kotlin/JVM binding exposure DEL7 needs.

> [Binding scope, after the Planner found that the benchmark's JNI bridge (`snb_minigraf`) calls core Rust directly] Core plus minigraf-java, matching what DEL7 needs.

> [Valid time] Any valid window — every ref asserted at the current transaction time, including future-dated and expired ones. Safest for a cascading delete, since none are left dangling. Differs from a default query.

## 2. Goal

A cascading delete (LDBC SNB Interactive v2 DEL7) must find every live typed reference **to** an entity — across attributes it did not anticipate — inside the same write transaction that will apply the delete, and must be able to refuse the delete when discovery cannot finish cheaply. Today the only route is an unbound-attribute query that falls back to a whole-EAVT scan and whose `:limit` does not bound work. Add a read-only lookup on both Rust write-transaction types and on the JVM transaction object that, given a target entity UUID, returns the complete set of live incoming `(source entity, attribute)` pairs, or an explicit *exhausted* outcome when either a links budget or a records-examined budget would be exceeded. Its record enumeration must depend only on the target's own incoming history — no scan, copy or re-index proportional to unrelated facts (bounded index navigation, whose depth may grow with the tree, is allowed) — in every place the transaction's facts can live: durable VAET, pending (uncheckpointed, including WAL-recovered), staged (small transaction), and large-transaction working trees.

## 3. Context & Investigation Findings

All core references are `v3` @ `33b4b165c`. minigraf-java references are `origin/v3` @ `0a2d4d65` (core pin still `32d71ce0`); start the binding work from that or a later `origin/v3`, not an older local branch. Sources: Claude-01, Codex-01..03.

### Issue and caller
- #924 (milestone v3.0.0) and ldbc-benchmarks #17 ("Bounded JNI delete prerequisite") list the required behaviour: exact typed Ref, no Keyword alias, unknown target → empty without dictionary mutation, staged retractions shadow older assertions, WAL-recovered data included, both budgets, distinct exhausted outcome, no whole-index fallback, overflow detected before mutation.
- The benchmark's JNI lane loads its own Rust crate (`ldbc-benchmarks/crates/snb-minigraf/src/bridge.rs`) which calls `db.begin_write()` (borrowed `WriteTransaction`, `examples/support/deep_delete.rs:33`). minigraf-java (`MiniGrafWriteTransaction`, UniFFI/JNA over `OwnedWriteTransaction`, `minigraf-java/src/lib.rs:1003-1096`) is the published JVM surface. Both are in scope per the user. The benchmark repo's core pin stays unchanged by this mission.

### Why a query cannot do this
- `src/query/engine/scan.rs:745-764`: `[?e ?a <ref>]` with unbound entity and attribute scans EAVT from the empty prefix; pending shadow setup is built first. `:limit` caps rows, not work.
- `scan.rs:~690`: query value matching treats a Keyword and a Ref to its entity as equal in some positions (#744); this lookup must not. A generic query is therefore not a valid oracle for tests.
- `src/db.rs:1245-1250`: `fact_log` refuses use during an active write.

### Transaction read state
- `TxState` (`db.rs:2008-2022`) is shared by `WriteTransaction` and `OwnedWriteTransaction`: `staged: Staged` while small, `large: Option<LargeTx>` once large (native file databases only).
- `TxState::read_snapshot` (`db.rs:2115-2128`): large → `OnDiskReader` over working trees (`persistent_facts/large_tx.rs:213-227`, own dictionary cache; contains committed + pending + finished statements, and `with_committed` empties the pending/staged view, `graph/storage.rs:372-383`); small → database snapshot + `with_staged(staged.read_view())`.
- A transaction becomes large only on a file database (`db.rs:2070-2081`). **In-memory transactions never become large**, so their staged set is unbounded.
- `Staged::read_view` (`db.rs:2428-2439`) clones every staged fact and `Snapshot::with_staged` (`graph/storage.rs:362-370`) rebuilds a full `PendingStore` from them **per read** — O(staged) setup that would make any lookup built on it linear in unrelated staged data.
- Promotion to large (`db.rs:2135-2175`), rejected statements, rollback and retry must leave no stale lookup state.

### Pending store
- `PendingStore` (`graph/storage.rs:59-129`) holds uncheckpointed facts with `Indexes` of **EAVT and AEVT only** (`storage/index.rs:194-212`; the "four indexes" comment at `storage.rs:167-169` is stale). No target-keyed pending access exists. `push`, `extend`, `of`, `load_facts` (`:845-855`, used by WAL replay at `db.rs:932-961`) and checkpoint publication (`:1002-1031`) are all insertion/lifecycle points.
- An in-memory database has `committed: None` (`storage.rs:689`): every fact is pending for its lifetime.
- #762 (bbtree pending indexes) is v3.next; this mission must not depend on it.

### Durable and large paths
- VAET key = `target eid ‖ attr iid ‖ source eid ‖ tx↓ ‖ vf ‖ vt ‖ asserted`, only for scalar `KeyValue::Ref` (`storage/keys.rs:779-812`). Keywords and sets never get VAET keys. No key-format change is needed.
- `KeySource::live_keys` / `OnDiskReader::live_walk` (`storage/mod.rs:555-574`, `storage/reader.rs:138-296`): `max` bounds **emitted** keys at triple boundaries; dead triples cost visits without counting; legacy winning groups can buffer several windows; seek-after-8-skips helps one long history but not many distinct dead triples. `LogSource::walk` rejects VAET (`reader.rs:441-453`).
- Durable and large paths use the same reader type; source UUIDs and attribute idents must be decoded through the selected reader's dictionary (a source or attribute first written in the large working tree is absent from the committed dictionary).
- `Dict::eid_of` (`storage/dict.rs:128-132`) is a read-only lookup. Absence in the committed dictionary says nothing about pending/staged data.

### Visibility rules to reuse
- Per triple `(e, a, v)`, the newest transaction decides; within it a tied retraction beats assertions; earlier windows never reappear (`graph/storage.rs:1067-1089`; #435/#477; design `2026-10-07-write-validation-and-statement-order-design.md` §1). Across layers, staged > pending > committed (`scan.rs:765-775` shadowing). Within one transaction, the last statement that wrote a triple decides (`db.rs:2443-2448`, `keep_last_statement`).
- Because the user chose "any valid window", valid-time filtering does **not** apply, so the staged `now` / `VALID_FROM_USE_TX_TIME` subtleties (`db.rs:2428-2439`, `storage.rs:362-368`) do not affect the result.

### JVM binding
- `minigraf-java/src/lib.rs:1020-1096`: `MiniGrafWriteTransaction` wraps `Mutex<Option<OwnedWriteTransaction>>`; finished → `API-018`; callback reentry is refused (`INT-001`) before the mutex (`:1040-1074`); counts are signed `i64`, bad counts/UUIDs → `API-017` (`:75-109`); Ref payloads are UUID strings (`:280-332`).
- Core pin is `32d71ce0` (`Cargo.toml:17-20`); the binding needs a pin containing the feature, and the bump brings in every core change since (bindings' CI must stay green).
- Tests: `java/src/test/kotlin/.../TransactionTest.kt`, `java/src/test/java/.../JavaInteropTest.java`, `consumer-test/`. Generated Kotlin is a build artifact.

### Budgets and size
- Binary-size gate on v3 is **1,250 KiB** (`.github/workflows/binary-size.yml:17`, 1,280,000 bytes; `PHILOSOPHY.md:126`). `TandemKit/Planner.md`'s 1,230 KiB is stale.
- Last error codes on v3: API-027, QRY-015, STG-048. No new code is expected.

## 4. Acceptance Criteria

Test names are suggestions; equivalent tests are fine. "Live" and "examined record" are defined in §5.

**Core API (Rust)**
1. **Both transaction forms.** `WriteTransaction` and `OwnedWriteTransaction` expose the same lookup (proposed `incoming_refs(target: EntityId, limits: IncomingRefLimits)`) with the same receiver style as `execute`, returning `Result<IncomingRefs, MinigrafError>`. It is available on every target where those types compile (native and WASI `wasm32`); `BrowserDb` is excluded. New public types are exported from the crate root and are `#[non_exhaustive]`; external code can still build the limits (constructor or builder, Generator's choice) — shown by a rustdoc example or an external integration test that constructs limits, calls the lookup and matches both outcomes.
2. **Outcome shape.** The outcome is either *complete* — every live link plus the number of records examined — or *exhausted* — which budget (links or records) and the records examined so far, with **no link list**. Exhausted is distinguishable from complete-empty and from an error without parsing text. Test: `incoming_refs_outcomes_are_distinct`.
3. **Exact typed results.** With several sources, several inbound attributes (including one the caller does not name), a self-reference, a same-spelling Keyword value, a UUID-looking String, a `Value::Set` containing the target's Ref and non-Ref values on the same attributes, the result is exactly the scalar-Ref links. Run on in-memory and file databases. Test: `incoming_refs_exact_types_and_attributes`.
4. **Unique pairs.** Each `(source, attribute)` appears once regardless of duplicate statements, history or several surviving legacy windows; two attributes from one source are two links. Tests: `incoming_refs_unique_pairs`, `incoming_refs_legacy_ties`.
5. **Valid time ignored (user decision).** A ref whose newest version is an assertion with a future `valid_from`, or a `valid_to` already in the past, **is** returned; a ref whose newest version is a retraction is not, whatever the windows. A newer assertion with an expired window still shadows an older one (no double-count, no resurrection). Test: `incoming_refs_any_valid_window`.
6. **Current write view.** Inside one transaction: a staged assertion is visible to a later lookup; a staged retraction hides a committed, pending or earlier-staged assertion; retract-then-reassert is visible; the last statement wins per triple; a rejected statement (e.g. API-011) contributes nothing. Tests: `incoming_refs_statement_order`, `incoming_refs_rejected_statement`.
7. **Unknown and dangling targets.** A UUID never seen anywhere → complete, zero links, and the transaction's later commit, `checkpoint`, reopen and `verify()` behave as if the lookup never ran (no dictionary id allocated in the committed or working dictionary). A target referenced only by pending or staged facts, or with no facts of its own, is found. Tests: `incoming_refs_unknown_target`, `incoming_refs_pending_only_target`, `incoming_refs_dangling_target`.
8. **Read-only and retryable.** Complete and exhausted lookups stage nothing, write no WAL entry, take no `tx_count`, cause no promotion or checkpoint, and leave the transaction usable; a retry with larger budgets, further writes, commit, rollback and drop behave as without the lookup. Core `commit`/`rollback` keep consuming the transaction (`db.rs:2610-2650`), so there is no finished Rust object to call; API-018 applies only to the JVM wrapper (AC17). Tests: `incoming_refs_read_only_and_retryable`, `incoming_refs_exhaustion_then_rollback`.
8a. **Failed handles: a lookup is a read.** A prior checkpoint failure (later writes STG-046) or WAL-append failure (later writes WAL-007) does not by itself make the lookup fail with STG-046 or WAL-007 — as queries keep working in that state (`db.rs:1391-1399`; `refuse_if_failed` at `db.rs:704-719` guards writes only). The lookup stays subject to its own budgets and returns the I/O, decode and lock errors its own reads hit; it need not fail or succeed together with a query, which reads different indexes. Existing query behaviour and rollback availability are unchanged. Test (fault-injected, distinct from an API-011 rejected statement): `incoming_refs_on_failed_handle` — in each failure state, with readable data and sufficient budgets, the result equals the independent exact-typed, any-valid-time oracle.

**Storage coverage (each layer, same expected sets)**
9. **Durable VAET:** committed + checkpointed, and after close/reopen. **Pending:** committed, not checkpointed. **WAL-recovered:** a child process commits and exits without checkpoint (as `tests/wal_test.rs:700-722` does); after reopen the links are returned, a small transaction can retract a recovered link (hidden), and after a forced promotion to large it stays hidden. **Large working tree:** a file transaction forced large with a small `TxOptions::chunk_facts`, including links asserted/retracted in earlier statements, sources/attributes first created inside the large transaction, and a lookup before and after promotion returning the same set. **In-memory database:** pending + staged. **Rollback:** a new transaction sees the pre-transaction set; **commit:** same set before/after checkpoint and reopen. Tests: `incoming_refs_checkpoint_reopen`, `incoming_refs_wal_recovery`, `incoming_refs_recovered_then_promoted`, `incoming_refs_promotion`, `incoming_refs_rollback`.
10. **Model-based check.** A proptest over random sequences of assert/retract statements, commits, rollbacks, checkpoints, reopens and forced promotions on a small universe (a few sources, attributes, targets, Keyword aliases) compares every lookup with an independent exact-typed, any-valid-time model (not a Datalog query). Test: `incoming_refs_model` (may live beside `tests/model_based_test.rs`).

**Budgets**
11. **Links budget boundary.** Given a sufficient `max_records`, with N live links: `max_links = N` → complete with N; `N − 1` → exhausted(links); `0` with no links → complete-empty; `0` with one link → exhausted(links). Huge caps (`usize::MAX`) work without allocating to the cap or overflowing. Tests: `incoming_refs_exact_link_cap`, `incoming_refs_link_overflow`, `incoming_refs_zero_and_huge_limits`.
12. **Records budget.** The lookup never examines more than `max_records` records in total across all layers; `records_examined` in either outcome is ≤ `max_records`. If it cannot establish the complete set within the budget it returns exhausted(records), including mid-way through one triple's version group. Tests: `incoming_refs_budget_shared_across_layers`, `incoming_refs_legacy_group_budget`.
13. **History exhaustion.** A target with zero or few live links but ≥1,000 distinct retracted or superseded incoming pairs, with `max_records` well below that, returns exhausted(records) — in pending, durable and large-working-tree representations. Separately, one pair with a long history (≥1,000 versions, newest an assertion): on a fixed fixture and representation, first take a **complete** run with a generous `max_records`, and let B be its `records_examined`; then the same lookup with `max_records = B` is complete and correct, and with `max_records = B − 1` it is exhausted(records) or complete-and-correct (an implementation may adapt its traversal to the budget; B need not be minimal). Skipping old versions by seek is allowed but not required. In every low-budget history test, no uncharged processing or materialisation of the target's records happens before exhaustion (checked by AC14's independent counter). Tests: `incoming_refs_dead_pairs_exhaust`, `incoming_refs_one_pair_long_history`.
14. **Independence from unrelated data.** At fixed target data (e.g. 3 live links) and fixed budgets, adding 100,000 unrelated facts — scalar facts, Keyword values, and refs to other targets — leaves `records_examined` exactly unchanged, in each of: durable, pending, staged (in-memory database, one transaction), and large working tree. Independently of the public count, test-only instrumentation (mechanism is the Generator's choice, e.g. `cfg(test)` or the `test-hooks` feature) counts the actual records the lookup enumerates, copies or indexes in **every** representation — index entries advanced over in durable and working-tree VAET (whether or not they match the target), and pending/staged facts touched during setup — and it too stays unchanged when the unrelated facts are added, for the first lookup in a transaction as well as repeats — except for a fixed number of prefix-end probes per participating range (e.g. reading the one successor key of another target that now sorts after this target's range, where before the range ended at end-of-index), which the instrumentation reports separately or allows explicitly. No enumeration continues past the target range and nothing grows with the number of unrelated facts. The fixture covers both the end-of-index and the non-matching-successor case. Bounded tree routing and page decoding may change with tree shape and are not asserted equal. Test: `incoming_refs_unrelated_growth`.
15. **Exhaustion precedence.** Exhausted(links) is reported only after N+1 distinct live links have been established; otherwise a lookup that runs out of records reports exhausted(records). Never a partial list.

**Owned transaction**
16. A lookup works after moving an `OwnedWriteTransaction` to another thread; concurrent readers still see only their committed snapshot; existing reentry/thread guards (INT-001) are unchanged. Tests extend `tests/owned_tx_test.rs` (`owned_incoming_refs_cross_thread` and guard cases).

**JVM binding (minigraf-java `v3`)**
17. `MiniGrafWriteTransaction` exposes the lookup (proposed `incomingRefs(target: String, maxLinks: Long, maxRecords: Long)`) returning a typed outcome (complete with links + records examined / exhausted with budget + records examined) and typed links (`source` canonical UUID string, `attribute` string). Invalid UUID, negative or unrepresentable budgets → API-017; finished transaction → API-018; callback reentry → INT-001 without blocking. Java callers distinguish outcomes without JSON or message parsing.
18. Kotlin (`TransactionTest.kt`) and plain Java (`JavaInteropTest.java`) tests cover complete, complete-empty, both exhaustion reasons, Keyword exclusion, retry with larger budgets, commit/reopen, rollback, finished-transaction and reentry errors. The packaged `consumer-test` calls the API from the built jar. Native shim `cargo test`, Gradle build and every existing CI job pass with the bumped core pin.

**Compatibility, size, performance, docs**
19. No file-format, WAL, grammar or Datalog change; no existing public API changes; the golden-file corpus, model-based, WAL, crash and owned-transaction suites pass; all existing tests pass; clippy and fmt clean (including the wasm32 lint job).
20. Binary size stays within the **1,250 KiB** gate (the PR reports before/after bytes).
21. Write-path cost of any new maintained pending/staged target structure is measured on the bench machine against the v3 base. The measured set includes (a) the existing insert/transaction, WAL-replay and checkpoint benchmarks as controls (they write integer values: `benches/minigraf_bench.rs:105-212`, `benches/minigraf_bench/chunked_transactions.rs`), and (b) timed workloads whose measured operation writes `Value::Ref` facts — pending insertion, small-transaction staging, WAL recovery, and checkpoint/promotion where the new structure is involved — using an existing fixture if one writes refs, else a small new bench. The PR's measurement report names each workload ID and scale, the statistic (median), base and branch numbers, and pending-store memory per ref fact. A median regression above 5% on any of them is reported to the user and needs their acceptance before the PR is marked ready.
22. Docs in sync: `CHANGELOG.md` (v3 Unreleased: new API), `CLAUDE.md` (test count, module notes for any new pending structure, fix the stale "four indexes" wording if touched), `docs/TEST_COVERAGE.md`, `ROADMAP.md` (#924), `README.md` (if it lists transaction methods), rustdoc on the API (semantics, both budgets, exhaustion, "require complete before deleting", scalar-Ref only, valid time ignored); minigraf-java `README.md` / `java/README.md`; docs site `content/database-tools.md` or `content/cookbook-application-workflows.md` (usage) and `content/architecture.md` (pending target access), all under `<!-- @since v3.0.0 -->`. `docs/ERROR_REFERENCE.md` only if an error meaning changes.

## 5. Edge Cases & Boundaries

### Definitions
- **Live link** — a pair `(source, attribute)` such that, in the transaction's current view (staged over pending over committed; or the working trees when large), the newest version of the triple `(source, attribute, Value::Ref(target))` is an assertion. Valid-time windows are ignored. Only stored facts; rule-derived facts are not links. Only a top-level `Value::Ref` — not a Keyword with the same spelling, not a String, not a Ref inside a `Value::Set`.
- **Examined record** — one stored fact version (index entry or pending/staged fact) belonging to the target's range that the lookup inspects, in any layer, whether it turns out live, superseded or retracted; charged when inspected, including lookahead used to detect overflow or settle a version group. Tree navigation, dictionary lookups and the prefix-end check are not records, but must not hide a scan: AC14's independent enumeration counter checks every representation. Beyond a fixed number of prefix-end probes per range, advancing over an index entry outside the target's range is enumeration, not navigation. Every inspected record of the target's own history, and lookahead used to settle a target version group, is charged to the budget. Versions skipped by an index seek without inspection are not charged. The budget is one total across all layers.
- **Records examined counts may differ** between representations (pending vs checkpointed vs working tree) for the same logical data; results must not.

### Boundaries
- `max_links = 0` / `max_records = 0` are real limits, not "unlimited". `max_records = 0` gives complete-empty only when emptiness is known without inspecting a record (e.g. the target is absent from every layer's dictionary/index); otherwise exhausted(records).
- Exactly `max_records` examined and the set is complete → complete. Never examine `max_records + 1`.
- No arithmetic overflow at `usize::MAX`; no allocation proportional to a requested cap.
- Result order is **unspecified**; tests compare sets.

### Transaction and lifecycle
- The lookup sees every earlier completed statement of the transaction, not a begin-time snapshot.
- Exhaustion does not undo the caller's own staged writes; the guarantee is that the lookup itself mutates nothing. DEL7 is expected to run discovery before staging its deletes.
- Cache warming is allowed; dictionary id allocation, WAL writes and staged changes are not.
- I/O, decode and lock errors are errors — never an empty complete or an exhausted outcome, never a partial list.
- Rejected statements, promotion, rollback, retry after exhaustion, checkpoint publication and WAL replay must leave any auxiliary lookup state consistent with the facts.

## 6. Key Decisions

| Decision | Rationale |
|---|---|
| Dedicated method, not a query-planner change | Needs an exhausted outcome and a work budget; Datalog `=` treats Keyword≈Ref (#744); planner changes would not meet the bound for staged/pending data. |
| Any valid window (user) | Cascading delete must not leave future-dated or expired-but-asserted refs dangling. Differs from a default query; documented. |
| Complete-or-exhausted batch, no partial links, no cursor | DEL7 needs a complete set before deleting; a cursor adds lifetime and continuation semantics nobody asked for. Retry with larger budgets instead. |
| Outcome enum, not error codes | Exhaustion is an expected outcome, not a failure; no new error code. |
| `records_examined` public | Lets the benchmark record bounded-visit evidence; tests assert it. |
| Scalar Ref only | Matches VAET; sets of refs would need another index. |
| Order unspecified | DEL7 doesn't need it; avoids a sort contract. |
| Pending/staged need maintained target-local access | Only way to meet AC14 for in-memory and pending data; cost measured (AC21), independent of #762. |
| Core + minigraf-java only (user) | DEL7 uses core directly; minigraf-java is the published JVM surface. Other bindings (Python, Android, Swift, Node, C, WASI REPL, browser) are not in this mission. |
| Base `v3`, no backport | New feature in the v3.0.0 milestone. |

## 7. Out of Scope

- Cascading delete itself, referential-integrity enforcement, relationship allowlists.
- DEL7 integration, benchmark pin changes, reference comparison, qualification.
- Datalog/grammar/planner changes, `:limit` semantics, AVET work (#518), pending-index redesign (#762).
- `:as-of` / chosen valid-time variants, multi-target lookup, continuation tokens, refs inside sets.
- Other bindings and `BrowserDb`; binding-template (unless the Generator finds the template must change for the Java binding to build — then report it).
- A database-level (non-transaction) lookup.

## 8. Possible Directions & Ideas (non-binding)

- The durable and large paths can share one VAET walk over `OnDiskReader`; `live_walk` (`reader.rs:156`) shows the triple/tx-group logic but counts only emitted keys.
- For pending and staged data, a target-keyed index maintained alongside EAVT/AEVT (ref facts only) is one option; another is keeping staged facts in a persistent indexed store instead of rebuilding per read. Either must survive promotion, rollback and checkpoint publication.
- `tests/model_based_test.rs` and `tests/reference_model/` show the existing model-based pattern; `tests/wal_test.rs:700-722` the child-process crash pattern; `tests/owned_tx_test.rs` the owned-transaction pattern.
- Measure on the bench machine (`benches/minigraf_bench.rs`); bench profile is opt-level z, so compare base vs branch on the same profile.
- The `test-driven-development` skill fits the budget/boundary tests.
