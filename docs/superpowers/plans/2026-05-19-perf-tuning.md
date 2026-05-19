# Performance Tuning Guide Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write a single `Performance-Tuning.md` wiki page and update navigation and BENCHMARKS.md to match.

**Architecture:** Four files touched: one new wiki page with full content, two wiki navigation files updated with one line each, and BENCHMARKS.md with one stale bullet replaced. No Rust code changes. Wiki changes are committed to the `.wiki/` git repo (separate from the main repo).

**Tech Stack:** Markdown, Minigraf Datalog, Rust API snippets (documentation only — nothing compiled).

---

## Context

- Design spec: `docs/superpowers/specs/2026-05-19-perf-tuning-design.md`
- GitHub issue: #191
- Wiki repo lives at `.wiki/` — it is its own git repo with a `master` branch, separate from the main repo.
- The wiki already has a Cookbook section in both `Home.md` and `_Sidebar.md` — follow the same pattern.
- `BENCHMARKS.md` Known Limitations first bullet is stale: it still says selective B+Tree lookup is "in the post-1.0 backlog" but #208 (selective lookup) and #207 (predicate push-down) are both closed and merged.

---

### Task 1: Create `Performance-Tuning.md`

**Files:**
- Create: `.wiki/Performance-Tuning.md`

- [ ] **Step 1: Create the file with the following exact content**

Use the Write tool (preferred) or an editor to create `.wiki/Performance-Tuning.md`. The bash
heredoc below is shown for reference but will break in most shells because the content contains
triple-backtick fences — use the Write tool instead.

Content to write:

```
# Performance Tuning

This page covers what affects Minigraf's speed, which configuration knobs exist, and how to write
queries that perform well. Organized cost-model first — the knobs only make sense once you
understand where time is spent.

---

## Cost Model

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
patterns in the query. If the count is 1–4, it uses index-backed fetches instead of a full scan;
beyond 4, a full scan is cheaper and the engine falls back to it automatically.

---

## Configuration Knobs

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

**`page_cache_size`** — File-backed databases only. The LRU cache holds recently read B+tree pages
in memory; a cache miss triggers a disk read. The default (256 pages = 1MB) covers a ~10K-fact
database comfortably. For a 100K-fact database with repeated queries over the same pages, raise to
1024–4096 pages. In-memory databases and the WASM browser backend have no use for this option — all
reads hit RAM regardless.

**`wal_checkpoint_threshold`** — Controls the write-latency / open-latency tradeoff. Lower = more
frequent checkpoints (smaller WAL, faster open, more checkpoint I/O). Higher = less frequent
checkpoints (larger WAL, faster writes, slower open on crash recovery). The default of 1000 suits
mixed workloads. For write-heavy batch ingestion, set to `usize::MAX` to disable auto-checkpoint
and call `db.checkpoint()` manually after the batch.

**`max_derived_facts` / `max_results`** — Safety limits, not performance knobs. Lower them to fail
fast on runaway recursive rules or unexpectedly large result sets; raise only if a legitimate query
hits the ceiling.

---

## Query Patterns

### Anchor with a concrete entity or attribute

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

Almost all real queries bind at least one attribute keyword, so selective fetch applies by default.

### Use `or` in a rule body, not mid-query

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

### Keep `not` / `not-join` selective

The negation check scans all matching facts for each binding. Place the most selective positive
patterns before `not` so it sees a small binding set. The optimizer reorders positive patterns by
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

### Use prepared queries for repeated patterns

`db.prepare()` pays the parse cost once. Each `execute()` substitutes bind values and runs the
already-parsed plan. Especially valuable for AI agents that issue the same query pattern in a loop.

```rust
let pq = db.prepare("(query [:find ?name :where [?e :person/name $name]])")?;
for name in names {
    let results = pq.execute([BindValue::Val(Value::String(name))])?;
}
```

### Limit recursive rule depth

Recursive rules use semi-naive fixed-point iteration. Each iteration extends the frontier by one
hop; deep chains require many iterations over growing intermediate tables. From benchmark data:
depth-10 chain = 2.75ms; depth-100 chain = 16s.

Where possible, bound the depth explicitly, or use `max_derived_facts` as a backstop.

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

---

## Benchmark Reference

Full results with per-group commentary: [BENCHMARKS.md](../blob/main/BENCHMARKS.md).

Live history across releases: [bencher.dev/perf/minigraf/plots](https://bencher.dev/perf/minigraf/plots).

Key tables to check before tuning:
- **Query Latency** — cost at your target DB size
- **Database Open / Replay** — startup cost
- **Batch Insert Throughput** — ingestion planning
- **Negation** and **Disjunction** — O(N²) cost at scale

**Reproducing locally:**

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

---

## Platform Notes

| Platform | Differences |
|---|---|
| **Native (file-backed)** | Full feature set. WAL, checkpoint, `page_cache_size`, and `PreparedQuery` all apply. |
| **Native (in-memory)** | No WAL or checkpoint. `page_cache_size` has no effect — all reads hit RAM. |
| **WASM (browser)** | `BrowserBufferBackend` pre-loads all pages into RAM at open time. `wal_checkpoint_threshold` is ignored (no WAL sidecar). `page_cache_size` has no effect — same as in-memory native. |
| **WASI** | Same as native file-backed. All knobs apply. |
| **Android / iOS (UniFFI)** | Same embedded model as native. Keep `wal_checkpoint_threshold` low (100–200) to reduce WAL replay time on cold open under OS storage restrictions. |
| **Python / Node.js / Java (FFI)** | `PreparedQuery` is not exposed over UniFFI or napi-rs — prepare/execute must be done in Rust. Per-call FFI overhead is negligible compared to query cost. |
```

