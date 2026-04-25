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

**Status**: ✅ Completed (March 2026)

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

**Note**: crates.io publish deferred to Phase 7.9 (API cleanup + publish prep: narrowing `lib.rs` exports, rustdoc sweep, clippy, `unwrap()` audit). Phase 6.5 (file format v6) is complete — the format is now stable enough to publish.

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

## Phase 7: Datalog Completeness ✅ COMPLETE

**Goal**: Complete the Datalog query engine — negation, aggregation, disjunction, temporal range queries, and prepared statements

**Status**: ✅ Complete (7.1a + 7.1b + 7.2a + 7.2b + 7.3 + 7.4 + 7.5 + 7.6 + 7.7a + 7.7b + 7.8 + 7.9 complete)

**Priority**: 🔴 Critical — without these, realistic production queries cannot be expressed in Datalog

**Rationale**: The highlighted use cases (agentic memory, audit, mobile, browser) all require at minimum negation and aggregation. Expanding to mobile and WASM platforms before the query engine can express production-grade queries means shipping an incomplete product to more places. All features are additive — existing queries continue to work unchanged. Semantics are well-established (Datomic and XTDB are production references for all three).

**Sub-phases**:
- **7.1a** ✅ Stratified Negation — `not`
- **7.1b** ✅ Stratified Negation — `not-join`
- **7.2a** ✅ Aggregation (`count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`, `:with`)
- **7.2b** ✅ Arithmetic & predicate expression clauses (`[(< ?v 100)]`, `[(+ ?a ?b) ?c]`, string predicates, type predicates)
- **7.2** ~~Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`) — includes arithmetic filter predicates~~ → split into 7.2a + 7.2b
- **7.3** ✅ Disjunction (`or` / `or-join`)
- **7.4** ✅ Query Optimizer Improvements / `filter_facts_for_query` snapshot fix
- **7.5** ✅ Tests + Error Coverage (617 tests; executor.rs 85.71%, evaluator.rs 89.29% branch coverage; CI coverage gate + nightly llvm-cov)
- **7.6** ✅ Temporal Metadata Bindings + Range Queries (`:db/valid-from`, `:db/valid-to`, `:db/tx-count` as queryable pseudo-attributes; unlocks Time Interval, Time-Point Lookup, Time-Interval Lookup query classes)
- **7.7a** Window Functions — `sum`, `count`, `min`, `max`, `avg`, `rank`, `row-number` with unbounded-preceding frame; `FunctionRegistry` introduced (built-ins only); `lag`/`lead` and sliding frames deferred to post-1.0 backlog
- **7.7b** ✅ User-Defined Functions (UDFs) — public `register_aggregate`/`register_predicate` API on the `FunctionRegistry` introduced in 7.7a
- **7.8** ✅ Prepared Statements (parse + plan once, execute many times, temporal bind slots; implemented after full clause set including predicate-argument positions)
- **7.9** ✅ Publish Prep (crates.io — API cleanup, rustdoc, clippy, `unwrap` audit, CI matrix)

### 7.1a Stratified Negation — `not` ✅ COMPLETE

**Goal**: Express "find entities where attribute X is absent" and similar absence queries.

**Why it's load-bearing**:
- Agentic memory: "what has the agent not verified?", "beliefs with no supporting evidence"
- Audit: "contracts without a sign-off event", "records missing a required field"
- Developer tooling: "modules with no dependents", "entities never retracted"
- Without negation these queries require pulling results into application memory and filtering — defeating the query engine

**Semantics**: Stratified negation (Datalog^¬) — the standard safe subset. The rule dependency graph is analysed at registration time; programs where negation creates a recursive cycle are rejected immediately with a clear error. Non-recursive negation is always safe. All variables in a `not` body must be bound by outer clauses (safety / range-restriction constraint, checked at parse time).

**Syntax** (Datomic-inspired):
```datalog
;; not — exclude bindings where sub-clause matches
(query [:find ?e
        :where [?e :person/name _]
               (not [?e :person/age _])])

;; not with rule invocation (requires stratification)
(query [:find ?person
        :where [?person :person/name ?name]
               (not (blocked ?person))])

;; not in a rule body
(rule [(eligible ?x)
       [?x :applied true]
       (not (rejected ?x))])
```

**Implementation**:
- `types.rs`: add `WhereClause::Not(Vec<WhereClause>)`; change `Rule.body` from `Vec<EdnValue>` to `Vec<WhereClause>`
- `parser.rs`: parse `(not ...)` in `:where` clauses and rule bodies; safety check at parse time; reject nested `not`
- `stratification.rs` (new): `DependencyGraph`, `stratify()` — Bellman-Ford constraint propagation, cycle detection at `>= N` strata
- `rules.rs`: call `stratify()` on `register_rule`; reject rule on negative cycle
- `evaluator.rs`: add `StratifiedEvaluator` (orchestrates strata); update `RecursiveEvaluator::evaluate_rule` to branch on `WhereClause` variants
- `executor.rs`: `execute_query_with_rules` uses `StratifiedEvaluator`; `execute_query` handles `not`-only queries as post-filters
- All changes additive; existing queries unaffected

**Spec**: `docs/superpowers/specs/2026-03-23-phase-7-1a-stratified-negation-design.md`

**Estimated complexity**: 2-3 weeks

### 7.1b Stratified Negation — `not-join` ✅ COMPLETE

**Goal**: Express negation with explicit variable sharing from the outer scope — necessary when the `not` body introduces variables that should be correlated with the outer query but are not mentioned in shared patterns.

**Syntax**:
```datalog
;; not-join — exclude with explicitly shared variables from outer scope
(query [:find ?e
        :where [?e :task/status :pending]
               (not-join [?e]
                 [?e :task/blocked-by _])])
