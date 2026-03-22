# Phase 6.4b: Benchmarks + Light Publish Prep — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run the existing Criterion benchmark suite, add heaptrack memory profiling, create `BENCHMARKS.md`, update README and CHANGELOG, clean up `Cargo.toml`, and enable GitHub Discussions — all targeting the v0.8.0 release.

**Architecture:** No new library code. The Criterion suite (`benches/minigraf_bench.rs`) already exists with 9 groups; this phase runs it, captures results, and documents them. Memory profiling is done via a new `examples/memory_profile.rs` binary and heaptrack. Cargo.toml gets a dead-dep removal and metadata fields. Documentation is added/updated in-place.

**Tech Stack:** Rust (cargo bench, cargo run --example), heaptrack (Linux heap profiler, `sudo pacman -S heaptrack` if needed), GitHub CLI (`gh`)

**Spec:** `docs/superpowers/specs/2026-03-22-phase6-4b-benchmarks-design.md`

---

## File Map

| File | Action | What changes |
|---|---|---|
| `Cargo.toml` | Modify | Remove dead `clap` dep; add metadata; bump version 0.7.0 → 0.8.0 |
| `examples/memory_profile.rs` | Create | Heaptrack profiling binary |
| `BENCHMARKS.md` | Create | Full benchmark + memory profile tables with machine spec |
| `README.md` | Modify | Update phase badge/status; add memory table; add BENCHMARKS.md link |
| `CHANGELOG.md` | Modify | Add v0.8.0 entry |

---

## Task 1: Cargo.toml — Remove Dead `clap` Dep, Add Metadata, Bump Version

**Files:**
- Modify: `Cargo.toml`

`clap` is listed in `[dependencies]` but is never imported anywhere in `src/` (verified: `grep -r "clap" src/` returns nothing). It leaks into every consumer's dependency graph unnecessarily.

- [ ] **Step 1: Remove `clap` and add metadata**

Edit `Cargo.toml`. The `[package]` section should look like:

```toml
[package]
name = "minigraf"
version = "0.8.0"
edition = "2024"
description = "Zero-config, single-file, embedded graph database with bi-temporal Datalog queries"
license = "MIT OR Apache-2.0"
authors = ["Aditya Mukhopadhyay"]
repository = "https://github.com/adityamukho/minigraf"
keywords = ["graph", "datalog", "bitemporal", "embedded", "database"]
categories = ["database", "embedded"]
readme = "README.md"
documentation = "https://docs.rs/minigraf"
```

Remove this line from `[dependencies]`:
```toml
clap = { version = "4.5", features = ["derive"] }
```

- [ ] **Step 2: Verify build and tests still pass**

```bash
cargo build 2>&1 | tail -5
cargo test 2>&1 | tail -5
```

Expected: `Compiling minigraf ...` then `test result: ok. 298 passed`

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: remove dead clap dep, add Cargo.toml publish metadata, bump to v0.8.0"
```

---

## Task 2: Create `examples/memory_profile.rs`

**Files:**
- Create: `examples/memory_profile.rs`

This binary accepts a fact count as a positional argument (default 10 000), inserts that many facts into a file-backed DB in a temp directory, checkpoints, and runs a representative query. It is the target for heaptrack profiling in Task 4.

- [ ] **Step 1: Create the binary**

```rust
// examples/memory_profile.rs
//! Memory profiling target for heaptrack.
//!
//! Usage: cargo run --example memory_profile --release -- <fact_count>
//!
//! Inserts <fact_count> `:eN :val N` facts into a checkpointed file-backed DB,
//! then runs a single point-entity query. Run under heaptrack to capture
//! peak heap and allocation counts.

use minigraf::OpenOptions;

