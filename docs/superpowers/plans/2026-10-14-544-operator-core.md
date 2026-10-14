# Operator core, Scan and oracle harness (#544) — Plan

Spec: `docs/superpowers/specs/2026-10-11-streaming-engine-design.md` §5, §6.1 (without
the joins), §6.3, §9 (#473, part 2 of 9). Folds in #518 (branch `perf/518-avet-lookup`,
fe545dc) and #530. PRs target `v3`; no closing keyword for #473. #518 and #530 close by
hand (`gh issue close`) when the PR that fixes each merges.

| PR | Steps | Branch |
|---|---|---|
| 1 | 1–6: key-level reads (#518 rebased), cells and batches, `Scan`, `Project`, planner v1 for plain single patterns, routing, oracle | `feat/544-operator-core` |
| 2 | 7–9: `Filter`/`Bind`, pseudo-attribute columns, cross-kind constants | `feat/544-exprs` |
| 3 | 10–12: lazy cursor with `close` and counters, measurement, docs | `feat/544-lazy-cursor` |

Each PR leaves `v3` with no unused code, and updates the test counts and `CHANGELOG.md`
for what it ships.

## Design decisions not fixed by the spec

1. **Which queries run on the new engine (#544).** `engine::plan` returns `None`, and the
   query runs on the existing executor, unless all of these hold:
   - no rule invocation, `not`, `not-join`, `or`, `or-join`;
   - exactly one pattern with a real attribute; any other pattern is a pseudo-attribute
     pattern (PR 2) whose entity is the same term as the real pattern's, and which
     `optimizer::plan` orders after it (if it is ordered first, today's matcher binds it
     over every fact of the entity, which is a join: #545);
   - `:find` holds plain variables only (aggregates and windows: #547);
   - `:as-of` is absent or a counter (`AsOf::Timestamp` filters on `tx_id`, not a key
     bound; it stays on the executor until #551);
   - no transaction overlay (`facts_override`): reads inside a `WriteTransaction` stay on
     the executor until #550 (§8.4);
   - each pattern position is a variable, `_`, or a constant kind the PR handles (PR 1:
     entity keyword/UUID; attribute keyword; value string, integer, boolean, `nil`;
     PR 2 adds value keyword, UUID and float, §5.4).
   The check runs after `INT-026`, `INT-027`, `QRY-007` and `check_find_functions`, so
   every error of today's executor is raised first, by the same code.
2. **Cells.** `enum Cell { Eid(u64), Iid(u64), Val(Scalar) }`,
   `enum Scalar { Null, Bool(bool), Int(i64), Float(f64), Str(Arc<str>), Long(ValueRef) }`.
   Pseudo-attribute values are `Val(Int)` cells (and `Val(Null)` for `:db/valid-at` under
   `:any-valid-time`), not a separate `Meta` variant: a variable bound by a pseudo-attribute
   and by a real value must compare as today's `Value::Integer`s do, and one integer
   variant gives that without a cross-variant rule. Deviation from spec §5.2 (`Meta(i64)`).
   A pending string over 64 bytes is `Str`; committed ones are `Long`. Equal long strings
   on both sides only need one form when a join compares them, so canonicalising (a DICT
   long-value lookup) is #545's.
3. **Batches.** `struct Batch { len: usize, cols: Vec<Vec<Cell>> }`, at most
   `BATCH_ROWS = 1024` rows; the planner gives each variable a column index
   (`Slots`). An operator is `trait Operator: Send { fn next_batch(&mut self) ->
   Result<Option<Batch>>; fn close(&mut self); fn counters(&self) -> Counters }`.
   `open` is the constructor.
4. **Ids (§5.3).** `IdSpace` per query: DICT ids from the committed reader, provisional
   ids from `next_eid` / `next_iid` of the snapshot's meta upwards (in-memory: from 1),
   with forward and reverse maps. `Project` turns ids back through the DICT or the
   reverse map. Counting its bytes is #549's, with the rest of the accounting (no
   reader of the count before then).
5. **Committed reads stay lock-free between batches.** `CommittedReader` gains a
   key-level method:
   `live_keys(&self, index, prefix, resume: Option<&[u8]>, as_of, max, out: &mut Vec<KeyFact>) -> Result<Option<Vec<u8>>>`.
   It is `OnDiskReader::live_scan` made resumable: it stops only at a triple boundary
   once `max` entries are out, and returns the triple to resume after (seek to
   `triple ‖ 0xFF`), so a triple's net-assert decision never spans two calls. A
   `LeafCursor` borrows the reader, so it is rebuilt per batch by seek (one root-to-leaf
   walk per 1,024 rows), as `FactLog` already does. `live_scan` itself becomes a loop
   over `live_keys`; `live_facts` keeps its signature for the executor. Plus the
   DICT accessors the engine needs: `eid_of`, `iid_of`, `uuid_of`, `name_of`,
   `tx_id_of`, `long_value`, `next_ids`. The default trait impls cover readers
   without keys (tests' `MemoryReader`, if any): `None` → no committed side.
6. **Pending facts and net-assert.** Every pending fact is newer than every committed
   one, so a triple with any pending record in the snapshot (position `< n`,
   `tx_count ≤ as_of`) is decided by its pending records alone, by the rule of
   `net_asserted_facts`; a triple without one is decided by the committed keys. `Scan`
   therefore:
   - at open, if there is a committed reader and the pending range is non-empty, builds
     `shadow: HashSet<Vec<u8>>`, the id-form triple bytes of the pending triples in its
     range whose entity, attribute and ref/keyword value are all in the DICT (others
     cannot equal a committed triple). Its bytes are counted as query state. For an
     in-memory database (no reader) it is empty and costs nothing;
   - reads committed keys, dropping a triple in `shadow`;
   - then reads the pending range (EAVT/AEVT `BTreeMap` resumed by key under a short
     read lock; AVET/VAET shapes read the attribute's AEVT range, or every pending fact
     when the attribute is unbound, and filter), grouping by triple and applying the
     net-assert rule per group.
   Output order is committed (index order) then pending (pending index order). The
   planner records no order in #544: nothing consumes it before `Sort` elision (#547),
   which needs a merged order and will add it. Deviation from spec §6.1 ("output in the
   index's order").
7. **Index choice** (`optimizer::select_index` extended, used by `Scan`): entity bound →
   EAVT (`e` or `e a` prefix); attribute and value bound → AVET (`a v`, #518's prefixes
   and value kinds); value a ref with attribute unbound → VAET; attribute bound → AEVT;
   otherwise EAVT whole. Residual positions (e.g. the value of `[:e ?a "x"]`, a repeated
   variable as in `[?x :a ?x]`, compared with today's exact `Value` equality) are
   checked on decoded keys before any id is translated.
8. **#518.** Its key prefixes (`attribute_value_prefix`, `attribute_string_prefix`,
   `attribute_kind_prefix`) and its tests come over; `Scan` uses them. Its changes to
   `selective_fact_fetch` and the new `live_facts` scans do not: the executor stays as
   it is, as the oracle. `tests/live_reads_test.rs` from the branch is rewritten to
   compare engine scans with whole-attribute reads. The #518 envelope rows move to PR 3's
   measurement.
9. **Oracle (§9).** A `test-oracle` Cargo feature (and `cfg(test)`) turns on a hook in
   the one place queries are routed (decision 10): every query the new engine accepts
   is also run by `DatalogExecutor` on the same `Snapshot` (`set_snapshot`), and the rows
   are compared as bags (rows keyed by `encode_value`, NaN canonical); a mismatch panics
   with both row counts and the query text only (no values: CodeQL). The existing suites
   run under the feature in a new CI job (`rust.yml`: `cargo test --features test-oracle`),
   which is how the model-based, property, golden-corpus and integration suites become
   differential without being edited. `tests/engine_oracle_test.rs` adds targeted
   shapes: every index choice, `:as-of`, retraction and re-assertion across a checkpoint
   (pending shadowing committed), keyword-as-ref and UUID-as-keyword values, floats
   within epsilon, long strings, in-memory and file-backed, before and after reopen.
10. **Routing.** Every query reaches `DatalogExecutor::execute_query` (`Minigraf::query`
    and `execute`, `PreparedQuery` after substitution, `BrowserDb`), so the engine is
    tried there, once: after the checks every query passes first, `engine::plan` and
    `engine::run` on the executor's snapshot, and the executor's own path
    (`execute_query_on_executor`) on `None`. `PreparedQuery` therefore plans per
    execution; keeping the operator tree across executions (spec §6.3 last paragraph)
    is #545's, when planning has join order worth caching.
11. **`Limit` / `Offset`.** Nothing in the query language reaches them until `:limit`
    syntax (#306), so adding them now is unused code. They move to #306, which is
    already next after #473; `close` propagation, which the cursor uses, stays here
    (PR 3). Deviation from the issue's checklist; the issue gets a comment.

## Steps

### 1. Key-level committed reads (PR 1)
- Rebase the useful parts of fe545dc (decision 8): `keys.rs` prefixes and their unit
  tests.
- `CommittedReader::live_keys` and the DICT accessors (decision 5); `live_scan` rewritten
  over `live_keys`.
- Unit tests (`reader.rs`): `live_keys` with `max` = 1, 2, 7, 1024 concatenated equals
  `live_scan` over random histories (the existing random-history differential test
  extended); a triple with > `SEEK_AFTER` history entries straddling a batch boundary;
  `as_of` cuts.

### 2. Cells, batches, ids (PR 1)
- `src/query/engine/{mod,cell,batch,ids}.rs` (decisions 2–4).
- Unit tests: provisional ids are stable within a query, never collide with DICT ids,
  round-trip through `Project`; byte counting grows with distinct ids.

### 3. `Scan` (PR 1)
- `src/query/engine/scan.rs` (decisions 6, 7): index choice, committed walk, shadow set,
  pending walk, net-assert, `:as-of` counter, `:valid-at` / default "now" from the
  snapshot / `:any-valid-time`, residual checks, output cells.
- Differential unit tests against `Snapshot::live_facts_of_scans` + the executor's
  valid-time step, over random histories with checkpoints between transactions, for
  every index choice.

### 4. `Project` and planner v1 (PR 1)
- `src/query/engine/{project,plan}.rs`: eligibility (decision 1, PR 1 subset), slots,
  `Scan` → `Project` tree; `Project` keeps the `:find` columns, drops rows with a `:find`
  variable no clause binds (as `extract_variables` does: such a query returns no rows)
  and turns cells into `Value`s for the rows it returns only (#530).

### 5. Routing and oracle (PR 1)
- Decision 10 (eager in PR 1: `engine::run` drains the tree into a `QueryResult`, so the
  cursor is unchanged until PR 3), decision 9 (feature, hook, CI job,
  `tests/engine_oracle_test.rs`).
- `cargo test --features test-oracle` over the whole suite locally; any mismatch is a
  bug in the engine, or a plan-dependent answer of today's executor (spec §5.4), which
  narrows eligibility and goes to #545.

### 6. PR 1 checks
- `cargo fmt`, clippy (native, `wasm`, `browser`/wasm32 targets), `cargo test`,
  `cargo test --features test-oracle`, binary size (`binary-size.yml` budget).
- #379 point queries and #518's `attribute_value` on the envelope at 100K/1M on the
  bench machine (`perf_envelope`): no regression on point queries; `attribute_value`
  at 1M near #518's 11 µs. Close #518 by hand after merge.

### 7. `Filter` and `Bind` (PR 2)
- `Filter` for `Expr { binding: None }`, `Bind` for `Expr { binding: Some }`: build a
  `Binding` of the expression's variables only (cells → `Value`), `eval_expr`, drop the
  row on `Err`; `Bind` turns the result back into a cell through `IdSpace`. Placement
  follows `optimizer::plan` (earliest point its variables are bound; never-bound
  variables → end → every row dropped, as today).

### 8. Pseudo-attributes (PR 2)
- `Scan` emits `valid-from`, `valid-to`, `tx-count` and, only when a pattern asks for it,
  `tx-id` (a DICT lookup per committed row; pending facts carry it) as `Val(Int)`
  columns; the planner wires each pseudo-attribute pattern to them (variable → that
  column, constant → a residual check, `:db/valid-at` → a constant column). `INT-026`
  unchanged.

### 9. Cross-kind constants (PR 2)
- Value keyword `:k`: AVET ranges for `Keyword(k)` and `Ref(edn_to_entity_id(:k))`
  (note: a keyword whose name parses as a UUID is that UUID, as `edn_to_entity_id`
  does, not its v5 hash; spec §5.3/§5.4 say "uuidv5", which the plan corrects).
- Value UUID `u`: `Ref(u)` range, plus the attribute's keyword range filtered by
  `edn_to_entity_id(name) == u`.
- Value float: the attribute's float range, filtered within `f64::EPSILON`.
- Entity and attribute positions keep today's rules (entity: keyword/UUID only;
  attribute: keyword only; other constants → no rows, as `match_component` gives).
- Oracle tests for each (the issue's "cross-kind matching rules pinned by oracle tests").

### PR 2 deviations
- `:db/valid-at` patterns stay on the executor. Its matcher binds `:db/valid-at` by
  scanning every fact of the entity (rows are a bag, one per fact), so it is a join, not
  a constant column (#545).
- A pseudo-attribute pattern runs on the engine only if its entity is the scan
  pattern's entity term (the same variable, or a constant naming the same entity), it
  is planned after the scan pattern, and no expression rebinds that variable: the
  executor's hidden-key fast path applies only then; otherwise it joins (#545).
- Steps run in `optimizer::plan` order, and a binding expression overwrites a bound
  variable as the executor does (`[(+ ?v 1) ?v]` is accepted by the parser).
- `INT-026` and `QRY-007` are raised before the engine runs, by the executor's own
  checks (`check_clauses`); the executor path keeps its order of errors.
- `attribute_string_prefix` is not ported: an exact long string resolves its value ref
  through the DICT and uses `attribute_value_prefix`, so nothing would call it.
- With the attribute unbound, cross-kind constants read EAVT (the entity's prefix, or
  the whole index) with a residual test, not VAET: a keyword or UUID constant also
  matches keyword values, which VAET does not hold.
- `KeySource::tx_id_of` added for `:db/tx-id` on committed rows (decision 5 listed it).

### 10. Lazy cursor (PR 3)
- `Cursor` holds the operator tree and the snapshot; `next_batch(max_rows)` pulls
  internal batches until `max_rows` rows are ready; errors surface from `next_batch`
  (already `Result`). `size_hint` stays `(0, None)` for engine cursors. `close` and drop
  call `close` down the tree.
- `Counters` (rows in/out, batches) on every operator; acceptance test (spec §10):
  close after the first batch, counters show no further rows read.
- Spec §10 acceptance test for snapshots: a lazy cursor's answer is unchanged by writes
  and page-freeing checkpoints between batches (on top of #543's test, now with real
  laziness).
- `tests/cursor_test.rs` and binding suites unchanged.

### 11. Measurement (PR 3)
- #394 envelope at 100K, 1M, 10M on the bench machine, idle: point queries, attribute
  scan, `attribute_value`; peak RSS of an attribute scan over 1M facts streamed in
  batches of 1,000. `docs/BENCHMARKS.md`: #518 note and rows.

### 12. Docs (PR 3)
- `CHANGELOG.md` (v3.0.0): single-pattern queries stream; `[?e :a C]` reads the AVET
  range (#518); ids until projection (#530).
- `CLAUDE.md` module list (`src/query/engine/`), test count; `ROADMAP.md`;
  `docs/TEST_COVERAGE.md`; `README.md` count.
- Docs site (`../minigraf-docs`): `content/architecture.md` query-engine section under
  `@since v3.0.0`.
- Comment on #544 for decision 11 (`Limit`/`Offset` → #306), and on #306.

## Bindings

No public API change and no new error code, so no binding cascade.