```

**Implementation**: Builds directly on 7.1a infrastructure (stratification, `StratifiedEvaluator`). Adds `WhereClause::NotJoin { vars: Vec<String>, clauses: Vec<WhereClause> }`, parser support, and executor handling for the explicit variable binding list.

**Estimated complexity**: 1 week (reuses all 7.1a infrastructure)

### 7.2a Aggregation ✅ COMPLETE

**Goal**: Express counting, summing, and extremes directly in queries rather than post-processing in application code.

**Status**: ✅ Complete (v0.11.0, 2026-03-25)

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

### 7.2b Arithmetic & Predicate Expressions ✅ COMPLETE

**Goal**: Express filter predicates and arithmetic bindings directly in `:where` clauses — without application-side post-processing.

**Status**: ✅ Complete (v0.12.0, 2026-03-25)

**Why it's load-bearing**: Required by Phase 7.6 (temporal range queries via `:db/valid-from` / `:db/valid-to`) and Phase 7.7b (UDF predicates via `FunctionRegistry`).

**Syntax**:
```datalog
;; Filter predicates — keep row if truthy
[(< ?age 30)]
[(>= ?salary ?min-salary)]
[(string? ?name)]
[(starts-with? ?tag "work")]
[(matches? ?email "^[^@]+@[^@]+$")]

;; Arithmetic bindings — bind result to output variable
[(+ ?price ?tax) ?total]
[(* ?price ?qty) ?subtotal]
[(integer? ?v) ?is-int]

;; Nested arithmetic
[(+ (* ?a 2) ?b) ?result]
```

**Implementation**:
- Types: `BinOp` (14 variants: Lt/Gt/Lte/Gte/Eq/Neq/Add/Sub/Mul/Div/StartsWith/EndsWith/Contains/Matches), `UnaryOp` (5 variants: StringQ/IntegerQ/FloatQ/BooleanQ/NilQ), `Expr` enum (Var/Lit/BinOp/UnaryOp), `WhereClause::Expr { expr, binding }` variant
- Parser: `parse_expr` / `parse_expr_clause`; dispatch at all 4 clause sites (`:where`, rule body, `not`, `not-join`); parse-time regex validation; forward-pass safety check (unbound variables rejected)
- Executor: `eval_expr`, `eval_binop`, `is_truthy`, `apply_expr_clauses`; type mismatches and div/0 silently drop the row; int/float promotion; NaN guard
- Optimizer: pass-through (`Expr` clauses not reordered — ordering guaranteed by safety check)

**Spec**: `docs/superpowers/specs/2026-03-25-phase-7-2b-arithmetic-predicates-design.md`

### 7.3 Disjunction (`or` / `or-join`) ✅ COMPLETE

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

### 7.4 Query Optimizer Improvements / `filter_facts_for_query` snapshot fix ✅ COMPLETE

**Status**: ✅ Completed (March 2026, v0.13.1)

- ✅ **Profiling integration** — Criterion profiling gate added; flamegraph support via `pprof` crate.
- ✅ **`filter_facts_for_query` snapshot fix** — `filter_facts_for_query` now returns `Arc<[Fact]>` instead of a throwaway `FactStorage`; eliminates O(N) four-BTreeMap index rebuild on every non-rules query call. `execute_query` path constructs zero `FactStorage` objects. `execute_query_with_rules` still converts `Arc<[Fact]>` back to `FactStorage` for `StratifiedEvaluator` (deferred to later phase). `apply_or_clauses` and `evaluate_not_join` signatures updated to accept `Arc<[Fact]>`. Evaluator loop: `accumulated_facts` computed once per iteration (was 4 separate `get_asserted_facts()` calls). `PatternMatcher::from_slice(Arc<[Fact]>)` constructor added.
- ✅ **Benchmark results**: ~62–65% speedup on non-rules queries at 10K facts (`query/point_entity/10k`: 22 ms → 8.6 ms; `aggregation/count_scale/10k`: 28 ms → 9.7 ms).
- ✅ 568 tests passing (390 unit + 172 integration + 6 doc); version bumped to v0.13.1.

**Note**: The following items were originally scoped here but are deferred to the post-1.0 backlog: cost-based optimizer extensions for new clause types, rule evaluation optimization for `not`/`or`/aggregate rules, and predicate push-down.

### 7.5 Tests + Error Coverage ✅ COMPLETE

**Status**: ✅ Completed (2026-03-31)

**What Was Built**:
- ✅ `tests/production_patterns_test.rs` — 8 cross-feature integration tests (not+as-of, not-join+count, count+not, count+valid-at, recursion+not, or+count, or+sum, count+as-of-sequence)
- ✅ `tests/error_handling_test.rs` — 8 integration-level error-path tests (runtime type errors, stratification rejections, parse safety errors)
- ✅ ~109 targeted unit tests in `executor.rs` and `evaluator.rs` for parser-unreachable branches and aggregation/arithmetic edge cases
- ✅ `cargo-llvm-cov` branch coverage documented in `CONTRIBUTING.md`
- ✅ CI coverage gate: tarpaulin `--fail-under 75` + Codecov 75% threshold enforcement
- ✅ Nightly `cargo-llvm-cov --branch` workflow with HTML artifact upload

**Coverage achieved**:
- `executor.rs`: 85.71% branch coverage (from ~75%)
- `evaluator.rs`: 89.29% branch coverage (from ~73%)
- `stratification.rs`: 100% branch coverage
- Remaining gaps: NaN-check defensive code unreachable via public API

**Note**: Stratification correctly detects negative cycles inside `or` branches. Tests `or_negative_cycle_rejected` and `test_or_with_not_cycle_rejected` verify this behavior.

**Deliverable**: 788 tests (unit + integration + doc); CI enforces ≥75% line coverage on every PR; nightly branch coverage report

**Timeline**: Completed 2026-03-31

---

### 7.6 Temporal Metadata Bindings + Range Queries ✅ COMPLETE

**Status**: ✅ Complete (v0.15.0, 2026-04-01)

**Summary**: `:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`, and `:db/valid-at` are now first-class bindable pseudo-attributes in Datalog `:where` patterns. All four temporal query classes are now reachable. 647 tests (438 unit + 209 integration).

**Goal**: Expose `valid_from`, `valid_to`, and `tx_count` as first-class bindable values in Datalog `:where` clauses, unlocking the full four-class taxonomy of temporal queries described in the bi-temporal literature.

**Background — the four temporal query classes**:

Most people are familiar with point-in-time queries (`:as-of`, `:valid-at`), but a complete bi-temporal query model covers four classes:

| Class | Description | Minigraf before 7.6 |
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

Arithmetic filter predicates — `[(op ?var literal)]` — are required for Time Interval and Time-Point Lookup queries. These predicates are also needed for aggregation (Phase 7.2). Phase 7.2 should implement the predicate evaluation infrastructure; Phase 7.6 then applies it to temporal metadata bindings.

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
- Pseudo-attribute without `:any-valid-time` returns an empty result set and surfaces a diagnostic error message (consistent with the "explicit errors over silent wrong answers" principle)

**Estimated complexity**: 2-3 weeks

---

### 7.7 Window Functions + UDFs

**Goal**: Expose `SUM OVER`–style window computations natively in Datalog `:find` clauses, and let embedders register custom aggregate and predicate functions at runtime.

**Why here (before 7.9 publish prep)**:

Phase 7.2 aggregation provides the grouping and accumulation infrastructure; Phase 7.6 pseudo-attributes expose `valid_from` / `valid_to` / `tx_count` as bindable values. Window functions are a direct extension: they apply aggregate semantics *over a partition of the current result set* while preserving per-row output — useful for ranked temporal queries and sliding-window analytics without a second query and application-side join.

UDFs are the natural generalisation: if the engine can call built-in aggregates via a `FunctionRegistry`, embedders can register their own functions against the same registry. Designing both behind a single `FunctionRegistry` abstraction means neither feature needs to be retrofitted onto the other.

**Dependency on Phase 7.2**: Phase 7.2 grouping and accumulation logic is the implementation substrate for window functions. Phase 7.2 must be complete before this phase begins.

**Dependency on Phase 7.6**: `:db/valid-from` / `:db/valid-to` / `:db/tx-count` as bindable values are the primary ordering/partitioning keys for bi-temporal window queries. Phase 7.6 should be complete or in progress.

---

#### 7.7a Window Functions ✅ COMPLETE

**Status**: ✅ Complete (v0.16.0, 2026-04-02)

**Summary**: Window functions (`sum`, `count`, `min`, `max`, `avg`, `rank`, `row-number`) added to the Datalog `:find` clause via `(func ?v :over (:partition-by ?p :order-by ?o))` syntax. `FunctionRegistry` introduced — all built-in aggregates migrated into it; `lag`/`lead` and sliding frames deferred to post-1.0 backlog. 705 tests.

**Syntax** (Datomic-inspired, Datalog-native):

```datalog
;; Running sum of salary ordered by hire date, partitioned by dept
(query [:find ?e (sum ?salary :over (partition-by ?dept :order-by ?hire-date))
        :where [?e :employee/dept ?dept]
               [?e :employee/salary ?salary]
               [?e :employee/hire-date ?hire-date]])

