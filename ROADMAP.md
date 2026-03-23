# Minigraf Roadmap

> The path from property graph PoC to production-ready bi-temporal Datalog database

**Philosophy**: Embedded graph memory for agents, mobile, and the browser — built on the SQLite approach: be boring, be reliable, be embeddable.

---

## Phase 1: Property Graph PoC ✅ COMPLETE

**Goal**: Prove the concept works

**Status**: ✅ Completed (December 2025)

**What Was Built**:
- ✅ Property graph model (nodes, edges, properties)
- ✅ In-memory storage engine
- ✅ Query parser
- ✅ Query executor
- ✅ Interactive REPL console
- ✅ Basic test coverage

**Deliverable**: Working in-memory graph database with REPL

---

## Phase 2: Embeddability ✅ COMPLETE

**Goal**: Make it truly embeddable with persistence

**Status**: ✅ Completed (January 2026)

**What Was Built**:
- ✅ Storage backend abstraction (FileBackend, MemoryBackend)
- ✅ Single-file `.graph` format (4KB pages)
- ✅ Cross-platform file format (endian-safe)
- ✅ Persistent graph storage with serialization
- ✅ Embedded database API (`Minigraf::open()`)
- ✅ Auto-save on drop
- ✅ Thread-safe concurrent access
- ✅ Comprehensive test suite (54 tests)
- ✅ Edge case and concurrency tests

**Deliverable**: Embeddable persistent graph database

**Philosophy Alignment**: ✅ Single-file, self-contained, embedded-first

**Learnings**: Storage layer is solid. Datalog chosen for better temporal support.

---

## Phase 3: Datalog Core ✅ COMPLETE

**Goal**: Implement core Datalog query engine

**Status**: ✅ Completed (January 2026)

**Priority**: 🔴 Critical - Foundation for everything else

### 3.1 EAV Data Model ✅

**Features**:
- ✅ Migrate from property graph to Entity-Attribute-Value model
- ✅ Fact representation: `(Entity, Attribute, Value)`
- ✅ Entities as UUIDs (keep existing ID system)
- ✅ Attributes as keywords (`:person/name`, `:friend`)
- ✅ Values: primitives or entity references
- ✅ Update storage format to support EAV tuples

**Technical Approach**:
```rust
struct Fact {
    entity: Uuid,
    attribute: String,  // Will become typed Keyword later
    value: Value,
    tx_id: TxId,        // Transaction that asserted this fact
    asserted: bool,     // true = assert, false = retract
}

enum Value {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Ref(Uuid),  // Reference to another entity
    Keyword(String),
    Null,
}
```

**Migration Path**:
- Keep existing node/edge types for backward compat
- Add EAV layer on top
- Gradually migrate tests
- Eventually deprecate property graph types

### 3.2 Datalog Parser ✅

**Features**:
- ✅ Parse basic Datalog queries (EDN syntax)
- ✅ Query structure: `[:find ?vars :where [clauses]]`
- ✅ Pattern matching: `[?e :attr ?v]`
- ✅ Variable binding
- ✅ Constants and entity references

**Example Queries**:
```datalog
;; Find all entities with a name
[:find ?e
 :where [?e :person/name _]]

;; Find friends of Alice
[:find ?friend
 :where
   [?alice :person/name "Alice"]
   [?alice :friend ?friend]]

;; Find names of Alice's friends
[:find ?name
 :where
   [?alice :person/name "Alice"]
   [?alice :friend ?friend]
   [?friend :person/name ?name]]
```

**Parser Components**:
- EDN reader (simple S-expression parser)
- Query validation
- Variable extraction
- Pattern recognition

### 3.3 Query Executor (Basic) ✅

**Features**:
- ✅ Pattern matching against fact database
- ✅ Variable unification
- ✅ Join multiple patterns
- ✅ Return results as tuples

**Implementation**:
- Naive evaluation initially (optimize in Phase 6)
- Iterate through facts, unify variables
- Cartesian product + filter (like nested loops)
- Return binding sets

### 3.4 Recursive Rules ✅

**Features**:
- ✅ Define rules: `[(rule-name ?args) [body]]`
- ✅ Stratified recursion (safe subset)
- ✅ Rule evaluation via semi-naive evaluation
- ✅ Transitive closure queries

**Example Rules**:
```datalog
;; Define reachability
[(reachable ?from ?to)
 [?from :connected ?to]]

[(reachable ?from ?to)
 [?from :connected ?intermediate]
 (reachable ?intermediate ?to)]

;; Use in query
[:find ?person
 :where
   (reachable :alice ?person)]
```

**Technical Approach**:
- Fixed-point iteration
- Track delta (new facts) each round
- Stop when no new facts produced
- Detect cycles, prevent infinite loops

### 3.5 REPL for Datalog ✅

**Features**:
- ✅ Interactive Datalog console
- ✅ Transact facts
- ✅ Query with Datalog
- ✅ Pretty-print results

**Commands**:
```clojure
;; Transact facts
(transact [[:alice :person/name "Alice"]
           [:alice :person/age 30]])

;; Query
(query [:find ?name :where [?e :person/name ?name]])

;; Define rule
(rule [(friends ?a ?b) [?a :friend ?b] [?b :friend ?a]])

;; Help
(help)

;; Exit
(exit)
```

### 3.6 Tests ✅

**Test Coverage**:
- ✅ EAV data model CRUD (8 tests)
- ✅ Datalog parser (all query forms) (15 tests)
- ✅ Pattern matching (6 tests)
- ✅ Variable unification (included in matcher tests)
- ✅ Multi-pattern joins (10 integration tests)
- ✅ Recursive rule evaluation (15 tests)
- ✅ Transitive closure queries (9 integration tests)
- ✅ Concurrency (7 integration tests)

**Total: 123 tests passing** ✅

**Deliverable**: ✅ Working Datalog query engine with recursion (Complete!)

**Timeline**: ✅ Completed in ~3 weeks (January 2026)

---

## Phase 4: Bi-temporal Support ✅ COMPLETE

**Goal**: Add transaction time and valid time

**Status**: ✅ Completed (March 2026)

**Priority**: 🔴 Critical - Core differentiator

### 4.1 Transaction Time ✅

**Features**:
- ✅ Every fact records when it was added (`tx_id` wall-clock millis, `tx_count` monotonic counter)
- ✅ Facts are never deleted, only retracted (`asserted=false`)
- ✅ Query as of past transaction counter: `[:as-of 50]`
- ✅ Query as of past wall-clock time: `[:as-of "2024-01-15T10:00:00Z"]`

**Data Model**:
```rust
struct Fact {
    entity: EntityId,
    attribute: Attribute,
    value: Value,
    tx_id: TxId,       // wall-clock millis since epoch (u64)
    tx_count: u64,     // monotonically incrementing counter (1, 2, 3…)
    valid_from: i64,   // millis since epoch (i64)
    valid_to: i64,     // millis since epoch; i64::MAX = "forever"
    asserted: bool,
}
```

### 4.2 Valid Time ✅

