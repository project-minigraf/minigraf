# Minigraf Roadmap

> The path from a production-ready bi-temporal Datalog database to a stronger ecosystem.

**Philosophy**: Embedded graph memory for agents, mobile, and the browser — built on the SQLite approach: be boring, be reliable, be embeddable.

Completed releases and their implementation details live in the [CHANGELOG](CHANGELOG.md). This document tracks planned, exploratory, and other future work.

---

## v3.0.0 — File Format Policy

**v3.0.0 supports file format v8 only.** Fixing #287's `EavtKey`/`AevtKey` missing-value-bytes data-loss bug bumps the format from v7 to v8. Support for v1–v6 is dropped at the same time. The v1–v6 → v7 auto-migration code in `persistent_facts.rs` can be removed when cutting this release; a v7→v8 migration replaces it. Any database opened at least once under a v1.x release will already be on v7; there are no known users on older formats.

This was the GitHub milestone named “2.0” before v2.0.0 used that version number for the kernel-locking and structured-error-code breaking changes; it was consequently renumbered to v3.0.0.

Scope:

- `EavtKey`/`AevtKey` missing-value-bytes fix and file-format v7→v8 migration (#287)
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

- The v3.0.0 file-format policy and its scoped work above
- Ecosystem work tracked in [`minigraf-examples`](https://github.com/project-minigraf/minigraf-examples)
- Developer tools tracked in `minigraf-inspector` and `minigraf-visualizer`

See [GitHub Issues](https://github.com/project-minigraf/minigraf/issues) for specific tasks, and the [CHANGELOG](CHANGELOG.md) for completed releases.

---

**Last updated**: August 2026 — v2.0.0 is the current release; v3.0.0 is the next core milestone described here.