;; Rank entities within a group by score
(query [:find ?e (rank :over (partition-by ?category :order-by ?score :desc))
        :where [?e :item/category ?category]
               [?e :item/score ?score]])

;; Cumulative fact count ordered by tx-count (bi-temporal use case)
(query [:find ?e (count ?e :over (order-by ?tx))
        :any-valid-time
        :where [?e :event/type :login]
               [?e :db/tx-count ?tx]])
```

**Supported window functions** (initial set):

| Function | Semantics |
|---|---|
| `sum ?v :over (…)` | Cumulative/partition sum |
| `count ?v :over (…)` | Cumulative/partition count |
| `min ?v :over (…)` | Running minimum |
| `max ?v :over (…)` | Running maximum |
| `avg ?v :over (…)` | Running average |
| `rank :over (…)` | Rank within partition |
| `row-number :over (…)` | Sequential row number within partition |

> **Note:** `lag`, `lead`, and the `:rows N preceding` sliding frame are deferred to the post-1.0 backlog. 7.7a ships `sum`, `count`, `min`, `max`, `avg`, `rank`, `row-number` with unbounded-preceding (cumulative from partition start to current row) only.

**`:over` clause sub-options**:

| Option | Description |
|---|---|
| `:partition-by ?var` | Reset accumulation per unique value of `?var` (like SQL `PARTITION BY`) |
| `:order-by ?var` (`:asc` / `:desc`) | Determines row order within each partition |
| Frame: `:rows-unbounded-preceding` (default) | Accumulate from first row in partition to current |

**Implementation**:
- Parser: add `WindowExpr` variant to the `:find` clause AST; parse `(func ?v :over (...))` forms
- Types: `FindSpec::Window { func: WindowFunc, var: Option<String>, partition_by: Option<String>, order_by: Option<String>, order: Order, frame: WindowFrame }`
- Executor: post-process binding set in three steps:
  1. Sort rows within each partition by the `order-by` key
  2. Walk sorted rows, accumulating the window function state per partition
  3. Annotate each binding with the computed window value
- No changes to the core evaluation engine — window computation is a purely post-evaluation pass, same as Phase 7.2 aggregation

**Estimated complexity**: 2-3 weeks

---

#### 7.7b User-Defined Functions (UDFs) ✅ COMPLETE

**Status**: ✅ Complete (v0.17.0, 2026-04-02)

**Summary**: `register_aggregate` and `register_predicate` public API added to `Minigraf`.
UDFs plug into grouping aggregation, window computation, and `:where` predicate filtering
via type-erased `Box<dyn Any + Send>` accumulators and `Arc<dyn Fn>` closures.
`FunctionRegistry` extended with `AggImpl` discriminator and `PredicateDesc`.
727 tests.

**Goal**: Allow embedders to extend the query engine with custom aggregate functions and filter predicates registered at runtime, using the same `FunctionRegistry` that built-in aggregates and window functions use.

**Why UDFs are safe for an embedded database**:

UDFs are registered as Rust closures or function pointers — they run in-process at the same trust level as the application. There is no sandbox boundary to cross and no serialization overhead. This is identical to SQLite's `sqlite3_create_function` — a well-proven pattern for embedded database extensibility that does not compromise the embedded-first, self-contained philosophy.

**API**:

```rust
// Register a custom aggregate function: geometric mean
db.register_aggregate(
    "geomean",
    // initialise accumulator
    || 0.0_f64,
    // step: (accumulator, next_value) -> accumulator
    |acc: f64, v: &Value| match v {
        Value::Float(f) => acc + f.ln(),
        Value::Integer(i) => acc + (*i as f64).ln(),
        _ => acc,
    },
    // finalise: (accumulator, count) -> Value
    |acc: f64, n: usize| Value::Float((acc / n as f64).exp()),
)?;

