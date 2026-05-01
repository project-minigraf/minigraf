# Phase 8.1a: Browser WASM Support — Design Spec

**Date**: 2026-04-17
**Issue**: project-minigraf/minigraf#129
**Status**: Approved — ready for implementation planning

---

## Goal

Ship Minigraf as an npm package (`@minigraf/core`) that runs natively in **browser
environments only** using WebAssembly (`wasm32-unknown-unknown` + `wasm-bindgen`), with
full TypeScript types and a page-granular IndexedDB storage backend.

**`@minigraf/core` is not compatible with Node.js or other server-side runtimes.**
Server-side Node.js is handled by `@minigraf/node` (Phase 8.3, `napi-rs`, native
bindings). These are distinct packages with different storage backends, different
loading mechanisms, and different performance profiles. Do not attempt to make
`@minigraf/core` work in Node.js — use the correct package for the target runtime.

---

## Core Decisions (rationale in each section)

| Decision | Choice |
|----------|--------|
| Storage bridge | Separate `BrowserDb` async façade; native `Minigraf` unchanged |
| `execute()` return type | JSON string |
| Durability model | Write-through: every transact flushes dirty pages to IndexedDB |
| Transaction API | `execute()` only; no `begin_write()` / `WriteTransaction` |
| Thread model | Single-threaded; `Rc<RefCell<...>>` throughout `BrowserDb` |

---

## Architecture

The central constraint is that the existing `StorageBackend` trait is synchronous
(`read_page` / `write_page` return `Result<T>` synchronously), but IndexedDB is
async-only. In browser WASM there is no way to block on an async call. Making
`StorageBackend` async would propagate `async` through `PersistentFactStorage`,
`Minigraf::execute`, and the entire public API — a large, invasive change with risk
to all other targets.

**Solution: separate async façade.**

Introduce a new `BrowserDb` type that owns a synchronous in-memory buffer
(`BrowserBufferBackend`) satisfying the existing `StorageBackend` contract, plus an
async `IndexedDbBackend` that mirrors dirty pages to IndexedDB after every write.
The Datalog engine (`FactStorage`, `RuleRegistry`, `FunctionRegistry`,
`PersistentFactStorage`, all query/executor code) is used **unchanged**. The native
`Minigraf` type is not touched.

```
JS/TS caller  (single-threaded browser)
      │  Promise<string>
      ▼
 BrowserDb                           src/browser/mod.rs
 Rc<RefCell<BrowserDbInner>>         #[wasm_bindgen] async façade
      │  sync                │  async (after sync save)
      ▼                      ▼
 PersistentFactStorage    IndexedDbBackend
 <BrowserBufferBackend>   src/browser/indexeddb.rs
      │                   raw async IDB get / put per page_id
      ▼
 BrowserBufferBackend     src/browser/buffer.rs
 HashMap<u64, Vec<u8>>    implements StorageBackend (sync, unchanged trait)
 + HashSet<u64> dirty
```

### Why `Rc<RefCell<...>>` not `Arc<Mutex<...>>`

Standard browser WASM (`wasm32-unknown-unknown`) is single-threaded. There is no
`SharedArrayBuffer` multi-threading by default. `#[wasm_bindgen]` types do not need
to be `Send + Sync`. `Rc<RefCell<...>>` is more idiomatic, avoids atomic ref-counting
overhead, and is honest about the single-threaded contract. If SharedArrayBuffer
threading is ever needed, this becomes a targeted revisit.

### Known constraint: eager page load on open

All pages are loaded from IndexedDB into `BrowserBufferBackend` during the async
`open()`. This is required because `StorageBackend::read_page` is synchronous — there
is no way to satisfy a synchronous read contract with on-demand async fetches in
browser WASM.

**Scale guideline** (comfortable without tuning):

| Facts | Approx pages | Buffer size on open |
|-------|-------------|---------------------|
| 1K    | ~40         | ~160 KB             |
| 10K   | ~400        | ~1.6 MB             |
| 20K   | ~800        | ~3.2 MB             |

