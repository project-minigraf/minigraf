# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## v0.23.0 — Phase 8.3b: Java Desktop JVM Bindings (2026-04-25)

### Added
- **Phase 8.3b**: Java desktop JVM bindings published to Maven Central as
  `io.github.adityamukho:minigraf-jvm:0.23.0`. Add to Gradle:
  `implementation("io.github.adityamukho:minigraf-jvm:0.23.0")`.
  Fat JAR with embedded natives for Linux x86_64/aarch64, macOS universal2,
  Windows x86_64. API: `MiniGrafDb.open(path)`, `MiniGrafDb.openInMemory()`,
  `.execute(datalog)`, `.checkpoint()`.
- `minigraf-ffi/java/`: Gradle 8.11 project — `build.gradle.kts`, `settings.gradle.kts`,
  `NativeLoader.kt` (runtime native extraction from JAR resources), and Gradle wrapper
- `minigraf-ffi/java/src/test/kotlin/.../BasicTest.kt`: JUnit 5 suite (in-memory, transact/query,
  error handling, file-backed persistence)
- `.github/workflows/java-ci.yml`: PR test matrix on 4 platforms (Linux x86_64, Linux aarch64,
  macOS universal2, Windows x86_64)
- `.github/workflows/java-release.yml`: release workflow — cross-compiles natives on 4 platforms,
  assembles fat JAR, publishes to Maven Central via Sonatype OSSRH

795 tests.

## v0.22.0 — Phase 8.3a: Python Bindings (2026-04-25)

### Added
- **Phase 8.3a**: Python bindings published to PyPI as `minigraf`.
  Install with `pip install minigraf`. API: `MiniGrafDb.open(path)`,
  `MiniGrafDb.open_in_memory()`, `.execute(datalog)`, `.checkpoint()`.
  Pre-built wheels for Linux x86_64/aarch64, macOS universal2, Windows x86_64.

## v0.21.1 — Patch: mobile/WASM docs (2026-04-19)

### Changed
- `src/lib.rs`: added **Feature Flags** section and **WebAssembly targets** subsection to crate-level docs — browser feature, `wasm32-unknown-unknown` target switcher note, and WASI build command
- `README.md`: updated "For Mobile Apps" section — replaced Phase 8 placeholder with current state, added Kotlin/Swift quick-start snippets and link to wiki integration guide
- Wiki `Use-Cases.md`: replaced Integration placeholder with full Android (Gradle setup, Kotlin API, error handling, threading) and iOS (SPM setup, Swift API, error handling, async) integration guides

795 tests.

## v0.21.0 — Phase 8.2: Android/iOS Mobile Bindings (2026-04-19)

### Added
- `minigraf-ffi` crate: UniFFI 0.31 bindings exposing `MiniGrafDb` (open, openInMemory, execute, checkpoint) and `MiniGrafError` (Parse, Query, Storage, Other) to Kotlin and Swift
- Android `.aar` release artifact, published to GitHub Packages (`io.github.adityamukho:minigraf-android`)
- iOS `.xcframework` release artifact, distributed via Swift Package Manager (`Package.swift` at repo root)
- `mobile.yml` CI workflow: cross-compiles Android targets with `cargo-ndk`, generates Kotlin/Swift UniFFI bindings, assembles AAR with Gradle, assembles xcframework with `xcodebuild`, and publishes both on every tag
- `docs-check` CI job in `rust.yml` and `release.yml` — gates releases on `cargo doc --all-features` passing cleanly

### Fixed
- `release.yml`: added `docs-check` to `host` job's `needs` and `if` condition
- `wasm-release.yml` / `mobile.yml`: retry loops extended from 20 to 40 attempts; `inputs.tag || github.ref_name` ordering corrected
- `minigraf-ffi/android/gradlew`: removed inner double-quotes from `DEFAULT_JVM_OPTS` and replaced xargs/sed eval block with direct `exec` — fixes "Could not find main class" and garbled usage output
- `minigraf-ffi/android/build.gradle.kts`: added `android { publishing { singleVariant("release") } }` — fixes AGP 8.x "SoftwareComponent 'release' not found"
- `mobile.yml` Package.swift commit: pushes to unprotected `swift-releases` branch and moves tag via `gh api -F force=true` — avoids branch-protection blocks and string/boolean type mismatch

795 tests.

## v0.20.1 — Patch: docs.rs browser module visibility (2026-04-19)

### Fixed
- `browser` module now appears on docs.rs: added `docsrs` to the `cfg` gate and `doc(cfg(...))` badge annotation (`src/lib.rs`)

## v0.20.0 — Phase 8.1: WebAssembly Support (2026-04-18)

