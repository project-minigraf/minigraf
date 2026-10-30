# Spec — 001-shared-reader (#797)

**Mission Type:** mixed (code + performance evidence)
**Base branch:** `v3` (v3.0.0 milestone). Worktree `.worktrees/797-shared-reader`, branch `perf/797-shared-reader`, created from `v3` at `15badca89`. No `main` backport: this is size/performance work, not data integrity.

## 1. User Intent

> #797

> [How to proceed] Helper extraction + A/B (Recommended): spec and implement
> the non-generic helper in a v3 worktree, A/B it against the baseline and the
> Box prototype on the bench (longer point runs, reader creation, cold reads,
> concurrency, WASI/browser), PR only if the numbers hold.

> [Invoking the planner] run the review

> [Q1, regression margin] ≤2% upper bound, mean: one-sided 95% upper bound on
> the per-block mean latency ratio ≤ 1.02 for each point family, in two
> fixed-size campaigns plus a baseline-vs-baseline control. Every other
> workload ≤ 1.05. p50/p99 reported but not gating.

> [Q2, size bar] Native ≥2,000 B, none grow: at least 2,000 bytes smaller on
> the native CLI, and no measured artifact (WASI, browser raw/packaged,
> library consumer) larger than baseline.

> [Q3, Wasm runtime] User-local tools on bench: install Wasmtime and wasm-pack
> under ~ on the bench machine (no sudo, removed afterwards). Browser runs use
> its headless Firefox. Native cold reads via posix_fadvise eviction.

Issue #797 (child of the #772 size audit), summarized: `OnDiskReader<B>` repeats its read logic per backend even though the tree/cache primitives below it take `dyn StorageBackend`. The audit's boxed prototype saved 7,448 native CLI bytes, with a +3.91% median point p50 change in a 32-pair rerun. Deliver a small, evaluated change only if its tradeoffs are acceptable, and compare against extracting shared non-generic helpers. Preserve lock lifetimes, cache ownership, error ordering, UUID batching, temporal visibility, snapshot pinning, and API/format behavior. Run full suites, platform and coverage gates on the bench. Keep the size cap, coverage floors, MSRV and portability. Reject the change if reliable performance is compromised.

## 2. Goal

Remove the per-backend duplication of the committed-fact reader's logic without changing observable behavior, API, file format, configuration or dependencies, and with read performance within the user-approved non-inferiority margins (AC 13–14). Decide by evidence. Build three variants on one pinned v3 revision, toolchain and lock:
- **B**: unchanged base.
- **H**: the shared-helper extraction. This is the candidate to ship.
- **P**: the audit's boxed prototype. A comparison only, never merged.

Measure all three for size on every shipped artifact and for runtime on the bench machine. Open a PR for H only if it meets every gate below. Otherwise publish the evidence and leave production source unchanged.

## 3. Context & Investigation Findings

Paths are relative to the worktree, at `15badca89`.

### Reader and dispatch
- `src/storage/reader.rs:22-36`: `OnDiskReader<B>` holds a concrete `MutexStorageBackend<B>` plus generation metadata (roots, DICT root, id counters, `last_tx`, one `SharedDictCache`, one `OnceLock<TxOrder>`). Every method is in an `impl<B>` block (`:38`, `:79`, `KeySource` `:300`, `LogSource` `:436`, `CommittedReader` `:482`), so each backend monomorphizes a copy.
- **Backend calls are already dynamic.** Every use of `&self.backend` goes to `DictReader` (`dict.rs:77`, `:90`), `LeafCursor` (`btree.rs:626`) or `prefix_scan` (`btree.rs:533`). All of them hold or take `&dyn StorageBackend`. `PageCache` calls `backend.read_page()` through `&dyn` on a miss (`cache.rs:94-130`). `CommittedReader`/`KeySource` are themselves trait objects (`storage/mod.rs:370`, `:510`; `activate` returns `Arc<dyn CommittedReader>`, `persistent_facts.rs:294`).
- P (`docs/audits/772/evidence/patches/reader-erasure.patch`) boxes the same adapter. The source shows no added vtable call. It adds one allocation per constructed reader and changes specialization/inlining. Whether LTO devirtualizes anything in B that P or H loses needs generated-code or runtime evidence. *(Correction of Claude-01, which claimed P adds a pointer hop per call.)*
- `MutexStorageBackend::read_page` (`btree.rs:880-883`) locks `Mutex<B>` for one backend call only, never across a scan, a dictionary translation or a callback. Its `page_count`/`partial_page`/`holds_every_page` behavior also matters. Note that the nearby comment claiming every other method fails is not literally accurate.
- With capacity zero, `PageCache` loads the page inside the current cache read guard. Miss-path lock lifetimes are not uniform today, and this mission doesn't change them.