The primary browser use case (agent memory) is expected to stay well under 20K facts.
Document this limit in `README.md` and `docs/wasi.md`. The long-term fix (post-1.0)
is making `StorageBackend` async-aware.

---

## Module Layout

All new code is additive. Zero changes to existing modules except the two
`#[cfg]`-gated additions in `lib.rs` and `storage/backend/mod.rs`.

```
src/
  browser/
    mod.rs          ← BrowserDb struct + all #[wasm_bindgen] exports
    buffer.rs       ← BrowserBufferBackend: sync HashMap + dirty HashSet
    indexeddb.rs    ← IndexedDbBackend: async open/read/write over web-sys IDB
  lib.rs            ← add pub mod browser (cfg-gated)
  storage/backend/
    mod.rs          ← add re-export of BrowserBufferBackend (cfg-gated)

examples/
  browser/
    index.html      ← minimal page that loads the WASM module directly (no bundler)
    app.js          ← plain JS: open → transact → query → console.log
    README.md       ← how to build (wasm-pack) and serve (python -m http.server)

.github/workflows/
  wasm-browser.yml  ← new CI job: build + headless Chrome test
```

---

## Public API (`src/browser/mod.rs`)

All methods are `async` and compile to JS `Promise`s via `wasm-bindgen`.

**Runtime requirement**: this API requires a browser environment with `window.indexedDB`
available. It will not function in Node.js, Deno, Bun, or any server-side runtime
(IndexedDB is absent; WASM file loading also differs). `open_in_memory()` avoids
IndexedDB but is still not supported or tested outside a browser context — use
`@minigraf/node` (Phase 8.3) for server-side use cases.

```rust
#[wasm_bindgen]
pub struct BrowserDb { /* Rc<RefCell<BrowserDbInner>> */ }

#[wasm_bindgen]
impl BrowserDb {
    /// Open or create a browser database backed by IndexedDB.
    /// `db_name` is the IndexedDB database name (also the object store name).
    /// Called as `await BrowserDb.open("mydb")` from JS — NOT `new BrowserDb()`.
    /// wasm-bindgen does not support async constructors; use a static factory method.
    #[wasm_bindgen(js_name = open)]
    pub async fn open(db_name: &str) -> Result<BrowserDb, JsValue>;

    /// Open an in-memory database (no IndexedDB — for testing).
    #[wasm_bindgen(js_name = openInMemory)]
    pub async fn open_in_memory() -> Result<BrowserDb, JsValue>;

    /// Execute a Datalog string (transact, retract, query, rule).
    /// Returns a JSON string. Reads return query results; writes return
    /// `{"transacted": <tx_id>}` or `{"retracted": <tx_id>}`.
    pub async fn execute(&self, datalog: String) -> Result<String, JsValue>;

    /// Flush all dirty pages to IndexedDB explicitly.
    /// Not required after individual `execute()` calls (write-through),
    /// but useful after bulk imports or to force consolidation.
    pub async fn checkpoint(&self) -> Result<(), JsValue>;

    /// Serialise the current database state to a portable `.graph` blob (Uint8Array).
    /// The blob is byte-for-byte compatible with native `.graph` files and can be
    /// opened with `Minigraf::open()` on any native platform.
    /// Pages are always exported in ascending `page_id` order.
    #[wasm_bindgen(js_name = exportGraph)]
    pub async fn export_graph(&self) -> Result<js_sys::Uint8Array, JsValue>;

    /// Replace the current database with the contents of a `.graph` blob.
    /// The blob must be a checkpointed native `.graph` file (no pending WAL).
    /// All existing data is overwritten.
    #[wasm_bindgen(js_name = importGraph)]
    pub async fn import_graph(&self, data: js_sys::Uint8Array) -> Result<(), JsValue>;
}
```

**Error handling rule**: all `anyhow::Error` values are mapped to `JsValue::from_str(&e.to_string())`. No error categorisation at the WASM boundary.

