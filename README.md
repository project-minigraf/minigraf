# Minigraf

[![crates.io](https://img.shields.io/crates/v/minigraf.svg)](https://crates.io/crates/minigraf)
[![docs.rs](https://docs.rs/minigraf/badge.svg)](https://docs.rs/minigraf)
[![Build Status](https://github.com/project-minigraf/minigraf/actions/workflows/rust.yml/badge.svg)](https://github.com/project-minigraf/minigraf/actions/workflows/rust.yml)
[![Lint Status](https://github.com/project-minigraf/minigraf/actions/workflows/lint.yml/badge.svg)](https://github.com/project-minigraf/minigraf/actions/workflows/lint.yml)
[![Coverage](https://codecov.io/gh/project-minigraf/minigraf/branch/main/graph/badge.svg)](https://codecov.io/gh/project-minigraf/minigraf)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](https://github.com/project-minigraf/minigraf#license)
[![Rust Edition](https://img.shields.io/badge/rust-2024-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2024/index.html)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-orange.svg)](Cargo.toml)

> **Embedded graph memory for AI agents, mobile apps, and the browser** — the SQLite of bi-temporal graph databases

A tiny, self-contained graph database with **Datalog queries** and **bi-temporal time travel**. Think SQLite, but for connected data with full history.

**[Try it in your browser — no install needed →](https://minigraf-playground.vercel.app/)**

**[Watch time travel in the visualizer →](https://project-minigraf.github.io/minigraf-visualizer/#sample=careers&tx=7&e=:alice&view=map)** Scrub transaction time and valid time, and see the graph change.

## Vision

Minigraf is a **single-file embedded graph database** that lets you:
- ✅ **Query relationships with Datalog** - Recursive rules, natural graph traversal
- ✅ **Time travel through history** - Bi-temporal queries (transaction time + valid time)
- ✅ **Window functions** - `sum/count/min/max/avg/rank/row-number/lag/lead :over (partition-by … :order-by …)` in `:find` clauses
- ✅ **Bounded and ordered results** - `:limit N` returns at most N rows and stops the work once it has them; `:order-by [?score :desc] :limit 10` keeps only the top ten as it reads, and `:offset` pages through an ordered answer
- ✅ **Prepared statements** - Parse + plan once with `$slot` bind tokens, execute thousands of times
- ✅ **Embed anywhere** - Native, WASM, mobile, IoT - one `.graph` file
- ✅ **Zero configuration** - Just `Minigraf::open("data.graph")` and you're done

For example, `(lag ?salary :over (:partition-by ?dept :order-by ?hire-date))` reads the preceding employee's salary within each department; the first row gets `nil`. `lead` reads the following row.

The unreleased v3.0.0 branch adds bounded sliding windows such as `(avg ?salary :over (:partition-by ?dept :order-by ?hire-date :rows 2 preceding))` (#183).

On v3.0.0, an ascending value-only query such as `[:find ?v :where [?e :score ?v] :order-by [?v] :limit 10]` can read directly in index order and stop early when all committed `:score` history has one value kind (integer, float, or Boolean) and its source range has no pending writes. A query whose full order cannot be proved uses the existing sort (#641). Built-in `count` and `count-distinct` can also release completed groups when one committed integer/Boolean/float key is proved ordered (including mixed integer/float groups), with no `:with`, windows, callbacks or `collect-set`. Memory then follows the active group and batches; distinct members within one group still grow. Other aggregates keep full-input validation. When every built-in window uses the same proved nonconstant partition key, the engine can also finish and release one partition at a time. Its memory follows the largest partition plus bounded batches; one growing partition still grows. Unpartitioned windows, incompatible keys, aggregate queries, callbacks and `collect-set` keep full-input evaluation. A surrounding sort or distinct operation can still require more memory.

On the unreleased v3 branch, selective in-memory ancestor queries can use entity probes instead of reading the whole edge attribute (#723). This is a query-planning optimization; it adds no storage index or configuration.

**Status**: See [ROADMAP.md](ROADMAP.md) for planned work and current direction.

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
minigraf = "2.0.5"
```

Or via cargo:

```sh
cargo add minigraf
```

Requires Rust 1.89 or newer (the minimum supported Rust version, set in `Cargo.toml`).

## Known issues

Bugs present in the current release, with affected versions, workarounds and fix versions, are listed in the pinned **[Known issues in the current release](https://github.com/project-minigraf/minigraf/issues/421)** issue. The most important one on v2.x:

- Two values of the same attribute for one entity written in a single `transact` (or retracted in a single `retract`) can read back as one value ([#371](https://github.com/project-minigraf/minigraf/issues/371)). Write or retract each value of a multi-valued attribute in its own call. The fix changes the file format and ships in v3.0.0.

v2.x gets data-integrity and security fixes for 12 months after v3.0.0 ships. See the [support policy](PHILOSOPHY.md#support-policy).

## Quick Start

```rust
use minigraf::{FactFilter, LogWriter, Minigraf, OpenOptions};

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

// Cursor — read a large answer in batches; the answer is fixed at open
let mut cursor = db.query("(query [:find ?name :where [?e :person/name ?name]])")?;
while let Some(batch) = cursor.next_batch(1000)? {
    for row in batch.rows() { /* row[0] is ?name */ }
}

// Fact log — every assertion and retraction, in transaction order, without Datalog
let mut log = db.fact_log(&FactFilter::new().attribute_prefix(":person/"))?;
while let Some(batch) = log.next_batch(1000)? {
    for rec in batch { /* rec.tx_count, rec.entity, rec.attribute, rec.value, rec.asserted */ }
}

// Read-only open — for backups and offline tools. Read-only handles share a file;
// a read-write handle (like `db` on myapp.graph) excludes them.
let archive = Minigraf::open_with_options("archive.graph", OpenOptions::new().read_only(true))?;

// Log writer — copy records into a new file with their transactions and valid-time
// windows kept; leave records out to purge them
let mut out = LogWriter::create("archive-clean.graph", OpenOptions::new())?;
for rec in archive.fact_log(&FactFilter::new())? {
    let rec = rec?;
    if rec.attribute != ":person/ssn" { out.append(&rec)?; }
}
out.advance_tx_count(archive.current_tx_count())?;
out.finish()?;

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
cargo test         # run the test suite
cargo run < demos/demo_recursive.txt   # recursive rules demo
```

Queries without aggregates return one row per matching binding, including repeated `:find` tuples.
Add the bare `:distinct` flag to remove repeated
output tuples: `(query [:find ?name :where [?e :person/name ?name] :distinct])`.
It runs after aggregates and windows, before ordering and pagination (#493).

## Demo

See a working implementation of **temporal reasoning** with Minigraf at [github.com/adityamukho/temporal_reasoning](https://github.com/adityamukho/temporal_reasoning) — an AI agent that uses Minigraf's bi-temporal model to store, correct, and audit beliefs.

The **[time travel visualizer](https://project-minigraf.github.io/minigraf-visualizer/)** runs Minigraf in your browser and draws its history. Step through transactions, move the valid-time cursor, and see each fact version on a bitemporal map. It opens `.graph` files too. For example, see [a salary that was recorded wrong and then corrected](https://project-minigraf.github.io/minigraf-visualizer/#sample=careers&tx=7&e=:alice&view=map).

See the [Datalog Reference](https://project-minigraf.github.io/minigraf-docs/latest/datalog-reference/) for the complete syntax.

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
| **WASM Ready** | ✅ Yes (browser + WASI) | ❌ No | ⚠️ Limited | ❌ No | ✅ Yes |

## Platform support

| Platform | Tier | Package | Install |
|---|---|---|---|
| Rust (native) | 1 | `minigraf` on crates.io | `cargo add minigraf` |
| Python | 1 | `minigraf` on PyPI | `pip install minigraf` |
| Browser WASM | 1 (from v3.0.0) | `@minigraf/browser` on npm | `npm install @minigraf/browser` |
| WASI | 2 (experimental) | `@minigraf/wasi` on npm | `npm install @minigraf/wasi` |
| Node.js | 2 (experimental) | `minigraf` on npm | `npm install minigraf` |
| Java/JVM | 2 (experimental) | `io.github.project-minigraf:minigraf-jvm` on Maven Central | see [docs](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/) |
| Android | 2 (experimental) | `io.github.project-minigraf:minigraf-android` (`.aar`) on Maven Central | see [docs](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/) |
| iOS / macOS | 2 (experimental) | `.xcframework` via Swift Package Manager ([minigraf-swift](https://github.com/project-minigraf/minigraf-swift)) | see [docs](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/) |
| C / FFI | 2 (experimental) | header + tarball on [minigraf-c releases](https://github.com/project-minigraf/minigraf-c/releases) | see [docs](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/) |

**Tier 1** bindings are fully tested and released at the same time as every core release. **Tier 2** bindings are built and smoke-tested, released on a best-effort schedule, and experimental. The Rust crate and Python are Tier 1; browser WASM joins them from v3.0.0. A binding moves to Tier 1 when real users need it, including the project's own published tools. Tier 1 operating systems and filesystems are Linux (ext4, xfs), macOS (APFS) and Windows (NTFS) on local disk; NFSv4 is supported with caveats, and NFSv3 `nolock` is unsupported for multiple writers. Details: [support tiers](PHILOSOPHY.md#support-tiers).

**Embedded graph memory for agents, mobile, and the browser — SQLite's simplicity + Datomic's temporal model.**

## Language Bindings

| Language | Package | Repo |
|---|---|---|
| Python | [`minigraf` on PyPI](https://pypi.org/p/minigraf) | [minigraf-python](https://github.com/project-minigraf/minigraf-python) |
| Node.js | [`minigraf` on npm](https://www.npmjs.com/package/minigraf) | [minigraf-node](https://github.com/project-minigraf/minigraf-node) |
| Browser WASM | [`@minigraf/browser` on npm](https://www.npmjs.com/package/@minigraf/browser) | [minigraf-wasm](https://github.com/project-minigraf/minigraf-wasm) |
| WASI | [`@minigraf/wasi` on npm](https://www.npmjs.com/package/@minigraf/wasi) | [minigraf-wasm](https://github.com/project-minigraf/minigraf-wasm) |
| Java | `io.github.project-minigraf:minigraf-jvm` on Maven Central | [minigraf-java](https://github.com/project-minigraf/minigraf-java) |
| Android | `io.github.project-minigraf:minigraf-android` on Maven Central | [minigraf-android](https://github.com/project-minigraf/minigraf-android) |
| iOS/macOS | Swift bindings | [minigraf-swift](https://github.com/project-minigraf/minigraf-swift) |
| C | C bindings | [minigraf-c](https://github.com/project-minigraf/minigraf-c) |

See the [Comparison](https://project-minigraf.github.io/minigraf-docs/latest/comparison/) page for detailed analysis including temporal vs. time-series databases.

### For AI Agents

Store what an agent believes, retract and correct without losing history, and replay past states to audit decisions. Every fact carries both transaction time (when it was recorded) and valid time (when it was true), so you can reconstruct the exact knowledge state at the moment of any past decision.

Pairs well with vector stores (GraphRAG pattern): the vector store answers "what is similar?"; Minigraf answers "what are the relationships, who recorded them, and what did we believe at time T?"

See it in the visualizer: [what an agent believed when it made a recommendation](https://project-minigraf.github.io/minigraf-visualizer/#sample=agent-memory&tx=3&vt=any&e=:user-ana), and the correction that followed (press → to step forward).

### For Mobile Apps

Offline-first storage with retroactive corrections — the bi-temporal model lets you correct a mis-entered value while preserving the original record ([see a correction in the visualizer](https://project-minigraf.github.io/minigraf-visualizer/#data=KHRyYW5zYWN0IHs6dmFsaWQtZnJvbSAiMjAyNS0wNi0wMSJ9CiAgICAgICAgICBbWzp1c2VyIDpoZWFsdGgvd2VpZ2h0LWtnIDgyLjVdXSkKKHJldHJhY3QgW1s6dXNlciA6aGVhbHRoL3dlaWdodC1rZyA4Mi41XV0pCih0cmFuc2FjdCB7OnZhbGlkLWZyb20gIjIwMjUtMDYtMDEifQogICAgICAgICAgW1s6dXNlciA6aGVhbHRoL3dlaWdodC1rZyA4MC41XV0p&title=Record+facts+offline%2C+correct+on+sync&vt=any&e=:user&view=map)). Native Kotlin and Swift bindings ship as an Android `.aar` (Maven Central) and an iOS `.xcframework` (Swift Package Manager) via [UniFFI](https://github.com/mozilla/uniffi-rs). No Rust required.

```kotlin
// Android (Kotlin)
val db = MiniGrafDb.open(context.filesDir.absolutePath + "/myapp.graph")
db.execute("""(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])""")
val json = db.execute("(query [:find ?name :where [?e :person/name ?name]])")
```

```swift
// iOS (Swift)
let db = try MiniGrafDb.open(path: docsURL.appendingPathComponent("myapp.graph").path)
try db.execute(datalog: #"(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])"#)
let json = try db.execute(datalog: "(query [:find ?name :where [?e :person/name ?name]])")
```

See the [Mobile Integration](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/#mobile-apps) section for full setup and usage docs (Gradle config, SPM integration, error handling, threading).

### For WASM / Browser

Published as [`@minigraf/browser`](https://www.npmjs.com/package/@minigraf/browser) on npm (IndexedDB-backed, `wasm-pack`). WASI build (`wasm32-wasip1`) available as [`@minigraf/wasi`](https://www.npmjs.com/package/@minigraf/wasi) on npm and as a GitHub Releases artifact (Wasmtime / Wasmer). See the [Use Cases](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/). The [playground](https://minigraf-playground.vercel.app/) and the [time travel visualizer](https://project-minigraf.github.io/minigraf-visualizer/) are both built on `@minigraf/browser`.

### For Python / Node.js / Java / C

Language bindings ship as `minigraf` on PyPI, `minigraf` on npm (Node.js native addon), `io.github.adityamukho:minigraf-jvm` on Maven Central, and a C header + prebuilt shared library on GitHub Releases. See the [Use Cases](https://project-minigraf.github.io/minigraf-docs/latest/use-cases/).

## Scope

Minigraf runs as:
- ✅ An embedded library
- ✅ A standalone binary (interactive REPL)
- ✅ Browser WASM — `@minigraf/browser` (IndexedDB-backed, `wasm-pack`)
- ✅ Server-side WASM — `wasm32-wasip1` / WASI (Wasmtime, Wasmer, Cloudflare Workers)
- ✅ Android, iOS, Python, Node.js, Java, C — via UniFFI / napi-rs / cbindgen

Minigraf will **not** be (by design):
- **Distributed** — no clustering, no sharding, no replication; each agent instance owns its own `.graph` file
- **Client-server** — no network protocol in core
- **Billion-node scale** — optimised for <1M nodes (like SQLite)
- **A time-series database** — Minigraf is a *temporal* database; see [Comparison](https://project-minigraf.github.io/minigraf-docs/latest/comparison/#influxdb--prometheus--timescaledb-time-series-databases)

## Roadmap

See [ROADMAP.md](ROADMAP.md) for planned work and current direction.

## Performance

Performance envelope on the `v3` branch (format v8), measured up to 10M facts on an otherwise idle server, two rounds (see [BENCHMARKS.md](docs/BENCHMARKS.md#executor-removal-551) for every shape, RSS and the method):

| Workload | 100K facts | 1M facts | 10M facts |
|---|---:|---:|---:|
| Point lookup (bound entity + attribute, incl. `:as-of` / `:valid-at`), p50 / p99 | 18–21 µs / 25–39 µs | 18–20 µs / 22–37 µs | 18–20 µs / 22–27 µs |
| Entity with 1,000 versions, p50 / p99 | 0.29–0.30 / 0.31–0.35 ms | 0.29–0.30 / 0.30–0.36 ms | 0.29–0.30 / 0.32–0.37 ms |
| Open a file (RSS after open: 3 MiB) | 0.12–0.16 ms | 0.25–0.27 ms | 0.13–0.23 ms |
| Checkpoint after 1 new fact, p50 / p99 | 0.7–0.9 / 1.4–1.6 ms | 0.9–1.8 / 2.7–2.8 ms | 1.1–1.5 / 2.2–2.9 ms |
| Attribute scan (N/6 rows) drained in batches of 1,000, p50 | 23 ms | 0.25 s | 2.5 s |
| Selective 2-clause join (1 row), p50 | 38 µs | 37 µs | 37 µs |
| Rule query with a constant (magic sets, 5–7 rows), p50 | 1.1 ms | 1.5 ms | 1.9 ms |

The table above is the #551 snapshot. A later [focused UUID projection comparison](docs/BENCHMARKS.md#projected-uuid-lookup-batching-530-unreleased) measures the 1M-fact attribute scan at 262 → 136 ms p50 after batched dictionary reads (#530); the other table scales were not remeasured in that comparison.

Point lookups, time travel on one entity, and joins and rule queries that a constant makes selective stay flat at every tested size. Queries that read a whole attribute (scans, aggregates, joins with nothing selective) grow linearly with the facts they read, in bounded memory: the 1.67M-row scan at 10M facts holds one batch, 7 MiB. Windows and whole transitive closures hold their rows, and `OpenOptions::query_memory_limit` stops a query that passes it with `QRY-012`. Graphs over 10M facts are not tested.

File-backed databases limit string values to 4,068 bytes (`WAL-003`). In-memory databases have no limit.

**Durability tuning:** every write is `fsync`'d immediately by default (`SyncMode::Full`). Bulk loaders/migrations that can safely re-run from a checkpoint watermark on failure can trade that for throughput with `OpenOptions::new().synchronous(SyncMode::Normal)` — `checkpoint()` still fsyncs unconditionally in both modes. See the [Performance Tuning](https://project-minigraf.github.io/minigraf-docs/latest/performance-tuning/#configuration-knobs) for the full tradeoff and the write-batching pattern that pairs with it.

**Running Minigraf where data loss matters:** [docs/DURABILITY.md](docs/DURABILITY.md) covers what survives a process crash and a power loss, recovery on open, supported filesystems and containers, backups and restores, `verify()` and `rebuild_indexes()`, upgrading from v2.x, and which errors mean retry, reopen or restore.

## Contributing

This is a solo-maintained project with a long-term vision. Read [PHILOSOPHY.md](PHILOSOPHY.md) and [ROADMAP.md](ROADMAP.md) before proposing features.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, code standards, and the PR process.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