**Features**:
- ✅ Facts carry validity period (`valid_from`, `valid_to`)
- ✅ `VALID_TIME_FOREVER = i64::MAX` sentinel for open-ended facts
- ✅ Query valid at specific time: `[:valid-at "2023-06-01"]`
- ✅ Default (no `:valid-at`): currently valid facts only
- ✅ `:any-valid-time` disables the valid time filter entirely
- ✅ Per-transaction and per-fact valid time overrides

### 4.3 Bi-temporal Queries ✅

```datalog
;; Full bi-temporal query
[:find ?status
 :valid-at "2023-06-01"
 :as-of "2024-01-15T10:00:00Z"
 :where [:alice :employment/status ?status]]

;; Transact with valid time
(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"}
          [[:alice :employment/status :active]])
```

### 4.4 Storage Format ✅

- **File format version** bumped 1→2
- Automatic v1→v2 migration on open (assigns `tx_count`, sets temporal defaults)
- Fixed latent Phase 3 bug: `tx_id` now preserved on load via `load_fact()`

### 4.5 Tests ✅

- ✅ 10 new integration tests (`tests/bitemporal_test.rs`)
- ✅ 39 new unit tests (types, storage, parser, executor)
- Transaction time travel (counter + timestamp)
- Valid time filtering (inside/outside range, boundary, default)
- Combined bi-temporal queries
- File format migration

**Deliverable**: ✅ Full bi-temporal Datalog database (Complete!)

**Timeline**: ✅ Completed in ~3 weeks (March 2026)

---

## Phase 5: ACID + WAL ✅ COMPLETE

**Goal**: Add crash safety and transactions

**Status**: ✅ Completed (March 2026)

**Priority**: 🟡 High

### 5.1 Write-Ahead Logging (WAL) ✅

**Features**:
- ✅ Fact-level sidecar WAL (embedded in `.graph` file)
- ✅ CRC32-protected WAL entries
- ✅ Crash recovery (WAL replay on open)
- ✅ Checkpoint mechanism (WAL → .graph, then WAL deleted)
- ✅ `FileHeader` v3 (`last_checkpointed_tx_count` field)

**Why Embedded WAL**:
- ✅ Maintains single-file philosophy
- ✅ Easy to backup/share (one file)
- ✅ Simpler user experience

### 5.2 Transactions ✅

**Features**:
- ✅ `WriteTransaction` API: `begin_write`, `commit`, `rollback`
- ✅ Thread-safe: concurrent readers + exclusive writer
- ✅ ACID compliance:
  - Atomicity: All-or-nothing transactions
  - Consistency: Enforced via WAL
  - Isolation: Exclusive write lock
  - Durability: WAL ensures persistence

**API**:
```rust
let mut tx = db.begin_write()?;
tx.execute("(transact [[:alice :person/name \"Alice\"]])")?;
tx.commit()?;  // or tx.rollback()?
```

### 5.3 Crash Recovery ✅

**Features**:
- ✅ Detect uncommitted WAL entries on open
- ✅ Replay committed WAL entries to reconstruct state
- ✅ CRC32 checksum validation
- ✅ Incomplete entries discarded (not replayed)

### 5.4 Tests ✅

**Test Coverage**:
- ✅ WAL write and read
- ✅ Transaction commit/rollback
- ✅ Crash recovery (WAL replay on open)
- ✅ Recovery from partial/corrupt writes
- ✅ Checkpoint correctness
- ✅ Concurrent readers + exclusive writer

**Total: 212 tests passing** ✅

**Deliverable**: ✅ ACID-compliant database with crash safety (Complete!)

**Timeline**: ✅ Completed in ~3 weeks (March 2026)

---

## Phase 6: Performance & Indexes 🚧 IN PROGRESS

**Goal**: Make queries fast

**Status**: 🚧 Phases 6.1, 6.2, 6.4a, and 6.4b complete; 6.5 next

**Priority**: 🟡 High

### 6.1 Covering Indexes ✅ COMPLETE

**What Was Built**:
- ✅ EAVT, AEVT, AVET, VAET covering indexes (Datomic-style, bi-temporal keys)
- ✅ `FactRef { page_id, slot_index }` — forward-compatible disk location pointer
- ✅ Canonical value encoding with sort-order-preserving byte comparison
- ✅ B+tree page serialisation for index persistence (`btree.rs`)
- ✅ `FileHeader` v4: `eavt/aevt/avet/vaet_root_page` + `index_checksum` (CRC32)
- ✅ Auto-rebuild on checksum mismatch
- ✅ File format v1/v2/v3→v4 migration on first save

### 6.2 Packed Pages + LRU Page Cache ✅ COMPLETE

**What Was Built**:
- ✅ Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page (~25× space reduction)
- ✅ LRU page cache with approximate-LRU semantics (read-lock on hits)
- ✅ `CommittedFactReader` trait: index-driven on-demand fact resolution
- ✅ Eliminated "load all facts at startup" — only pending facts in memory
- ✅ EAVT/AEVT range scans for `get_facts_by_entity` / `get_facts_by_attribute`
- ✅ `FileHeader` v5 (`fact_page_format` byte); auto v4→v5 migration on open
- ✅ `OpenOptions::page_cache_size(usize)` — tune cache capacity (default 256 pages = 1MB)

### 6.3 Query Optimization ✅ COMPLETE

**Features**:
- ✅ Selectivity-based join reordering (Phase 6.1, `optimizer.rs`)
- ✅ Index selection per pattern (`IndexHint` enum)

**Deferred to Phase 7**:
- Cost-based optimization improvements — better informed once negation, aggregation, and disjunction are implemented; the optimizer needs to cost-estimate query shapes that don't exist yet
- Rule evaluation optimization — same rationale; recursive rule evaluation interacts with the new clause types

**Note**: Phase 6.3 has no dedicated release version. Its completed items shipped as part of Phase 6.1 (v0.6.0); its deferred items will ship as part of Phase 7 (v0.9.0). The release strategy jumps directly from v0.7.0 (Phase 6.2) to v0.8.0 (Phase 6.4) by design.

### 6.4a Retraction Semantics Fix + Edge Case Tests ✅ COMPLETE

**What Was Fixed**:
- ✅ Retraction semantics in `executor.rs:filter_facts_for_query` Step 2: now computes the *net view* per `(entity, attribute, value)` triple via `net_asserted_facts()` — the record with the highest `tx_count` in the tx window determines whether the triple is asserted or retracted
- ✅ `net_asserted_facts()` helper (`src/graph/storage.rs`): groups by EAV triple, keeps latest by `tx_count`, discards if latest is a retraction; shared by executor and storage
- ✅ `check_fact_sizes()` early validation (`src/db.rs`): rejects oversized facts before WAL write using `MAX_FACT_BYTES` constant from `packed_pages.rs`

**Tests Added**:
- ✅ `tests/retraction_test.rs` — 7 tests: assert/retract (no `:as-of`), as-of snapshot before/after retraction, re-assert after retract, `:any-valid-time` combo, recursive rule retraction visibility
- ✅ `tests/edge_cases_test.rs` — 4 tests: oversized-fact file-backed error path, `MAX_FACT_BYTES` boundary, in-memory has no size limit

