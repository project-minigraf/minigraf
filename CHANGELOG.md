# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## v2.0.0 — 2026-08-26

This was originally slated as v1.3.0, matching the GitHub milestone name under
which most of this work was tracked. It ships as v2.0.0 instead: this
project's CHANGELOG header (above) commits to Semantic Versioning, and under
strict SemVer any backward-incompatible change to the public API past 1.0.0
requires a major version bump, not a minor one. The breaking changes below —
a public method return-type change, two `OpenOptions` struct-literal breaks
plus `#[non_exhaustive]` closing that gap for good, Windows lock-semantics
changes, and an MSRV bump — are exactly that. GitHub milestones v1.4.0,
v1.5.0, v1.6.0, and the pre-existing untitled "2.0" (format v8 + window
completeness + FFI expansion) were renumbered to v2.1.0, v2.2.0, v2.3.0, and
v3.0.0 respectively to keep the sequence consistent.

### Breaking-ish changes

- **Adding `OpenOptions::allow_unlocked` broke struct-literal construction.**
  At the time this field landed, `OpenOptions` had all-public fields and was
  not `#[non_exhaustive]`, so under Rust's semver rules any downstream code
  building it as
  `OpenOptions { wal_checkpoint_threshold: .., page_cache_size: .., .. }`
  without `..Default::default()` no longer compiled. Callers who spread
  `..Default::default()`, or who use the chainable builder methods, were
  unaffected. This branch had to update two such literals in its own tree —
  the `Default` impl itself and one test — and only the latter is a
  "downstream literal" in the sense that matters to a consumer.

  This was the **first post-1.0 field addition** to `OpenOptions`. The
  earlier `max_derived_facts` and `max_results` fields landed in v0.19.0,
  before 1.0, so they set no precedent under the stability guarantee in
  PHILOSOPHY.md §7. Combined with the `synchronous` field below, it's why
  `OpenOptions` is now `#[non_exhaustive]` — see that entry.