// Register a custom filter predicate
db.register_predicate(
    "email?",
    |v: &Value| matches!(v, Value::String(s) if s.contains('@')),
)?;
```

**Use in queries**:

```datalog
;; Custom aggregate
(query [:find ?dept (geomean ?score)
        :where [?e :employee/dept ?dept]
               [?e :employee/score ?score]])

;; Custom predicate
(query [:find ?e
        :where [?e :person/email ?addr]
               (email? ?addr)])

;; Custom aggregate as window function
(query [:find ?e (geomean ?score :over (partition-by ?dept :order-by ?score))
        :where [?e :employee/dept ?dept]
               [?e :employee/score ?score]])
```

**Implementation**:
- `FunctionRegistry` struct (new, in `src/query/datalog/functions.rs`): `HashMap<String, AggregateDesc>` + `HashMap<String, PredicateDesc>`
  - `AggregateDesc`: init closure + step closure + finalise closure + optional window-compatible flag
  - `PredicateDesc`: one-argument `Fn(&Value) -> bool` closure
- All built-in aggregates (Phase 7.2) and window functions (Phase 7.7a) are registered into `FunctionRegistry` at startup — UDFs use exactly the same path
- Parser: recognise registered function names in `:find` aggregate positions and `:where` predicate call positions at parse time (registry consulted at parse time for validation)
- `Minigraf::register_aggregate(name, init, step, finalise)` and `Minigraf::register_predicate(name, fn)` — new public API methods, callable before or after `open()`
- Functions are not persisted to the `.graph` file — they must be re-registered on each open, exactly as SQLite requires (this is correct: executable code is never stored in the data file)

**Tests**:
- Custom aggregate: compute geometric mean over a result set
- Custom aggregate: empty result set returns `Null` (consistent with Phase 7.2 `count` empty-result semantics)
- Custom predicate: filter binding set using an embedder-provided function
- UDF as window function: custom aggregate used in `:over` clause
- Name collision: registering a name that shadows a built-in returns a clear error
- Unknown function name in `:find` at parse time: clear parse error, not a runtime panic
- Thread safety: registry is `Arc<RwLock<FunctionRegistry>>`; concurrent reads + rare writes

**Estimated complexity**: 2-3 weeks

---

**Phase 7.7 deliverable**: Window aggregates (`sum over`, `rank`, `lag`, `lead`, etc.) expressible natively in Datalog `:find`; embedder-registered aggregate and predicate UDFs callable from any query; all built-in aggregates and window functions unified under `FunctionRegistry`; new public API methods included in Phase 7.9 publish surface

**Estimated total Phase 7.7 complexity**: 4-6 weeks

---

### 7.8 Prepared Statements ✅ COMPLETE

**Status**: ✅ Complete (v0.18.0, 2026-04-04)

**Summary**: `Minigraf::prepare(query_str)` returns a `PreparedQuery` that can be executed many
times with different `BindValue` bindings. Named `$slot` tokens are substituted at execute time
via deep-clone + AST walk; the executor, optimizer, and matcher are unchanged.
780 tests.

**Goal**: Parse and plan a query once; execute it repeatedly with different bind values — including temporal filters — without re-parsing or re-planning on each call.

**Why now (after Phase 7.1–7.7, not before)**:

Phase 7 adds negation (stratification analysis), aggregation (post-processing), disjunction (branch evaluation), temporal metadata pseudo-attributes (`:db/valid-from` / `:db/valid-to` / `:db/tx-count`), arithmetic filter predicates, window functions, and UDFs — after which the plan cost becomes meaningfully larger. Designing bind parameter syntax *after* the full clause set exists means the syntax doesn't need revisiting when new clause types are added; in particular, predicate-argument positions (`[(>= ?vf $threshold)]`) introduced in Phase 7.6 and UDF predicate positions from Phase 7.7b are included in the bind-slot position table from the start. Before Phase 7, the parse + plan cost is small enough that caching it offers negligible benefit.

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

### 7.9 Publish Prep (crates.io) ✅ COMPLETE

**Goal**: Make the public API clean, documented, and safe before publishing to crates.io.

**Status**: ✅ Complete (v0.19.0, 2026-04-08)

**Scope**:
- Narrow `lib.rs` exports — expose only `Minigraf`, `WriteTransaction`, and the query/result types; mark internal types (`PersistentFactStorage`, `FileHeader`, `PAGE_SIZE`, `Repl`, `Wal`, etc.) as `pub(crate)` or remove re-exports
- Rustdoc sweep — add doc comments with examples to all public API items
- Clippy clean — `cargo clippy -- -D warnings` passes with zero warnings
- `cargo doc --no-deps` builds without warnings
- `unwrap()`/`expect()` audit — remove from all library code paths (tests and binary are exempt)
- Verify `Cargo.toml` description is accurate and compelling
- Confirm `README.md` quick-start example compiles and runs
- `cargo test` verified on Linux, macOS, and Windows (CI matrix)
- Publish `0.x` to crates.io

**Note**: No breaking changes to the `execute()`/`query` string API. Internal visibility tightening only.

**Estimated complexity**: 1-2 weeks

---

## Phase 8: Cross-Platform Expansion 🔄 IN PROGRESS

**Goal**: WASM, mobile, language bindings

**Status**: 🔄 In Progress (Phase 8.1 complete — v0.20.0)

**Priority**: 🟢 Medium

**Rationale**: Cross-platform expansion delivers a complete query engine (Phase 7) to more environments — not an incomplete one. Phase 7 ships first.

### 8.1 WebAssembly Support ✅ COMPLETE

**Goal**: Run Minigraf as a WASM module in two distinct environments: browser (JavaScript/TypeScript) and server-side WASM runtimes (WASI).

**Status**: ✅ Completed (April 2026) — v0.20.0

There are two separate compilation targets with different requirements:

#### 8.1a Browser (`wasm32-unknown-unknown` + `wasm-bindgen`) ✅

**Features**:
- ✅ `IndexedDbBackend`: page-granular IndexedDB storage using `web-sys` + `wasm-bindgen`
- ✅ `BrowserDb` public API annotated with `#[wasm_bindgen]` (execute, checkpoint, export_graph, import_graph)
- ✅ Built with `wasm-pack` — generates `pkg/` with JS glue and TypeScript `.d.ts`
- ✅ TypeScript definitions auto-generated by `wasm-pack`
- ✅ npm publish deferred to Phase 8.3 (bindings complete; package story not yet finalized)
- ✅ Export/import API for portable `.graph` blob (same binary format as native)
- ✅ `optimizer.rs` disabled under `wasm` feature flag

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

