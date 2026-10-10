# Long-running soak test at scale — Design

**Issue:** #392, tracker #383
**Milestone:** v3.0.0. Test and workflow only; no library change.
**Branches:** a v3 PR first, then a `main` PR with the same harness adapted to
the v2 API (no `verify()`). The weekly workflow alternates between the two.

## 1. Goal

No test today runs for hours or at the size of a real deployment. The nightly
smoke suite has a 15-minute timeout and 5,000 facts. Bugs like #370 appeared
only after several checkpoints on a real workload. The soak test grows a
file-backed database to 10M+ facts with mixed batches, multi-valued
attributes, retracts and long histories on hot entities. It checkpoints and
reopens many times, checks answers against a reference throughout, and records
the numbers #394 needs.

## 2. Measured feasibility (local, `--profile bench`)

| | v3 (format v8) | main (format v7) |
|---|---|---|
| Insert rate (100-fact batches) | ~70K facts/s | ~70K facts/s |
| Checkpoint at 1M / 3.8M facts | 180 ms / 250 ms | 1.0 s / 2.6 s (linear) |
| File size per fact | ~110 B | ~430 B (≈4.3 GB at 10M) |

A GitHub runner is about 2–3× slower. Both branches reach 10M facts well
inside the time budget, so the run is time-boxed and the extra time goes into
churn.

## 3. Shape

`tests/soak_test.rs`, `#![cfg(not(target_arch = "wasm32"))]`, two tests:

- `soak` (`#[ignore]`): the full run, tuned by environment variables.
- `soak_full_scan_child`: a no-op unless the parent sets
  `MINIGRAF_SOAK_SCAN_DB` (§6).
- `soak_short` (runs in the per-PR suite): the same driver with a tiny
  configuration (about 20K facts, 3 reopens, 3 checks), a few seconds in a
  debug build. It keeps the harness compiling and correct.