**TypeScript types** (auto-generated by `wasm-pack` — do not write by hand):
```typescript
export class BrowserDb {
  static open(db_name: string): Promise<BrowserDb>;
  static openInMemory(): Promise<BrowserDb>;
  execute(datalog: string): Promise<string>;
  checkpoint(): Promise<void>;
  exportGraph(): Promise<Uint8Array>;
  importGraph(data: Uint8Array): Promise<void>;
}
```

---

## `BrowserBufferBackend` (`src/browser/buffer.rs`)

```rust
pub struct BrowserBufferBackend {
    pages: HashMap<u64, Vec<u8>>,  // page_id → 4KB page bytes
    dirty: HashSet<u64>,           // page_ids written since last take_dirty()
}

impl BrowserBufferBackend {
    pub fn new() -> Self;

    /// Load pages from an existing snapshot (used during open). Dirty set starts empty.
    pub fn load_pages(pages: HashMap<u64, Vec<u8>>) -> Self;

    /// Load pages and mark all of them dirty (used during import_graph).
    /// Equivalent to load_pages() followed by marking every page_id dirty.
    pub fn load_pages_all_dirty(pages: HashMap<u64, Vec<u8>>) -> Self;

    /// Drain and return the dirty page set. Clears the set.
    /// Called by BrowserDb after pfs.save() to get the list of pages to flush.
    pub fn take_dirty(&mut self) -> HashSet<u64>;

    /// Read a page by ID (does not mark dirty).
    pub fn get_page(&self, page_id: u64) -> Option<&Vec<u8>>;

    /// Total number of pages (for export: iterate 0..page_count()).
    pub fn page_count(&self) -> u64;
}

impl StorageBackend for BrowserBufferBackend {
    fn write_page(&mut self, page_id: u64, data: &[u8]) -> Result<()>;  // marks dirty
    fn read_page(&self, page_id: u64) -> Result<Vec<u8>>;
    fn sync(&mut self) -> Result<()>;   // no-op (sync handled by IndexedDbBackend)
    fn page_count(&self) -> Result<u64>;
    fn is_new(&self) -> bool;           // true iff pages is empty
}
```

This type is fully testable with native `cargo test` — no WASM needed.

---

## `IndexedDbBackend` (`src/browser/indexeddb.rs`)

Does **not** implement `StorageBackend` — it is not synchronous. Called directly by
`BrowserDb` after `pfs.save()`.

```rust
pub struct IndexedDbBackend {
    db: web_sys::IdbDatabase,
    store_name: String,
}

impl IndexedDbBackend {
    /// Open or create an IndexedDB database with a single object store.
    pub async fn open(db_name: &str) -> Result<Self, JsValue>;

    /// Load all pages from IndexedDB into a HashMap (used during BrowserDb::open).
    /// Returns pages keyed by page_id.
    /// Implementation: call `getAllKeys()` and `getAll()` on the object store in a
    /// single read transaction, then zip the two result arrays into a HashMap.
    /// Both calls share the same IDB transaction to guarantee consistency.
    pub async fn load_all_pages(&self) -> Result<HashMap<u64, Vec<u8>>, JsValue>;

    /// Write a set of pages to IndexedDB in a single transaction.
    /// Each page is stored as: key = page_id (u64), value = 4KB Uint8Array.
    pub async fn write_pages(&self, pages: &[(u64, &[u8])]) -> Result<(), JsValue>;
}
```

**IndexedDB object store schema**:
```
store name:  <db_name>          (same as the database name by default)
key:         page_id (u64, stored as JS number)
value:       page bytes (4096-byte Uint8Array)
key path:    none (out-of-line key)
auto-increment: false
```

---

## Data Flow

### `BrowserDb::open(db_name)`