**Deliverable**: ✅ Browser WASM with `BrowserDb` API, page-granular IndexedDB backend, and TypeScript types. Cross-platform `.graph` compatibility verified between native and browser via committed fixture and `import_graph` round-trip test.

#### 8.1b Server-side WASM (`wasm32-wasip1` / WASI) ✅

**Goal**: Run Minigraf inside server-side WASM runtimes (Wasmtime, Wasmer, Cloudflare Workers with WASI, Fastly Compute).

**Why this matters**: Agent frameworks running in sandboxed server-side WASM environments (more secure than Docker containers) can embed Minigraf without any JavaScript bridge. Standard Rust code — no `wasm-bindgen`, no `#[wasm_bindgen]` annotations needed.

**Features**:
- ✅ `FileBackend` verified under WASI's capability-based filesystem
- ✅ Compiles to `wasm32-wasip1` target (formerly `wasm32-wasi`)
- ✅ No storage backend changes needed — WASI exposes a filesystem API; `FileBackend` works as-is with capability grants
- ✅ Validated with Wasmtime and Wasmer runtimes in CI (`wasm-wasi.yml`)

**Build**:
```bash
cargo build --target wasm32-wasip1 --release
```

**Constraint**: WASI filesystem access requires the host runtime to grant explicit capability permissions (`--dir` in Wasmtime). Document this for users.

**Deliverable**: ✅ Minigraf `.wasm` binary runs under Wasmtime/Wasmer with file-backed storage; suitable for use in Cloudflare Workers (WASI) and similar edge runtimes

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
- ✅ `minigraf-ffi` crate with UniFFI proc-macro bindings (uniffi 0.31.1, proc-macro approach)
- ✅ GitHub Actions cross-compilation matrix (Android ABIs + iOS targets)
- ✅ `.xcframework` and `.aar` assembly in CI (`mobile.yml`)
- ✅ Swift Package manifest (`Package.swift` at repo root, CI-updated checksum)
- ✅ Android Gradle project with GitHub Packages publishing
- ✅ Memory/resource management: `Arc<Mutex<Minigraf>>` behind UniFFI object ref-counting

**Post-1.0 FFI deferrals** (complex to represent safely over UniFFI):
- 🎯 `register_aggregate` / `register_predicate` over UniFFI — requires closure-passing across FFI (not supported by UniFFI 0.31.1); needs a callback-based redesign
- 🎯 `prepare()` / `PreparedQuery` over UniFFI — bind-slot substitution requires a stateful handle; design TBD once basic FFI API is proven stable

### 8.3 Language Bindings

**Goal**: Python and C FFI as the highest-priority non-mobile targets (covers scripting, agent frameworks, and "any other language via C").

#### 8.3a Python Bindings ✅ COMPLETE

**Status**: ✅ Completed (April 2026) — v0.22.0

**Features**:
- ✅ Python bindings via UniFFI (reusing `minigraf-ffi` crate from Phase 8.2)
- ✅ `maturin` build backend — generates wheels via `minigraf-ffi/python/`
- ✅ Python package `minigraf` with `MiniGrafDb` class: `open(path)`, `open_in_memory()`, `execute(datalog)`, `checkpoint()`
- ✅ Published to PyPI as `minigraf` (`pip install minigraf`)
- ✅ Pre-built wheels for Linux x86_64/aarch64, macOS universal2, Windows x86_64
- ✅ CI workflow (`python-release.yml`): cross-compiles via `maturin` on every release tag
- ✅ PR CI (`python-pr.yml`): validates build on every pull request