| Variable | Default (`soak`) | Meaning |
|---|---|---|
| `MINIGRAF_SOAK_MINUTES` | 20 | Wall-clock budget for the workload |
| `MINIGRAF_SOAK_TARGET_FACTS` | 10,000,000 | Live facts the growth phase reaches |
| `MINIGRAF_SOAK_CHECK_MINUTES` | 10 | Interval between sampled checks |
| `MINIGRAF_SOAK_CHURN_TPS` | 50 | Write rate cap in the churn phase (§5) |
| `MINIGRAF_SOAK_SEED` | random, printed | Replays a run |
| `MINIGRAF_SOAK_METRICS` | `target/soak/metrics.jsonl` | Metrics output |
| `MINIGRAF_SOAK_FULL_SCAN_MAX` | 3,000,000 | Largest live count the final full scan runs at (§6) |
| `MINIGRAF_SOAK_DIR` | a temp dir | Where the `.graph` lives (the workflow points it at the runner's large disk) |

It runs under `--profile bench` (optimized, `panic = "unwind"`). The release
profile aborts on panic and cannot run tests.

## 4. Reference without storing 10M facts

The process's own peak RSS is a measured number, so the reference must stay
small. Every ordinary entity's facts follow from its index, and only changes
are stored.

**Ordinary entities** `:e{n}` (`n = 0, 1, 2, …`). `content(n)` is a fixed
function of `n` and the seed:

- `:p/name "name-{n}"` (unique, for AVET lookups)
- `:p/score` integer `hash(n) % 1000`
- `:p/shard{n % 1024}` integer `n` (a sparse attribute: about 2,000 entities
  each at 10M facts, so an AEVT scan of one shard is cheap to check exactly)
- `:p/tag`, 1–4 keyword values in the same transaction (multi-valued, #371)
- every 50th entity: `:p/bio`, a string over 64 bytes (value pages)
- every 10th entity: `:p/ref`, a ref to `:e{hash(n) % n}`

Each fact of `content(n)` has a slot number. An entity is created in one
transaction, and a batch holds several entities. The reference keeps:

- `next_entity`: entities `0..next_entity` exist.
- `flipped`: slots `(entity, slot)` currently retracted, a `Vec` plus an
  index map so a random one can be picked in O(1).
  A retract op picks a random existing entity and slot. If the slot is live it
  is retracted and inserted into `flipped`; if not it is re-asserted and
  removed.
- `live_count`: total facts minus `flipped.len()`.

The expected live facts of any entity, or of any shard, are computed from
`content` and `flipped`. Every entity can be sampled.

**Hot entities** `:h0`–`:h31` carry long histories. Each churn step changes a
few of them: retract and re-assert `:h/state` (one of 8 values), and add or
retract `:h/tag` values (multi-valued). The reference keeps every hot change
as `(tx_count, triple, assert|retract)`, so `:as-of` at any past transaction
can be checked. A record packs into one `u64`; a 5-hour run keeps a few
million of them.

**Transaction counter:** each `execute` increments the reference `tx`. After
every reopen `current_tx_count()` must equal it.

All facts use the default valid time. Valid-time windows are covered by the
model-based and property tests (#385, #386).

## 5. Workload

One loop until the budget ends. Each step picks one op:

| Op | Weight (growth / churn) | Detail |
|---|---|---|
| Grow | 70% / 10% | New entities; batch size log-uniform over 1–2,000 facts |
| Retract/re-assert | 15% / 50% | 1–50 random ordinary slots: live ones retracted in one `retract`, or retracted ones re-asserted in one `transact` |
| Hot churn | 15% / 40% | 1–8 changes on hot entities in one `transact` or `retract` |

The growth phase runs at full speed until `live_count` reaches the target
(the run fails if the budget ends first). The churn phase uses the rest of the
budget at a capped write rate, `MINIGRAF_SOAK_CHURN_TPS` (default 50
transactions per second): at full speed it would add tens of millions of fact
versions, more than main's in-memory store and disk can hold. Time between
writes goes to extra checked reads: an EAVT (90%) or AVET (10%) lookup of a random entity,
compared with the reference and timed like the scheduled checks. Re-asserts
are chosen once `flipped` holds 200K slots, so the live count stays roughly
flat while the history grows by a few million versions.

Interleaved, by a seeded schedule:

- `checkpoint()` every 20–200 transactions (v3: thousands of checkpoints per
  run). Times are recorded.
- Every handle is opened with `wal_checkpoint_threshold(usize::MAX)`, so only
  the schedule above checkpoints and dropping a handle does not.
- Reopen every 2–10 minutes: half the time right after `checkpoint()` (open
  reads the file only), half with up to 200 transactions still in the WAL
  (open replays them). Open time is recorded for each kind.

## 6. Checks

Every `MINIGRAF_SOAK_CHECK_MINUTES`, after every reopen, and at the end:

- **EAVT:** `[:e{n} ?a ?v]` for 200 random entities and all 32 hot entities;
  rendered rows equal the reference.
- **AVET:** `[?e :p/name "name-{n}"]` for 20 random entities: present
  exactly when that slot is live. This shape scans the whole attribute today
  (#518, about 0.5 s at 1M facts), so it is sampled less.
- **AEVT:** `[?e :p/shard{k} ?v]` for 4 random shards; the result equals
  every live shard slot of that shard.
- **`:as-of`:** for 4 hot entities and 3 random past transactions each, the
  rows equal the history replayed to that transaction.
- **Counter:** `current_tx_count()` equals the reference.

- **Full scan:** the row count of `[?e ?a ?v]` equals `live_count` plus the
  hot entities' live facts. A query builds its whole answer when it opens
  (#432), so a full scan costs memory per row: about 300 B at 1M simple
  facts, about 1.1 KB here (2.2 GB at 2M); a `count` aggregate costs the same.
  The allocator keeps that memory after the query, so in this process it would
  hide a leak of that size from §8. The scan therefore runs in a child process
  (the test binary re-run as `soak_full_scan_child`, as `crash_kill_test`
  does) after a checkpoint, with the parent's handle closed: once, when the
  live count first reaches 2M, and at the end only up to
  `MINIGRAF_SOAK_FULL_SCAN_MAX` (default 3M) live facts. The child reports the
  row count, the time and its peak RSS.

At the end, after a final reopen:

- **v3 only:** `verify()` reports no problems.

Rendered rows never carry a `Uuid` (CLAUDE.md test conventions); failures name
the seed, the step and the check.

## 7. Metrics

The test appends one JSON line per check to the metrics file:

```json
{"t_s": 3600, "phase": "churn", "tx": 41230, "live_facts": 10012345,
 "versions": 12500000, "file_bytes": 1234567890, "open_ms_file": 812, "open_ms_wal": 940,
 "reopens": 40, "checkpoints": 2100, "checkpoint_ms_p50": 210, "checkpoint_ms_max": 900,
 "rss_bytes": 512000000, "peak_rss_bytes": 700000000,
 "query_us": {"eavt": [p50, p99], "avet": [...], "aevt": [...], "as_of": [...]}}
```

Lines written after a full scan also carry `full_scan_ms` and
`full_scan_peak_rss_bytes` (the child's). RSS
and peak RSS come from `/proc/self/status` (`VmRSS`, `VmHWM`); elsewhere they
are `null`.

## 8. Memory bound

After the growth phase the test records a baseline: RSS right after a reopen.
At each later post-reopen check:

- **v3:** RSS must stay below `1.25 × baseline + 256 MiB`. The page cache is
  bounded and the reference grows only slowly, so the steady state is flat.
- **main:** v2 keeps facts in memory, so RSS rightly grows with history. RSS
  per fact version must stay below `1.25 ×` the baseline ratio, plus 256 MiB.

## 9. Workflow `soak.yml`

Weekly, alternating like `bench.yml` and `crash-kill.yml`: main on Saturday
01:00 UTC, v3 on Sunday 01:00 UTC, chosen from the cron string; also
`workflow_dispatch` with a `ref` input. The same file is committed to both
branches.

- `ubuntu-latest`, `timeout-minutes: 360`.
- `MINIGRAF_SOAK_DIR=/mnt/soak`: the runner's spare disk holds main's
  multi-GB file.
- `cargo test --profile bench --test soak_test soak -- --ignored --nocapture`
  with `MINIGRAF_SOAK_MINUTES=300`.
- Always upload `metrics.jsonl` and the test log as artifacts (90-day
  retention), which feed #394.
- On failure, open or comment on a "Weekly soak failure (<ref>)" issue with the
  seed and the log tail, as `crash-kill.yml` does.

## 10. Out of scope

Kills mid-write (#384), fault injection (#390), valid-time windows (#385,
#386), multi-process access, and other OSes (RSS is read from `/proc`).
Publishing the performance envelope is #394; this test only produces the
numbers.