**Test Coverage**: 298 tests (213 unit + 79 integration + 6 doc)

---

### 6.4b Benchmarks + Light Publish Prep ✅ COMPLETE

**Benchmark Suite**:
- ✅ Full Criterion suite run (9 groups; insert, query, time-travel, recursion, open, checkpoint, concurrency)
- ✅ Memory profiling via heaptrack at 10K / 100K / 1M facts (peak heap 14 MB → 136 MB → 1.33 GB)
- ✅ `BENCHMARKS.md` — full tables + machine spec + known limitations + reproduction instructions
- ✅ `README.md` Performance section updated with Phase 6.4b numbers and link to `BENCHMARKS.md`
- ✅ `examples/memory_profile.rs` — heaptrack profiling binary

**Light Publish Prep** (low-risk, no API changes):
- ✅ Removed dead `clap` dependency from library `[dependencies]`
- ✅ Complete `Cargo.toml` metadata (`repository`, `keywords`, `categories`, `readme`, `documentation`)
- ✅ Version bumped to v0.8.0

**Community Infrastructure**:
- ✅ GitHub Discussions enabled — minimum viable channel for questions and feedback
- ✅ `CONTRIBUTING.md` — dedicated contributing guide (extracted and expanded from README)
- ✅ `CODE_OF_CONDUCT.md` — Contributor Covenant reference
- ✅ Issue templates — bug report and feature request (`.github/ISSUE_TEMPLATE/`)
- ✅ PR template — checklist enforcing test/clippy/fmt/philosophy checks (`.github/pull_request_template.md`)
- ✅ `CODEOWNERS` — auto-assigns maintainer as reviewer on every PR

**Note**: crates.io publish deferred to after Phase 6.5. The file format will change (v6) in 6.5; publishing before that would expose early users to an immediate forced migration. The full pre-publishing checklist (API narrowing, rustdoc, clippy, `unwrap()` sweep) is a Phase 6.5 gate item.

---

## Phase 6.5: On-Disk B+Tree Indexes ✅ COMPLETE

**Goal**: Replace the current paged-blob index serialisation with proper on-disk B+tree pages, so index lookups and range scans never require loading the full index into memory

**Status**: ✅ Completed (March 2026)

**Priority**: 🟡 High — required before Phase 8 (mobile); conditional on Phase 6.4 findings

**Rationale**:

The current index implementation (`btree.rs`) is a paged blob serialiser, not a true on-disk B+tree. The full round-trip is:

- **Open**: load all index pages → deserialize entire `BTreeMap` into memory
- **Query**: use in-memory `BTreeMap` (fast for small indexes)
- **Checkpoint**: serialize entire `BTreeMap` → rewrite all index pages (100% write amplification)

This works well at small scale. At 100K–1M facts — or on a mobile device with constrained RAM shared across all apps — the full in-memory index becomes a hard constraint. Phase 6.4 benchmarks will quantify the memory cost; this phase addresses it.

A proper on-disk B+tree maps directly onto the existing `StorageBackend` trait:
- Each B+tree node is one 4KB page (internal node or leaf node)
- **Open**: read root page only
- **Lookup**: traverse pages on demand — the LRU cache already handles page-level caching
- **Range scan**: follow leaf-node chain pages
- **Insert**: write only the path from root to modified leaf (typically 2–4 pages)

The LRU page cache (`cache.rs`) already abstracts page-level I/O correctly; this phase plugs a proper B+tree into that abstraction.

**File format**: v6

Current v5 stores index data as paged blobs (page type `0x11`). v6 introduces proper B+tree node pages (internal nodes + leaf nodes, new page type). The four index root page pointers in `FileHeader` already exist (`eavt_root_page`, `aevt_root_page`, `avet_root_page`, `vaet_root_page`) — they just point to paged blobs today and will point to B+tree roots after this phase.

**Migration**: v5→v6 reads the old paged-blob indexes, rebuilds them into proper B+tree pages, writes new root pointers to the header. Automatic on first open, same pattern as all prior migrations.

**Implementation plan**:

1. **B+tree node page format**: Define internal node layout (keys + child page IDs) and leaf node layout (keys + `FactRef` values) within a 4KB page. Fill factor ~75% to leave room for insertions without immediate splits.
2. **B+tree operations**: `search(key)`, `range_scan(start, end)`, `insert(key, value)`, `split_node()`. These operate on pages via `StorageBackend` + LRU cache.
3. **Index integration**: Replace `write_all_indexes` / `read_*_index` in `btree.rs` with the new B+tree backed by pages. Update `persistent_facts.rs` to use page-level index operations instead of full-BTreeMap serialisation.
4. **Remove load-all-at-startup**: Index no longer needs to be loaded into memory on open. `FactStorage` index lookups go through the page cache.
5. **File format v6 + migration**: New `FileHeader` version, v5→v6 migration on first checkpoint after open.
6. **Tests**: B+tree node split/merge correctness, range scan across multiple leaf pages, index rebuild from fact pages, v5→v6 migration roundtrip, concurrent read/write correctness.

**Expected impact**:
- Memory: index memory usage drops from O(facts) to O(cache_pages) — same bound as fact pages
- Write amplification: checkpoint writes O(changed_paths) pages instead of O(all_index_pages)
- Startup: open time drops from O(index_size) to O(1)
- Mobile: makes Minigraf viable on memory-constrained devices without special tuning

**Deliverable**: All four covering indexes (EAVT, AEVT, AVET, VAET) backed by proper on-disk B+tree pages; file format v6 with automatic v5 migration; index memory usage proportional to cache size, not database size

**Timeline**: 4-6 weeks

---

## Phase 7: Datalog Completeness 🎯 FUTURE

**Goal**: Complete the Datalog query engine — negation, aggregation, disjunction, temporal range queries, and prepared statements

**Status**: 🎯 Planned

**Priority**: 🔴 Critical — without these, realistic production queries cannot be expressed in Datalog

**Rationale**: The highlighted use cases (agentic memory, audit, mobile, browser) all require at minimum negation and aggregation. Expanding to mobile and WASM platforms before the query engine can express production-grade queries means shipping an incomplete product to more places. All features are additive — existing queries continue to work unchanged. Semantics are well-established (Datomic and XTDB are production references for all three).