**Deliverable**: ✅ `pip install minigraf` — Python bindings published to PyPI with pre-built platform wheels.

#### 8.3b Desktop JVM Bindings ✅ COMPLETE

**Status**: ✅ Completed (April 2026) — v0.23.0

**Features**:
- ✅ Desktop JVM bindings via UniFFI (reusing `minigraf-ffi` crate from Phase 8.2)
- ✅ Gradle 8.11 project in `minigraf-ffi/java/` — fat JAR with embedded platform natives
- ✅ `NativeLoader.kt` — runtime native extraction from JAR resources via `System.load()`
- ✅ `MiniGrafDb` class: `open(path)`, `openInMemory()`, `execute(datalog)`, `checkpoint()`
- ✅ Published to Maven Central as `io.github.adityamukho:minigraf-jvm`
- ✅ Embedded natives for Linux x86_64/aarch64, macOS universal2, Windows x86_64
- ✅ CI workflow (`java-release.yml`): cross-compiles natives, assembles fat JAR, publishes on release tag
- ✅ PR CI (`java-ci.yml`): validates build and JUnit 5 tests on every pull request

**Deliverable**: ✅ `implementation("io.github.adityamukho:minigraf-jvm:0.23.0")` — JVM bindings published to Maven Central.

#### 8.3c/8.3d Remaining Language Bindings 🎯 FUTURE

**Features**:
- 🎯 8.3c: C header (`minigraf.h`) via `cbindgen` for any language with a C FFI — v0.24.0
- 🎯 8.3d: Node.js / TypeScript bindings via `napi-rs` — v0.25.0
- 🎯 Published to npm (`@minigraf/core`)

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

## Post-1.0 Performance Backlog

Known O(N²) hotspots discovered during benchmarking (v0.13.0). Each has a well-understood O(N) fix but touches the query evaluator rather than the optimizer, so they are deferred beyond v1.0 to avoid expanding Phase 7.4's scope.

### Hash-Join for Negation Inner Loop

**Problem**: `not` / `not-join` evaluation re-scans all candidate facts once per outer binding — O(outer × inner) = O(N²). Observed: 13 s (`not_scale/10k`), 23 s (`not_join_scale/10k`).

**Fix**: Pre-compute the exclusion set from the `not` body once → `HashSet<Value>`. Probe per outer binding in O(1). Overall: O(N).

### Hash-Join for Disjunction (`or` / `or-join`) Inner Loop

**Problem**: `apply_or_clauses` evaluates each branch against the full incoming binding set (seeded re-scan) — O(seeds × facts) = O(N²). Observed: 74 s (`or_scale/10k`), 73 s (`or_join_scale/10k`). (Rules are exempt — they start from an empty binding, giving O(N).)

**Fix**: Evaluate each branch from an empty seed, then intersect/project results back onto the incoming bindings using a hash lookup. Overall: O(N) per branch.

### Hash-Join for `with`-Grouped Aggregation Cross-Product

**Problem**: `with_grouped_sum` triggers a two-pattern cross-product join without a hash-join step — O(N²). Observed: 67 s (`with_grouped_sum/10k`).

**Fix**: Add a hash-join planning step in the aggregation post-processor for multi-pattern `with` clauses.

### Cost-Based Optimizer Extensions for New Clause Types

Extend `optimizer.rs` `plan()` with cost estimates for negation sub-queries, aggregate post-processing, and disjunction branch selection. Requires the new clause types to exist (satisfied by 7.1–7.3) and profiling data to guide estimates. Deferred from Phase 7.4 to avoid expanding scope before v1.0.

### Rule Evaluation Optimization

Improve semi-naive evaluation for rules that include `not`, `or`, and aggregate expressions — currently routes to the mixed-rules path but does not apply any cost-aware ordering. Deferred from Phase 7.4.

### Predicate Push-Down

Push `Expr` predicate clauses (e.g. `[(> ?age 30)]`) down to filter bindings as early as possible rather than applying them as a final post-processing pass. Currently `apply_expr_clauses` runs after all pattern matching. A natural complement to the `filter_facts_for_query` snapshot fix, but kept separate to avoid expanding Phase 7.4's scope.

### B+Tree Selective Lookup (Range-Scan Predicate Push-Down)

**Problem**: `filter_facts_for_query` step 1 calls `get_all_facts()`, which performs a full B+tree range scan regardless of query predicates. Every query pays O(N) I/O even when the query pattern binds a specific entity or attribute that could be resolved in O(log N) via an existing EAVT/AEVT index key lookup.

**Fix**: Inspect query patterns before calling `get_all_facts()`. If a pattern binds a concrete entity (or entity + attribute), use `get_facts_by_entity` / `get_facts_by_attribute` to fetch only the relevant subset from the on-disk B+tree. This makes point-entity and point-attribute queries sub-linear in total fact count.

**Scope**: Requires changes to `filter_facts_for_query` and the query planner to propagate bound values from patterns into the storage fetch call. More invasive than the Phase 7.4 snapshot fix; deferred to avoid destabilising the pre-1.0 query path.

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
- crates.io publish deferred to Phase 7.9 (API cleanup + publish prep)

### v0.18.0 - ✅ Phase 7.8 (Prepared Statements)
- ✅ `$identifier` bind slot tokens: `EdnValue::BindSlot`, `AsOf::Slot`, `ValidAt::Slot`, `Expr::Slot` AST variants
- ✅ `BindValue` enum: `Entity(Uuid)`, `Val(Value)`, `TxCount(u64)`, `Timestamp(i64)`, `AnyValidTime`
- ✅ `PreparedQuery` struct — stores parsed AST + optimised plan; re-executes against live fact store
- ✅ `Minigraf::prepare(query_str) -> Result<PreparedQuery>` and `PreparedQuery::execute(bindings)` public API
- ✅ Deep-clone + AST walk substitution (executor, optimizer, matcher unchanged)
- ✅ Panic guards in `executor.rs` and `storage.rs` for unsubstituted slots
- ✅ `BindValue` and `PreparedQuery` re-exported from `lib.rs`
- ✅ `tests/prepared_statements_test.rs` — 17 integration tests; 780 tests total

