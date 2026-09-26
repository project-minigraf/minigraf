# Minigraf Roadmap

> The path to a production-ready bi-temporal Datalog database, and then to a stronger ecosystem. Production readiness is tracked in #383.

**Philosophy**: Embedded graph memory for agents, mobile, and the browser — built on the SQLite approach: be boring, be reliable, be embeddable.

Completed releases and their implementation details live in the [CHANGELOG](CHANGELOG.md). This document tracks planned, exploratory, and other future work.

---

## v2.0.2 — Final Planned v2.x Release

v2.0.2 is a patch release on file format v7 with no API change. It is the last planned v2.x release. New feature work targets v3.x.

Scope (milestone [v2.0.2](https://github.com/project-minigraf/minigraf/milestone/8)):

- Merged performance mitigations (#323, #381, #315)
- Parent-directory fsync after creating or deleting files (#389)
- `or-join` returning an error when the clauses before it match no rows (#405)
- A clear error for `$slot` queries run through `execute()` (#407)
- CI hardening: benchmark alerting (#393), MSRV, semver and dependency checks (#395)
- Stability, support and known-issues docs and process (#397, #399, #400). These define how v2.x is supported after v3.0.0.

Known issue on v2.x: same-transaction multi-valued facts can read back as one value (#371). The fix needs file format v8 and ships in v3.0.0. Workaround: write or retract each value of a multi-valued attribute in its own call.

---

## v3.0.0 — File Format v8 and Data Integrity

**v3.0.0 supports file format v8 only.** Fixing the #287/#371 data-loss bug adds the value to the `EavtKey`/`AevtKey` index keys, which bumps the format from v7 to v8. Support for v1–v6 is dropped at the same time. The v1–v6 → v7 auto-migration code in `persistent_facts.rs` can be removed when cutting this release; a v7→v8 migration replaces it. Any database opened at least once under a v1.x or v2.x release is already on v7.

This was the GitHub milestone named “2.0” before v2.0.0 used that version number for the kernel-locking and structured-error-code breaking changes; it was consequently renumbered to v3.0.0.

Scope (milestone [v3.0.0](https://github.com/project-minigraf/minigraf/milestone/1)):

- v8 index keys and v7→v8 migration (#287, #371; done on the `v3` branch)
- Per-page checksums for fact and B+tree pages (#388)
- Crash-atomic `save()` (#374)
- Index verify and rebuild (#373)
- O(live) point queries on v8 keys (#379)
- Release-gate test suite: golden files (#391), crash data checks (#384), model-based tests (#385), wider property tests (#386), fuzz corpus and operation-sequence target (#387, #375), fault injection (#390), soak test (#392), coverage gates (#396)
- Performance envelope (#394) and durability guide (#398)

---

## v3.1.0 — Additive Features

Scope (milestone [v3.1.0](https://github.com/project-minigraf/minigraf/milestone/9)):

- Rule persistence (#241)
- Query profiler (#185)
- `:limit` / `:offset` (#306, #310)
- Set membership predicates (#316)
- `lag`/`lead` window functions (#182) and sliding row frames, `:rows N preceding` (#183)
- UniFFI: `PreparedQuery` (#181), UDF registration (#180), `OpenOptions` (#322)
- Temporal graph-traversal research (#273; may result in documentation or a blog post rather than code)

---

## Planned Work: Ecosystem & Tooling

**Goal**: Improve developer experience and grow the ecosystem without expanding the core unnecessarily.

### Developer Tools

- 🎯 Database inspector/debugger (separate repository: [minigraf-inspector](https://github.com/project-minigraf/minigraf-inspector))
- 🎯 Query profiler
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
- v3.0.0 (format v8 and data integrity) on the `v3` branch, tracked in #383
- v3.1.0 features after v3.0.0 ships
- Ecosystem work tracked in [`minigraf-examples`](https://github.com/project-minigraf/minigraf-examples)
- Developer tools tracked in `minigraf-inspector` and `minigraf-visualizer`

See [GitHub Issues](https://github.com/project-minigraf/minigraf/issues) for specific tasks, and the [CHANGELOG](CHANGELOG.md) for completed releases.

---

**Last updated**: September 2026. v2.0.1 is the current release. v2.0.2 is the final planned v2.x release and ships from `main`. v3.0.0 work, including the file-format v8 fix for #287/#371, is developed on the long-lived `v3` branch and merges into `main` at the v3.0.0 cut. New features follow in v3.1.0.