```
1. IndexedDbBackend::open(db_name)         async — opens/creates IDB database + object store
2. idb.load_all_pages()                    async — single IDB getAll() call
3. BrowserBufferBackend::load_pages(map)   sync  — populate buffer, dirty = empty
4. PersistentFactStorage::new(buffer, ..)  sync  — replay header, B+tree, facts
5. Wrap in Rc<RefCell<BrowserDbInner>>
```

### `db.execute("(transact [...])")`

```
1. parse_datalog_command()                 sync
2. materialize + stamp facts               sync  (same logic as native Minigraf)
3. apply facts to FactStorage              sync
4. pfs.save()                             sync  — updated pages written to BrowserBufferBackend
5. buffer.take_dirty()                    sync  — drain HashSet<u64>
6. idb.write_pages(dirty_pages)           async — one IDB put transaction for all dirty pages
7. return JSON.stringify({transacted: tx_id})
```

### `db.execute("(query [...])")`

```
1. parse_datalog_command()                 sync
2. DatalogExecutor::execute()              sync  — reads FactStorage (in-memory only)
3. serialize QueryResult → JSON            sync
— no IndexedDB access for reads
```

### `db.export_graph()`

```
1. pfs.save()                             sync  — ensure buffer reflects latest state
2. collect pages 0..buffer.page_count()   sync  — sorted by page_id (ascending)
3. concatenate into Vec<u8>               sync
4. return as Uint8Array
```

### `db.import_graph(data)`

```
1. split data into 4KB chunks                    sync
2. BrowserBufferBackend::load_pages_all_dirty()  sync  — populate buffer, ALL pages dirty
3. idb.write_pages(buffer.take_dirty())          async — flush all pages to IndexedDB
4. PersistentFactStorage::new(buffer, ..)        sync  — reload state from fresh pages
5. replace self.inner
```

---

## File Format Compatibility

The `.graph` export is **byte-for-bit compatible** with native `.graph` files.

Verified from source (`storage/mod.rs`, `btree_v6.rs`, `btree.rs`):
- `FileHeader::to_bytes()` / `from_bytes()` — every integer field uses explicit
  `to_le_bytes()` / `from_le_bytes()` (not native byte order)
- `btree_v6.rs` — all integer page fields use `to_le_bytes()` / `from_le_bytes()`
- Packed fact pages — `postcard` encoding (endian-agnostic, varint)

WASM32 is little-endian; x86_64 and ARM64 are little-endian. The explicit LE encoding
means the format is portable even to hypothetical big-endian hosts.

**Constraints on import**:
1. **Export pages in sorted order**: `BrowserBufferBackend` uses a `HashMap` — the
   export must sort page IDs ascending before concatenating. Unsorted output produces
   a corrupt file (header at wrong offset).
2. **Import native files only after checkpoint**: the WAL sidecar (`.graph.wal`) is a
   separate file not present in the browser. If a native file has un-checkpointed WAL
   entries, those writes are absent from the main file and will be silently missing
   after import. Call `db.checkpoint()` on native before exporting for browser import.
   Document this prominently.

---

## Feature Flags

```toml
# Cargo.toml

[features]
default = []
wasm = []          # existing — gates query optimizer only
browser = [        # new — gates all browser WASM code
  "dep:wasm-bindgen",
  "dep:wasm-bindgen-futures",
  "dep:web-sys",
  "dep:js-sys",
]

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen         = { version = "0.2", optional = true }
wasm-bindgen-futures = { version = "0.4", optional = true }
js-sys               = { version = "0.3", optional = true }
web-sys              = { version = "0.3", optional = true, features = [
  "IdbDatabase",
  "IdbFactory",
  "IdbIndex",
  "IdbObjectStore",
  "IdbOpenDbRequest",
  "IdbRequest",
  "IdbTransaction",
  "IdbTransactionMode",
  "Window",
] }
```

All `browser` module code is gated:
```rust
#[cfg(all(target_arch = "wasm32", feature = "browser"))]
```

Native `cargo build` and `cargo test` never compile or link these dependencies.