**Sub-phases**:
- **7.1** Stratified Negation (`not` / `not-join`)
- **7.2** Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`) — includes arithmetic filter predicates
- **7.3** Disjunction (`or` / `or-join`)
- **7.4** Query Optimizer Improvements (deferred from Phase 6.3)
- **7.5** Tests + Error Coverage (≥90% branch coverage target)
- **7.6** Prepared Statements (parse + plan once, execute many times, temporal bind slots)
- **7.7** Temporal Metadata Bindings + Range Queries (`:db/valid-from`, `:db/valid-to`, `:db/tx-count` as queryable pseudo-attributes; unlocks Time Interval, Time-Point Lookup, Time-Interval Lookup query classes)

### 7.1 Stratified Negation (`not` / `not-join`)

**Goal**: Express "find entities where attribute X is absent" and similar absence queries.

**Why it's load-bearing**:
- Agentic memory: "what has the agent not verified?", "beliefs with no supporting evidence"
- Audit: "contracts without a sign-off event", "records missing a required field"
- Developer tooling: "modules with no dependents", "entities never retracted"
- Without negation these queries require pulling results into application memory and filtering — defeating the query engine

**Semantics**: Stratified negation (Datalog^¬) — the standard safe subset. The rule dependency graph is analysed at query time; programs where negation creates a recursive cycle are rejected with a clear error (unstable semantics). Non-recursive negation is always safe.

**Syntax** (Datomic-inspired):
```datalog
;; not — exclude bindings where sub-clause matches
(query [:find ?e
        :where [?e :person/name _]
               (not [?e :person/age _])])

;; not-join — exclude with shared variables from outer scope
(query [:find ?e
        :where [?e :task/status :pending]
               (not-join [?e]
                 [?e :task/blocked-by _])])
```

**Implementation**:
- Parser: add `not` and `not-join` clause types to `WhereClause` enum
- Stratification analysis: build rule dependency graph, detect negation cycles, return error if unstable
- Executor: evaluate negative sub-query against current binding set, subtract matching bindings
- All changes additive; existing queries unaffected

**Estimated complexity**: 2-4 weeks

### 7.2 Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`)

**Goal**: Express counting, summing, and extremes directly in queries rather than post-processing in application code.

**Why it's load-bearing**:
- Audit / compliance: "how many transactions in this time window?", "total value asserted per entity"
- Agentic memory: "how many beliefs does the agent hold about entity X?"
- Analytics: "most-referenced entity", "earliest valid-from per attribute"
- Without aggregation, every app that needs a count writes its own post-processing loop

**Syntax** (Datomic-inspired):
```datalog
;; count
(query [:find (count ?e)
        :where [?e :person/name _]])

;; sum with grouping (:with specifies grouping variables)
(query [:find ?dept (sum ?salary)
        :with ?e
        :where [?e :employee/dept ?dept]
               [?e :employee/salary ?salary]])

;; min / max
(query [:find (min ?ts)
        :where [?e :event/timestamp ?ts]])

;; distinct collect
(query [:find (distinct ?tag)
        :where [?e :note/tag ?tag]])
```

**Implementation**:
- Parser: add aggregate expression variants to the `:find` clause; add `:with` clause
- Types: `FindSpec::Aggregate { func, var }`, `AggregateFunc` enum
- Executor: post-process binding sets — group by non-aggregate find variables, apply aggregate functions; no changes to core evaluation engine
- All changes additive

**Estimated complexity**: 2-3 weeks

### 7.3 Disjunction (`or` / `or-join`)

**Goal**: Express "match condition A or condition B" without running two queries and unioning in application code.

**Why it's useful** (lower urgency than 7.1 and 7.2 — can be worked around, but becomes painful in complex rules):
- "Find notes tagged :work or :urgent"
- "Find entities where :status is :active or :pending"
- Recursive rules with branching reachability conditions

**Syntax** (Datomic-inspired):
```datalog
;; or — all branches must bind the same variables
(query [:find ?e
        :where (or [?e :task/status :active]
                   [?e :task/status :pending])])

;; or-join — branches may bind different variables, explicit join vars declared
(query [:find ?e
        :where (or-join [?e]
                 [?e :employee/dept :engineering]
                 (and [?e :employee/role :contractor]
                      [?e :employee/dept :product]))])
```

**Implementation**:
- Parser: add `or` and `or-join` clause types; add `and` grouping clause
- Executor: evaluate each branch independently against current binding set, union results, deduplicate
- `or-join`: validate that declared join variables appear in all branches
- All changes additive

**Estimated complexity**: 2-3 weeks

### 7.4 Query Optimizer Improvements (deferred from Phase 6.3)

- 🎯 Cost-based optimization improvements — extend `optimizer.rs` with cost estimates for the new clause types (negation sub-queries, aggregate post-processing, disjunction branch selection)
- 🎯 Rule evaluation optimization — improve semi-naive evaluation for rules that include `not`, `or`, and aggregate expressions

**Note**: These items were originally scoped in Phase 6.3 but deferred here because meaningful cost estimates require the new clause types to exist first.

### 7.5 Tests + Error Coverage

- Unit tests for each new clause type (parser, types, matcher)
- Integration tests covering realistic production query patterns:
  - Absence queries with `not` / `not-join`
  - Aggregation with grouping, bi-temporal filters, and recursive rules
  - Disjunction in flat queries and rules
- Stratification rejection tests: programs with negation cycles must produce clear errors, not incorrect results
- Regression suite: all existing tests continue to pass

**Error handling coverage sweep**: Phase 7 adds significant new code paths (stratification analysis, aggregate post-processing, branch evaluation). Bring error-path coverage for new code to parity with happy-path coverage from the start, rather than letting it lag. Target: ≥90% overall branch coverage by end of Phase 7.

**Deliverable**: A Datalog engine that can express any query a production workload is likely to require — negation, aggregation, disjunction, and recursion, composable with bi-temporal filters; query optimizer extended to cost the new clause types; ≥90% branch coverage

**Timeline**: 6-8 weeks

### 7.6 Prepared Statements

**Goal**: Parse and plan a query once; execute it repeatedly with different bind values — including temporal filters — without re-parsing or re-planning on each call.

**Why now (after Phase 7.1–7.4, not before)**:

Phase 7 adds negation (stratification analysis), aggregation (post-processing), and disjunction (branch evaluation) — after which the plan cost becomes meaningfully larger. Designing bind parameter syntax *after* the full clause set exists also means the syntax doesn't need revisiting when new clause types are added. Before Phase 7, the parse + plan cost is small enough that caching it offers negligible benefit.

**Motivation — the agentic memory loop**:

The primary use case is an agent running the same query pattern thousands of times per session with different entity IDs and temporal coordinates:

```datalog
;; "What did the agent believe about entity X at transaction T?"
(query [:find ?belief
        :as-of $tx
        :where [$entity :belief/value ?belief]])
```

Without parameterised temporal filters, a new query string must be prepared for every `$tx` value — re-parsing and re-planning each time, defeating the purpose entirely. The query shape (patterns, join order, index selection) is identical regardless of what tx count, timestamp, or entity is supplied; only the substituted values change at execution time.

**Syntax**:

`$identifier` tokens in any bind-able position are treated as named slots:

```datalog
;; Bind slots in :where patterns, :as-of, and :valid-at
(query [:find ?status
        :as-of $tx
        :valid-at $date
        :where [$entity :employment/status ?status]])
```

**Bind slot positions and permitted types**:

| Position | Permitted `BindValue` variants | Notes |
|---|---|---|
| Entity position in pattern | `Entity(Uuid)` | `[$entity :attr ?val]` |
| Value position in pattern | `Val(Value)` | `[?e :attr $val]` — any `Value` variant |
| `:as-of` | `TxCount(u64)`, `Timestamp(i64)` | Counter or wall-clock millis |
| `:valid-at` | `Timestamp(i64)`, `AnyValidTime` | millis or `:any-valid-time` sentinel |