### v0.17.0 - ✅ Phase 7.7b (UDFs)
- ✅ User-defined functions (UDFs) via `register_fn`
- ✅ Built-in aggregates (`count-distinct`, `avg`, `sum`, `min`, `max`) and window functions (`row_number`, `rank`, `dense_rank`)
- ✅ `count-distinct` O(n log n) with `Ord` impl for `Value`
- ✅ File format v7 (84 bytes) with automatic v6 migration
- ✅ 505 tests passing
- ✅ `src/query/datalog/stratification.rs`: `DependencyGraph`, `stratify()` — Bellman-Ford cycle detection; negative cycles rejected at rule registration time with a clear error
- ✅ `(not clause…)` in queries and rule bodies — safety check requires all body vars bound by outer clauses
- ✅ `(not-join [?v…] clause…)` — existential negation with explicit join-variable sharing; body-only variables are fresh/unbound
- ✅ `StratifiedEvaluator`: stratifies rules, applies `not`/`not-join` per-binding filters in mixed-rule strata
- ✅ `evaluate_not_join`: handles `Pattern` and `RuleInvocation` body clauses; queries accumulated derived facts
- ✅ 407 tests passing; `tests/negation_test.rs` (10) + `tests/not_join_test.rs` (14) added

### v0.11.0 - ✅ Phase 7.2a (Aggregation)
- ✅ `count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max` in `:find` clause
- ✅ `:with` grouping clause — variables that participate in grouping but are excluded from output
- ✅ `AggFunc` enum, `FindSpec` enum; `DatalogQuery.find` migrated from `Vec<String>` to `Vec<FindSpec>`
- ✅ `apply_aggregation` post-processing in `executor.rs`; parse-time validation (aggregate vars must be bound)
- ✅ `tests/aggregation_test.rs` — 24 integration tests; 461 tests passing

### v0.12.0 - ✅ Phase 7.2b (Arithmetic & Predicate Expression Clauses)
- ✅ `BinOp` (14 variants), `UnaryOp` (5 variants), `Expr` AST, `WhereClause::Expr { expr, binding }` in `types.rs`
- ✅ Filter predicates: `[(< ?age 30)]`, `[(string? ?v)]`, `[(starts-with? ?tag "work")]`, `[(matches? ?email "...")]`
- ✅ Arithmetic bindings: `[(+ ?price ?tax) ?total]`, `[(* ?a ?b) ?r]`, nested `[(+ (* ?a 2) ?b) ?result]`
- ✅ `parse_expr` with parse-time regex validation; forward-pass safety check at all 4 dispatch sites (`:where`, rule body, `not`, `not-join`)
- ✅ `eval_expr` / `is_truthy`: int/float promotion, integer division truncation, NaN guard, type mismatch → row drop, div/0 → row drop
- ✅ `tests/predicate_expr_test.rs` — 28 integration tests; 527 tests passing (365 unit + 156 integration + 6 doc)

### v0.13.0 - ✅ Phase 7.3 (Disjunction — `or` / `or-join`)
- ✅ `WhereClause::Or(Vec<Vec<WhereClause>>)` and `WhereClause::OrJoin { join_vars, branches }` variants
- ✅ `(or ...)` / `(or-join [?v...] ...)` in `:where` clauses and rule bodies; `(and ...)` grouping
- ✅ `match_patterns_seeded` on `PatternMatcher`; `evaluate_branch` / `apply_or_clauses` in `executor.rs`
- ✅ `DependencyGraph::from_rules` refactored with recursive `collect_clause_deps`
- ✅ `tests/disjunction_test.rs` — 16 integration tests; 562 tests passing (384 unit + 172 integration + 6 doc)

### v0.13.1 - ✅ Phase 7.4 (`filter_facts_for_query` snapshot fix)
- ✅ `filter_facts_for_query` returns `Arc<[Fact]>` — eliminates O(N) four-BTreeMap index rebuild on every non-rules query call
- ✅ `execute_query` path constructs zero `FactStorage` objects; `execute_query_with_rules` still converts for `StratifiedEvaluator`
- ✅ `PatternMatcher::from_slice(Arc<[Fact]>)` constructor added
- ✅ `apply_or_clauses` and `evaluate_not_join` signatures updated to accept `Arc<[Fact]>`
- ✅ Evaluator loop: `accumulated_facts` computed once per iteration (was 4 separate `get_asserted_facts()` calls)
- ✅ ~62–65% speedup on non-rules queries at 10K facts (`query/point_entity/10k`: 22 ms → 8.6 ms; `aggregation/count_scale/10k`: 28 ms → 9.7 ms)
- ✅ 568 tests passing (390 unit + 172 integration + 6 doc)

### v0.14.0 - ✅ Phase 7.5 (Tests + Error Coverage)
- ✅ `tests/production_patterns_test.rs` — 8 cross-feature integration tests (not+as-of, not-join+count, recursion+not, or+count, or+sum, count+valid-at, count+as-of-sequence)
- ✅ `tests/error_handling_test.rs` — 8 error-path integration tests; 1 ignored (confirmed or+neg-cycle stratification bug)
- ✅ Stream 3: ~53 unit tests for parser-unreachable branches in `executor.rs` and `evaluator.rs`
- ✅ Branch coverage: `executor.rs` ~85.71% (up from ~75%), `evaluator.rs` ~89.29% (up from ~73%)
- ✅ 617 tests passing (424 unit + 187 integration + 6 doc)