**Binary size target**: < 1 MB gzipped for `minigraf_bg.wasm`. `wasm-pack` runs
`wasm-opt` automatically in release mode. Audit with `wasm-size` or `twiggy` if the
budget is exceeded.

---

## CI

New file: `.github/workflows/wasm-browser.yml`

```yaml
name: WASM Browser
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
jobs:
  wasm-browser:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-unknown-unknown
      - name: Install wasm-pack
        run: curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh
      - name: Build
        run: wasm-pack build --target web --features browser
      - name: Check gzipped binary size
        run: |
          SIZE=$(gzip -c pkg/minigraf_bg.wasm | wc -c)
          echo "Gzipped WASM size: ${SIZE} bytes"
          [ "$SIZE" -lt 1048576 ] || echo "WARNING: exceeds 1MB gzipped budget"
      - name: Test (headless Chrome)
        run: wasm-pack test --headless --chrome --features browser
```

The existing `rust.yml` (native, macOS, Windows) is not modified.

---

## Tests

### `BrowserBufferBackend` — native `cargo test`

In `src/browser/buffer.rs`, standard `#[test]` functions (no WASM needed):

- Write a page → assert page_id appears in `take_dirty()`
- `take_dirty()` clears the dirty set (second call returns empty)
- `read_page()` after `write_page()` returns identical bytes
- `page_count()` reflects the number of distinct page IDs written
- `load_pages()` pre-populates buffer with no dirty pages

### `BrowserDb` — `wasm-bindgen-test` (headless Chrome)

In `src/browser/mod.rs`, gated `#[cfg(all(target_arch = "wasm32", feature = "browser"))]`:

```rust
#[wasm_bindgen_test]
async fn persistence_round_trip() {
    // open → transact → close → reopen → query
    // must return same result after reopen
}

#[wasm_bindgen_test]
async fn export_import_round_trip() {
    // export_graph() → import_graph() → query
    // results must match; exported bytes must start with b"MGRF"
}

#[wasm_bindgen_test]
async fn read_does_not_touch_indexeddb() {
    // open → execute query (no prior transact) → verify IDB write count = 0
}

#[wasm_bindgen_test]
async fn open_in_memory_no_idb() {
    // open_in_memory() → transact → query → checkpoint → no IDB calls
}
```

### Manual integration test

`examples/browser/` — no bundler, no npm install:

```bash
wasm-pack build --target web --features browser
cd examples/browser
python3 -m http.server 8080
# open http://localhost:8080 in Chrome, check console
```

`app.js` opens a `BrowserDb`, transacts two facts, queries them, logs the JSON result.
If the console shows the expected JSON, the example passes.

---

## Package scope and runtime compatibility

| Package | Target | Storage | Build tool | Phase |
|---------|--------|---------|------------|-------|
| `@minigraf/core` | Browser only | IndexedDB | `wasm-pack --target web` | 8.1a (this) |
| `@minigraf/node` | Node.js / server | Filesystem | `napi-rs` | 8.3 |

`wasm-pack --target web` generates ES-module JS glue that loads the `.wasm` file via
`fetch()` and calls `window.indexedDB` for storage. Neither is available in Node.js in
the same form. A Node.js WASM build would require `--target nodejs` (CommonJS,
`fs.readFileSync` for WASM loading) **and** a different storage backend — that is
exactly what Phase 8.3 provides, as native bindings rather than WASM.

**Do not add a `--target nodejs` build to this phase.** It is out of scope and
would require a different storage backend design.

---

## Out of Scope for this Phase

- Node.js / server-side runtime support — use `@minigraf/node` (Phase 8.3)
- SharedArrayBuffer / WASM threads — `BrowserDb` is single-threaded by design
- Explicit write transactions (`begin_write` / `WriteTransaction`) — single `(transact [...])` batching is sufficient
- On-demand page fetching from IndexedDB — requires async `StorageBackend`, post-1.0
- npm publish — deferred to Phase 8 completion issue (#133)
- Service worker / offline caching — out of scope entirely