Attribute positions are intentionally **not** parameterisable — substituting the attribute name at execution time would make it impossible to select the correct index at prepare time, defeating plan caching entirely.

**API**:

```rust
// Prepare once — parses, validates, and computes the query plan
let prepared = db.prepare(
    "(query [:find ?status
             :as-of $tx
             :valid-at $date
             :where [$entity :employment/status ?status]])"
)?;

// Execute many times — plan reused, only bind values substituted
let r1 = prepared.execute(&[
    ("tx",     BindValue::TxCount(50)),
    ("date",   BindValue::Timestamp(1_685_577_600_000)),
    ("entity", BindValue::Entity(alice_id)),
])?;

let r2 = prepared.execute(&[
    ("tx",     BindValue::TxCount(75)),
    ("date",   BindValue::Timestamp(1_701_388_800_000)),
    ("entity", BindValue::Entity(bob_id)),
])?;

// Existing db.execute() string API is unchanged — no breaking change
```

**Plan stability note**:

The query optimizer uses selectivity estimates to pick join order. Different `:as-of` values could in theory affect fact counts and therefore optimal join order. The standard trade-off (used by PostgreSQL for generic plans) applies: use the plan computed at `prepare()` time and accept that it may be marginally suboptimal for some bind values. The amortised parse + plan saving across thousands of executions far outweighs occasional suboptimal join order.

**Implementation**:
- Parser: recognise `$identifier` as a `BindSlot` token in entity, value, `:as-of`, and `:valid-at` positions
- `DatalogQuery` type: add `bind_slots: Vec<BindSlot>` field
- New `PreparedQuery` struct: stores parsed AST + optimised plan + slot positions
- `Minigraf::prepare(query_str) -> Result<PreparedQuery>` — new public API method
- `PreparedQuery::execute(bindings: &[(&str, BindValue)]) -> Result<QueryResult>` — substitutes values, runs execution against current fact store state
- `db.execute(str)` path unchanged — no breaking change

**Tests**:
- Prepare + execute with entity bind slots
- Prepare + execute with value bind slots
- Prepare + execute with `:as-of $tx` (counter and timestamp variants)
- Prepare + execute with `:valid-at $date` and `:valid-at` `AnyValidTime`
- Combined temporal + entity parameterisation (the primary agentic loop pattern)
- Error: missing bind value at execute time
- Error: type mismatch (e.g., `Val` supplied for an `:as-of` slot)
- Attribute position rejected as a bind slot at prepare time

**Estimated complexity**: 2-3 weeks

---

### 7.7 Temporal Metadata Bindings + Range Queries

**Goal**: Expose `valid_from`, `valid_to`, and `tx_count` as first-class bindable values in Datalog `:where` clauses, unlocking the full four-class taxonomy of temporal queries described in the bi-temporal literature.

**Background — the four temporal query classes**:

Most people are familiar with point-in-time queries (`:as-of`, `:valid-at`), but a complete bi-temporal query model covers four classes:

| Class | Description | Minigraf before 7.7 |
|---|---|---|
| **Point-in-Time** | Snapshot of state at a specific moment | ✅ `:as-of` / `:valid-at` |
| **Time Interval** | Facts alive at any point during [T1, T2] | ⚠️ `:any-valid-time` only (no range predicate) |
| **Time-Point Lookup** | Given objects + criteria, find *when* those states existed | ❌ temporal metadata not queryable |
| **Time-Interval Lookup** | Find interval(s) where object states matched criteria | ❌ temporal metadata not queryable |

The root gap: `valid_from`, `valid_to`, and `tx_count` are stored per-fact but are invisible to the Datalog query engine. Classes 3 and 4 are entirely unreachable, and class 2 is a blunt instrument.

**Pseudo-attributes** (built-in, read-only, never stored as facts):

| Pseudo-attribute | Type | Meaning |
|---|---|---|
| `:db/valid-from` | `i64` (Unix ms) | Fact's valid-time start |
| `:db/valid-to` | `i64` (Unix ms) | Fact's valid-time end (`i64::MAX` = forever) |
| `:db/tx-count` | `u64` | Transaction counter at which fact was written |
| `:db/tx-id` | `Uuid` | Transaction UUID |

These bind as ordinary `?var` in patterns alongside `:any-valid-time`, which disables the engine's automatic valid-time filter so that temporal metadata is accessible:

```datalog
;; Time Interval — facts alive at any point during [T1, T2]
;; (valid_from <= T2 AND valid_to >= T1)
(query [:find ?e ?name
        :any-valid-time
        :where [?e :person/name ?name]
               [?e :db/valid-from ?vf]
               [?e :db/valid-to ?vt]
               [(<= ?vf 1704067200000)]   ;; vf <= T2
               [(>= ?vt 1696118400000)]]) ;; vt >= T1

;; Time-Point Lookup — find all moments when Alice's salary exceeded 100k
(query [:find ?vf
        :any-valid-time
        :where [:alice :person/salary ?s]
               [:alice :db/valid-from ?vf]
               [(> ?s 100000)]])

;; Time-Interval Lookup — find intervals when Alice was employed
(query [:find ?vf ?vt
        :any-valid-time
        :where [:alice :employment/status :employed]
               [:alice :db/valid-from ?vf]
               [:alice :db/valid-to ?vt]])
```

**Dependency on Phase 7.2**:

Arithmetic filter predicates — `[(op ?var literal)]` — are required for Time Interval and Time-Point Lookup queries. These predicates are also needed for aggregation (Phase 7.2). Phase 7.2 should implement the predicate evaluation infrastructure; Phase 7.7 then applies it to temporal metadata bindings.

**Implementation**:

- Parser: recognise `:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id` as `PseudoAttribute` tokens in attribute position
- Executor: when a `PseudoAttribute` appears, bind the corresponding field from the matched fact instead of filtering on a stored attribute value
- `FactStorage`: ensure `get_facts_by_*` scan paths return temporal metadata alongside matched facts when pseudo-attributes are present in the query plan
- Optimizer: pseudo-attribute patterns do not drive index selection (no AVET entry exists for pseudo-attributes); they are applied as post-scan filters
- `:any-valid-time` required in query to suppress automatic valid-time filtering when `:db/valid-from` / `:db/valid-to` are used in patterns

**Tests**:

- Time Interval query: facts alive during a half-open interval
- Time Interval query: facts alive for the *entire* interval (stricter predicate)
- Time-Point Lookup: find historic time points matching a value threshold
- Time-Interval Lookup: enumerate all validity intervals for an entity-attribute pair
- Bind `:db/tx-count` in `:where` and join it with a tx-time `:as-of` query
- `:db/tx-id` binding and join across two entities written in the same transaction
- Pseudo-attribute in entity or value position is rejected at parse time (parse error)
- Pseudo-attribute without `:any-valid-time` emits a warning (or returns an empty result set with a diagnostic, TBD)

**Estimated complexity**: 2-3 weeks

---

