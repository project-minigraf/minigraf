# Minigraf

[![crates.io](https://img.shields.io/crates/v/minigraf.svg)](https://crates.io/crates/minigraf)
[![docs.rs](https://docs.rs/minigraf/badge.svg)](https://docs.rs/minigraf)
[![Build Status](https://github.com/adityamukho/minigraf/actions/workflows/rust.yml/badge.svg)](https://github.com/adityamukho/minigraf/actions/workflows/rust.yml)
[![Clippy Status](https://github.com/adityamukho/minigraf/actions/workflows/rust-clippy.yml/badge.svg)](https://github.com/adityamukho/minigraf/actions/workflows/rust-clippy.yml)
[![Coverage](https://codecov.io/gh/adityamukho/minigraf/branch/main/graph/badge.svg)](https://codecov.io/gh/adityamukho/minigraf)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/adityamukho/minigraf#license)
[![Rust Edition](https://img.shields.io/badge/rust-2024-orange.svg)](https://blog.rust-lang.org/2024/10/17/Rust-1.82.0.html)
[![Phase](https://img.shields.io/badge/phase-7.9%20complete-blue.svg)](https://github.com/adityamukho/minigraf/blob/main/ROADMAP.md)

> **Embedded graph memory for AI agents, mobile apps, and the browser** — the SQLite of bi-temporal graph databases

A tiny, self-contained graph database with **Datalog queries** and **bi-temporal time travel**. Think SQLite, but for connected data with full history.

## Vision

Minigraf is a **single-file embedded graph database** that lets you:
- ✅ **Query relationships with Datalog** - Recursive rules, natural graph traversal
- ✅ **Time travel through history** - Bi-temporal queries (transaction time + valid time)
- ✅ **Window functions** - `sum/count/min/max/avg/rank/row-number :over (partition-by … :order-by …)` in `:find` clauses
- ✅ **Prepared statements** - Parse + plan once with `$slot` bind tokens, execute thousands of times
- ✅ **Embed anywhere** - Native, WASM, mobile, IoT - one `.graph` file
- ✅ **Zero configuration** - Just `Minigraf::open("data.graph")` and you're done

**Status**: See [ROADMAP.md](ROADMAP.md) for current phase and what's next.

## Why Datalog?

**Datalog is fundamentally better for graphs than SQL-like languages:**

1. **Recursive by design** - Multi-hop traversals are natural, not an afterthought
2. **Simpler to implement** - Smaller spec = more reliable, faster to production
3. **Perfect for temporal** - Time is just another dimension in relations
4. **Proven at scale** - 40+ years of research, production use (Datomic, XTDB)
5. **Graph-native** - Facts (Entity-Attribute-Value) are literally edges
6. **LLM-friendly** - The small, uniform grammar (`[?e :attr ?v]` patterns, no JOIN variants, no subquery nesting) is easy for AI coding assistants to generate correctly from a few examples; the entire language fits in a system prompt

## Installation

```toml
[dependencies]
minigraf = "0.19"
```

Or via cargo:

```sh
cargo add minigraf
```

## Quick Start

```rust
use minigraf::{Minigraf, OpenOptions};

// Open or create a file-backed database
let db = OpenOptions::new().path("myapp.graph").open()?;

// Add facts
db.execute(r#"(transact [[:alice :person/name "Alice"]
                         [:alice :person/age 30]
                         [:alice :friend :bob]
                         [:bob :person/name "Bob"]])"#)?;

// Query with Datalog
let results = db.execute(r#"
    (query [:find ?friend-name
            :where [:alice :friend ?friend]
                   [?friend :person/name ?friend-name]])
"#)?;

// Explicit transaction — all-or-nothing
let mut tx = db.begin_write()?;
tx.execute(r#"(transact [[:alice :person/age 31]])"#)?;
tx.commit()?;

// Time travel — query as of past transaction counter
db.execute("(query [:find ?age :as-of 1 :where [:alice :person/age ?age]])")?;

// Recursive rule — transitive reachability
db.execute(r#"(rule [(reachable ?a ?b) [?a :friend ?b]])
              (rule [(reachable ?a ?b) [?a :friend ?m] (reachable ?m ?b)])"#)?;

// Prepared statement — parse + plan once, execute many times
use minigraf::BindValue;
let pq = db.prepare("(query [:find ?name :as-of $tx :where [$entity :person/name ?name]])")?;
let r1 = pq.execute(&[("tx", BindValue::TxCount(1)), ("entity", BindValue::Entity(alice_id))])?;
let r2 = pq.execute(&[("tx", BindValue::TxCount(2)), ("entity", BindValue::Entity(bob_id))])?;
```

```bash
cargo run          # interactive Datalog REPL
cargo test         # run 780 tests
cargo run < demos/demo_recursive.txt   # recursive rules demo
```

## Demo

See a working implementation of **temporal reasoning** with Minigraf at [github.com/adityamukho/temporal_reasoning](https://github.com/adityamukho/temporal_reasoning) — an AI agent that uses Minigraf's bi-temporal model to store, correct, and audit beliefs.

See the [Datalog Reference](https://github.com/adityamukho/minigraf/wiki/Datalog-Reference) wiki page for the complete syntax.

## Why Minigraf?

No other database offers this combination:

| Feature | Minigraf | XTDB | Cozo | Neo4j | SQLite |
|---|---|---|---|---|---|
| **Query Language** | Datalog | Datalog | Datalog | Cypher | SQL |
| **Single File** | ✅ Yes | ❌ No | ❌ No | ❌ No | ✅ Yes |
| **Bi-temporal** | ✅ Yes | ✅ Yes | ⚠️ Time travel | ❌ No | ❌ No |
| **Embedded** | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No | ✅ Yes |
| **Graph Native** | ✅ Yes | ✅ Yes | ✅ Yes | ✅ Yes | ❌ No |
| **Rust** | ✅ Yes | ❌ Clojure | ✅ Yes | ❌ Java | ❌ C |
| **WASM Ready** | 🎯 Phase 8 | ❌ No | ⚠️ Limited | ❌ No | ✅ Yes |

**Embedded graph memory for agents, mobile, and the browser — SQLite's simplicity + Datomic's temporal model.**

See the [Comparison](https://github.com/adityamukho/minigraf/wiki/Comparison) wiki page for detailed analysis including temporal vs. time-series databases.

### For AI Agents

Store what an agent believes, retract and correct without losing history, and replay past states to audit decisions. Every fact carries both transaction time (when it was recorded) and valid time (when it was true), so you can reconstruct the exact knowledge state at the moment of any past decision.

Pairs well with vector stores (GraphRAG pattern): the vector store answers "what is similar?"; Minigraf answers "what are the relationships, who recorded them, and what did we believe at time T?"

### For Mobile Apps

Offline-first storage with retroactive corrections — the bi-temporal model lets you correct a mis-entered value while preserving the original record. Phase 8 will ship iOS `.xcframework` and Android `.aar` via UniFFI.

### For WASM / Browser

Phase 8 target: page-granular IndexedDB backend, `wasm-pack` packaging, npm release as `@minigraf/core`. Also supports server-side WASM via WASI.

See the [Use Cases](https://github.com/adityamukho/minigraf/wiki/Use-Cases) wiki page for detailed guides on all three targets.

## Scope

Minigraf runs as:
- ✅ An embedded library
- ✅ A standalone binary (interactive REPL)
- 🎯 A WebAssembly module (Phase 8)

Minigraf will **not** be (by design):
- **Distributed** — no clustering, no sharding, no replication; each agent instance owns its own `.graph` file
- **Client-server** — no network protocol in core
- **Billion-node scale** — optimised for <1M nodes (like SQLite)
- **A time-series database** — Minigraf is a *temporal* database; see [Comparison](https://github.com/adityamukho/minigraf/wiki/Comparison#influxdb--prometheus--timescaledb-time-series-databases)

## Roadmap

See [ROADMAP.md](ROADMAP.md) for the full phase plan, current status, and release strategy.

## Performance

Benchmarks on Intel Core i7-1065G7 @ 1.30GHz, 16 GB RAM, Rust 1.92.0. See [BENCHMARKS.md](BENCHMARKS.md) for full tables.

| Metric | Result |
|---|---|
| Insert (in-memory, single fact) | ~2.7 µs — flat across 1K–100K facts |
| Insert (file-backed, WAL) | ~3.6 µs — flat across 1K–100K facts |
| Point query at 1M facts | 4.3–4.5 s (O(N) scan; Phase 7 target: predicate pushdown) |
| Open time at 1M facts | 1.31 s (2.4× faster than v5 — indexes no longer loaded into RAM) |
| Peak heap at 1M facts | 1.05 GB (~21% less than v5 — indexes paged in on demand) |

File-backed databases enforce a maximum fact size of **4 080 serialised bytes** per fact. In-memory databases have no limit.

## Contributing

This is a hobby project with a long-term vision. Read [PHILOSOPHY.md](PHILOSOPHY.md) and [ROADMAP.md](ROADMAP.md) before proposing features.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code standards, and the PR process.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