fn main() -> anyhow::Result<()> {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("profile.graph");
    let path_str = path.to_str().unwrap();

    let db = OpenOptions::new().path(path_str).open()?;

    // Insert in batches of 100, matching the bench helper pattern
    const BATCH: usize = 100;
    for batch_start in (0..n).step_by(BATCH) {
        let batch_end = (batch_start + BATCH).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{} :val {}]", i, i));
        }
        cmd.push_str("])");
        db.execute(&cmd)?;
    }

    db.checkpoint()?;

    // Representative point-entity query
    let _ = db.execute("(query [:find ?v :where [:e0 :val ?v]])")?;

    eprintln!("memory_profile: inserted and queried {} facts", n);
    Ok(())
}
```

- [ ] **Step 2: Add `tempfile` is already a dev-dependency — verify example compiles**

```bash
cargo build --example memory_profile 2>&1 | tail -5
```

Expected: `Finished dev [unoptimized + debuginfo]` with no errors.

- [ ] **Step 3: Smoke-test the binary**

```bash
cargo run --example memory_profile -- 100 2>&1
```

Expected output: `memory_profile: inserted and queried 100 facts`

- [ ] **Step 4: Commit**

```bash
git add examples/memory_profile.rs
git commit -m "feat: add memory_profile example for heaptrack profiling"
```

---

## Task 3: Run Criterion Benchmarks

**Files:** None modified — this task captures results for use in Task 5.

The Criterion suite is in `benches/minigraf_bench.rs`. It has 9 top-level groups. This step runs it and saves a named baseline.

**Note:** The full benchmark run takes 15–30 minutes. The `recursion/chain depth_100` benchmark is known to be slow (~15 s per sample); Criterion may time out on it — if it does, extend `measurement_time` for that group only or accept the warning and move on.

- [ ] **Step 1: Run the full suite and save baseline**

```bash
cargo bench --bench minigraf_bench -- --save-baseline main 2>&1 | tee /tmp/bench_output.txt
```

Expected: A long stream of Criterion output. Each benchmark prints lines like:
```
insert/single_fact/1k   time:   [2.300 µs 2.350 µs 2.400 µs]
```

- [ ] **Step 2: Verify HTML reports were generated**

```bash
ls target/criterion/
```

Expected: directories named `insert`, `insert_file`, `query`, `time_travel`, `recursion`, `open`, `checkpoint`, `concurrent`, `concurrent_file`.

- [ ] **Step 3: Extract key numbers**

From `/tmp/bench_output.txt`, note the **median** (middle value of the three-element `[low median high]` tuple) for the following benchmarks — these will populate `BENCHMARKS.md` in Task 5:

**Insert (in-memory)** — `insert/*`:
- `single_fact/1k`, `single_fact/10k`, `single_fact/100k`
- `batch_100/1k`, `batch_100/10k`, `batch_100/100k`
- `explicit_tx/1k`, `explicit_tx/10k`, `explicit_tx/100k`

**Insert (file-backed)** — `insert_file/*`: same sub-groups as above

**Query** — `query/*`:
- `point_entity/1k`, `.../10k`, `.../100k`, `.../1M`
- `point_attribute/...` (same scales)
- `join_3pattern/...` (same scales)

**Time travel** — `time_travel/*`:
- `as_of_counter/...` (1k, 10k, 100k, 1M)
- `valid_at/...` (1k, 10k, 100k, 1M) — if this sub-group does not appear in the bench output, skip it and record `as_of_counter` only

**Recursion** — `recursion/*`:
- `chain/depth_10`, `chain/depth_100`
- `fanout/...`

**Open** — `open/*`:
- `checkpointed/1k`, `.../10k`, `.../100k`, `.../1M`
- `wal_replay/1k`, `.../10k`

**Checkpoint** — `checkpoint/*`:
- `1k`, `10k`

**Concurrent (in-memory)** — `concurrent/*`:
- `readers/4`, `readers/8`, `readers/16`
- `readers_plus_writer/4`, `readers_plus_writer/8`, `readers_plus_writer/16`
- `serialized_writers/2`, `.../4`, `.../8`, `.../16`

**Concurrent (file-backed)** — `concurrent_file/*`: same sub-groups

---

## Task 4: Run Heaptrack Memory Profiling

**Files:** None modified — this task captures results for use in Task 5.

- [ ] **Step 1: Install heaptrack if not present**

```bash
which heaptrack || sudo pacman -S heaptrack
```

Expected: either a path like `/usr/bin/heaptrack` or a pacman install. Confirm with `heaptrack --version`.

- [ ] **Step 2: Build release example binary**

```bash
cargo build --example memory_profile --release 2>&1 | tail -3
```

Expected: `Finished release [optimized]`

- [ ] **Step 3: Profile at 10K facts**

```bash
heaptrack ./target/release/examples/memory_profile 10000 2>&1 | tee /tmp/heap_10k.txt
```

When heaptrack finishes, it prints a summary line like:
```
heaptrack stats:
  allocations: 123456
  leaked allocations: 0
  temporary allocations: 12345
peak heap memory consumption: 4.50MB
peak RSS: 6.00MB
```

Record: **peak heap** at 10K facts.

- [ ] **Step 4: Profile at 100K facts**

```bash
heaptrack ./target/release/examples/memory_profile 100000 2>&1 | tee /tmp/heap_100k.txt
```

Record: **peak heap** at 100K facts.

- [ ] **Step 5: Profile at 1M facts**

```bash
heaptrack ./target/release/examples/memory_profile 1000000 2>&1 | tee /tmp/heap_1m.txt
```

This may take 1–2 minutes. Record: **peak heap** at 1M facts.

**What to expect:** Peak heap should grow roughly linearly with fact count (each `Fact` struct is ~200 bytes serialised + in-memory overhead). The page cache ceiling is 256 pages × 4KB = 1MB; after checkpoint the in-memory working set is mostly the pending-facts vec (empty) + index structures.

---

## Task 5: Create `BENCHMARKS.md`

**Files:**
- Create: `BENCHMARKS.md` (repo root)

Use the numbers captured in Tasks 3 and 4. The existing README already contains detailed benchmark tables; `BENCHMARKS.md` is the authoritative standalone reference that also includes the machine spec header, memory profile, and full interpretation notes.

- [ ] **Step 1: Create `BENCHMARKS.md`**

Replace `<MACHINE_SPEC>`, `<RUST_VERSION>`, `<DATE>`, and all `<...>` placeholders with the actual values from your run.

```markdown
# Minigraf Benchmarks

## Environment

| Field | Value |
|---|---|
| CPU | <CPU model, e.g. Intel Core i7-1065G7 @ 1.30GHz> |
| RAM | <RAM, e.g. 16GB> |
| OS | <OS, e.g. Manjaro Linux 6.12> |
| Rust | <rustc --version output> |
| minigraf | 0.8.0 |
| Criterion | 0.5 |
| Benchmark date | <YYYY-MM-DD> |

Full HTML reports are generated by `cargo bench` and stored under `target/criterion/`.

---

## Insert Throughput (in-memory)

Steady-state cost of a single insert into a pre-populated in-memory database.
All three write modes cost essentially the same — the dominant cost is EAV indexing and the `Arc<RwLock<...>>` write-lock.

| Pre-populated | Single fact | Batch (100) | Explicit tx |
|---|---|---|---|
| 1K  | <µs> | <µs (~X µs/fact)> | <µs> |
| 10K | <µs> | <µs (~X µs/fact)> | <µs> |
| 100K | <µs> | <µs (~X µs/fact)> | <µs> |

## Insert Throughput (file-backed, WAL-appended)

Same workloads with a file-backed database. WAL entries are OS-buffered (no explicit
fsync), adding only modest overhead vs in-memory.

| Pre-populated | Single fact | Batch (100) | Explicit tx |
|---|---|---|---|
| 1K  | <µs> | <µs (~X µs/fact)> | <µs> |
| 10K | <µs> | <µs (~X µs/fact)> | <µs> |
| 100K | <µs> | <µs (~X µs/fact)> | <µs> |

> **Finding:** [note if file-backed is close to in-memory or significantly slower]

---

## Query Latency (in-memory)

Queries run against a pre-populated in-memory database. All three query types show
**O(N) scaling** — the query executor performs a full linear scan of facts
rather than using the covering indexes (known limitation; Phase 6.5 target).

| DB size | Point entity | Point attribute | 3-pattern join |
|---|---|---|---|
| 1K  | <ms> | <ms> | <ms> |
| 10K | <ms> | <ms> | <ms> |
| 100K | <ms> | <ms> | <ms> |
| 1M | <s> | <s> | <s> |

> **Finding:** Query latency scales linearly with DB size. The covering indexes
> (EAVT, AEVT, AVET, VAET) are persisted to disk but not yet used for in-memory
> query execution. Using them would reduce point lookups to O(log N).

---

## Time-Travel Overhead (`:as-of`, `:valid-at`)

Temporal filtering adds negligible overhead on top of base query cost.

| DB size | Base query | `:as-of` | `:valid-at` | Overhead |
|---|---|---|---|---|
| 1K  | <ms> | <ms> | <ms> | <%> |
| 10K | <ms> | <ms> | <ms> | <%> |
| 100K | <ms> | <ms> | <ms> | <%> |
| 1M | <s> | <s> | <s> | <%> |

---

## Transitive Closure (Recursive Rules)

Semi-naive evaluation on two graph shapes.

| Scenario | Time |
|---|---|
| chain, depth 10 | <ms> |
| chain, depth 100 | <s> |
| fanout (width 10, depth 3, ~1 110 nodes) | <s> |

---

## Database Open Time

Time to open an existing database, including header read, packed page traversal, and
index reconstruction. Scales linearly with fact count (O(N)) — Phase 6.5 target.

| DB size | Checkpointed (clean) | WAL replay |
|---|---|---|
| 1K  | <ms> | <ms> |
| 10K | <ms> | <ms> |
| 100K | <ms> | — |
| 1M | <s> | — |

> WAL replay is only benchmarked at 1K and 10K; 1M WAL replay is excluded due to
> prohibitively slow setup cost.

---

## Checkpoint Cost

Time to flush WAL entries to packed pages and delete the WAL sidecar.

| WAL size | Checkpoint time |
|---|---|
| 1K facts | <ms> |
| 10K facts | <ms> |

---

## Concurrent Throughput

Multi-threaded performance via `std::sync::RwLock` (reads) and `Mutex` (writes).
Numbers represent the **maximum elapsed time across all threads** (not aggregate throughput).

### In-memory (10K-fact DB)

| Scenario | 4 threads | 8 threads | 16 threads |
|---|---|---|---|
| readers only | <ms> | <ms> | <ms> |
| readers + 1 writer | <ms> | <ms> | <ms> |

| Scenario | 2 threads | 4 threads | 8 threads | 16 threads |
|---|---|---|---|---|
| serialized writers | <µs> | <µs> | <µs> | <µs> |

### File-backed (10K-fact DB)

| Scenario | 4 threads | 8 threads | 16 threads |
|---|---|---|---|
| readers only | <ms> | <ms> | <ms> |
| readers + 1 writer | <ms> | <ms> | <ms> |

| Scenario | 2 threads | 4 threads | 8 threads | 16 threads |
|---|---|---|---|---|
| serialized writers | <µs> | <µs> | <µs> | <µs> |

---

## Memory Profiles (heaptrack, release build)

Peak heap consumption measured with heaptrack on the `memory_profile` example binary
(file-backed DB, fully checkpointed, no WAL). Page cache ceiling: 256 pages × 4KB = 1MB.

| Fact count | Peak heap | Notes |
|---|---|---|
| 10 000 | <MB> | |
| 100 000 | <MB> | |
| 1 000 000 | <MB> | |

---

## Interpretation

### Key Findings

[Fill in after seeing results — e.g. "Insert throughput is flat across scales; query latency is O(N)"]

### Known Limitations

- **Recursion is O(depth²) on chain graphs** — semi-naive evaluation recomputes delta sets
  per stratum; depth_100 is the practical ceiling in the bench suite.
- **Concurrent benchmark numbers are wall-clock max across threads**, not aggregate
  throughput — not directly comparable to single-threaded latency figures.
- **`open/checkpointed` at 1M facts measures cold-read latency** — the 256-page LRU
  cache (1MB) is far smaller than a 1M-fact dataset (~160MB packed), so every open
  reads from disk. This is worst-case open cost, not cached access.

### Optimization Targets (Phase 6.5)

| Issue | Current | Phase 6.5 Target |
|---|---|---|
| Query O(N) linear scan | ~2 s at 1M facts | O(log N) via in-memory index lookup |
| Recursive evaluator O(depth²) | ~15 s at depth 100 | O(depth × delta) true semi-naive |
| Open time O(N) | ~3 s at 1M facts | O(1) header + lazy page loading |
```

- [ ] **Step 2: Verify the file looks correct**

```bash
wc -l BENCHMARKS.md
```

Expected: > 100 lines.

- [ ] **Step 3: Commit**

```bash
git add BENCHMARKS.md
git commit -m "docs: add BENCHMARKS.md with Criterion results and heaptrack memory profiles"
```

---

## Task 6: Update `README.md`

**Files:**
- Modify: `README.md`

Three targeted changes: (a) update the phase badge and status text, (b) add a memory profile table to the Performance section, (c) add a link to `BENCHMARKS.md`.

- [ ] **Step 1: Update phase badge (line 7)**

Find:
```markdown
[![Phase](https://img.shields.io/badge/phase-6.4a%20complete-blue.svg)](https://github.com/adityamukho/minigraf/blob/main/ROADMAP.md)
```

Replace with:
```markdown
[![Phase](https://img.shields.io/badge/phase-6.4b%20complete-blue.svg)](https://github.com/adityamukho/minigraf/blob/main/ROADMAP.md)
```

- [ ] **Step 2: Update status text (line 21)**

Find:
```markdown
**Status**: Early development. Phase 6.4a complete (Retraction semantics fix + edge case tests). Now starting Phase 6.4b (Criterion benchmarks + crates.io publish). Note: Phase 6.3 (query optimization) was completed as part of Phase 6.1.
```

Replace with:
```markdown
**Status**: Early development. Phase 6.4b complete (Criterion benchmarks + memory profiling). crates.io publish deferred to after Phase 6.5 (file format v6). Note: Phase 6.3 (query optimization) was completed as part of Phase 6.1.
```

- [ ] **Step 3: Update the "Next" feature bullet (line 59)**

Find:
```markdown
- 🎯 **Next: Criterion benchmarks** - Performance at scale + crates.io publish (Phase 6.4b)
```

Replace with:
```markdown
- 🎯 **Next: On-disk B+tree indexes** - O(log N) query performance + file format v6 + crates.io publish (Phase 6.5)
```

- [ ] **Step 4: Add BENCHMARKS.md link and memory section to Performance section**

Find (line ~303):
```markdown
## Performance

Benchmarks run with `cargo bench` on Intel i7-1065G7 (1.3 GHz base, Linux). Full HTML
reports generated in `target/criterion/` after running `cargo bench`.
```

Replace with the following — **you must substitute the actual machine spec** (run `lscpu | grep "Model name"`, `free -h`, `uname -r`, `rustc --version`) **before committing**. Do not commit the `<...>` placeholders:
```markdown
## Performance

Benchmarks run with `cargo bench` on <CPU model, Linux>. Full results including machine
spec and memory profiles are in [BENCHMARKS.md](BENCHMARKS.md). HTML reports are
generated in `target/criterion/` by `cargo bench`.
```

Then find the `### Summary of optimization targets` section and insert a `### Memory usage` section immediately before it:

```markdown
### Memory usage (heaptrack, release build, file-backed, checkpointed)

| Fact count | Peak heap |
|---|---|
| 10 000 | <MB> |
| 100 000 | <MB> |
| 1 000 000 | <MB> |

Peak heap is dominated by the in-memory pending-facts buffer during insertion;
after `checkpoint()` the working set drops to the page cache ceiling (256 pages × 4KB = 1MB)
plus index structures.

```

- [ ] **Step 5: Verify README renders correctly**

```bash
grep -n "BENCHMARKS.md\|Phase 6.4b\|Next:" README.md | head -10
```

Expected: lines referencing BENCHMARKS.md, "6.4b complete", "Phase 6.5".

- [ ] **Step 6: Commit**

```bash
git add README.md
git commit -m "docs: update README for Phase 6.4b — add BENCHMARKS.md link, memory section, status"
```

---

## Task 7: Update `CHANGELOG.md`

**Files:**
- Modify: `CHANGELOG.md`

Insert a new `## [0.8.0]` entry at the top of the changelog (after the header, before `## [0.7.1]`).

- [ ] **Step 1: Insert v0.8.0 entry**

Find:
```markdown
## [0.7.1] - 2026-03-22
```

Insert immediately before it:

```markdown
## [0.8.0] - 2026-03-22

### Added
- `BENCHMARKS.md` — full Criterion benchmark results at 1K/10K/100K/1M facts with machine spec, HTML report references, and heaptrack memory profiles
- `examples/memory_profile.rs` — heaptrack profiling binary; accepts fact count as positional arg
- `Cargo.toml` metadata: `repository`, `keywords`, `categories`, `readme`, `documentation` fields
- Memory profile table in `README.md` "Performance" section

### Changed
- `README.md` Performance section now links to `BENCHMARKS.md` for full benchmark details
- Phase badge and status text updated to reflect Phase 6.4b completion
- crates.io publish deferred to Phase 6.5 (after file format v6 lands)

### Removed
- Dead `clap` dependency from `[dependencies]` — `clap` was listed but never imported in library or binary code

```

- [ ] **Step 2: Verify**

```bash
head -30 CHANGELOG.md
```

Expected: `## [0.8.0] - 2026-03-22` as the first changelog entry.

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: add CHANGELOG.md v0.8.0 entry for Phase 6.4b"
```

---

## Task 8: Enable GitHub Discussions

**Files:** None.

- [ ] **Step 1: Enable Discussions via GitHub API**

```bash
gh api repos/adityamukho/minigraf -X PATCH -f has_discussions=true
```

Expected: JSON response with `"has_discussions": true`.

- [ ] **Step 2: Verify**

```bash
gh api repos/adityamukho/minigraf --jq '.has_discussions'
```

Expected: `true`

- [ ] **Step 3: Commit**

No file changes — this step is complete. Optionally note it in the final commit message of the branch.

---

## Final Verification

- [ ] **Run full test suite**

```bash
cargo test 2>&1 | tail -5
```

Expected: `test result: ok. 298 passed`

- [ ] **Push all commits**

```bash
git log --oneline -8
git push
```

Confirm all Phase 6.4b commits are visible on `origin/main`.