## Phase 8: Cross-Platform Expansion 🎯 FUTURE

**Goal**: WASM, mobile, language bindings

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

**Rationale**: Cross-platform expansion delivers a complete query engine (Phase 7) to more environments — not an incomplete one. Phase 7 ships first.

### 8.1 WebAssembly Support

**Goal**: Run Minigraf as a WASM module in two distinct environments: browser (JavaScript/TypeScript) and server-side WASM runtimes (WASI).

There are two separate compilation targets with different requirements:

#### 8.1a Browser (`wasm32-unknown-unknown` + `wasm-bindgen`)

**Features**:
- 🎯 `IndexedDbBackend`: stub already exists in `src/storage/backend/indexeddb.rs`; implement using `web-sys` + `wasm-bindgen`
- 🎯 Annotate public API with `#[wasm_bindgen]` to expose it to JavaScript
- 🎯 Build with `wasm-pack` — it handles compilation, JS glue code, and TypeScript `.d.ts` generation automatically
- 🎯 TypeScript definition file auto-generated by `wasm-pack` — gives IDE auto-complete and type safety to JS/TS consumers (including AI coding assistants)
- 🎯 Publish to npm via `wasm-pack publish` — makes Minigraf discoverable to web-based AI frameworks (LangChain.js, etc.)
- 🎯 Export API to reconstruct a portable `.graph` blob from IndexedDB (same binary format as native)
- 🎯 Disable `optimizer.rs` under `wasm` feature flag (already gated)

**IndexedDB storage design: page-granular, not single-blob**

A naive implementation would store the entire `.graph` file as a single IndexedDB blob. This is simple but has unacceptable write amplification at any non-trivial scale:

| Scale | File size | Single-blob save cost |
|-------|-----------|----------------------|
| 10K facts | ~1.6MB | Write 1.6MB on every checkpoint |
| 100K facts | ~16MB | Write 16MB on every checkpoint |
| 1M facts | ~160MB | Write 160MB on every checkpoint |

The correct approach maps directly onto Minigraf's existing `StorageBackend` trait:

```
IndexedDB object store: { key: page_id (u64), value: page_bytes (4KB Uint8Array) }
```

- `read_page(id)` → IndexedDB `get(page_id)` — async, one record
- `write_page(id, bytes)` → IndexedDB `put(page_id, bytes)` — only dirty pages written
- The LRU page cache already sits in front of `StorageBackend` — hot pages never hit IndexedDB at all
- On checkpoint, only pages modified since the last checkpoint are written back

This is not a compromise of the single-file philosophy — logically, it is still one database. Physically, IndexedDB is a key-value store, not a filesystem; storing pages as records is the correct abstraction. The `.graph` binary format exists for portability between environments, not as a storage constraint within a browser.

**Export / import for portability**:
- Export: read all pages from IndexedDB in `page_id` order, concatenate, offer as a `.graph` file download
- Import: read a `.graph` file, split into pages, write each page to IndexedDB

This is a deliberate operation (a button or API call), not transparent — which is the right model for a browser environment.

**Browser-specific constraints**:
- No filesystem access in `wasm32-unknown-unknown` — all storage goes through IndexedDB
- No threads in standard browser WASM — lock-free or single-threaded execution paths required
- Binary size budget: target <1MB gzipped; audit dependencies under `wasm` feature

**Build toolchain**:
```bash
# Install wasm-pack
cargo install wasm-pack

# Build for browser (generates pkg/ with .js, .d.ts, .wasm)
wasm-pack build --target web

# Publish to npm
wasm-pack publish
```

**Deliverable**: npm package `@minigraf/core` — browser apps can `import { Minigraf } from '@minigraf/core'` and get full TypeScript types; page-granular IndexedDB backend keeps write amplification proportional to actual changes

#### 8.1b Server-side WASM (`wasm32-wasip1` / WASI)

**Goal**: Run Minigraf inside server-side WASM runtimes (Wasmtime, Wasmer, Cloudflare Workers with WASI, Fastly Compute).

**Why this matters**: Agent frameworks running in sandboxed server-side WASM environments (more secure than Docker containers) can embed Minigraf without any JavaScript bridge. Standard Rust code — no `wasm-bindgen`, no `#[wasm_bindgen]` annotations needed.

**Features**:
- 🎯 Verify `FileBackend` works correctly under WASI's capability-based filesystem
- 🎯 Compile to `wasm32-wasip1` target (formerly `wasm32-wasi`)
- 🎯 No storage backend changes needed — WASI exposes a filesystem API, so `FileBackend` works as-is with capability grants
- 🎯 Validate with Wasmtime and Wasmer runtimes in CI

**Build**:
```bash
cargo build --target wasm32-wasip1 --release
```

**Constraint**: WASI filesystem access requires the host runtime to grant explicit capability permissions (`--dir` in Wasmtime). Document this for users.

**Deliverable**: Minigraf `.wasm` binary runs under Wasmtime/Wasmer with file-backed storage; suitable for use in Cloudflare Workers (WASI) and similar edge runtimes

### 8.2 Mobile Bindings

**Goal**: Ship Minigraf as a drop-in native library for Android (Kotlin/Java) and iOS (Swift), with pre-built artifacts so mobile developers don't need to touch Rust.

**Architecture: SDK approach, not engine-only**

Exposing a raw Rust crate and expecting mobile developers to write their own JNI/FFI layer creates a prohibitively high barrier to entry. The standard pattern (used by Mozilla Application Services, Matrix.org SDK, etc.) is to ship pre-generated language bindings as part of the release artifacts.

**Crate structure** (workspace):
```
minigraf/             ← core Rust library (current crate, no mobile-specific code)
minigraf-ffi/         ← separate crate: UniFFI bridge, no core logic
  src/lib.rs          ← #[uniffi::export] wrappers around minigraf public API
  minigraf.udl        ← UniFFI interface definition (or use proc-macro approach)
```

**Why UniFFI**:
- Developed by Mozilla, production-proven (Firefox for Android, iOS)
- Generates Kotlin, Swift, and Python bindings from a single interface definition
- Handles complex types (strings, enums, `Option`, `Result`) without manual C-style boilerplate
- Alternative: Diplomat (stricter, good for multi-language SDKs); Flutter Rust Bridge (if Flutter is a target)

**Cross-compilation targets**:
- Android: `aarch64-linux-android` (modern 64-bit), `armv7-linux-androideabi` (legacy 32-bit), `x86_64-linux-android` (emulator)
- iOS: `aarch64-apple-ios` (physical devices), `aarch64-apple-ios-sim` (simulators on Apple Silicon)

**Build outputs**:
- Android: `libminigraf.so` per ABI → bundled into a `.aar` (Android Archive) for easy Gradle import
- iOS: Static `.a` per target → lipo'd and wrapped into an `.xcframework` (Apple's standard multi-arch bundle)
- Both generated by a GitHub Actions workflow on every release tag

**Release artifacts** (GitHub Releases):
```
minigraf-android-v0.9.0.aar       ← drop into libs/ in Android project
MinigrafKit-v0.9.0.xcframework    ← add to Xcode project
MinigrafKit-v0.9.0.zip            ← Swift Package Manager checksum source
```

