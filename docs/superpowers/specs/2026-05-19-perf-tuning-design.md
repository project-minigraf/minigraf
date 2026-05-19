---
name: Performance tuning guide (#191)
description: Design spec for the Minigraf performance tuning guide — single wiki page covering cost model, configuration knobs, query patterns, benchmark reference, and platform notes
type: spec
issue: 191
wave: 8
---

# Performance Tuning Guide — Issue #191

## Overview

A single wiki page (`Performance-Tuning.md`) covering everything a user needs to tune Minigraf for
their workload. The page is organized "understand first": cost model before knobs, so the knobs make
sense in context. It is the reference complement to the tutorials — scannable, not narrative.

## Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Location | Wiki (`Performance-Tuning.md`) | Consistent with tutorials and cookbook |
| Structure | Single page, anchored sections | Tunable surface is small; splitting would fragment a short document |
| Order | Cost model → knobs → query patterns → benchmarks → platform notes | Knobs only make sense after the cost model is established |
| Code style | Rust API + Datalog examples as needed | Knobs are Rust API; query patterns are Datalog |
| BENCHMARKS.md | Reference by link, do not duplicate | Numbers change; the wiki page should not go stale |

## Sections

### Section 1 — Cost Model

A table of every operation with its cost class and a short note. Purpose: give users a mental model
before they reach for any knob.

| Operation | Cost | Notes |
|---|---|---|
| Insert / retract (any DB size) | O(1) | WAL append; independent of `.graph` size |
| Query — bound entity or attribute | O(k) | Selective index fetch; k = facts for that entity/attr |
| Query — no bound entity/attr | O(facts) | Full scan + in-memory filter |
| Expression predicate `[(> ?x N)]` | O(N), early | Pushed down at first bound variable |
| `not` / `not-join` | O(N²) worst case | Inner loop re-scans per binding |
| `or` / `or-join` (mid-query) | O(N²) worst case | Branch expansion over full binding set |
| `or` in a rule body | O(N) | Rule starts from empty binding — no re-scan |
| Recursive rules | Super-linear | Semi-naive; deep chains are expensive |
| Window functions | O(N log N) | Sort pass over result set |
| Aggregates (`count`, `sum`, `min`, `max`) | O(N) | Single pass |
| `(sum :with ?x)` cross-product join | O(N²) | No hash-join; avoid on large datasets |
| Open (file-backed) | O(facts) | Page-cache warming; B+tree roots loaded lazily |
| Checkpoint | O(facts) | WAL flush + B+tree rebuild across all 4 indexes |

**Selective fetch threshold:** the engine counts distinct bound entities + bound attributes across all
patterns in the query. If the count is 1–4, it uses index-backed fetches (EAVT/AVET scans for those
specific entities/attributes) instead of a full scan. Beyond 4, a full scan is cheaper and the engine
falls back to it automatically.

### Section 2 — Configuration Knobs

Four fields on `OpenOptions`, shown as a builder code block:

```rust
let db = OpenOptions::new()
    .page_cache_size(1024)          // 1024 × 4KB = 4MB
    .wal_checkpoint_threshold(500)  // checkpoint every 500 WAL entries
    .max_derived_facts(50_000)      // recursive rule safety ceiling
    .max_results(500_000)           // result set safety ceiling
    .path("my.graph")
    .open()?;
```

| Option | Default | Unit |
|---|---|---|
| `page_cache_size` | 256 | pages (1 page = 4KB) |
| `wal_checkpoint_threshold` | 1000 | WAL entries |
| `max_derived_facts` | 100,000 | derived facts per rule iteration |
| `max_results` | 1,000,000 | total query results |

Guidance per knob:

**`page_cache_size`** — File-backed databases only. The LRU cache holds recently read B+tree pages
in memory; a cache miss triggers a disk read. The default (256 pages = 1MB) covers a ~10K-fact
database comfortably. For a 100K-fact database with repeated queries over the same pages, raise to
1024–4096 pages. In-memory databases and the WASM browser backend allocate the cache but never use
it (all reads hit RAM regardless) — the option has no effect for those backends.

**`wal_checkpoint_threshold`** — Controls the write-latency / open-latency tradeoff. Lower = more
frequent checkpoints (smaller WAL, faster open, more checkpoint I/O). Higher = less frequent
checkpoints (larger WAL, faster writes, slower open on crash recovery). Default of 1000 suits mixed
workloads. For write-heavy batch ingestion, raise to `usize::MAX` to disable auto-checkpoint and
call `db.checkpoint()` manually after the batch.

**`max_derived_facts` / `max_results`** — Safety limits, not performance knobs. Lower them to fail
fast on runaway recursive rules or unexpectedly large result sets; raise only if a legitimate query
hits the ceiling.

### Section 3 — Query Patterns

Five prefer/avoid pairs, each with a one-line rationale and a Datalog or Rust example.

**1. Anchor with a concrete entity or attribute**

A full scan occurs only when both entity and attribute positions are unbound variables in every
pattern. Binding either triggers the selective index fetch path.

```datalog
; Full scan — both entity and attribute are variables
(query [:find ?e ?a ?v :where [?e ?a ?v]])

; Selective fetch — concrete attribute keyword
(query [:find ?e ?name :where [?e :person/name ?name]])

; Selective fetch — concrete entity keyword
(query [:find ?name :where [:alice :person/name ?name]])
```

**2. Use `or` in a rule body, not mid-query**

`or` mid-query re-evaluates branches over the full incoming binding set (O(N²)). The same logic
expressed as a rule starts from an empty binding and expands O(N).

```datalog
; O(N²) — or mid-query
(query [:find ?e :where (or [?e :tag :a] [?e :tag :b])])

; O(N) — equivalent rule body
(rule [(tagged ?e)] [?e :tag :a])
(rule [(tagged ?e)] [?e :tag :b])
(query [:find ?e :where (tagged ?e)])
```

**3. Keep `not` / `not-join` selective**

The negation check scans all matching facts for each binding. Place the most selective positive
patterns first so `not` sees a small binding set. The optimizer reorders positive patterns by
selectivity but does not move `not` / `or` — manual ordering still matters.

```datalog
; Worse — not sees all entities
(query [:find ?e
        :where [?e :role :admin]
               (not [?e :status :suspended])
               [?e :dept :engineering]])

; Better — most selective pattern first
(query [:find ?e
        :where [?e :dept :engineering]
               [?e :role :admin]
               (not [?e :status :suspended])])
```

**4. Use prepared queries for repeated patterns**

`db.prepare()` pays the parse cost once. Each `execute()` substitutes bind values and runs the
already-parsed plan. Especially valuable for AI agents that issue the same query pattern in a loop.

```rust
let pq = db.prepare("(query [:find ?name :where [?e :person/name $name]])")?;
for name in names {
    let results = pq.execute([BindValue::Val(Value::String(name))])?;
}
```

**5. Limit recursive rule depth**

Recursive rules use semi-naive fixed-point iteration. Each iteration extends the frontier by one
hop; deep chains require many iterations over growing intermediate tables (depth-10 = 2.75ms;
depth-100 = 16s from benchmark data). Where possible, bound the depth explicitly or use
`max_derived_facts` as a backstop.

```datalog
; Unbounded — can be very slow on deep graphs
(rule [(reachable ?a ?b)] [?a :edge ?b])
(rule [(reachable ?a ?b)] [?a :edge ?mid] (reachable ?mid ?b))

; Bounded — stops at depth 5
(rule [(reachable-5 ?a ?b 0)] [?a :edge ?b])
(rule [(reachable-5 ?a ?b ?d)]
      [?a :edge ?mid]
      (reachable-5 ?mid ?b ?prev)
      [(+ ?prev 1) ?d]
      [(< ?d 5)])
```

### Section 4 — Benchmark Reference

Links to full numbers and reproduction commands. No numbers duplicated on the wiki page — they
change per release and the source of truth is `BENCHMARKS.md`.

Reproduction commands:

```bash
# Run all Criterion benchmark groups (HTML report in target/criterion/)
cargo bench

# Run a specific group
cargo bench -- "insert"
cargo bench -- "query"
cargo bench -- "negation"
cargo bench -- "concurrent_btree_scan"

# Memory profile with heaptrack (requires heaptrack installed)
cargo build --release --example memory_profile
heaptrack ./target/release/examples/memory_profile 100000
heaptrack_print -f heaptrack.memory_profile.*.zst --merge-backtraces=0
```

Key tables to consult in BENCHMARKS.md: Query Latency (query cost at your target DB size), Database
Open / Replay (startup cost), Batch Insert Throughput (ingestion planning), Negation and Disjunction
(O(N²) cost confirmation).

### Section 5 — Platform Notes

| Platform | Differences |
|---|---|
| **Native (file-backed)** | Full feature set. WAL, checkpoint, `page_cache_size`, and `PreparedQuery` all apply. |
| **Native (in-memory)** | No WAL or checkpoint. `page_cache_size` is allocated but all reads hit RAM — the option has no effect. Tracked for fix in #274. |
| **WASM (browser)** | `BrowserBufferBackend` pre-loads all pages into RAM at open time. `wal_checkpoint_threshold` is ignored (no WAL sidecar). `page_cache_size` has no effect — same as in-memory native. Tracked for fix in #275. |
| **WASI** | Same as native file-backed except filesystem access goes through the WASI sandbox. All knobs apply. |
| **Android / iOS (UniFFI)** | Same embedded model as native. Keep `wal_checkpoint_threshold` low (100–200) to reduce WAL replay time on cold open under OS storage restrictions. |
| **Python / Node.js / Java (FFI)** | `PreparedQuery` is not exposed over UniFFI or napi-rs — prepare/execute must be done in Rust. Per-call FFI overhead is negligible compared to query cost. |

## Navigation Changes

### `Home.md`

Add a **Performance** entry in the Reference section (not a new section — it is a reference page):

```markdown
- [Performance Tuning](Performance-Tuning) — cost model, configuration knobs, query patterns,
  and benchmark reference
```

### `_Sidebar.md`

Add under Reference:

```markdown
- [Performance Tuning](Performance-Tuning)
```

## BENCHMARKS.md Update

The Known Limitations section still says "Index-based predicate pushdown for sub-linear lookups is
in the post-1.0 backlog (B+Tree Selective Lookup)" — this is stale since #208 is closed. The
implementation task must update that paragraph to reflect the current state (selective fetch
implemented, `not`/`or` O(N²) is the remaining limitation).

## Acceptance Criteria Mapping

| Acceptance criterion (issue #191) | Covered by |
|---|---|
| Explain indexes | Section 1 cost model (selective fetch path, EAVT/AVET index description) |
| Page cache sizing | Section 2 (`page_cache_size` knob) |
| Checkpointing | Section 2 (`wal_checkpoint_threshold` knob + manual `checkpoint()`) |
| Prepared statements | Section 3 (query pattern 4) |
| Common query-shape costs | Section 1 (cost table) + Section 3 (patterns 1–5) |
| Link to benchmark results and reproduction commands | Section 4 |
| Guidance for native, WASM, mobile, language bindings | Section 5 |

## Out of Scope

- Hash-join optimization for `not`/`or` — tracked separately in post-1.0 backlog.
- Query profiler / `EXPLAIN`-style output — tracked as issue #185 (Wave 5, milestone 2.0).
- SIMD / hardware-specific tuning — covered by #229 (Wave 2, closed); numbers in BENCHMARKS.md.
