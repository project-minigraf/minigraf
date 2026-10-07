# Binding surface for cursors, open options, the fact log and the log writer — design

**Issues:** #462 (binding cursors), #465 (`open_with_options`), #467 (`LogWriter` and
`FactRecord`). **Core:** #432 (PR #461), #430 (PR #463), #429 (PR #464), #431 (PR #466).
**Branch:** `v3` in core and in every binding repo.

## Goal

Every binding can do what the offline migration in temporal_reasoning#378 needs:
open a source read-only, stream its query answers and its full fact log, and write a
new file from the records. The shape is the same in every binding, with each
language's idiom on top.

## Branching

The bindings pin minigraf from crates.io, but these APIs exist only on the
unreleased `v3` branch. Each binding repo gets a `v3` branch that depends on core by
git revision:

```toml
minigraf = { git = "https://github.com/project-minigraf/minigraf", rev = "<v3 sha>" }
```

Feature PRs target `v3`, and each repo's CI runs on PRs to `v3`. `main` keeps serving
v2.x patch releases. At the v3.0.0 release the dependency switches to
`minigraf = "=3.0.0"` and `v3` becomes `main`, as in core.

## Core additions (this PR)

- **`API-017`** `invalid argument: {}`, from `MinigrafError::invalid_argument(detail)`.
  A binding returns it for an argument it cannot convert, such as an entity id that is
  not a UUID string or a number out of range. The Rust API takes typed arguments and
  never returns it.
- **`API-018`** `{} is closed`, from `MinigrafError::closed(object)`. A binding
  cannot consume an object its caller still holds, so a log writer used after `finish`
  or `close` returns it. Rust consumes these objects and never returns it.
- **`BrowserDb.query(datalog)`** returns a `BrowserCursor` (`vars()`,
  `nextBatch(maxRows)` with JSON rows or `undefined`, `close()`). `transact`, `retract`
  and `rule` are `API-012`, and bind slots are `API-010`, as in `Minigraf::query`.

Bindings build these errors with the core constructors, so every binding reports the
same code and text, and `docs/ERROR_REFERENCE.md` stays the one registry.

## Shared shape

| Concept | UniFFI (Python, Java, Android, Swift) | Node (napi) | C | Browser |
|---|---|---|---|---|
| Open options | `OpenOptions` record, every field optional | options object | JSON object string | not applicable |
| Open | `MiniGrafDb.open_with_options(path, options)` | `MiniGrafDb.openWithOptions(path, options)` | `minigraf_open_with_options(path, json)` | — |
| Cursor | `db.query(datalog) -> MiniGrafCursor` | `db.query(datalog) -> Cursor` | `minigraf_query` | `BrowserDb.query` |
| Cursor rows | JSON string of rows, as in `execute()` | JSON string of rows | JSON string of rows | JSON string of rows |
| Fact log | `db.fact_log(filter) -> MiniGrafFactLog` | `db.factLog(filter)` | `minigraf_fact_log(db, json)` | not exposed |
| Fact record value | `MiniGrafValue` enum | `{ type, value }` object | `{"type", "value"}` JSON | — |
| Log writer | `MiniGrafLogWriter.create(path, options)` | `LogWriter.create(path, options)` | `minigraf_log_writer_create` | not built on wasm32 |

### Decisions

1. **Cursor rows stay JSON.** A batch is a JSON array of rows encoded exactly like the
   `results` of `execute()`, so a cursor and `execute()` give the same rows and every
   binding reuses its existing decoding. `max_rows == 0` counts as 1, a batch is never
   empty, and the end is `None`/`null`/`undefined`. After `close()` a cursor returns
   the end, not an error.
2. **Fact records are lossless.** A migration must write back exactly what it read, so
   `FactRecord.value` keeps the value's type, and a `Ref` and a string with the same
   text stay different. UniFFI bindings get an enum with the variants `Text`, `Int64`,
   `Float64`, `Bool`, `Ref`, `Keyword` and `Null`; these names avoid `String`, `Int`,
   `Float` and `Boolean`, which would shadow built-in types in Kotlin and Swift. JSON
   and Node use `{ "type": "string" | "integer" | "float" | "boolean" | "ref" |
   "keyword" | "null", "value": ... }`. Entity ids and `Ref` values are UUID strings;
   anything else is `API-017`.
3. **64-bit fields stay exact.** `valid_to` is `i64::MAX` for forever. Node uses
   `BigInt` for `txCount`, `txId`, `validFrom` and `validTo`, because a JS number
   cannot hold `i64::MAX`. JSON writes them as integers.
4. **Fact filter.** A record with optional `attributes`, `attribute_prefixes`,
   `entities`, `tx_from`, `tx_to` (inclusive), `order` (`tx` or `storage`) and
   `window`. An absent field does not filter.
5. **`append_batch(records)`** appends in order and stops at the first rejected
   record. The error is that record's coded error with ` (batch index N)` appended,
   and the records before it stay appended, as if appended one by one.
6. **Writer lifetime.** `finish()` consumes the core writer behind a mutex. `close()`
   drops it unfinished, which deletes `<path>.partial`, and is a no-op when already
   finished or closed. Every other call after `finish` or `close` is `API-018`.
   Python's context manager finishes on a clean exit and closes on an exception; Java's
   `close()` (`AutoCloseable`) frees the writer, which abandons an unfinished build.
7. **Options map onto `OpenOptions` builders.** In the UniFFI shim every count
   (options, `tx_count`, `tx_id`, batch sizes, filter bounds) is a signed integer
   (`i64`, `i32` for batch sizes): Kotlin's `ULong`/`UInt` are name-mangled and cannot
   be called from Java. A negative or oversized value is `API-017`. C and JSON use
   unsigned integers and `size_t`. An absent field keeps the Rust default.
8. **Kotlin renames `close`.** UniFFI's Kotlin objects are `AutoCloseable`, and
   `close()` frees the Rust object, which releases a cursor or fact log and abandons an
   unfinished writer. The shim's own `close` methods clash with it, so the Kotlin
   bindings rename them in `uniffi.toml`: `release()` (cursor, fact log) and `abandon()`
   (writer).
9. **Errors keep their code in the message.** Every binding already surfaces
   `[CODE] message` (the `Display` of `MinigrafError`), so API-014, API-015, API-017,
   API-018, STG-042 and STG-043 reach callers like every other code. (UniFFI wraps
   it: Python's `e.msg`, Kotlin's `Minigraf.errorMessage(e)`, Swift's `e.message`.)

### Not applicable

- **Browser (`BrowserDb`, minigraf-wasm):** `read_only`, `allow_unlocked`,
  `page_cache_size`, the WAL options and `LogWriter` are file-backed only. BrowserDb
  keeps its pages in memory, has no WAL and no file lock, and `LogWriter` is not built
  on wasm32. The fact log is not exposed in the browser; the cursor is.
- **WASI package:** it ships the REPL binary, not a library API, so there is nothing to
  expose.
- **`PreparedQuery.query`** follows #181 (v3.1.0).

## Tests

Each binding tests, in its own language:

- cursor batches of 1, 7 and 1,000 equal `execute()`, `close()` after the first batch,
  a cursor opened before a `transact` keeps the earlier answer, and a non-query is
  `API-012`;
- a read-only open with queries working, a write giving `API-014`, a missing file
  `STG-042`, and two read-only handles on one file;
- a round trip through `fact_log` and the log writer that gives the same records and
  `current_tx_count`, a hole from a skipped transaction, `API-015` and `STG-043`, and a
  writer closed without `finish` leaving no file.