**Swift Package Manager support**:
- `Package.swift` pointing to the `.xcframework` release artifact
- Allows `swift package add https://github.com/adityamukho/minigraf` in Xcode

**Maven / Gradle support**:
- Publish `.aar` to GitHub Packages or Maven Central
- `implementation("io.github.adityamukho:minigraf-android:0.9.0")`

**Features** (implementation order):
- 🎯 `minigraf-ffi` crate with UniFFI proc-macro bindings
- 🎯 GitHub Actions cross-compilation matrix (Android ABIs + iOS targets)
- 🎯 `.xcframework` and `.aar` assembly in CI
- 🎯 Swift Package manifest
- 🎯 Android Gradle integration example
- 🎯 Memory/resource management: ensure `Minigraf` handle lifecycle is safe across FFI boundary

### 8.3 Language Bindings

**Goal**: Python and C FFI as the highest-priority non-mobile targets (covers scripting, agent frameworks, and "any other language via C").

**Features**:
- 🎯 Python bindings via UniFFI (same `.udl` / proc-macro as mobile — no extra code)
- 🎯 C header (`minigraf.h`) via `cbindgen` for any language with a C FFI
- 🎯 Node.js / TypeScript bindings via `neon` or `napi-rs`
- 🎯 Published to PyPI (`minigraf`), npm (`@minigraf/core`)

**Note**: Python and C bindings share the UniFFI / cbindgen work done for mobile — the incremental cost is small once Phase 8.2 is complete.

**Deliverable**: Run anywhere - desktop, mobile, web, embedded; official packages on crates.io, PyPI, npm, Maven, Swift Package Index

**Timeline**: 3-4 months

---

## Phase 9: Ecosystem & Tooling 🎯 FUTURE

**Goal**: Developer experience and ecosystem

**Status**: 🎯 Planned

**Priority**: 🟢 Medium

### 9.1 Developer Tools

**Features**:
- 🎯 Database inspector/debugger
- 🎯 Query profiler
- 🎯 Time travel visualizer
- 🎯 Migration tools

### 9.2 Documentation

**Features**:
- 🎯 Complete API reference (auto-generated via docs.rs; supplement with narrative guides)
- 🎯 Datalog language specification
- 🎯 Cookbook: common patterns (graph traversal, audit queries, time travel idioms)
- 🎯 Performance tuning guide
- 🎯 Error message guide — every user-facing error has a documented cause and resolution

### 9.3 Integration Examples

**Goal**: Close the gap between "interesting concept" and "I can use this today" for the agent and mobile audiences.

**Features**:
- 🎯 GraphRAG pattern: runnable example wiring Minigraf to a vector store (entity UUID as the bridge between fuzzy retrieval and structured graph traversal)
- 🎯 LangChain / LangChain.js integration example — agent memory backed by Minigraf
- 🎯 LlamaIndex integration example — Minigraf as a knowledge graph store
- 🎯 Standalone `examples/` crate with annotated end-to-end scenarios (agentic memory, offline-first mobile, audit log)

**Note**: These are documentation and example artifacts, not library features. They are the difference between "technically impressive" and "I can adopt this." Prioritise before or alongside the Phase 8 platform launch so the new audiences arriving via npm/PyPI/Swift Package Index have something runnable to start from.

### 9.4 Ecosystem Libraries

**Features**:
- 🎯 Graph algorithms (as separate crate)
- 🎯 Schema validation (optional)
- 🎯 Import/export tools
- 🎯 Backup utilities

**Deliverable**: Production-ready ecosystem

**Timeline**: Ongoing

---

### 9.5 Database Branching / Forking (Exploratory)

**Goal**: Allow a Minigraf database to be forked into an independent copy — a new `.graph` file pre-populated with all facts from the parent at a given transaction count.

**Conceptual basis**:

In the bi-temporal model, all temporal dimensions and fact versions together represent *one version of reality*. A branch is a child reality pre-populated from a parent reality. This maps naturally to Minigraf's single-file philosophy: one file = one reality; `db.branch()` produces a new, independent `.graph` file.

```rust
// Fork the database at its current state
let branched_db = db.branch("branch.graph")?;

// Or fork at a specific past transaction
let branched_db = db.branch_as_of("branch.graph", tx_count)?;

// The branch is a fully independent Minigraf database
branched_db.execute("(transact [[:x :y 1]])")?;
// — does not affect the parent
```

**Use cases**:
- Speculative writes: fork, experiment, discard or merge back
- Snapshot distribution: ship a read-only fork to a client
- Test isolation: fork a production-seeded database for testing
- Agent sandboxing: fork the shared knowledge base into a private per-agent copy

**Philosophy alignment**: Single-file, zero-configuration, no server. A fork is just a file copy + replay, consistent with the embedded philosophy.

**Status**: Exploratory — depends on Phase 8 (stable public API) being complete. Implementation complexity is low (checkpoint + file copy + optional tx-count truncation); the main work is API design and ensuring the WAL is fully flushed before the fork.

**Timeline**: Phase 9 or later, conditional on user demand

---

## Release Strategy

### v0.1.0 - ✅ Phase 1 Complete (PoC)
- In-memory property graph
- REPL console

### v0.2.0 - ✅ Phase 2 Complete (Embeddable)
- Persistent storage
- Embedded database API
- Cross-platform file format
- Auto-save

### v0.3.0 - ✅ Phase 3 Complete (Datalog Core)
- ✅ EAV data model
- ✅ Datalog queries
- ✅ Recursive rules
- ✅ Pattern matching
- ✅ Semi-naive evaluation
- ✅ 123 tests passing

### v0.4.0 - ✅ Phase 4 Complete (Bi-temporal)
- ✅ Transaction time (`tx_id`, `tx_count`)
- ✅ Valid time (`valid_from`, `valid_to`)
- ✅ Time travel queries (`:as-of`, `:valid-at`)
- ✅ File format v2 with v1 migration
- ✅ 172 tests passing

### v0.5.0 - ✅ Phase 5 Complete (ACID + WAL)
- ✅ Write-ahead logging (fact-level sidecar WAL, CRC32-protected)
- ✅ `WriteTransaction` API (begin_write, commit, rollback)
- ✅ Crash recovery (WAL replay on open)
- ✅ FileHeader v3 (`last_checkpointed_tx_count`)
- ✅ Thread-safe: concurrent readers + exclusive writer
- ✅ 212 tests passing

### v0.6.0 - ✅ Phase 6.1 Complete (Covering Indexes + Query Optimizer)
- ✅ EAVT, AEVT, AVET, VAET covering indexes with bi-temporal keys
- ✅ B+tree index persistence (FileHeader v4)
- ✅ Selectivity-based query plan optimizer (`optimizer.rs`)
- ✅ CRC32 index sync check; auto-rebuild on mismatch
- ✅ File format v1/v2/v3→v4 migration