- **`OpenOptions` gains a second post-1.0 field: `synchronous`** (#302). Same semver consequence as `allow_unlocked` above — a struct-literal `OpenOptions { .. }` built without `..Default::default()` no longer compiles. One in-tree literal (`src/db.rs`, a test) needed updating; every other in-tree construction already used the spread form or the chainable builder.
- **`OpenOptions` is now `#[non_exhaustive]`.** This is a stronger break than the two field additions above: it disallows *any* struct-literal construction from outside this crate, including the `..Default::default()` spread form that made those additions non-breaking for spread-form callers. Construct via `OpenOptions::new()`/`OpenOptions::default()` plus the chainable builder methods instead. Landed alongside the `allow_unlocked`/`synchronous` fields specifically so this is the last time a new `OpenOptions` field forces a major-version bump. A new `wal_checkpoint_threshold(n)` builder method was added in the same change — the field had none before, which is why every in-tree construction site (tests, benches, one example) used a struct literal for it and had to be converted to the builder form.
- **Minimum supported Rust version is now 1.89** (was effectively 1.85, implied
  by `edition = "2024"`). Required for `std::fs::File::try_lock`, stabilised in
  1.89. Declared as `rust-version` in `Cargo.toml`, so cargo reports a clear
  error rather than a confusing one.
- **On Windows, an open database can no longer be read through any other file
  handle** — including another handle in the same process. `LockFileEx` locks
  are mandatory rather than advisory and exclude every handle but the one
  holding them, so copying or backing up an open `.graph` file now fails on
  Windows where it previously succeeded, and so does opening it yourself for a
  raw read while the database is live. The error is os error 33, "another
  process has locked a portion of the file", even when the other process is
  you. Unix locks remain advisory and are unaffected. Close the database
  before reading the file directly — which also avoids a torn read.
- **`open()` now fails on filesystems that cannot lock at all** rather than
  proceeding unprotected. Set `OpenOptions::allow_unlocked(true)` to accept the
  risk on single-writer deployments.
- **This detection does not cover NFSv3 exports mounted with `-o nolock`**
  (#334). Linux's NFS client serves `flock`/OFD locks for a `nolock` mount out
  of its own local, in-kernel lock table instead of contacting the server, so
  `try_lock` reports an ordinary local success and looks identical to a mount
  where locking genuinely works — `allow_unlocked` is never consulted because
  the code never learns the mount can't really lock. Two separate client hosts
  writing the same `nolock` export can each believe they hold the lock with no
  cross-host coordination. There is no way to detect this from the lock call's
  result, so `nolock` NFSv3 exports are an unsupported deployment for
  multi-writer use — the same category as running mixed Minigraf versions
  against one file, below. See `docs/ERROR_REFERENCE.md` STG-027.
- **A database genuinely locked by another process now reports its error
  later than before** — about 375ms later. `FileBackend::open_with` retries a
  cross-process `WouldBlock` up to 10 times with backoff doubling from 5ms and
  capped at 50ms before concluding another process holds the lock. This exists
  because `fork` transiently duplicates the lock-holding descriptor into a
  child until `execve` closes its close-on-exec descriptors, so any process
  that spawns subprocesses — the normal pattern for the Python and Node
  bindings — could see a spurious `WouldBlock` from its own child. SQLite
  solves the same problem with `busy_timeout`; this is the same idea, bounded
  so a real conflict still fails, just not instantly. A same-process conflict
  (this process already holds the file open) is unaffected: it fails
  immediately, since waiting could never help.
- **A refused open on a filesystem that cannot lock may leave a 0-byte
  `.graph` file behind.** The file must exist before it can be locked, so the
  create-then-lock ordering now creates before it can know the lock will be
  refused. Harmless — the next open sees `is_new` and initialises normally —
  but worth knowing if you inspect the filesystem after a failed open.
- **`Minigraf`'s, `WriteTransaction`'s, and `PreparedQuery`'s public methods now
  return `Result<T, MinigrafError>` instead of `anyhow::Result<T>`** (#277).
  `MinigrafError` implements `std::error::Error` (so `?` still works against
  `anyhow::Result`/`Box<dyn Error>` callers) and adds `.category() -> ErrorCategory`
  and `.code() -> &'static str` for structured error matching. This lands the
  foundation for issue #277; every error currently surfaces as the generic
  `INT-000` code until each error category's follow-up PR wires up its real
  `docs/ERROR_REFERENCE.md` codes.
  Every error's `Display`/`to_string()` output now also carries a `[CODE] `
  prefix (e.g. `[INT-000] unclassified internal error: ...`) that it did not
  have before — a behavior change beyond the type signature. Anything that
  string-matches error output is affected, including the interactive REPL's
  printed error messages and all 7 language-binding repos (Python, Node,
  WASM, Java, Android, Swift, C).
- **`src/query/datalog/parser.rs` now carries real `PRS-0xx` codes instead of
  falling back to `INT-000`** (#358, #277 2/6). All 79 documented `PRS-0xx`
  codes are wired up via `bail_coded!`/`err_coded!` at every parser call site
  that maps onto one; a parse error's `.code()` is now e.g. `"PRS-005"`
  ("Unclosed list") instead of the generic `"INT-000"`, and its
  `Display`/`to_string()` prefix changes to match. `parser.rs`'s internal
  signatures changed from `Result<_, String>` to `anyhow::Result<_>`
  (crate-internal only — the public `db.execute()`/`db.prepare()` surface is
  unchanged, still `Result<T, MinigrafError>`). Two documented codes,
  PRS-028 ("window expression cannot be empty") and PRS-049 ("unexpected end
  of fact vector"), are wired up but unreachable through the public API — the
  `has_over`/`len() >= 4` conditions that guard their call sites already
  guarantee the "bad" case can't occur, so no input reaches them. This is a
  pre-existing property of the parser logic, not something this PR changed.
  Two static message strings changed wording to match their
  `docs/ERROR_REFERENCE.md` "Error text" verbatim (previously drifted):
  `"Retract argument must be a vector"` → `"...must be a vector of facts"`
  (PRS-044). Anything that string-matches parser error output — including
  the interactive REPL and all 7 language-binding repos — is affected by
  both the code prefix and this wording change.
- **Storage/file-format errors now surface their real `STG-0xx` code** (#359,
  part of #277's rollout): all 27 documented `STG-0xx` codes are wired up
  across `src/storage/{mod,persistent_facts,btree,btree_v6,packed_pages}.rs`
  and `src/storage/backend/file.rs` — header validation (`STG-001`–`STG-009`),
  header re-read failures (`STG-010`), on-disk B+tree/packed-page corruption
  (`STG-011`–`STG-015`), a poisoned backend mutex (`STG-016`), page-count
  arithmetic overflow guards (`STG-017`–`STG-024`), and the three lock
  conflict errors (`STG-025`–`STG-027`). `docs/ERROR_REFERENCE.md`'s STG
  section's "Error text" lines were rewritten from one-off example values to
  canonical `{}`-placeholder templates so they byte-for-byte match the
  `REGISTRY` entries the sync test checks against.
  `FileBackend::open_with` no longer collapses a more specific header-parse
  failure (e.g. a bad magic number) into the generic `STG-010` — it now
  propagates a `CodedError` already present in the chain as-is, only falling
  back to `STG-010` for a genuinely uncoded (e.g. raw I/O) failure.
  Not covered by a dedicated regression test: `STG-011`'s call site is
  guarded by a `debug_assert!` that always fires first in a debug/test
  build, so it's release-build-only and unreachable under `cargo test`; the
  8 arithmetic-overflow guards (`STG-017`–`STG-024`) and `STG-027` (a
  filesystem that refuses locking outright — no CI runner provides one) are
  covered by the registry↔doc sync test and code review but have no
  dedicated trigger test, consistent with their own "should not occur under
  normal operation" documentation. Two `bail!` sites with no matching
  documented `STG` code ("Header checksum mismatch", reached on a genuine
  on-disk corruption of an otherwise-valid header, and `into_backend`'s
  multiple-owners precondition) were deliberately left uncoded (`INT-000`)
  rather than assigned an undocumented code — left for the final API+INT
  audit PR.
- **`src/wal.rs`'s errors now carry real `WAL-0xx` codes instead of falling
  back to `INT-000`** (#360, toward #277): a bad `.wal` magic number is
  `WAL-001`, an unsupported WAL version is `WAL-002`, a fact whose serialised
  size exceeds the WAL's per-entry limit (~4080 bytes) is `WAL-003`, a fact
  whose serialised size exceeds `u32::MAX` is `WAL-004` (practically
  unreachable — WAL-003's limit fires first), a WAL header's `num_facts`
  exceeding the platform `usize` is `WAL-005` (unreachable on any 64-bit
  target), and a failure to delete the sidecar `.wal` file after a successful
  checkpoint is `WAL-006`. `docs/ERROR_REFERENCE.md`'s WAL-002/003/004/006
  "Error text" entries were rewritten from a concrete example value to the
  canonical `{}`-placeholder template form so the registry↔doc sync test can
  compare them byte-for-byte; the concrete example now lives in each entry's
  prose instead.
- **Query executor errors now carry real `QRY-00N` codes** (#361, part of
  #277) instead of the generic `INT-000` fallback: `QRY-001` invalid entity,
  `QRY-002` attribute must be a keyword, `QRY-003` cannot transact a
  pseudo-attribute, `QRY-004` invalid value, `QRY-005` transaction failed,
  `QRY-006` retraction failed, `QRY-007` unknown predicate, `QRY-008`
  functions lock poisoned, `QRY-009` rules lock poisoned — the full QRY
  category (9 of 9 documented codes; see `docs/ERROR_REFERENCE.md`). Anything
  that was matching on `MinigrafError::code() == "INT-000"` for one of these
  conditions now sees the specific code instead. `QRY-001`/`004`/`005`/`006`'s
  `ERROR_REFERENCE.md` "Error text" entries were also rewritten from a
  concrete example (e.g. `` `Invalid entity: "not-a-uuid"` ``) to the
  canonical `{}`-placeholder template form (`` `Invalid entity: {}` ``) they
  share with `REGISTRY`, per the design spec's migration convention; the
  worked example moved to each entry's "Example" section, unchanged.
  `QRY-001`..`006` (the transact/retract validation codes) are wired into
  `DatalogExecutor::execute_transact`/`execute_retract`, but that code path
  is not reachable through `Minigraf::execute()` today — the public write
  path routes `transact`/`retract` through `Minigraf::materialize_transaction`/
  `materialize_retraction` (`src/db.rs`) instead, which raise their own
  uncoded (still `INT-000`) errors, two of which — "attribute must be a
  keyword" and "cannot transact a pseudo-attribute" — are already earmarked
  as `API-003`/`API-004` for the API+INT category PR (#277 step 6).
- **The core binary size budget is raised from 1MB to ~1.2MB** (`binary-size.yml`'s
  `SIZE_LIMIT_BYTES`, and PHILOSOPHY.md §4's stated target), toward #277.
  The project was already at the 1MB wire before #277 started (measured
  ~1034 KiB pre-rollout on a controlled local build); #360 (WAL, 6 codes) and
  #361 (QRY, 9 codes) together added only ~9 KiB, and even that was enough to
  fail CI. All existing size levers — `opt-level = "z"`, `lto = true`,
  `codegen-units = 1`, `strip = "symbols"`, and the non-generic
  `bail_coded!`/`err_coded!`/`format_template` design (no per-call-site
  monomorphization, unlike the generic `build_btree` fixed for the same
  reason in v0.x) — were already in place; `cargo bloat` shows no single
  dominant symbol to cut, just diffuse growth across ~2000 small functions.
  Extrapolating to the full 130-code rollout (PRS: 79, STG: 27, WAL: 6,
  QRY: 9, API+INT: 9) puts the total growth at roughly 60-70 KiB over the
  pre-#277 baseline, which no further micro-optimization of already-maxed
  settings can absorb. ~1.2MB keeps meaningful headroom above that projection
  without abandoning the philosophy's small-binary principle.
- **Every remaining `bail!`/`anyhow!` call site in the crate now carries a
  structured code — nothing falls through to `INT-000` by default anymore**
  (#362, #277 step 6/6, the final sub-PR of the umbrella issue). Two parts:

  **Part 1 — `src/db.rs`'s API-layer call sites now carry their real
  `API-0xx` codes**: `API-001` write lock poisoned (`execute`/`begin_write`/
  `checkpoint`), `API-002` unexpected command variant in the write path
  (structurally unreachable — kept for defensive completeness, no test
  possible), `API-003`/`API-004` attribute-must-be-keyword /
  cannot-transact-a-pseudo-attribute at the `materialize_transaction`/
  `materialize_retraction` layer (mirroring `QRY-002`/`QRY-003`), `API-005`/
  `API-006`/`API-007` `db.prepare()` rejecting `transact`/`retract`/`rule`,
  `API-008` function registry lock poisoned (`register_aggregate`/
  `register_predicate`), `API-009` WAL not initialized (also structurally
  unreachable — `wal_write_stamped_batch` always populates `wal` two lines
  above the `ok_or_else`). Regression tests assert the exact code for every
  reachable one, including two new lock-poisoning tests that deliberately
  panic while holding `write_lock`/`functions` (via `catch_unwind`) to
  exercise `API-001`/`API-008` for real rather than by inspection.
  `db.rs`'s own "invalid entity"/"invalid value" checks in
  `materialize_transaction`/`materialize_retraction` have no `API-0xx`
  counterpart in `docs/ERROR_REFERENCE.md` (only `QRY-001`/`QRY-004` do, and
  those cover a different, unreachable-from-`db.rs` call path — see the QRY
  entry above) and a `WriteTransaction`-already-active reentrancy check has
  no documented code at all, so all three became new `INT-0xx` codes
  (`INT-001`, `INT-002`, `INT-003`) — part of the audit below.

  **Part 2 — every other uncoded `bail!`/`anyhow!` site crate-wide
  (~250 of them, across 18 files: `temporal.rs`, `parser.rs`'s long-tail
  internal-error/syntax branches the PRS migration left uncoded,
  `evaluator.rs`, `executor.rs`'s remaining sites, `prepared.rs`,
  `functions.rs`, `rules.rs`, `stratification.rs`, `graph/storage.rs`,
  `storage/mod.rs`, `storage/persistent_facts.rs`, `storage/packed_pages.rs`,
  `storage/btree.rs`, `storage/btree_v6.rs`,
  `storage/backend/{file,memory}.rs`, `storage/cache.rs`,
  `browser/buffer.rs`) — assigned one of 52 new `INT-0xx` codes**
  (`INT-004`–`INT-055`; `docs/ERROR_REFERENCE.md`'s old "Appendix: Internal
  Errors" bullet list is gone, replaced by `INT-006`'s formal entry, which
  covers the same 8 "internal parser error: expected X token" strings via a
  `{}` placeholder). Mechanically-repeated call sites share one
  parameterised code rather than getting one each — mirroring the pattern
  `QRY-005`/`STG-016`/`WAL-006` already established for "wrap an arbitrary
  underlying cause" and "one poisoned-lock code per resource": e.g. `INT-011`
  covers all ten "unexpected end of `<clause>`" parser messages, `INT-018`
  covers all four "variable not bound by any outer/earlier clause" messages,
  `INT-050` covers all thirteen lock-poisoned sites across four files
  (`{}` names the specific lock), and `INT-048`/`INT-049` are broad
  "storage: arithmetic overflow"/"storage: internal invariant violation"
  buckets covering dozens of B+tree/packed-page bounds-check and
  overflow-guard sites that were never meaningfully distinct from one
  another. A handful of genuinely user-reachable conditions that
  the original five category PRs simply never got to — the recursion/
  derived-facts/result-limit family in `evaluator.rs` (`INT-020`) and the
  runtime aggregate type-mismatch family in `functions.rs` (`INT-041`–
  `INT-044`) among them — are `INT-0xx` rather than `QRY-0xx` purely because
  this PR's scope is "assign every remaining site an `INT` code," not
  "reopen the closed PRS/QRY/STG/WAL category PRs to extend their code
  ranges"; two existing regression tests in `tests/error_codes_qry_test.rs`
  that had locked in `INT-000` for exactly these two families before this PR
  (`recursive_rule_derived_facts_limit_returns_int000`,
  `aggregate_type_mismatch_returns_int000`) were updated in place to assert
  their new codes instead, and two similarly-named unit tests in
  `src/query/datalog/evaluator.rs` were fixed the same way.
- **The `REGISTRY`↔`docs/ERROR_REFERENCE.md` sync test is now a full
  bidirectional equality check** (#362, #277 step 6/6), replacing the
  subset-only check the foundation PR (#357) introduced. Renamed
  `registry_is_a_subset_of_error_reference_doc` →
  `registry_matches_error_reference_doc_bidirectionally`: it now also
  asserts every documented `### CODE` section with an "Error text" line has
  a matching `REGISTRY` row, not just the reverse. This is only safe now
  that Part 2 above means no documented code can be missing a registry
  entry because its call site hasn't been migrated yet — the last such gap
  is closed. The final tally: 122 pre-#362 `ErrorCode` variants (`PRS`×79,
  `QRY`×9, `STG`×27, `WAL`×6, `INT-000`×1) plus 9 new `API-0xx` and 55 new
  `INT-0xx` (`INT-001`–`INT-055`) = 186 total.

### Bug fixes

- **A `.graph` file is no longer bricked when its holder dies in a container**
  (#317): `FileLock` recorded the holder's PID in a `.graph.lock` sidecar and
  refused to open when that PID equalled our own. Every container's main process
  is PID 1 in its own namespace, so a container killed with SIGKILL left the
  sidecar holding `1`, and the replacement container — also PID 1 — read its own
  PID back and refused. The refusal was permanent; no code path could clear it,
  and an operator had to delete the file by hand. The `pid == our_pid` refusal
  was itself the fix for #304, so the two defects were entangled.

  A PID cannot serve as a liveness token: it is not unique across PID namespaces
  and is recycled within one. The sidecar is gone, and the lock now lives in the
  kernel via `std::fs::File::try_lock` on the `.graph` file itself. The kernel
  releases it whenever the process exits, however it exits, so a crashed holder
  leaves nothing behind. Because `flock` and OFD locks attach to the open file
  description rather than the process, a second open within one process is also
  denied, so #304 stays fixed with no PID bookkeeping at all.

  A leftover `.graph.lock` from a version before 1.3 is ignored and never
  deleted — it no longer has any effect, and a still-running old process may
  still depend on it. Running mixed versions against one file is not
  supported; upgrade all writers together.

  Reported and diagnosed by [@ocasazza](https://github.com/ocasazza) in #317,
  who supplied the reproduction, identified the PID-namespace mechanism, and
  correctly traced the refusal back to the #304 fix that caused it. That
  analysis is what made this straightforward to verify. One of the tests from
  their proposed patch is the ancestor of the regression test that now covers
  this.

- **Keywords may contain `?`**: `:person/alive?` now lexes as a single keyword. Previously `?` terminated the keyword, so `[:e :alive? true]` lexed as `:alive` followed by a stray `?` symbol and was rejected as a four-element fact — reporting `Optional 4th element of a fact must be a map`, which named the value rather than the keyword. `?` is a constituent character in EDN keywords and predicate-style names (`:artist/dead?`) are idiomatic. Query variables are unaffected: a `?` not preceded by `:` still begins a symbol. `tests/grammar/grammar.pest` updated to match.

- **The parser no longer silently discards trailing input after a complete form** (#305). `parse_edn` returned successfully as soon as it parsed one complete top-level value, without checking whether any tokens remained — so `[1 2 3] garbage ) ] } 12345` parsed as `[1 2 3]` with everything after it dropped, no error, no indication anything was ignored. `parse_edn` now errors if tokens remain once the top-level value is parsed. Separately, `(query [...])` and `(rule [...])` accepted and silently ignored any arguments after their vector, the same defect `(transact ...)`/`(retract ...)` already closed for #303 — so `(query [:find ...] :max-results 5)` parsed as the query alone, discarding `:max-results 5` (the correct place for that clause is *inside* the vector: `[:find ... :max-results 5]`) rather than rejecting the malformed call. Both now reject extra trailing arguments with the same "unexpected trailing argument(s)" wording `transact`/`retract` already use.

### Performance

- **`BrowserDb` no longer allocates a redundant LRU page cache** (#275): `open_in_memory()`, `open()`, and `import_graph()` now pass page cache capacity `0` to `PersistentFactStorage::new` instead of `256`. The backing `BrowserBufferBackend` is already a `HashMap`-resident, fully-loaded page store, so every `PageCache` hit was a second RAM-to-RAM copy plus LRU bookkeeping for no benefit.

  This required fixing `PageCache` itself: capacity `0` previously meant *unbounded* (the eviction loop's `capacity > 0` guard skipped eviction but insertion still ran unconditionally), which would have made this change strictly worse — an ever-growing duplicate of every page ever read, never reclaimed. `PageCache::new(0)` now means the cache is disabled outright: `get_or_load` always reads through to the backend and `put_dirty` is a no-op. No existing caller passed `0` before this change, so no other behaviour is affected.

- **`not`/`not-join` clauses are pushed to the earliest position where their variables are bound** (#248): `optimizer::plan()` now accepts `Not`/`NotJoin` alongside `Pattern`/`Expr` and interleaves each one right after the pattern that completes its required variables, instead of always applying it as a global post-filter after every pattern (and after `or`/`or-join`). This shrinks the intermediate binding set flowing into any patterns that follow it in the plan. A clause whose variables are only ever bound by an `or`/`or-join` (not by any pattern in the same clause list) is deferred to the post-filter exactly as before — `or`/`or-join` isn't visible to the planner, so pushing it early would run the clause before its dependency exists. Benchmark: `negation/not_pushdown_selectivity` (10k facts, three joins after the `not`) shows the effect scaling with the excluded fraction, from ~2% at 0% excluded up to ~55% faster at 100% excluded.

- **`or`/`or-join` branches can now short-circuit** (#250): branches are still sorted cheapest-first (landed earlier), but once the branches evaluated so far already produce a match for every incoming binding, remaining (costlier) branches are skipped instead of always being evaluated. This is always safe for `or-join` — branches are projected down to already-bound outer variables before merging, so a skipped branch could only have produced a row identical to one already kept. For plain `or`, it's only safe when the branches evaluated so far introduce no variable beyond what's already bound in the incoming scope (a pure filter) — a branch that binds a genuinely new variable can legitimately contribute a distinct row for an already-"covered" key, so that case still evaluates every branch, exactly as before. Benchmark: `disjunction/or_short_circuit` and `disjunction/or_join_short_circuit` (10k facts, both branches fully covering) show ~11% and ~27% improvement respectively.

- **WAL writes no longer pay a full `fsync()`, and can skip the per-write flush entirely** (#302): `WalWriter::append_entry` called `File::sync_all()` (fsync) after every write, flushing inode metadata the WAL — a pure-append file — never needed alongside the data. It now calls `File::sync_data()` (fdatasync) instead: same durability guarantee, one fewer metadata flush, for every existing caller, no other change required. A new `OpenOptions::synchronous` field (`SyncMode::Full`, the default, matches this fdatasync behavior; `SyncMode::Normal` skips the per-write flush altogether) lets bulk loaders/migrations that can safely re-run from a checkpoint watermark trade per-write durability for throughput — `checkpoint()` still fsyncs the main file unconditionally in both modes, so it remains the hard durability boundary regardless of `synchronous`. Separately: `begin_write()` + N `execute()` + one `commit()` already collapsed N facts into a single WAL write / single fsync before this change; that existing batching pattern is now documented (README, wiki Performance Tuning page) since it was easy to miss.

## v1.2.3 — 2026-08-10

No changes to the core crate. Released so the language bindings have a version
to republish on, carrying a fix that lives in a binding rather than here.

v1.2.2 made a second handle on an already-open file an error (#304). The
UniFFI bindings and the C API already had a way to release a handle on demand
(`destroy()` and `minigraf_close` respectively), but the Node binding did not,
and JavaScript has no deterministic destructor — so reopening a `.graph` file
in one process became impossible there. `minigraf-node` gained a `close()`
method (project-minigraf/minigraf-node#1); this release is what lets it ship.

Binding versions track the core version, so all seven bindings republish at
1.2.3.

## v1.2.2 — 2026-08-10

Drop-in replacement for v1.2.1. No file-format changes, no public API changes.

### Bug fixes

- **Two handles on one file in the same process** (#304): `FileLock::acquire` treated a lock file whose holder PID equalled our own as stale, deleted it, and opened anyway — on the theory that it could only be a leaked handle. The far more likely cause is a handle that is open and in use right now elsewhere in the process, so the "self-heal" silently produced two live `FileBackend`s on one file. Each caches its own `header.page_count`, allocates new pages from that count, and bounds-checks `read_page` against it, so the two page tables diverge. That — not an allocation off-by-one — is the source of the intermittent `Page N out of bounds (total pages: M)`. It also corrupted the lock itself: whichever handle dropped first ran `FileLock::drop` and removed the lock file that by then belonged to the survivor, leaving a live handle unlocked and admitting other processes too. A lock held by our own PID is now refused with an error naming the same-process case; a lock held by a *different*, dead process is still reclaimed as before. **Behaviour change:** callers that relied on reopening a path they already hold open now get an error instead of silent corruption, and should reuse the existing handle (`Minigraf` is cheap to clone and all clones share one database).

- **String comparison in predicates**: `[(< ?a ?b)]`, `>`, `<=` and `>=` now order two strings lexicographically instead of failing. Previously every ordering predicate routed through `to_float_pair`, which errors on non-numeric operands, so a string comparison silently matched no rows — and because both directions returned nothing, a range filter dropped the entire relation rather than partitioning it. This most often bit ISO-8601 timestamps stored as strings. The predicate path now agrees with `value_lt` / `value_cmp`, which `min`, `max` and `:order-by` already use. Mixed-type comparison (string vs number) remains an error, and NaN comparisons remain false.

## v1.2.1 — 2026-06-28

Drop-in replacement for v1.2.0. No file-format changes, no public API changes.

### Bug fixes

- **Magic sets `fb` adornment**: queries with a literal keyword in value position (e.g. `(reports-to ?emp :alice)`) now return correct results instead of empty results (#298). Root cause: seed fact used an ephemeral random UUID as entity; the 1-arg guard bound the variable to that UUID rather than the keyword value. Fixed with a deterministic sentinel entity (`Uuid::new_v5`) and a 2-arg guard that binds from value position.
- **Magic sets recursive rules**: `(reach :a ?y)` with intermediate variable `?mid` and keyword entity binding now evaluates correctly (#297).

## v1.2.0 — 2026-06-26

Drop-in replacement for v1.1.x. No file-format changes, no public API breaking changes. Upgrading requires no code changes.

### Features

- **Magic Sets rewriting** — recursive Datalog queries with bound arguments are now automatically rewritten top-down via magic sets, propagating bound values into recursive rules to avoid full-relation scans (#289)
  - Adornment classification, seed fact generation, magic guard injection, SCC-aware propagation rule generation, full `rewrite()` wired into `execute_query_with_rules`
  - Limitation: mutual recursion through negation is not rewritten (documented in ROADMAP §9.6)
- **Per-query complexity limits** — `:max-derived-facts` and `:max-results` clauses added to `query`; global default raised to 1,000,000 derived facts (#288, #290)

### Bug fixes

- `selective_fact_fetch`: include `asserted` flag in dedup key — previously retracted facts could shadow live facts under certain access patterns (#285, #286)
- `selective_fact_fetch`: restore per-pattern entity priority lost in a prior refactor (#283)
- v5→v6 migration: fix hang caused by using `header.page_count` as the B-tree start page instead of the correct offset (#272)

### Performance

- Eliminate backend mutex hold on cache hits in `CommittedFactLoaderImpl::resolve` — read path no longer acquires the write lock when the page is cached (#279)
- Pre-build `MutexStorageBackend` in `CommittedFactLoaderImpl` to eliminate one `Arc::clone` per `resolve()` call (#280, #281)

### Infrastructure

- Split Java, Android, Swift, and C bindings into independent repos under `project-minigraf` org — completes the #231 follow-up work.
  - `minigraf-java`: https://github.com/project-minigraf/minigraf-java
  - `minigraf-android`: https://github.com/project-minigraf/minigraf-android
  - `minigraf-swift`: https://github.com/project-minigraf/minigraf-swift
  - `minigraf-c`: https://github.com/project-minigraf/minigraf-c
- `minigraf-ffi` yanked from crates.io — UniFFI layer inlined into each binding repo's private shim; use the per-language packages instead
- `cascade.yml` updated to dispatch `core-release` to all 7 binding repos on every version tag
- All binding repos migrated to OIDC trusted publishing — `NPM_TOKEN` and `CARGO_REGISTRY_TOKEN` secrets removed; crates.io uses `rust-lang/crates-io-auth-action@v1`, npm uses `setup-node@v6` OIDC
- Python, Node, and WASM binding repos now include a `prepare` job that pins the `minigraf` dependency version in `Cargo.toml` and commits before building — consistent with Java, Android, Swift, and C

### Documentation

- Add `docs/ERROR_REFERENCE.md`: full inventory of user-facing errors (PRS/QRY/STG/WAL/API categories, 113 entries) with cause, resolution steps, and bad-input examples; docs-only reference codes PRS-001…API-009 (#192)

## v1.1.1 — 2026-05-17

Patch release. Fixes cargo-dist Windows build failure that prevented REPL binaries and crates.io publish from completing for v1.1.0. No code changes to the library itself.

### Build

- Exclude `fuzz` crate from workspace so cargo-dist can build on Windows (MSVC linker incompatible with `#![no_main]` libFuzzer targets) — fixes #263

## v1.1.0 — 2026-05-17

Drop-in replacement for v1.0.0. No file-format changes, no public API changes, no query surface changes. Upgrading requires no code changes.

### Performance

- Hash-join replaces nested-loop join for multi-clause queries — O(N) instead of O(N²) for large fact sets (#202, #203, #204)
- Selective B+Tree fact fetch: queries with bound entity/attribute skip full-scan and read only relevant index pages (#208)
- Predicate push-down into join ordering (#207)
- Cost-based clause ordering for `not`/`or` rules (#206, #205)
- SIMD crossover analysis and benchmarking infrastructure (#229)

### Bug fixes

- Fixed critical correctness and durability bugs found during deep audit (#225): fact visibility edge cases, WAL entry ordering, checkpoint atomicity
- Fixed read-only handle `Drop` triggering unnecessary checkpoint, modifying the file on close (#226)
- `QueryResult::Transacted` and `Retracted` reverted to tuple variants — struct-variant form introduced post-1.0 was a breaking change (#261)
- Added `Minigraf::current_tx_count() -> u64` as additive API to expose the `:as-of` monotonic counter without breaking existing pattern matches

### Reliability & testing

- WAL fault injection harness (`FaultInjectingBackend`) with 9 crash-recovery tests (#209, #210, #214)
- Storage/migration resilience: migration matrix, index corruption recovery, concurrency stress tests (#215, #216, #217)
- Property-based query correctness tests against a reference evaluator (proptest, #212)
- Datalog parser/evaluator fuzz targets with seed corpus (#213)
- Per-module branch coverage gates in CI (#219)
- Long-haul smoke suite: 500 entities × 10 write/read/checkpoint cycles (#220)
- XTDB and Datomic semantic compatibility tests (#221)

### Internal

- Workspace-wide clippy lint enforcement — 336 violations fixed (#232)
- Grammar conformance test harness: pest shadow grammar + EDN corpus (#233)
- CI: codecov-action v3→v5, actions-rs→dtolnay migration

## Reliability Hardening — 2026-05-17

### Summary

Six PRs hardening the v1.0.0 codebase: WAL fault injection, storage/migration resilience, query correctness (property-based + coverage gates), long-haul smoke testing, and XTDB/Datomic semantic compatibility. No API changes, no file-format changes, no new runtime dependencies.

**PR #254 — #209 + #210 + #214: WAL Fault Injection**

- `FaultInjectingBackend` (`#[cfg(test)]`-only wrapper around `StorageBackend`) — configurable write-fail, flush-fail, read-fault injection
- 9 new WAL fault tests: write-fail propagation, flush-fail without data corruption, read-fault on WAL replay, CRC corruption discard, checkpoint atomicity under backend failure, partial checkpoint recovery via WAL replay, multi-writer serialisation, concurrent write+checkpoint, backend error propagation as `Err` not panic

**PR #257 — #215 + #216 + #217: Storage & Migration Resilience**

- `tests/migration_matrix_test.rs` — 5 migration tests: v7 round-trip, v3 empty migrate, corrupt magic returns `Err`, unsupported version returns `Err`, WAL replay idempotent
- `tests/index_corruption_test.rs` — 5 corruption-resilience tests: checksum mismatch triggers index rebuild, btree leaf/internal corrupt pages return `Err` without panic, root pointer mismatch handled, non-critical corruption still serves queries on good data
- 5 new concurrency stress tests in `tests/concurrency_test.rs`: stress readers during writer, failed write then success, rollback after partial work, open/write/checkpoint/query loop per thread, nightly stress loop (`#[ignore]`)

**PR #256 — #212 + #213 + #219: Property-Based Testing & Coverage Gates**

- `tests/property_test.rs` (proptest, `cfg(not(wasm32))`): 3 property tests — EAV model correctness vs naive reference evaluator, bi-temporal monotonicity, retract visibility invariant
- `.github/workflows/coverage-gates.yml` — per-module branch coverage thresholds; CI fails if coverage drops below gate

**PR #258 — #220: Long-Haul Smoke Suite**

- `tests/smoke_test.rs` (`#[ignore]` nightly): `smoke_large_graph_10_cycles` — 500 entities × 10 attributes × 10 update cycles; 7 invariants: active count (333), retracted count, fact count bounds, temporal snapshot integrity, prepared query consistency, recursive rule transitive closure, WAL checkpoint round-trip
- `.github/workflows/smoke.yml` — nightly 5am UTC, 15-min timeout, runs `--include-ignored`

**PR #259 — #221: XTDB & Datomic Compatibility Corpus**

- `tests/xtdb_compat_test.rs` — 10 semantic ports (Apache 2.0): EAV, tx-time `:as-of`, valid-time `:valid-at`, retraction (current + historical), Datalog join, negation, recursive rules, parameterised queries, combined bi-temporal
- `tests/datomic_compat_test.rs` — 9 independently written semantic ports: datom model, multi-entity attribute, tx-time `:as-of`, retract-entity, multi-variable `:find`, ground-value binding, parameterised query (prepared), named reusable rules, predicate expression filter

### Tests

935 tests passing (943 total, 8 ignored: 6 or+neg-cycle stratification doc tests, 1 nightly concurrency stress, 1 nightly smoke).

---

## Optimizer & Benchmarks — 2026-05-16

### Summary

Three optimisation PRs extending the v1.0.0 performance work. No API changes, no file-format changes, no new runtime dependencies. Test count unchanged at 850.

**PR #249 — #207 + #206: Predicate Push-Down & Mixed Rule Optimization**

- `optimizer::plan()` extended to accept `Expr` (predicate/arithmetic) clauses; they are interleaved at the earliest position where all their variables are bound, minimising intermediate binding sets
- `StratifiedEvaluator` gains mixed-rule path: when a rule stratum contains both positive-only rules and rules with `not`/`not-join`, the evaluator now evaluates all positive rules first and applies negation filters in a second pass within the same stratum — eliminates a class of ordering-dependent bugs in cross-stratum negation

**PR #251 — #205: Cost-Based `not`/`or` Ordering**

- `optimizer::plan()` assigns selectivity estimates to `not`/`not-join` and `or`/`or-join` clauses; they are sorted after their ground-variable producers but before unconstrained pattern scans
- `not`/`not-join` blocks placed after the patterns that produce their join variables (was: end of plan regardless); `or`/`or-join` blocks placed by estimated output cardinality

**PR #253 — #229: SIMD Benchmarking & Crossover Analysis**

- `benches/simd_helpers.rs`: `valid_time_filter_simd`, `as_of_filter_simd`, `sum_simd_i64` — portable SIMD kernels using `wide::i64x4` / `u64x4`
- Criterion benchmark groups: `simd_temporal`, `simd_as_of`, `simd_aggregate` — scalar vs. SIMD crossover analysis at 1K–1M facts
- Analysis result: SIMD crossover occurs at ~8K–16K facts per query; scalar path preferred below that threshold; SIMD integration deferred to post-1.0 backlog pending real-workload profiling

### Tests

850 tests passing (844 passing + 6 ignored: confirmed `or`+neg-cycle stratification bug, deferred to post-1.0 backlog). All optimizer PRs are optimisations; no new integration tests added.

---

## Performance — 2026-05-15

### Summary

Four O(N²) query-engine bottlenecks eliminated. No API changes, no file-format changes, no new dependencies.

**PR #246 — #208: B+Tree Selective Lookup**

- `get_facts_by_entity`, `get_facts_by_attribute`, `get_facts_by_entity_attribute` promoted from `#[cfg(test)]` to production in `src/graph/storage.rs`
- New `selective_fact_fetch` helper in `executor.rs`: inspects query patterns for bound entity literals and bound attribute strings; calls index-driven fetches instead of `get_all_facts()` when ≤4 distinct lookups detected
- `as_of` queries continue to use full scan (required for correctness)
- New benchmark groups: `btree_lookup/entity_point`, `btree_lookup/attribute_scan`

**PR #247 — #202 + #203 + #204: Hash-Join Cluster**

- **#202** (`executor.rs`): `not`/`not-join` bodies pre-computed once into `HashSet<Vec<(String, Value)>>` keyed on join variables; O(1) probe per outer binding replaces O(N) re-scan. `normalize_value` handles `keyword→Ref` representation asymmetry in value position.
- **#203** (`executor.rs`): `or`/`or-join` branches now evaluated from a single empty seed; branch results hash-joined back onto incoming bindings on shared user-visible variables (`__`-prefixed metadata keys excluded). `or-join` projects to `join_vars` before joining.
- **#204** (`matcher.rs`): `join_with_pattern` detects join variable (entity position first, value position second), builds `HashMap<Value, Vec<Bindings>>` once, probes per existing binding O(1). `normalize_join_value` handles `keyword→Ref`. Falls back to nested-loop when no join variable found.

### Tests

850 tests passing (844 passing + 6 ignored: confirmed `or`+neg-cycle stratification bug, deferred to post-1.0 backlog).

---

## v1.0.0 — Cross-Platform Release (2026-05-01)

### Milestone

This is the **v1.0.0 release**. The public Rust API and the `.graph` file format are now stable
and committed to semantic versioning. File format stability is guaranteed from this release.

### Cross-platform summary

All cross-platform targets have shipped:

- **8.1a** — Browser WASM (`BrowserDb`, `IndexedDbBackend`, `@minigraf/browser` on npm) — v0.20.0
- **8.1b** — Server-side WASM (`wasm32-wasip1` / WASI, Wasmtime/Wasmer CI) — v0.20.0
- **8.2** — Mobile bindings (Android `.aar` on GitHub Packages, iOS `.xcframework` via SPM, UniFFI) — v0.21.0
- **8.3a** — Python (`minigraf` on PyPI, pre-built wheels) — v0.22.0
- **8.3b** — Java/JVM (`io.github.adityamukho:minigraf-jvm` on Maven Central, fat JAR) — v0.23.0
- **8.3c** — C FFI (`minigraf.h` + platform tarballs on GitHub Releases) — v0.24.0
- **8.3d** — Node.js (`minigraf` on npm, pre-built `.node` binaries) — v0.25.0

### Also in this release

- `pkg/` renamed to `minigraf-wasm/`, `swift/` renamed to `minigraf-swift/` — consistent
  top-level naming across all workspace packages (issue #179)
- `@minigraf/browser` now published to npm on every tagged release (issue #179)
- `@minigraf/wasi` published to npm on every tagged release (issue #178) — WASI binary packaged for Node.js WASI consumers
- Per-platform READMEs added: `minigraf-wasm/`, `minigraf-node/`, `minigraf-ffi/python/`,
  `minigraf-c/`, `minigraf-ffi/java/`

### Tests

795 tests passing (788 passing + 7 ignored: confirmed `or`+neg-cycle stratification bug,
deferred to post-1.0 backlog).

## v0.25.0 — 2026-04-26

### Added
- **Node.js bindings** published to npm as `minigraf`.
  Install with `npm install minigraf`. No build step required — prebuilt
  `.node` binaries for Linux x86_64/aarch64, macOS universal2, Windows x86_64.
  API: `new MiniGrafDb(path)`, `MiniGrafDb.inMemory()`, `.execute(datalog)`,
  `.checkpoint()`. Full TypeScript definitions included.

## v0.24.0 — C Bindings (2026-04-26)

### Added
- **C bindings** distributed as GitHub Releases tarballs.
  Download `minigraf-c-v0.24.0-<platform>.tar.gz` (Linux/macOS) or `.zip` (Windows)
  from the release page. Each archive contains the prebuilt shared library plus
  `minigraf.h`. API: `minigraf_open`, `minigraf_open_in_memory`, `minigraf_execute`,
  `minigraf_string_free`, `minigraf_checkpoint`, `minigraf_close`, `minigraf_last_error`.
  Memory contract mirrors SQLite: `minigraf_execute` returns a heap-allocated JSON string;
  call `minigraf_string_free` to release it.
- `minigraf-c/`: new workspace crate (`cdylib` + `staticlib`) — `Cargo.toml`, `src/lib.rs`
- `minigraf-c/cbindgen.toml`: cbindgen 0.29.2 configuration
- `minigraf-c/include/minigraf.h`: committed stable header (cbindgen-generated)
- `.github/workflows/c-ci.yml`: PR test matrix on 4 platforms + header drift check
- `.github/workflows/c-release.yml`: release workflow — builds + packages platform tarballs,
  uploads to GitHub Releases

795 tests.

## v0.23.0 — Java Desktop JVM Bindings (2026-04-25)

### Added
- **Java desktop JVM bindings** published to Maven Central as
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

## v0.22.0 — Python Bindings (2026-04-25)

### Added
- **Python bindings** published to PyPI as `minigraf`.
  Install with `pip install minigraf`. API: `MiniGrafDb.open(path)`,
  `MiniGrafDb.open_in_memory()`, `.execute(datalog)`, `.checkpoint()`.
  Pre-built wheels for Linux x86_64/aarch64, macOS universal2, Windows x86_64.

## v0.21.1 — Patch: mobile/WASM docs (2026-04-19)

### Changed
- `src/lib.rs`: added **Feature Flags** section and **WebAssembly targets** subsection to crate-level docs — browser feature, `wasm32-unknown-unknown` target switcher note, and WASI build command
- `README.md`: updated "For Mobile Apps" section — replaced the cross-platform placeholder with current state, added Kotlin/Swift quick-start snippets and link to wiki integration guide
- Wiki `Use-Cases.md`: replaced Integration placeholder with full Android (Gradle setup, Kotlin API, error handling, threading) and iOS (SPM setup, Swift API, error handling, async) integration guides

795 tests.

## v0.21.0 — Android/iOS Mobile Bindings (2026-04-19)

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

## v0.20.0 — WebAssembly Support (2026-04-18)

### Added
- **Browser WASM** (`wasm32-unknown-unknown` + `wasm-bindgen`):
  - `BrowserDb` public API: `open_in_memory()`, `execute()`, `checkpoint()`, `export_graph()`, `import_graph()`
  - `BrowserBufferBackend` — in-memory `StorageBackend` over a flat page buffer, identical byte layout to the native `.graph` format
  - `IndexedDbBackend` — page-granular IndexedDB storage (one 4 KB entry per page); only dirty pages written on checkpoint
  - `wasm-pack` build workflow (`wasm32-unknown-unknown --features browser`) generating `minigraf-wasm/` with JS glue and TypeScript definitions
  - `wasm-bindgen-test` browser integration tests (Chrome + Firefox via `wasm-pack test`)
- **Server-side WASM** (`wasm32-wasip1` / WASI):
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

## v0.19.0 — Publish Preparation (2026-04-08)

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

## v0.18.0 — Prepared Statements (2026-04-04)

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

## v0.17.0 — User-Defined Functions (2026-04-02)

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

## v0.16.0 — Window Functions (2026-04-02)

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

## v0.15.0 — Temporal Metadata Bindings (2026-04-01)

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

## v0.14.0 — Tests + Error Coverage (2026-03-31)

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
- `tests/negation_test.rs` — 10 integration tests for `not`: basic absence, multi-clause, rule body, time-travel, negative cycle rejection
- `tests/not_join_test.rs` — 14 integration tests for `not-join`: basic exclusion, multiple join vars, multi-clause body, rule body, `:as-of`, `:valid-at`, negative cycle at registration, `not`+`not-join` coexistence, `RuleInvocation` in body end-to-end

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
- README status text updated to reflect the completed benchmark and publish-preparation work
- crates.io publication deferred to a later preparation release (API cleanup; file format v6 now complete)

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
  facts (`valid_from <= now < valid_to`). Existing databases from that era are unaffected
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