### Added
- **Phase 8.1a** — Browser WASM (`wasm32-unknown-unknown` + `wasm-bindgen`):
  - `BrowserDb` public API: `open_in_memory()`, `execute()`, `checkpoint()`, `export_graph()`, `import_graph()`
  - `BrowserBufferBackend` — in-memory `StorageBackend` over a flat page buffer, identical byte layout to the native `.graph` format
  - `IndexedDbBackend` — page-granular IndexedDB storage (one 4 KB entry per page); only dirty pages written on checkpoint
  - `wasm-pack` build workflow (`wasm32-unknown-unknown --features browser`) generating `pkg/` with JS glue and TypeScript definitions
  - `wasm-bindgen-test` browser integration tests (Chrome + Firefox via `wasm-pack test`)
- **Phase 8.1b** — Server-side WASM (`wasm32-wasip1` / WASI):
  - `FileBackend` verified under WASI's capability-based filesystem (no backend changes needed)
  - CI workflow (`wasm-wasi.yml`) builds, unit-tests, and smoke-tests under Wasmtime and Wasmer on every push/PR
  - Thread-dependent tests gated with `#[cfg(not(target_os = "wasi"))]`
- **Cross-platform compatibility tests** (issue #150):
  - `tests/cross_platform_compat_test.rs` — native round-trip (raw page byte copy) and fixture-readability tests
  - `tests/fixtures/compat.graph` — committed v7 binary fixture containing `:alice :name "Alice"` and `:alice :age 30`
  - `examples/generate_compat_fixture.rs` — reproducible fixture generator (native only; no-op on wasm32)
  - `native_fixture_readable_by_browser_db` wasm-bindgen-test — loads native fixture via `BrowserDb::import_graph`, verifies both facts
- Release workflow: WASM artifacts (WASI binary + browser tarball) built and attached on every tag; `cargo publish` to crates.io on release

795 tests.

## v0.19.0 — Phase 7.9: Publish Prep (2026-04-08)

### Changed (breaking — internal visibility only)
- `Minigraf::repl()` factory method replaces direct `Repl::new(FactStorage)` constructor — users call `db.repl().run()` instead
- All internal types narrowed to `pub(crate)`: `FactStorage`, `PersistentFactStorage`, `FileHeader`, `StorageBackend`, `DatalogExecutor`, `PatternMatcher`, `Fact`, `TxId`, `VALID_TIME_FOREVER`, `Wal`, and all related internals
- `Minigraf::inner_fact_storage()` removed (was unused)

### Added
- `Minigraf::repl(&self) -> Repl<'_>` — constructs an interactive REPL session; `Repl` now borrows `&Minigraf` for lifetime safety
- Full rustdoc on all public API items with `# Examples` doctests
- `[package.metadata.docs.rs]` in `Cargo.toml` — docs.rs builds with `all-features = true`
- `#![warn(missing_docs)]` — enforces documentation coverage going forward
- crates.io and docs.rs badges in `README.md`
- Installation section in `README.md` (`cargo add minigraf` / `[dependencies]` block)
- macOS and Windows added to CI test matrix (`rust.yml`)
- Strict `cargo clippy -- -D warnings` step in `rust-clippy.yml`

### Fixed
- Bare `.unwrap()` in library code replaced with `.expect("lock poisoned")` (RwLock operations in `cache.rs`, `evaluator.rs`) and `.expect("WAL not initialized")` (`db.rs`)
- `FileHeader::to_bytes` now takes `self` by value (clippy `wrong_self_convention`)
- Broken intra-doc link `[Repl::run]` in `db.rs` fixed to `[crate::repl::Repl::run]`

788 tests.

## v0.18.0 — Phase 7.8: Prepared Statements (2026-04-04)

### Added
- `Minigraf::prepare(query_str) -> Result<PreparedQuery>` — parse and plan a query once,
  returning a `PreparedQuery` that can be executed many times with different bind values
- `PreparedQuery::execute(bindings: &[(&str, BindValue)]) -> Result<QueryResult>` — substitute
  named `$slot` tokens and run against the current fact store state; plan is reused on each call
- `BindValue` enum — `Entity(Uuid)`, `Val(Value)`, `TxCount(u64)`, `Timestamp(i64)`,
  `AnyValidTime`; each variant is permitted only in the appropriate bind-slot position
- `$identifier` bind slot tokens in parser — accepted in entity position, value position,
  `:as-of`, and `:valid-at`; attribute position is intentionally rejected at prepare time
- `EdnValue::BindSlot(String)`, `AsOf::Slot(String)`, `ValidAt::Slot(String)`,
  `Expr::Slot(String)` AST variants (parse-only; panic at runtime if unsubstituted)
- `BindValue` and `PreparedQuery` re-exported from `lib.rs` (public API surface)
- `tests/prepared_statements_test.rs` — 17 integration tests covering all slot positions,
  combined temporal + entity parameterisation, plan reuse, and all error paths

### Internal
- `src/query/datalog/prepared.rs` — new module: `prepare_query()`, substitution logic,
  19 unit tests; manual `Debug` impl for `PreparedQuery` (avoids `FactStorage: Debug` bound)
- Panic guards (no slot-name interpolation) in `executor.rs` (4 sites) and `storage.rs` (1 site)
  for unsubstituted slot variants; CodeQL-safe (no user-controlled string in panic message)

### Unchanged
- `db.execute(str)` string API — no breaking change
- Executor, optimizer, matcher — no changes required

## v0.17.0 — Phase 7.7b: User-Defined Functions (2026-04-02)

### Added
- `Minigraf::register_aggregate(name, init, step, finalise)` — register a custom aggregate
  function usable in both `:find` grouping and `:over` (window) clauses
- `Minigraf::register_predicate(name, f)` — register a single-argument filter predicate
  usable in `[(name? ?var)]` `:where` clauses
- `FunctionRegistry::register_aggregate_desc` / `register_predicate_desc` (internal API)
- `WindowFunc::Udf(String)` and `UnaryOp::Udf(String)` AST variants for runtime-resolved functions
- `UdfOps`, `AggImpl`, `PredicateDesc` types in `functions.rs`

### Changed
- `AggregateDesc` now uses `AggImpl` discriminator instead of `window_compatible`+`window_ops`
- `apply_expr_clauses` now returns `Result<Vec<Binding>>` and accepts `&FunctionRegistry`
- `eval_expr` accepts `Option<&FunctionRegistry>` for UDF predicate resolution
- `WindowSpec::func_name()` now returns `String` instead of `&'static str`
- Parser emits `Udf` variants for unknown names instead of erroring (runtime validation)

### Test count: 727 tests

## v0.16.0 — Phase 7.7a: Window Functions (2026-04-02)

### Added
- **Window functions** in Datalog `:find` clause: `(sum ?v :over (...))`, `(count ?v :over (...))`, `(min ?v :over (...))`, `(max ?v :over (...))`, `(avg ?v :over (...))`, `(rank :over (...))`, `(row-number :over (...))` with unbounded-preceding (cumulative from partition start to current row) frame
- **`:partition-by ?var`** optional clause: absent means whole result set is one partition
- **`:order-by ?var`** required in every `:over` clause; `:desc` optional (default ascending)
- **`FunctionRegistry`** (`src/query/datalog/functions.rs`): string-keyed registry of aggregate descriptors; all built-in aggregates migrated into it; `window_ops` (init/step/finalise) on window-compatible entries; `is_builtin` flag separates built-ins from future UDFs
- **Mixed queries**: regular aggregates and window functions may coexist in the same `:find` clause; aggregates collapse rows first, windows annotate over collapsed rows
- **`AggregateDesc`**, **`AggState`**, **`WindowOps`** types in `functions.rs`
- **`WindowFunc`**, **`Order`**, **`WindowSpec`**, **`FindSpec::Window`** types in `types.rs`
- `tests/window_functions_test.rs`: 12 integration tests (cumulative sum, running count/min/avg, rank with ties, row-number, partition-by, desc ordering, mixed aggregate+window, single-row and empty-result edge cases, lag/lead parse rejection)

### Changed
- `FindSpec::Aggregate { func }`: type of `func` changed from `AggFunc` enum to `String`; dispatch goes through `FunctionRegistry` — internal change, no public API impact
- `AggFunc` enum removed from `types.rs`; all aggregate dispatch centralised in `functions.rs`
- `apply_aggregation` and `apply_agg_func` removed from `executor.rs`; replaced by `apply_post_processing` + helpers

### Total
707 tests (unit + integration + doc)

## v0.15.0 — Phase 7.6: Temporal Metadata Bindings (2026-04-01)

### Added
- **Temporal pseudo-attributes**: `:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`, and `:db/valid-at` are now first-class bindable values in Datalog `:where` patterns
- `PseudoAttr` enum and `AttributeSpec` wrapper type in `types.rs` — clean type-safe representation for real vs. pseudo attributes in `Pattern`
- `parse_query_pattern` in `parser.rs` — detects `:db/*` keywords in the attribute position; rejects them in entity/value positions (parse error)
- `PatternMatcher::from_slice_with_valid_at` constructor — passes query-level `valid_at` into the matcher
- Hard-error guard in executor: per-fact pseudo-attrs (`:db/valid-from`, `:db/valid-to`, `:db/tx-count`, `:db/tx-id`) require `:any-valid-time`; error message tells user exactly what to add
- `:db/valid-at` binds the effective query timestamp: explicit `:valid-at <ts>` → `Value::Integer(ts)`, no `:valid-at` → `Value::Integer(now)`, `:any-valid-time` → `Value::Null`
- `:any-valid-time` now accepted as a standalone top-level query keyword (previously required `:valid-at :any-valid-time` form)
- `tests/temporal_metadata_test.rs`: 16 new integration tests covering time-interval range queries, time-point lookups, tx-time correlation, `:db/valid-at` semantics, and all parse/runtime error guards

### Total
647 tests (438 unit + 209 integration)

## v0.14.0 — Phase 7.5: Tests + Error Coverage (2026-03-31)

### Added
- `tests/production_patterns_test.rs`: 8 cross-feature integration tests combining not+as-of, not-join+count, count+not, count+valid-at, recursion+not, or+count, or+sum, count+as-of-sequence
- `tests/error_handling_test.rs`: 8 integration-level error-path tests covering runtime type errors (sum/string, sum/mixed, max/boolean), stratification errors (negative cycles), and parse safety errors (not-join unbound join var, or mismatched vars, aggregate unbound var)
- Stream 3: ~109 unit tests for parser-unreachable branches and aggregation/arithmetic edge cases in `executor.rs` and `evaluator.rs`
- `cargo-llvm-cov` branch coverage command documented in `CONTRIBUTING.md`
- CI coverage enforcement: `cargo-tarpaulin --fail-under 75` gates every PR; Codecov 75% threshold with 2% drop tolerance; `fail_ci_if_error: true`
- Nightly `cargo-llvm-cov --branch` workflow: uploads LCOV to Codecov (`branch-coverage` flag) and attaches HTML artifact (30-day retention); also triggerable via `workflow_dispatch`
- Codecov badge added to `README.md`

### Coverage
- Branch coverage: `executor.rs` ~85.71% (from ~75%), `evaluator.rs` ~89.29% (from ~73%)
- Remaining uncovered branches: NaN-check defensive code not reachable via public API
- Total: 617 tests (424 unit + 187 integration + 6 doc)

### Known Issues
- `or`-with-negative-cycle: stratification does not currently detect negative cycles inside `or` branches. Tracked via `#[ignore]` in `tests/error_handling_test.rs::or_negative_cycle_rejected`.

## [0.13.1] — 2026-03-27

### Performance

- **`filter_facts_for_query` snapshot fix** — function now returns `Arc<[Fact]>` instead of a throwaway `FactStorage`, eliminating the O(N) four-BTreeMap index rebuild that occurred on every non-rules query call. `execute_query` path constructs zero `FactStorage` objects. `execute_query_with_rules` still converts `Arc<[Fact]>` back to `FactStorage` for `StratifiedEvaluator` (deferred).
- ~62–65% speedup on non-rules queries at 10K facts: `query/point_entity/10k` 22 ms → 8.6 ms; `aggregation/count_scale/10k` 28 ms → 9.7 ms.
- Evaluator loop: `accumulated_facts` computed once per iteration (was 4 separate `get_asserted_facts()` calls).

### Added

- `PatternMatcher::from_slice(Arc<[Fact]>)` constructor — creates a matcher from an immutable fact snapshot without index reconstruction.

### Technical

- `apply_or_clauses` and `evaluate_not_join` signatures updated to accept `Arc<[Fact]>` instead of `&FactStorage`.
- 6 new tests: 4 in `matcher.rs` (unit), 2 in `executor.rs` (unit).

### Tests

- Total: 568 tests passing (390 unit + 172 integration + 6 doc)

## [0.13.0] — 2026-03-26

### Added
- **Disjunction (`or` / `or-join`)**: queries and rule bodies can now use `(or branch1 branch2 ...)` and `(or-join [?v...] branch1 branch2 ...)` where-clauses. Branches support all other clause types including `not`, `not-join`, `Expr`, and nested `or`/`or-join`. `(and ...)` groups multiple clauses into a single branch.
- `match_patterns_seeded` on `PatternMatcher` for seeded branch evaluation.
- `evaluate_branch` and `apply_or_clauses` as `pub(crate)` helpers in `executor.rs`.

### Technical
- `WhereClause` enum gains `Or(Vec<Vec<WhereClause>>)` and `OrJoin { join_vars, branches }` variants.
- `DependencyGraph::from_rules` refactored with recursive `collect_clause_deps` helper; `Or`/`OrJoin` branches contribute positive dependency edges.
- Rules with `or`/`or-join` in their bodies route to the `mixed_rules` path in `StratifiedEvaluator`.

## [0.12.0] - 2026-03-25

### Added
- `BinOp` enum (14 variants: `Lt`, `Gt`, `Lte`, `Gte`, `Eq`, `Neq`, `Add`, `Sub`, `Mul`, `Div`, `StartsWith`, `EndsWith`, `Contains`, `Matches`) in `types.rs`
- `UnaryOp` enum (5 variants: `StringQ`, `IntegerQ`, `FloatQ`, `BooleanQ`, `NilQ`) in `types.rs`
- `Expr` enum (`Var`, `Lit`, `BinOp`, `UnaryOp`) — composable expression AST in `types.rs`
- `WhereClause::Expr { expr: Expr, binding: Option<String> }` variant — `None` = filter, `Some(var)` = arithmetic binding
- `parse_expr_arg` / `parse_expr` / `parse_expr_clause` in `parser.rs`; dispatch at all 4 clause sites (query `:where`, rule body, `not` body, `not-join` body)
- Parse-time regex validation for `matches?` patterns via `regex-lite`; invalid patterns are rejected with a clear error
- `check_expr_safety` + `check_expr_safety_with_bound` in `parser.rs` — forward-pass safety check; recurses into `not`/`not-join` bodies; unbound `Expr::Var` references are rejected at parse time
- `outer_vars_from_clause` updated for `WhereClause::Expr` — binding variable contributes to scope for subsequent clauses
- `eval_expr`, `eval_binop`, `is_truthy`, `apply_expr_clauses` in `executor.rs` — evaluate expression trees against a binding; type mismatches and div/0 silently drop the row
- `apply_expr_clauses_in_evaluator` in `evaluator.rs` — sibling helper for rule body and `not-join` evaluation paths
- `not_body_matches` in `executor.rs` updated to seed with outer binding for expr-only `not` bodies
- `tests/predicate_expr_test.rs` — 28 integration tests covering all operators, silent-drop semantics, integer division, NaN, int/float promotion, string predicates, regex, expr in `not` body, expr in rule body, bi-temporal + expr, arithmetic into aggregate

### Semantics
- Comparison operators (`<`, `>`, `<=`, `>=`) require both operands to be numeric (`Integer` or `Float`); type mismatch → row dropped
- `=` / `!=` use structural equality on `Value` — type mismatch returns `false`/`true`, not an error
- Integer `+` `Float` promotes to `Float`; integer division truncates; division by zero → row dropped; NaN result → row dropped
- `is_truthy`: `Boolean(true)` → true; non-zero `Integer` or `Float` → true; everything else (including `Keyword`, `Ref`, `Null`, zero, empty string, `Boolean(false)`, `-0.0`) → false
- `matches?` pattern compiled at eval time via `regex-lite`; pattern must be a string literal validated at parse time

### Tests
- Added `tests/predicate_expr_test.rs` (28 integration tests)
- Total: 527 tests passing (365 unit + 156 integration + 6 doc)

## [0.11.0] - 2026-03-25

### Added
- Aggregation in `:find` clause: `count`, `count-distinct`, `sum`, `sum-distinct`, `min`, `max`
- `:with` grouping clause — variables that participate in grouping but are excluded from output rows
- `AggFunc` enum and `FindSpec` enum in `src/query/datalog/types.rs`; `DatalogQuery.find` migrated from `Vec<String>` to `Vec<FindSpec>`; `DatalogQuery.with_vars: Vec<String>` field added
- `apply_aggregation` post-processing step in `executor.rs` — runs after binding collection when any aggregate is present
- `extract_variables` helper in `executor.rs` — non-aggregate extraction path (replaces inline loops)
- `apply_agg_func` and `value_type_name` helpers in `executor.rs`
- `parse_aggregate` helper in `parser.rs`; `:find` arm extended to accept `EdnValue::List` (aggregate expressions); `:with` keyword arm added
- Parse-time validation: aggregate variables must be bound in `:where`; `:with` without any aggregate is rejected
- `tests/aggregation_test.rs` — 24 integration tests covering all aggregates, `:with`, rules, negation, temporal filters

### Semantics
- `count`/`count-distinct` with no grouping vars on zero bindings → `[[0]]` (SQL behavior)
- All other aggregates on zero bindings → empty result set
- All aggregates skip `Value::Null` silently (SQL behavior)
- Type mismatches (e.g. `sum` on `String`) fail fast with a runtime error
- `min`/`max` on mixed `Integer`/`Float` is a runtime error
- `:with ?v` adds `?v` to the grouping key without adding it to output columns

### Tests
- Added `tests/aggregation_test.rs` (24 integration tests)
- Total: 461 tests passing (327 unit + 128 integration + 6 doc)

## [0.10.0] - 2026-03-24

### Added
- `src/query/datalog/stratification.rs` — `DependencyGraph` and `stratify()`: analyse rule dependency graphs at registration time; programs with negative cycles are rejected with a clear error
- `WhereClause::Not(Vec<WhereClause>)` and `WhereClause::NotJoin { join_vars, clauses }` variants in `types.rs`; all exhaustive matches updated
- `(not clause…)` in `:where` and rule bodies — stratified negation where all body variables must be pre-bound by outer clauses
- `(not-join [?v…] clause…)` — existentially-quantified negation with explicit join-variable declaration; body variables not in `join_vars` are fresh/unbound
- Safety check at parse time: every `not` body variable must be bound by an outer clause; every `join_vars` variable in `not-join` must be bound by an outer clause
- Nesting constraint: `not-join` cannot appear inside `not` or another `not-join` — rejected at parse time
- `StratifiedEvaluator` in `evaluator.rs`: stratifies rules, runs positive rules first, then applies `not`/`not-join` filters per binding for mixed rules
- `evaluate_not_join` free function in `evaluator.rs`: builds partial binding from `join_vars`, converts `Pattern` and `RuleInvocation` body clauses to patterns, runs `PatternMatcher`; returns `true` if body is satisfiable (reject outer binding)
- `rule_invocation_to_pattern` extracted as `pub(super)` free function from `RecursiveEvaluator`
- Two not-post-filter sites in `executor.rs` now handle both `Not` and `NotJoin` via `evaluate_not_join`
- `tests/negation_test.rs` — 10 integration tests for `not` (Phase 7.1a): basic absence, multi-clause, rule body, time-travel, negative cycle rejection
- `tests/not_join_test.rs` — 14 integration tests for `not-join` (Phase 7.1b): basic exclusion, multiple join vars, multi-clause body, rule body, `:as-of`, `:valid-at`, negative cycle at registration, `not`+`not-join` coexistence, `RuleInvocation` in body end-to-end

### Changed
- `Rule.body` changed from `Vec<EdnValue>` to `Vec<WhereClause>` to support negation clauses alongside patterns
- `executor.rs` `execute_query_with_rules` now delegates to `StratifiedEvaluator` instead of `RecursiveEvaluator` directly
- `rules.rs` `register_rule` runs `stratify()` after each registration; returns `Err` on negative cycle (rules are not registered on error)

## [0.9.0] - 2026-03-23

### Added
- `src/storage/btree_v6.rs` — proper on-disk B+tree for all four covering indexes (EAVT, AEVT, AVET, VAET); each B+tree node is one 4KB page (internal + leaf), with `build_btree` for bulk-load and `range_scan` for leaf-chain traversal
- `OnDiskIndexReader` struct + `CommittedIndexReader` trait — page-cache-backed index lookup replacing the full in-memory BTreeMap; index memory usage is now O(cache_pages), not O(facts)
- `MutexStorageBackend<B>` adapter — holds backend mutex only for the duration of a single `read_page` call on a cache miss; cache-warm pages require no lock, enabling concurrent range scans to proceed in parallel
- `tests/btree_v6_test.rs` — 8 integration tests covering B+tree insert/range-scan, multi-page leaf chains, concurrent scan correctness with Barrier-synchronised threads, and v5→v6 migration roundtrip
- `test_concurrent_range_scans_correctness` unit test in `btree_v6.rs` — verifies all 8 concurrent threads return identical non-empty scan results
- `bench_concurrent_btree_scan` Criterion benchmark — measures wall-clock latency at 2/4/8 concurrent EAVT range scans; results updated in `BENCHMARKS.md`
- `FileHeader` v6 (80 bytes): adds `fact_page_count u64` field at bytes 72–80; automatic v5→v6 migration on first checkpoint

### Changed
- `FORMAT_VERSION` bumped 5→6; v5 databases auto-migrated on first save
- `BENCHMARKS.md` updated with v6 open/memory improvements, concurrent B+tree scan results, heaptrack v6 numbers, and a "How to read these numbers" methodology section
- `README.md` and `BENCHMARKS.md`: performance table updated to reflect v6 open-time reduction (~2.4×) and peak-heap reduction (~21%)

### Fixed
- Concurrent B+tree range scans no longer serialise on cache-warm pages — `4→8 thread` scaling ratio improved from ~2.2× to ~1.9×

## [0.8.0] - 2026-03-22

### Added
- `BENCHMARKS.md` — full Criterion benchmark results at 1K/10K/100K/1M facts with machine spec, HTML report references, and heaptrack memory profiles
- `examples/memory_profile.rs` — heaptrack profiling binary; accepts fact count as positional arg
- `Cargo.toml` metadata: `repository`, `keywords`, `categories`, `readme`, `documentation` fields
- Memory profile table in `README.md` "Performance" section

### Changed
- `README.md` Performance section now links to `BENCHMARKS.md` for full benchmark details
- Phase badge and status text updated to reflect Phase 6.4b completion
- crates.io publish deferred to Phase 7.8 (API cleanup + publish prep; file format v6 now complete)

### Removed
- Dead `clap` dependency from `[dependencies]` — `clap` was listed but never imported in library or binary code

## [0.7.1] - 2026-03-22

### Fixed
- Retraction semantics in Datalog queries: `filter_facts_for_query` Step 2 now computes the *net view* per `(entity, attribute, value)` triple via `net_asserted_facts()`. Previously, retracted facts continued to appear in query results because the original assertion record remained in the append-only log. Now, for each EAV triple in the tx window, only the record with the highest `tx_count` is considered — if it is a retraction, the triple is excluded from results.
- Oversized facts are now rejected early in `db.rs` (`check_fact_sizes`) before any WAL write, using the `MAX_FACT_BYTES` constant (4 080 bytes) exported from `packed_pages.rs`. Previously, oversized facts could cause a panic deep in the page-packing path.

### Added
- `net_asserted_facts(facts: Vec<Fact>) -> Vec<Fact>` helper in `src/graph/storage.rs`: groups facts by EAV triple, keeps the record with the highest `tx_count`, and discards the triple if that record is a retraction. Used by both `executor.rs` and `storage.rs`.
- `check_fact_sizes(facts: &[Fact])` in `src/db.rs`: validates all facts against `MAX_FACT_BYTES` and returns a descriptive `Err` before writing to the WAL.
- `MAX_FACT_BYTES: usize` constant in `src/storage/packed_pages.rs`: `PAGE_SIZE - PACKED_HEADER_SIZE - 4` = 4 080 bytes.
- `tests/retraction_test.rs` — 7 integration tests covering: assert/retract with no `:as-of`, as-of snapshot before/after retraction boundary, re-assert after retract, `:any-valid-time` with retraction, recursive rule retraction visibility at and before the retraction boundary.
- `tests/edge_cases_test.rs` — 4 integration tests covering: oversized-fact file-backed error path, `MAX_FACT_BYTES` exact boundary (accepted), `MAX_FACT_BYTES + 1` (rejected), in-memory database has no size limit.

## [0.7.0] - 2026-03-22

### Added
- Packed fact pages (`page_type = 0x02`): ~25 facts per 4KB page, ~25× disk space reduction vs v4
- LRU page cache (`src/storage/cache.rs`): configurable capacity (default 256 pages = 1MB)
- `OpenOptions::page_cache_size(usize)` — tune page cache capacity
- `CommittedFactReader` trait: index-driven fact resolution via page cache (no startup load-all)
- File format v5: `fact_page_format` header field; auto-migration from v4 on first open
- Page-based CRC32 checksum (v5): streams raw committed pages instead of all facts

### Changed
- `PersistentFactStorage::new()` takes `page_cache_capacity: usize` as second argument
- Committed facts no longer loaded into `Vec<Fact>` at startup; only pending facts held in memory
- `FactStorage::get_facts_by_entity`, `get_facts_by_attribute` use EAVT/AEVT index range scans

### Fixed
- v4 databases auto-migrated to v5 packed format on first open (no data loss)

## [0.6.0] - 2026-03-21

### Added
- Four Datomic-style covering indexes (EAVT, AEVT, AVET, VAET) with bi-temporal keys (`valid_from`, `valid_to` in all key tuples)
- `FactRef { page_id: u64, slot_index: u16 }` — forward-compatible disk location pointer (slot_index=0 in 6.1)
- Canonical value encoding (`encode_value`) with sort-order-preserving byte representation
- B+tree page serialization for index persistence (`src/storage/btree.rs`)
- `FileHeader` v4 (72 bytes): adds `eavt_root_page`, `aevt_root_page`, `avet_root_page`, `vaet_root_page` (4×8=32 bytes), `index_checksum` (u32), replacing the `reserved` field
- CRC32 sync check on open: index mismatch triggers automatic rebuild
- `FactStorage::replace_indexes()` and `index_counts()` for index lifecycle management
- Query optimizer (`src/query/datalog/optimizer.rs`): `IndexHint` enum, `select_index()`, `plan()` with selectivity-based join reordering
- Join reordering skipped under `wasm` feature flag
- `Cargo.toml` `[features]` section with `default = []` and `wasm = []`
- 6 integration tests in `tests/index_test.rs` for save/reload, bi-temporal, recursive rules regression

### Changed
- `FactStorage` internal structure: `FactData { facts, indexes }` under single `Arc<RwLock<FactData>>` for consistent snapshots
- `PersistentFactStorage::save()` writes index B+tree pages and updates header checksum
- `PersistentFactStorage::load()` performs sync check and fast-path index load
- `executor::execute_query()` now calls `optimizer::plan()` before pattern matching
- File format version bumped 3→4; automatic v1/v2/v3→v4 migration on first save
- `FORMAT_VERSION` constant updated to 4

### Fixed
- NaN values in `Value::Float` now canonicalize to a single bit pattern in index encoding (deterministic sort order)

## [0.5.0] - 2026-03-21

### Added
- Write-ahead log (WAL): fact-level sidecar `<db>.wal` with CRC32-protected binary entries
- `WriteTransaction` API: `begin_write()` / `commit()` / `rollback()` for explicit ACID transactions
- Crash recovery: WAL entries replayed on open; corrupt/partial entries discarded at first bad CRC32
- Checkpoint: `checkpoint()` flushes WAL facts to `.graph` and deletes the WAL; auto-checkpoint on configurable threshold
- `FileHeader` v3: `last_checkpointed_tx_count` field (repurposes unused `edge_count` slot)
- `FactStorage` helpers: `get_all_facts()`, `restore_tx_counter()`, `allocate_tx_count()`
- `OpenOptions` builder: `OpenOptions::new().path("db.graph").open()` or `Minigraf::in_memory()`
- `--file <path>` CLI flag for the REPL binary
- 41 new tests covering WAL, crash recovery, transactions, and checkpoint

### Changed
- `src/minigraf.rs` replaced by `src/db.rs` — `Minigraf`, `OpenOptions`, `WriteTransaction` public API
- File format version bumped 2→3; automatic v1/v2→v3 migration on first checkpoint
- REPL version string now tracks `CARGO_PKG_VERSION` automatically

### Fixed
- WAL-before-apply ordering: facts are now applied to in-memory state only after the WAL entry is fsynced, ensuring crash safety for both implicit (`execute()`) and explicit (`WriteTransaction`) write paths

## [0.4.0] - 2026-03-21

### Added
- Bi-temporal support: every fact now carries transaction time (`tx_id`, `tx_count`)
  and valid time (`valid_from`, `valid_to`)
- `:as-of N` query modifier for transaction time travel (counter or ISO 8601 timestamp)
- `:valid-at "date"` query modifier for valid time point-in-time queries
- `:valid-at :any-valid-time` to disable valid time filtering
- `(transact {:valid-from ... :valid-to ...} [...])` syntax for specifying valid time
- Per-fact valid time override in transact (4-element fact vectors with metadata map)
- File format version 2 with automatic migration from version 1

### Changed
- **Breaking behaviour**: queries without `:valid-at` now return only currently valid
  facts (`valid_from <= now < valid_to`). Existing Phase 3 databases are unaffected
  because all migrated facts have `valid_to = MAX`.
- `FactStorage::transact()` now accepts an optional `TransactOptions` parameter

### Fixed
- `PersistentFactStorage::load()` previously discarded original `tx_id` when loading
  facts from disk, making time-travel queries on persisted databases incorrect

## [0.3.0] - 2026-03-10

### Added
- Datalog core implementation with recursive rules
- Entity-Attribute-Value (EAV) data model
- Pattern matching with variable unification
- Semi-naive evaluation for recursive rules
- Transitive closure support with cycle handling
- Rule registry for rule management
- Persistent storage with postcard serialization
- REPL with multi-line command support and comments
- 123 comprehensive tests (94 unit + 26 integration + 3 doc)

### Changed
- Replaced GQL-inspired syntax with Datalog EDN syntax
- Data model changed from property graph to EAV triples
- Query executor rewritten for Datalog pattern matching

## [0.2.0] - 2026-02-01

### Added
- Persistent storage backend with `.graph` file format
- StorageBackend trait for platform abstraction
- FileBackend implementation (4KB pages, cross-platform)
- MemoryBackend for testing
- PersistentGraphStorage layer for serialization
- Embedded API (`Minigraf::open()`, `Minigraf::execute()`)
- Auto-save on drop

### Changed
- Graph storage now supports persistence

## [0.1.0] - 2026-01-15

### Added
- Initial release
- In-memory property graph implementation
- Basic graph operations (nodes, edges, properties)
- Interactive REPL
- Thread-safe storage with `Arc<RwLock<>>`