### v0.7.0 - ✅ Phase 6.2 Complete (Packed Pages + LRU Cache)
- ✅ Packed fact pages (~25 facts/page, ~25× disk space reduction)
- ✅ LRU page cache (configurable, default 256 pages = 1MB)
- ✅ `CommittedFactReader` trait: on-demand fact loading (no startup load-all)
- ✅ FileHeader v5 (`fact_page_format` byte); auto v4→v5 migration
- ✅ 280 tests passing

### v0.7.1 - ✅ Phase 6.4a Complete (Retraction Semantics Fix + Edge Case Tests)
- ✅ Fixed retraction semantics in Datalog queries (`net_asserted_facts` helper)
- ✅ `check_fact_sizes` / `MAX_FACT_BYTES`: early oversized-fact validation before WAL write
- ✅ `tests/retraction_test.rs` — 7 new retraction integration tests
- ✅ `tests/edge_cases_test.rs` — 4 new edge case integration tests
- ✅ 298 tests passing

### v0.8.0 - ✅ Phase 6.4b (Criterion Benchmarks + Light Publish Prep)
- Run existing Criterion suite; validated performance numbers at 10K / 100K / 1M facts
- Memory profiling via heaptrack
- `BENCHMARKS.md` with full result tables; "Performance" section in README
- `clap` moved to binary-only dep; `Cargo.toml` metadata completed
- GitHub Discussions enabled

### v0.9.0 - ✅ Phase 6.5 (On-Disk B+Tree Indexes + **crates.io publish gate**)
- ✅ Proper on-disk B+tree for all four covering indexes (EAVT, AEVT, AVET, VAET)
- ✅ Index memory usage proportional to cache size, not database size (2.4× open-time speedup at 1M facts)
- ✅ File format v6 (80 bytes) with automatic v5 migration
- ✅ `MutexStorageBackend<B>`: per-page locking for concurrent range scans; cache-warm pages lock-free
- ✅ 331 tests passing; `tests/btree_v6_test.rs` covers B+tree correctness and concurrency
- crates.io publish deferred to after Phase 7 API cleanup (narrowing lib.rs exports, rustdoc sweep)

### v1.0.0 - 🎯 Phase 7 (Datalog Completeness)
- Stratified negation (`not` / `not-join`)
- Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`) + arithmetic filter predicates
- Disjunction (`or` / `or-join`)
- Query optimizer improvements (cost-based, rule evaluation)
- Prepared statements with temporal bind slots
- Temporal metadata pseudo-attributes (`:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`)
- Full four-class temporal query taxonomy (point-in-time, time interval, time-point lookup, time-interval lookup)
- ≥90% branch coverage

### v1.1.0 - 🎯 Phase 8 (Cross-platform)
- WASM support (browser + WASI)
- Mobile bindings (iOS + Android)
- Language bindings (Python, C, Node.js)

### v1.0.0 - 🎯 Production Ready (12-15 months)
- Stable API
- Stable file format
- Comprehensive tests
- Full documentation
- Performance validated
- Backwards compatibility promise

**Stability Promise**: After v1.0.0, we commit to:
- Backwards-compatible file format (decades)
- Stable public API (semantic versioning)
- Migration tools for any format changes
- Long-term support

---

## Decision Framework

When evaluating features, ask:

1. **Does it align with philosophy?** (embedded, reliable, simple, bi-temporal)
2. **Is it needed for target use cases?** (audit, event sourcing, knowledge graphs)
3. **Does it compromise reliability?** (stability over features)
4. **Can it be a separate crate?** (keep core small)

**Say NO to**:
- Distributed consensus
- Multi-datacenter replication
- Built-in ML/AI
- Features only useful at massive scale
- Complex configuration
- Breaking the single-file philosophy

**Say YES to**:
- Crash safety
- Data integrity
- Temporal queries
- Query performance
- Developer experience
- Cross-platform support

---

## Timeline (Rough Estimates)

- ✅ Phase 1: Complete (December 2025)
- ✅ Phase 2: Complete (January 2026)
- ✅ Phase 3: Complete (January 2026) - Datalog core with recursive rules
- ✅ Phase 4: Complete (March 2026) - Bi-temporal support
- ✅ Phase 5: Complete (March 2026) - ACID + WAL
- ✅ Phase 6.1: Complete (March 2026) - Covering Indexes + Query Optimizer
- ✅ Phase 6.2: Complete (March 2026) - Packed Pages + LRU Cache
- ✅ Phase 6.4a: Complete (March 2026) - Retraction semantics fix + edge case tests
- ✅ Phase 6.4b: Complete (March 2026) - Benchmarks + light publish prep
- ✅ Phase 6.5: Complete (March 2026) - On-disk B+tree indexes, file format v6, concurrent scan per-page locking
- 🎯 Phase 7: 8-12 weeks (Datalog Completeness — negation, aggregation, disjunction, prepared statements, temporal metadata bindings; ≥90% branch coverage) - **NEXT**
- 🎯 Phase 8: 3-4 months (Cross-platform — WASM, mobile, language bindings)
- 🎯 Phase 9: Ongoing (Ecosystem — integration examples, cookbook, GraphRAG/LangChain examples)
- 🎯 **v1.0.0: 9-12 months**

**Note**: This is a hobby project. Timeline is flexible but realistic.

---

## Current Focus

**Right Now**: Phase 6.5 Complete — Starting Phase 7 (Datalog Completeness)

**Phase 6.5 Achievements**:
1. ✅ `src/storage/btree_v6.rs`: proper on-disk B+tree with `build_btree`, `OnDiskIndexReader`, `CommittedIndexReader` trait
2. ✅ `MutexStorageBackend<B>`: backend mutex held per page read; cache-warm scans acquire no lock
3. ✅ FileHeader v6 (80 bytes): adds `fact_page_count` field; auto v5→v6 migration on checkpoint
4. ✅ 331 tests; `tests/btree_v6_test.rs` with 8 integration tests + concurrent correctness unit test
5. ✅ BENCHMARKS.md updated: open-time 2.4× faster at 1M, peak heap 21% lower, concurrent scan scaling improved
6. ✅ Version bumped to v0.9.0; crates.io publish checklist unblocked

**Immediate Next Steps (Phase 7)**:
1. Implement stratified negation (`not` / `not-join`)
2. Add aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`)
3. Add disjunction (`or` / `or-join`)
4. Push error-path coverage from ~82% toward ≥90% target

**Key Decisions Made**:
- ✅ Datalog query language (simpler, better for temporal)
- ✅ Bi-temporal as first-class feature (not afterthought)
- ✅ Keep single-file philosophy
- ✅ Recursive rules with semi-naive evaluation
- ✅ UTC-only timestamps (avoids chrono GHSA-wcg3-cvx6-7396)
- ✅ Packed pages over one-per-page (philosophy: small binary, efficient storage)
- ✅ Approximate LRU (read-lock on hits — avoids write-lock contention)
- ✅ Target 9-12 months to v1.0 (ahead of schedule!)

See [GitHub Issues](https://github.com/adityamukho/minigraf/issues) for specific tasks.

---

Last Updated: Phase 6.4b Complete - Benchmarks + Light Publish Prep (March 2026)