### When readers are constructed (from Codex-01, verified)

| Path | Construction |
|---|---|
| Open (incl. v7 migration), read-only open | one reader for the chosen meta (`persistent_facts.rs:94`, `:109`, `:125`, `:140`) |
| Dirty save/checkpoint | new reader per published generation; a clean save does not activate (`:433`, `:504`) |
| Index rebuild | new reader after commit (`:716`) |
| Large transaction commit | new reader (`large_tx.rs:315`) |
| Ordinary query/cursor/fact log | clones the installed `Arc`, pins separately (`graph/storage.rs:949`, `:966`) — **no construction** |
| Read command inside a large file-backed `WriteTransaction` | **a fresh reader per command** over the working roots, with its own DICT cache (`db.rs:1777`, `:1795`; `large_tx.rs:213-218`) |

The audit's warm loop opens before timing (`examples/perf_envelope.rs:835`, `:887`), so P's allocation cannot explain its steady-state point delta. Construction is measured separately, including repeated reads inside a large transaction.

### Which backend each artifact instantiates

| Artifact | Reader backend(s) |
|---|---|
| Native CLI | `FileBackend`, `MemoryBackend` (`main.rs:18`; `db.rs:573`, `:669`) |
| WASI CLI (`wasm32-wasip1`) | `MemoryBackend` only (`main.rs:7`) |
| BrowserDb Wasm | `BrowserBufferBackend` only, including `openInMemory` (`browser/mod.rs:36`, `:60`, `:85`, `:274`), page cache capacity 0 |
| Tests | also `CountingBackend`, `FaultInjectingBackend` (not shipped) |

So the native saving comes from two backends. Single-backend Wasm artifacts may save little or nothing. `Minigraf::in_memory` installs an initially empty committed reader and keeps later data pending (`db.rs:669-678`), so WASI CLI queries alone never exercise a populated committed tree. Browser query reads are RAM reads with the cache disabled. IndexedDB is touched only on open/import/flush (`browser/mod.rs:158`).