- [ ] **Step 2: Verify the file was created correctly**

```bash
head -5 .wiki/Performance-Tuning.md
```

Expected output:
```
# Performance Tuning

This page covers what affects Minigraf's speed, which configuration knobs exist, and how to write
queries that perform well. Organized cost-model first — the knobs only make sense once you
```

Also verify no broken code fence markers (triple backtick inside a heredoc can occasionally mis-parse — check visually):

```bash
grep -c '^\`\`\`' .wiki/Performance-Tuning.md
```

Expected: `24` (12 opening + 12 closing fences across 6 code blocks).

> **Note:** If the heredoc approach causes issues with the nested code fences, create the file using the Write tool or a text editor instead. The content is defined above.

---

### Task 2: Update wiki navigation

**Files:**
- Modify: `.wiki/Home.md`
- Modify: `.wiki/_Sidebar.md`

The current end of the `## Pages` section in `Home.md` (line 43):
```
- **[Learning Resources](Learning-Resources)** — Curated links for Datalog, temporal databases, and SQLite internals
```

The current end of the `## Reference` section in `_Sidebar.md` (line 27):
```
- [Learning Resources](Learning-Resources)
```

- [ ] **Step 1: Add Performance Tuning to `Home.md` Pages section**

Open `.wiki/Home.md`. After the `Learning Resources` line in `## Pages`, add:

```markdown
- **[Performance Tuning](Performance-Tuning)** — cost model, configuration knobs, query patterns, and benchmark reference
```

The `## Pages` section should end as:

```markdown
- **[Comparison](Comparison)** — Side-by-side comparison with XTDB, Cozo, Datomic, GraphLite, petgraph, IndraDB, SurrealDB, and time-series databases; temporal vs. time-series explainer
- **[Learning Resources](Learning-Resources)** — Curated links for Datalog, temporal databases, and SQLite internals
- **[Performance Tuning](Performance-Tuning)** — cost model, configuration knobs, query patterns, and benchmark reference
```

- [ ] **Step 2: Add Performance Tuning to `_Sidebar.md` Reference section**

Open `.wiki/_Sidebar.md`. After the `Learning Resources` line in `## Reference`, add:

```markdown
- [Performance Tuning](Performance-Tuning)
```

The `## Reference` section should end as:

```markdown
## Reference

- [Home](Home)
- [Datalog Reference](Datalog-Reference)
- [Architecture](Architecture)
- [Use Cases](Use-Cases)
- [Comparison](Comparison)
- [Learning Resources](Learning-Resources)
- [Performance Tuning](Performance-Tuning)
```

- [ ] **Step 3: Verify both navigation files contain the new entry**

```bash
grep "Performance-Tuning" .wiki/Home.md .wiki/_Sidebar.md
```