### ✅ Phase 7 (Datalog Completeness) — shipped v0.11.0–v0.19.0
- Stratified negation (`not` / `not-join`)
- Aggregation (`count`, `sum`, `min`, `max`, `distinct`, `:with`) + arithmetic filter predicates
- Disjunction (`or` / `or-join`)
- Query optimizer improvements (cost-based, rule evaluation)
- Temporal metadata pseudo-attributes (`:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`)
- Full four-class temporal query taxonomy (point-in-time, time interval, time-point lookup, time-interval lookup)
- Window functions (`sum/count/rank/lag/lead :over (partition-by … :order-by …)`) — `SUM OVER`–style analytics in Datalog `:find`
- UDFs: embedder-registered aggregate and predicate functions via `FunctionRegistry`
- Prepared statements with temporal bind slots (including predicate-argument and UDF positions)
- ≥90% branch coverage
- Stable API, stable file format, comprehensive tests, full documentation, performance validated

### v1.0.0 - 🎯 Phase 8 (Cross-platform)
- WASM support (browser + WASI) ✅ Phase 8.1 complete (v0.20.0)
- Mobile bindings (iOS + Android)
- Language bindings (Python, C, Node.js)

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
- ✅ Phase 7.1: Complete (March 2026) - Stratified negation (`not` / `not-join`), 407 tests
- ✅ Phase 7.2a: Complete (March 2026) - Aggregation (`count`/`sum`/`min`/`max`/`distinct`/`:with`), 461 tests
- ✅ Phase 7.2b: Complete (March 2026) - Arithmetic & predicate expression clauses, 527 tests
- ✅ Phase 7.3: Complete (March 2026) - Disjunction (`or` / `or-join`), 562 tests
- ✅ Phase 7.4: Complete (March 2026) - `filter_facts_for_query` snapshot fix, eliminate 4-index rebuild, 568 tests
- ✅ Phase 7.5: Complete (March 2026) - Cross-feature tests, error-path coverage, ~86-89% branch coverage, 617 tests
- ✅ Phase 7.6: Complete (April 2026) - Temporal metadata bindings (`:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`, `:db/valid-at`), 647 tests
- ✅ Phase 7.7a: Complete (April 2026) - Window functions (`sum/count/min/max/avg/rank/row-number :over`), `FunctionRegistry`, 705 tests
- ✅ Phase 7.7b: Complete (April 2026) - User-Defined Functions (`register_aggregate`/`register_predicate`), 727 tests
- ✅ Phase 7.8: Complete (April 2026) - Prepared statements (`$slot` bind tokens, temporal bind slots, plan reuse), 780 tests
- ✅ Phase 7.9: Complete (April 2026) - Publish prep (crates.io API cleanup, Rustdoc sweep, `unwrap` audit, CI matrix), 788 tests
- ✅ Phase 8.1: Complete (April 2026) - Browser WASM (`BrowserDb`, IndexedDB backend) + WASI (`wasm32-wasip1` CI) + cross-platform compat tests, 795 tests
- 🔄 Phase 8.2: In progress — Mobile bindings (Android `.aar` + iOS `.xcframework` via UniFFI 0.31.1), 815 tests
- 🔄 Phase 8: In progress (Cross-platform — WASM ✅, mobile, language bindings → **v1.0.0**)
- 🎯 Phase 9: Ongoing (Ecosystem — integration examples, cookbook, GraphRAG/LangChain examples)

**Note**: This is a hobby project. Timeline is flexible but realistic.

---

## Current Focus

**Right Now**: Phase 8.3 In Progress — Language Bindings

**Phase 8.2 Progress** ✅ COMPLETE:
1. ✅ Workspace conversion (`minigraf-ffi` crate added)
2. ✅ `MiniGrafDb` FFI API (open, open_in_memory, execute, checkpoint) via UniFFI 0.31.1
3. ✅ Android Gradle project (`.aar` assembly + GitHub Packages publishing)
4. ✅ Swift Package Manager manifest (`Package.swift`)
5. ✅ CI workflows: `mobile.yml` (Android/iOS), `wasm-release.yml` (WASM artifacts)
6. ✅ 815 tests passing (795 core + 20 FFI)

**Phase 8.3 Progress**:
- ✅ 8.3a: Python bindings via UniFFI — PyPI `minigraf` — v0.22.0 COMPLETE
- ✅ 8.3b: Desktop JVM bindings — fat JAR on Maven Central (`io.github.adityamukho:minigraf-jvm:0.23.0`) — v0.23.0 COMPLETE
- 🎯 8.3c: C header via `cbindgen` (v0.24.0)
- 🎯 8.3d: Node.js bindings via `napi-rs` (v0.25.0)

**Immediate Next Steps (Phase 8.3)**:
1. 8.3c: C header via `cbindgen`
2. 8.3d: Node.js bindings via `napi-rs`

**Key Decisions Made**:
- ✅ Datalog query language (simpler, better for temporal)
- ✅ Bi-temporal as first-class feature (not afterthought)
- ✅ Keep single-file philosophy
- ✅ Recursive rules with semi-naive evaluation
- ✅ UTC-only timestamps (avoids chrono GHSA-wcg3-cvx6-7396)
- ✅ Packed pages over one-per-page (philosophy: small binary, efficient storage)
- ✅ Approximate LRU (read-lock on hits — avoids write-lock contention)
- ✅ Phase 8 = v1.0.0 (not Phase 7 — cross-platform completion is the 1.0 milestone)

See [GitHub Issues](https://github.com/adityamukho/minigraf/issues) for specific tasks.

---

**Last Updated**: Phase 8.3b Complete (April 2026) — 795 tests passing, v0.23.0