### Semantics the change must keep (reference, not exhaustive)
- `live_walk` (`reader.rs:138-257`, `:276-297`) covers whole-triple resume, the newest kept transaction deciding a triple, same-transaction retraction dominance, distinct valid windows, and the seek after 8 skips.
- `LogSource::walk` (`:448`) rejects AVET/VAET with `INT-049` *before* the empty-root check, and resumes after an exact key (live scans resume after a triple).
- `any_key` (`:408`) checks raw presence, including retracted/historical keys. Planner proofs (#641) rely on this.
- `uuids_of` (`dict.rs:192`) expects sorted distinct ids, uses dense cursor walks above thresholds and exact sparse reads otherwise, and may deliver a callback prefix before an error.
- `tx_order` is lazy and cached per reader. A failed computation must not cache an order.
- Much of `CommittedReader`'s fact-returning code (`reader.rs:491-579`) is `cfg(test)`, so moving it alone saves nothing in release builds.

### Audit evidence and harness limits
- Audit baseline `3d19abbce`. The five storage files and `perf_envelope.rs` are unchanged since then, but other v3 changes shift the link. Historical bytes are not reused; all three variants are measured fresh on the same base.
- Recomputed from `reader-point-runtime.json` (32 pairs): point +3.91% (paired range −11.95% to +19.28%), attribute/value +0.84%, as-of +2.02%. Point baseline p50 is 18.1 µs. Socket split: +4.95% with the baseline on node 1, +2.65% on node 3, so placement must be balanced.
- **Harness limits:** `perf_envelope.rs:890` caps a shape at 200 runs (~3.6 ms of point work), whatever `compare.py`'s 2-second budget says. Warmup is 3 queries (`:881`). Each sample opens and drains a new cursor (`:907`), with no prepared execution or cursor reuse. The JSON records row counts and percentiles, not full results or raw durations. Placement is NUMA-node affinity, not per-core pinning.
- Size gate: `.github/workflows/binary-size.yml:17`, 1,263,616 bytes (1,234 KiB). `TandemKit/Planner.md`'s "1230 KiB" is stale. #794/#795 merged since the audit. #796 (dist LTO) is separate and must not be combined with this measurement.

### Bench machine (checked 2026-10-30)
It has Firefox and an ext4 root filesystem. It lacks Wasmtime, Chromium, Node and wasm-pack, and has no sudo. Cold-file eviction can use `posix_fadvise(POSIX_FADV_DONTNEED)` on the fixture, which needs no root.

## 4. Acceptance Criteria

**Variants and reproducibility**
1. B, H and P are built from one pinned v3 commit with one `Cargo.lock`, toolchain, target set, features and profiles. Each result records the patch/commit, lock hash, `rustc -V`, harness version, fixture checksum and command. Nothing unrelated is in any variant.

**Behavior of H**
2. No public API, error code, file format, configuration or dependency change. The `semver-checks` job's own output shows no API change. On v3 PRs that job is `continue-on-error` (`.github/workflows/policy.yml:61-70`), so a green PR status alone doesn't prove this. `docs/ERROR_REFERENCE.md` is unchanged.
3. Constructing a reader, or entering shared code, allocates nothing B does not. Ordinary queries keep reusing the installed reader.
4. The backend mutex is held only for one backend call, as in B. Cache-hit paths take no backend lock. The page cache stays one `Arc<PageCache>` per `PersistentFactStorage`, shared by every generation reader and by `large_reader`, with unchanged generation-bound behavior (`persistent_facts.rs:43`, `:147-150`, `:294-300`; `large_tx.rs:218-225`). `SharedDictCache` and `tx_order` stay per reader, as `OnDiskReader::new` creates them today. A large-transaction reader still never shares the committed reader's DICT cache.
5. All results (values and multiplicities), the documented ordering, error codes and first-error behavior match B. This is shown by the suites in AC 9, not by row counts alone. The `CountingBackend` page-read bounds in the `storage::reader` and `storage::dict` tests still hold. A reader entry point without a direct test gets one.

**Size (Q2)**
6. On the same base, measure B, H and P for: the gate's stripped native CLI, the WASI CLI, raw and wasm-bindgen-packaged browser Wasm, and the stripped opt-level-3 library consumer (audit `matrix.sh` method). **H passes** if the native CLI is at least 2,000 bytes smaller than B, *and* no measured artifact is larger than B.
7. The binary-size gate passes at its current limit with unchanged build settings.

**Correctness gates (bench machine first)**
8. `cargo test --all-features` and the `test-hooks` suites pass, with counts recorded. The fault matrix runs with disk-backed durability coverage, and the `minigraf_shuttle` deterministic suite passes in its cfg build. Interrupted runs don't count.
9. These named suites must pass, as part of the above: `storage::reader`, `storage::dict`, `storage::persistent_facts` (incl. `large_tx`: read-your-writes, rejected-statement rollback, readers spanning large transactions), and `tests/` `cursor_test`, `fact_log_test`, `live_reads_test`, `point_query_history_test`, `snapshot_atomicity_test`, `concurrency_test`, `read_only_open_test`, `wal_test`, `golden_corpus_test`, `migration_matrix_test`, `integrity_test`, `index_corruption_test`, the `tests/answers/` frozen answers, and the set/temporal suites.
10. Coverage gates pass with unchanged floors. The reader's file floor (97% lines / 94% branches, `coverage-thresholds.toml` `[files."src/storage/reader.rs"]`) applies to every extracted reader logic, wherever it ends up. If that logic moves to another file, equivalent protection at no lower floor is enforced there, and the mapping is recorded in the evidence. Being covered only by the whole-crate totals (94/84) doesn't count.
11. `cargo +1.89.0 check --all-features --locked`, CI-toolchain clippy and `cargo fmt --check` are clean. The WASI tests (Wasmtime) and the BrowserDb headless suites (Chrome and Firefox, as `.github/workflows/wasm.yml:23-85` runs them) pass. The PR's CI covers whatever the bench cannot.

**Runtime (Q1, Q3)**
12. **Method.** Measure B vs H and B vs P on the bench machine (Linux x86_64), with balanced order and placement across NUMA nodes 1/3 and the same fixture for every variant:
    - The unit is an independent process block. The primary estimand is each block's candidate/baseline ratio of steady-state mean ns/op, compared as logs.
    - Each block has real warmup and at least 1 s of timed work per variant for point families (the 200-run cap does not apply to the decisive runs).
    - A baseline-vs-baseline (B vs B) control shows the harness resolves a 2% margin.
    - The block count is fixed from a pilot *before* acceptance runs (start at ≥32 blocks), and the analysis method is fixed then too. Pilot data is kept separate and not counted.
    - No retry-until-pass. Inconclusive is not pass.
    - Every comparison checks complete results equal to B, not only counts.
13. **Primary gate.** For each point family (entity/attribute, attribute/value, `:as-of` transaction counter), the one-sided 95% upper confidence bound of H's mean-latency ratio is at most **1.02**. It must hold in **each of two** fixed-size campaigns run at different times, in both the opt-level-3 consumer and the gate's release configuration. Point keys cover fixed and varied keys and existing and missing entities. Entry points cover ordinary queries, prepared queries and cursors pulled in several batches.
14. **Secondary gate.** For every other workload, the upper bound is at most **1.05**. For throughput, the bound applies to inverse throughput. The workloads:
    - all 17 audit warm-read shapes;
    - reader construction: direct construction and drop, open, dirty checkpoint publish, and repeated reads inside a large transaction (a clean no-op checkpoint doesn't count);
    - populated committed scans, short and long histories, dense and sparse UUID projection;
    - application cache disabled (OS cache warm), labeled as such;
    - cold file reads (ext4 fixture, `posix_fadvise(DONTNEED)` per the user's Q3 choice). A **cold observation** is one query or scan traversal that starts with every application-cache and OS page-cache page it will read absent. Each cold observation is reset independently, for each variant. Residency is checked after eviction (e.g. `mincore`/`fincore`) and recorded as evidence that eviction worked. Warmup and result validation must not prewarm a cold observation. These cold rules take precedence over AC 12's warmup: warmup may prepare code and runtime state only. Variants never run concurrently against the same fixture file during cold measurement (separate copies, or one at a time). Reset and setup time are excluded from latency. Warm repeats are never averaged into the cold metric. A process block may contain several independently reset observations. No claim is made about the device's own cache. If effective eviction can't be shown, the cold category stays outstanding and blocks the PR;
    - concurrent read-only queries at 1, 8 and 44 threads;
    - long cursor walks and a paged fact-log walk;
    - WASI under Wasmtime, covering both the real in-memory CLI path and a populated committed-memory-reader fixture (internal test/bench code only, with no new public API);
    - BrowserDb point and scan queries in headless Firefox, kept separate from IndexedDB open/flush.

    Report p50, p99 (per-query samples, not batch averages), absolute deltas, allocations and RSS for each workload, with no pooled score. A required category left unmeasured or inconclusive blocks the PR.
15. Wasmtime and wasm-pack are installed user-locally on the bench machine and removed afterwards, along with fixtures and temp directories. The evidence never contains addresses, credentials, or any other access or identifying information. It does keep the non-identifying measurement metadata needed to interpret it: CPU model and topology, SMT/NUMA placement, OS/kernel, storage and cache conditions, and runtime, packaging and build versions, as `docs/audits/772/README.md:9-20` does.

**Evidence and outcome**
16. Evidence lives under `docs/audits/797/`: scripts, raw records, and a README with the method, the pinned inputs, the pilot-derived sample sizes fixed in advance, and per-workload tables for H and P. Historical `docs/audits/772/` files are not modified.
17. **Draft PR:** when H passes every bench-verifiable gate (AC 2–10, the bench-runnable parts of AC 11, and AC 12–14, all measured on the bench), open a draft PR against `v3` with H and its evidence. It links #797 without a closing keyword, and lists explicitly the checks still pending in CI (Chrome browser suite, macOS/Windows native, anything else the bench can't run). **Ready:** only after those CI checks pass, AC 2's semver output is inspected, and the Codex review is done. CI build or test success never stands in for a required benchmark category.
18. **H fails, P passes:** report both. P is not shipped without a new maintainer decision.
19. **Neither passes:** a docs-only PR to `v3` with the evidence. Comment on #797 with the verdict and recommend closing. No production source changes.
20. **Docs sync, H shipped:** a `CHANGELOG.md` v3.0.0 entry with the measured saving. `docs/BENCHMARKS.md` gets a link to the evidence. Update the `CLAUDE.md` `reader.rs` line, `docs/TEST_COVERAGE.md` and the `CLAUDE.md` test count only if they change. `ROADMAP.md`, `README.md` and the docs site change only if one of their claims becomes false (none is expected for an internal refactor). No version bump or tag.

## 5. Edge Cases & Boundaries
- Empty roots (EAVT/AEVT/AVET/VAET/DICT) and absent entities/idents keep their early returns, with no new reads. An invalid log index still errors on an empty tree.
- History: same-transaction assert and retract, distinct valid windows, triples with no live rows, a bound below, inside or above the stored range, and irregular or decreasing `tx_id`s.
- Resume: exactly-full and partial batches, many versions of one triple, the last triple, and seeks across leaves. Batch size never changes retraction dominance. Early callback stop and callback errors keep their prefix/error behavior.
- UUID batches at 63/64 ids, the density threshold, the 1,024-id chunk boundary, invalid order or duplicates, missing UUIDs, corrupt UUID bytes, and an error after a callback prefix (tests at `reader.rs:660`, `dict.rs:650`, `:721`).
- Long strings and sets still read the right value pages, and short values still avoid value-page reads.
- Cursors and fact logs survive later writes and checkpoints (`tests/cursor_test.rs:263`). Partial pulls, close/drop and an error on a later pull are unchanged.
- Large-transaction reads see finished staged statements, then drop their reader state. A rollback or a rejected statement leaves no uncommitted id visible to later committed reads (`large_tx.rs:895`, `:974`, `:1300`).
- Concurrent cache hits never take the backend mutex. Poisoned locks and corrupt pages keep their handling, and no helper takes the backend lock around code that reads through the same adapter.
- Measurement labels: capacity zero is not cold storage, a new process is not an evicted OS cache, browser buffer reads are RAM reads, and WASI CLI queries are not committed-tree reads.
- Environmental failures, missing tools and statistical uncertainty are recorded as missing evidence, never as passes. Thresholds are not changed after candidate results are seen.

## 6. Key Decisions
- **H is the candidate; P is the comparison** (user, 2026-10-30). Sharing the logic needs no boxing, since backend calls are already dynamic.
- **Margins (user, Q1):** point families ≤ 1.02 upper bound on the mean ratio, in two campaigns with a B-vs-B control; everything else ≤ 1.05; p50/p99 reported, not gating.
- **Size bar (user, Q2):** at least 2,000 native bytes saved, and no artifact grows.
- **Wasm runtime on the bench (user, Q3)** with user-local tools and Firefox headless; cold native reads via `posix_fadvise`.
- **Not shipping is a valid outcome.** If H fails, its evidence lands as a docs-only PR (Claude's default; Codex had left this open).
- **The base contract is preserved:** shared code doesn't mean shared mutable cursors, longer locks, global DICT caches or merged generations. The helper's shape is the Generator's choice.
- **Round 3 (Codex-02, all accepted):** page cache shared per storage, not per reader (H1); a defined and validated cold observation (H2); the reader coverage floor follows extracted code (M1); draft-PR gates split from ready gates (M2); non-identifying bench metadata is kept (M3); the goal is worded around the margins (L1); semver output is checked directly (L2).
- **Doc sync (resolved with Codex-02):** Codex asked to update `CLAUDE.md`, `ROADMAP.md`, `README.md`, `docs/TEST_COVERAGE.md`, `CHANGELOG.md`, `docs/BENCHMARKS.md` and to cross-check the docs site. Claude's position: the project rule is "keep docs from contradicting each other". An internal refactor makes no user-visible claim false, so AC 20 requires the CHANGELOG and benchmark link, and the rest only where text changes.

## 7. Out of Scope
- Changing `LeafCursor`, `DictReader`, `MutexStorageBackend`, `PageCache`, lock granularity, cache policy, or the UUID/history algorithms, including to recover lost speed.
- Erasing other generics (`PersistentFactStorage<B>`, browser/IndexedDB code).
- Size-cap changes. #794/#795/#796 work, or combining their changes into the measured variants.
- v2.x backport. New public or benchmark APIs, options, dependencies, or file/WAL format changes.
- Claims about environments that were not measured.

## 8. Possible Directions & Ideas (non-binding)
- A borrowed non-generic read context, or shared routines for the large bodies (`dict_txs`, `live_walk`, the tx-order/time-bound logic, log walk/decode), may give most of the sharing. Check emitted symbols (`cargo bloat`, symbol sizes) before widening the refactor, because source sharing doesn't guarantee smaller machine code.
- A companion to `perf_envelope` could separate construction, first use and steady state. Leave the original for broad regression detection.
- `docs/audits/772/compare.py`, `prototypes.py` and `matrix.sh` are a starting harness.
- Measure the reader directly as well as through the public API, so parser time can't hide a reader regression. The public-API result is the decision metric.
- Remember that `bench` profile is opt-level z, while `envelope` is opt-level 3.

## Open Questions
None. Q1–Q3 were answered by the user.
