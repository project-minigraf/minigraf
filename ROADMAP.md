# Minigraf Roadmap

> The path from a production-ready bi-temporal Datalog database to a stronger ecosystem.

**Philosophy**: Embedded graph memory for agents, mobile, and the browser — built on the SQLite approach: be boring, be reliable, be embeddable.

Completed releases and their implementation details live in the [CHANGELOG](CHANGELOG.md). This document tracks planned, exploratory, and other future work.

---

## v2.0.2 — Final Planned v2.x Release

**The v2.x line (`main`) ends with a patch.** File format stays v7 and the public API does not change. All new feature work has moved to v3.x. Tracker: #383.

Scope:

- Already merged: bound-entity point queries narrowed to the queried attribute (#323 mitigation), exact attribute-scan bounds (#381), and checkpoints that copy untouched index leaves (#315 mitigation)
- fsync the parent directory after creating the `.graph`/WAL file and after deleting the WAL (#389)
- `or-join` returns an empty result, not INT-031, when the clauses before it match no rows (#405)
- A clear user error for `$slot` queries run through `execute()`, not internal INT-025 (#407)
- Benchmark regression alerting that is not silently muted (#393)
- CI: MSRV build, public-API semver check, dependency policy (#395)
- Stability and support policy, including how long v2.x gets data-integrity fixes after v3.0.0 (#397)
- Known-issues process: data-integrity issues stay open until the fix is in a published release (#399)
- Support tiers for language bindings and platforms (#400)

**Known issue on all v2.x releases:** same-transaction multi-valued facts read back as one value (#371, #287). The fix needs format v8 and ships in v3.0.0. Workaround: write or retract each value of a multi-valued attribute in its own `transact`/`retract` call.

---

## v3.0.0 — File Format v8 and Data Integrity

**v3.0.0 supports file format v8 only.** Fixing #287/#371 (`EavtKey`/`AevtKey` carry no value bytes, so same-transaction multi-values collapse) bumps the format from v7 to v8. Support for v1–v6 is dropped at the same time. The v1–v6 → v7 auto-migration code in `persistent_facts.rs` can be removed when cutting this release; a v7→v8 migration replaces it. Any database opened at least once under a v1.x release will already be on v7; there are no known users on older formats. Every other layout change this release needs goes into v8 too, so there is no separate v9.

This was the GitHub milestone named “2.0” before v2.0.0 used that version number for the kernel-locking and structured-error-code breaking changes; it was consequently renumbered to v3.0.0. It is developed on the long-lived `v3` branch and merges into `main` at the cut.

Scope — format and storage integrity:

- v8 index keys with value bytes and v7→v8 migration (#371, #287; done on `v3`)
- Per-page CRC32 checksums on fact pages and B+tree pages (#388)
- Crash-atomic `save()`, with checkpoints proportional to the change (#374)
- Index `verify` and public `rebuild_indexes()` (#373)
- Net-assert on v8 keys before resolving facts, for O(live) point queries (#379)
- `btree_page` fuzz target that reaches node decoding (#375)

Scope — release-gate testing and documentation:

- Golden-file compatibility corpus for every format version, frozen before v8 work goes further (#391)
- SIGKILL crash test that checks committed data through EAVT, AEVT and full scan (#384)
- Model-based test of transact/retract/checkpoint/reopen (#385)
- Wider property tests: file-backed, batched, retracts, temporal, joins (#386)
- Persistent fuzz corpus and an operation-sequence target (#387)
- Fault injection across save/WAL/recovery (#390)
- Long-running soak test at scale (#392)
- Benchmarks at 100K–10M facts and a published performance envelope (#394)
- Higher storage/WAL coverage gates and branch-coverage gates (#396)
- Durability, recovery and operations guide (#398)

---

## v3.1.0 — Features

Additive features, kept out of v3.0.0 so the #371 fix is not held up by them:

- Rule persistence — design first (#241)
- Query profiler (#185)
- Native `:limit N` / `:offset` for plain `:find` queries (#306, #310)
- Set membership predicates (#316)
- `lag`/`lead` window functions (#182)
- Sliding row frames — `:rows N preceding` (#183; builds on #182)
- `PreparedQuery` over UniFFI (#181)
- UDF registration over UniFFI (#180)
- `OpenOptions` exposed over UniFFI, so embedders can suppress the close-time checkpoint (#322)
- Temporal graph-traversal research (#273; may result in documentation or a blog post rather than code)

---

## Planned Work: Ecosystem & Tooling

**Goal**: Improve developer experience and grow the ecosystem without expanding the core unnecessarily.

### Developer Tools

- 🎯 Database inspector/debugger (separate repository: [minigraf-inspector](https://github.com/project-minigraf/minigraf-inspector))
- 🎯 Query profiler (#185, v3.1.0)
- 🎯 Time-travel visualizer (separate repository: [minigraf-visualizer](https://github.com/project-minigraf/minigraf-visualizer))

### Integration Examples

**Tracked in**: [`minigraf-examples`](https://github.com/project-minigraf/minigraf-examples).

- 🎯 GraphRAG example that combines Minigraf with a vector store
- 🎯 LangChain / LangChain.js integration example
- 🎯 LlamaIndex integration example
- 🎯 Annotated end-to-end scenarios for agent memory, offline-first mobile, and audit logs

### Ecosystem Libraries

- 🎯 Graph algorithms as a separate crate
- 🎯 Optional schema validation
- 🎯 Import/export tools
- 🎯 Backup utilities

### Database Branching / Forking (Exploratory)

Allow a Minigraf database to be forked into an independent `.graph` file pre-populated with facts from the parent at a given transaction count.

Potential uses include speculative writes, snapshot distribution, test isolation, and agent sandboxing. The design must preserve the single-file, zero-configuration model; a fork should behave as an independent file copy with a fully flushed WAL.

**Status**: Exploratory. Pursue only with demonstrated user demand.

### Magic Sets with Stratified Negation (Exploratory)

Magic-sets rewriting is not applied to mixed rules containing `not`/`not-join`; those use full semi-naive evaluation. Extending it to negation is well studied but requires care to avoid unsound propagation across stratification boundaries.

**Pursue only if** profiling demonstrates a real bottleneck in negation-heavy recursive workloads.

---

## Performance Direction

Future performance work will be driven by production profiles and reproducible benchmarks. The [benchmark documentation](docs/BENCHMARKS.md) records measurements; completed performance work is recorded in the [CHANGELOG](CHANGELOG.md).

---

## Decision Framework

When evaluating features, ask:

1. Does it align with the philosophy? (embedded, reliable, simple, bi-temporal)
2. Is it needed for target use cases? (audit, event sourcing, knowledge graphs)
3. Does it compromise reliability? (stability over features)
4. Can it be a separate crate? (keep the core small)

**Say no to**:

- Distributed consensus
- Multi-datacenter replication
- Built-in ML/AI
- Features useful only at massive scale
- Complex configuration
- Breaking the single-file philosophy

**Say yes to**:

- Crash safety
- Data integrity
- Temporal queries
- Query performance
- Developer experience
- Cross-platform support

---

## Current Focus

- v2.0.2, the final planned v2.x release, on `main`
- v3.0.0 (format v8 and data integrity) on the `v3` branch, then v3.1.0 features
- Production readiness for high-stakes deployments, tracked in #383
- Ecosystem work tracked in [`minigraf-examples`](https://github.com/project-minigraf/minigraf-examples)
- Developer tools tracked in `minigraf-inspector` and `minigraf-visualizer`

See [GitHub Issues](https://github.com/project-minigraf/minigraf/issues) for specific tasks, and the [CHANGELOG](CHANGELOG.md) for completed releases.

---

**Last updated**: September 2026 — v2.0.1 is the current release. v2.0.2 is the final planned v2.x release and ships from `main` first; v3.0.0 work, including the file-format v8 fix for #287/#371, is developed on the long-lived `v3` branch and merges into `main` at the v3.0.0 cut.