Expected output (two matching lines, one per file):
```
.wiki/Home.md:- **[Performance Tuning](Performance-Tuning)** — cost model, configuration knobs, query patterns, and benchmark reference
.wiki/_Sidebar.md:- [Performance Tuning](Performance-Tuning)
```

---

### Task 3: Fix stale BENCHMARKS.md Known Limitations

**Files:**
- Modify: `BENCHMARKS.md:454`

The current line 454 in `BENCHMARKS.md`:
```
- **Query scan is O(facts)**: Queries resolve all facts matching the range scan, then filter in memory. The per-query index rebuild (EAVT/AEVT/AVET/VAET) was eliminated in Phase 7.4 for the non-rules path. Index-based predicate pushdown for sub-linear lookups is in the post-1.0 backlog (B+Tree Selective Lookup).
```

This is stale: #208 (selective B+Tree lookup) and #207 (predicate push-down) are both closed and merged.

- [ ] **Step 1: Replace the stale Known Limitations first bullet**

In `BENCHMARKS.md`, replace the first bullet of `## Known Limitations` (the entire line starting with `- **Query scan is O(facts)**`) with:

```markdown
- **Query scan**: Queries with a concrete entity or attribute keyword in at least one pattern use selective index-backed fetches — O(k), where k = facts for that entity/attribute (#208). Queries with no bound entity or attribute fall back to a full scan — O(facts). Expression predicates are pushed down to the earliest point where their variables are bound (#207). `not` / `not-join` and `or` / `or-join` mid-query remain O(N²) in the worst case — no hash-join step yet.
```

- [ ] **Step 2: Verify the replacement**

```bash
grep -A2 "## Known Limitations" BENCHMARKS.md
```

Expected output:
```
## Known Limitations

- **Query scan**: Queries with a concrete entity or attribute keyword in at least one pattern use selective index-backed fetches — O(k), where k = facts for that entity/attribute (#208). Queries with no bound entity or attribute fall back to a full scan — O(facts). Expression predicates are pushed down to the earliest point where their variables are bound (#207). `not` / `not-join` and `or` / `or-join` mid-query remain O(N²) in the worst case — no hash-join step yet.
```

- [ ] **Step 3: Commit the BENCHMARKS.md fix to the main repo**

```bash
git add BENCHMARKS.md
git commit -m "docs: update BENCHMARKS.md Known Limitations — selective lookup and predicate push-down shipped (#207, #208)"
```

---

### Task 4: Commit and push wiki repo

**Files:**
- `.wiki/Performance-Tuning.md` (created in Task 1)
- `.wiki/Home.md` (modified in Task 2)
- `.wiki/_Sidebar.md` (modified in Task 2)

- [ ] **Step 1: Verify git status in the wiki repo**

```bash
cd .wiki && git status
```

Expected output: three modified/new files listed:
```
On branch master
Changes not staged for commit:
  modified:   Home.md
  modified:   _Sidebar.md

Untracked files:
  Performance-Tuning.md
```

- [ ] **Step 2: Commit all three wiki files**

```bash
cd .wiki && git add Performance-Tuning.md Home.md _Sidebar.md && git commit -m "docs: add Performance-Tuning wiki page and navigation (#191)"
```

- [ ] **Step 3: Push wiki to origin**

```bash
cd .wiki && git push origin master
```

Expected: push succeeds with no errors. If the push is rejected due to upstream changes, pull and rebase first:

```bash
cd .wiki && git pull --rebase origin master && git push origin master
```

- [ ] **Step 4: Verify the wiki page is live**

```bash
cd .wiki && git log --oneline -3
```

Expected: the new commit is the most recent entry.

Also verify the new page exists at the expected path:

```bash
ls .wiki/Performance-Tuning.md
```

Expected: file present.

- [ ] **Step 5: Commit the plan file to the main repo and close #191**

```bash
git add docs/superpowers/plans/2026-05-19-perf-tuning.md
git commit -m "docs: add implementation plan for performance tuning guide (#191)"
```

Then close the issue:

```bash
gh issue close 191 --comment "Performance-Tuning wiki page published. Navigation updated in Home.md and _Sidebar.md. BENCHMARKS.md Known Limitations updated to reflect #207 and #208 completion."
```
