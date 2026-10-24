# Prepared queries in every binding (#181)

**Milestone:** v3.0.0, branch `v3`. No change to the `.graph` format or the WAL format.
**Bindings:** all seven. The UniFFI bindings (`minigraf-binding-template`, `minigraf-python`, `minigraf-java`, `minigraf-android`, `minigraf-swift`), `minigraf-node` (napi-rs), `minigraf-c` (cbindgen) and `minigraf-wasm`, whose `BrowserDb` lives in core (`src/browser/mod.rs`). Each binding changes on its `v3` branch.
**Overlaps:** #180 (`2026-10-24-udf-uniffi-design.md`) edits the same `MiniGrafDb` impl in the UniFFI bindings, and its #734 guards add `INT-001` to `prepare` and `PreparedQuery::execute`/`query`. #306 (`2026-10-24-limit-order-by-design.md`) adds `:limit`/`:offset` slots that raise `INT-033`/`INT-035`. #316 (`2026-10-24-set-values-design.md`) adds `Value::Set`, which a value bind carries. See §9.
**Status:** draft 2026-10-24; in review.

## 1. Decisions

1. **Every binding gets prepared queries.** `prepare(text)` returns a handle with `execute(bindings)` (the JSON `execute` returns today) and `query(bindings)` (the binding's existing cursor). The handle parses and plans once, like the Rust `PreparedQuery`.
2. **Bindings are a map from slot name to bind value.** A slot is named without its `$`. A name the query does not use is ignored, as in Rust.
3. **A bind value is a typed value or a plain host value.** The typed form names all five `BindValue` variants. A plain host value (integer, float, string, boolean, null, and a UUID where the language has a UUID type) binds as a value, using the rules in §4.
4. **A mistake in the bindings is an API error.** The three errors are missing slot, slot in attribute position, and wrong kind of bind value. They are raised today as `INT-033`, `INT-034` and `INT-035`, so the UniFFI bindings report them as "unknown error". They get new `API` codes (§5). The `INT` codes stay registered and are marked deprecated.
5. **Lifetime follows the binding's close.** Where a binding has no close (UniFFI, `BrowserDb`), a prepared handle keeps its database open until both are freed. Where it has one (Node `close()`, C `minigraf_close`), closing the database also releases every prepared query made from it. A prepared handle used after that fails with `API-018` (`[API-018] database is closed`), the existing code for a closed binding object (§6).
6. **`Entity` and `Val(Ref)` are interchangeable.** An entity slot accepts `Val(Value::Ref(u))`, and a value or rule-argument slot accepts `Entity(u)` as `Value::Ref(u)`. Today each is `INT-035` in the other's position. Without this change a host UUID could not be a plain bind value, because its position would decide which variant it needs. This is a core change (§3).

## 2. Terms

- **Slot**: a `$name` in the query text: an entity, value, expression or rule-argument position, `:as-of` or `:valid-at`, and, after #306, `:limit`/`:offset`.
- **Bind value**: what a binding passes for one slot: the binding's typed form of `BindValue`, or a plain host value.
- **Plain value**: a host value of a type listed in §4, given where a bind value is expected. It is never `TxCount`, `Timestamp` or `AnyValidTime`.
- **Prepared handle**: the binding object returned by `prepare`.

## 3. Core changes

Everything here is on `v3`, in one PR with the `BrowserDb` work (§7.4).

- **Error codes** (§5). The three new codes replace every `bail_coded!(ErrorCode::Int033/Int034/Int035, ..)` in `prepared.rs`, with the same arguments and texts. `ErrorCategory::Api`.
- **Entity/Ref** (decision 6). `resolve_entity_slot` accepts `Val(Value::Ref(u))` and gives `EdnValue::Uuid(u)`. `resolve_value_slot` and `resolve_val_slot` accept `Entity(u)` and give `Value::Ref(u)`. Every other mismatch is still the type-mismatch code. The `BindValue::Entity` doc comment that says "Not valid in rule-invocation argument positions — use `Val(Value::Ref(uuid))` there" goes.
- No other change to `PreparedQuery` or `Minigraf::prepare`. The guards from #734 come with #180 PR 0.

Verified on `v3` (2026-10-24, scratch test): a `PreparedQuery` that outlives its `Minigraf` still runs. It also keeps the file registered and locked until the query is dropped. Reopening the path in the same process gives `STG-025`, and from another process `STG-026`. So the Rust handle never reads a file someone else can write. It does mean a binding with a close must drop its prepared queries when it closes (§6).

## 4. Bind values in each binding

The typed form is the same five variants everywhere:

| Variant | Slot positions | Payload |
|---|---|---|
| value | entity (as a `Ref`), value, expression, rule argument | a typed value: the binding's existing value type (§4.1) |
| entity | entity, value, rule argument (decision 6) | a UUID string |
| tx count | `:as-of` | an integer ≥ 0 |
| timestamp | `:as-of`, `:valid-at` | Unix milliseconds |
| any valid time | `:valid-at` | none |

A UUID string that does not parse, or a negative tx count, is `API-017`, as for every other FFI argument.

### 4.1 Typed values

The value variant reuses each binding's existing value type, so a fact read from a fact log can be bound back exactly:

- UniFFI: `MiniGrafValue` (`Text`, `Int64`, `Float64`, `Bool`, `Ref`, `Keyword`, `Null`).
- Node: `FactValue` (`{ type: 'string' | 'integer' | 'float' | 'boolean' | 'ref' | 'keyword' | 'null', value }`).
- C and `BrowserDb`: the same `{"type": ..., "value": ...}` object as Node's, which is also the `value` object of C's fact-log records.

### 4.2 Plain values

| Host value | Binds as |
|---|---|
| boolean | `Val(Boolean)` |
| integer | `Val(Integer)`; outside `i64` is `API-017` |
| float | `Val(Float)` |
| string | `Val(String)`. A string is never read as a keyword or a UUID: use the typed form for those |
| null / `None` / `nil` | `Val(Null)` |
| UUID type (Python `uuid.UUID`, `java.util.UUID`, Swift `UUID`) | `Entity(u)`, which by decision 6 also fits value and rule-argument slots |

Per language:

- **Python** (`minigraf/__init__.py`): `bool` is checked before `int`. `int` and `float` follow their types. Any other type is `TypeError`. The typed forms are exported as `minigraf.Entity(uuid)`, `minigraf.TxCount(n)`, `minigraf.Timestamp(ms)`, `minigraf.ANY_VALID_TIME`, and `minigraf.Value.*` for a typed value. `prepared.execute(age=30)` (keyword arguments) and `prepared.execute({"age": 30})` (a dict) are both accepted.
- **Kotlin/Java** (`Minigraf.kt` in `minigraf-java`, and a new `Minigraf.kt` in `minigraf-android`, both in package `io.github.project_minigraf.minigraf`): `Boolean`; `Byte`, `Short`, `Int` and `Long` as integers; `Float` and `Double` as floats; `String`; `null`; `java.util.UUID`. Kotlin gets `execute(vararg bindings: Pair<String, Any?>)` and `query(...)` extensions. Java gets `Minigraf.execute(prepared, Map<String, Object>)` and `Minigraf.query(...)` statics. Any other type is `IllegalArgumentException`.
- **Swift** (`Sources/MinigrafKit/`): a `MinigrafBindable` protocol adopted by `Bool`, `Int`, `Int32`, `Int64`, `Double`, `Float`, `String`, `UUID` and `MiniGrafBindValue`. The extension methods take `[String: MinigrafBindable?]`, where `nil` is `Null`.
- **Node** (`index.d.ts` + `src/lib.rs`): `boolean`; `bigint` as an integer; a `number` that `Number.isInteger` accepts as an integer and any other `number` as a float; `string`; `null`. `undefined`, and any object that is not a typed form, is a `TypeError`. A float with no fractional part (`2.0`) needs the typed form `{ type: 'float', value: 2 }`. JavaScript has no UUID type.
- **C** (bindings as a JSON object): `true`/`false`; a JSON number without fraction or exponent as an integer; any other number as a float (`serde_json` keeps `2.0` a float); a string; `null`. A nested object must be a typed form.
- **`BrowserDb`**: the same rules as Node, applied to a JavaScript object read with `js_sys` (`Object::entries`, `Reflect`). `bigint` works, so `JSON.stringify` is not used.

Sets (#316): when `Value::Set` lands, each binding's typed value gains its set form, and the plain values gain the host set type (Python `set`/`frozenset`, Kotlin/Java `Set`, Swift `Set`, a JS `Set`, a C/JSON typed form only). Whichever of #316's binding work and this spec's binding work lands second adds this.

## 5. Errors

| Code | Name | Category | Message (unchanged from the `INT` code) |
|---|---|---|---|
| `API-022` (new) | Missing bind value | Api | `missing bind value for slot '$<name>'` |
| `API-023` (new) | Bind slot in attribute position | Api | `bind slot '$<name>' is not permitted in attribute position; ...` |
| `API-024` (new) | Bind value type mismatch | Api | `slot '$<name>' in <position> position requires <kinds>, got <kind>` |
| `INT-033`, `INT-034`, `INT-035` | (deprecated) | Internal | never raised from v3.0.0 |

The numbers are provisional. #316's spec takes `API-021`, and whichever spec merges later takes the next free numbers. `API-023` is raised by `prepare`; the other two by `execute`/`query`.

The binding-side errors:

| Error | UniFFI | Node | C | `BrowserDb` |
|---|---|---|---|---|
| core error | `MiniGrafError` by category, message `[CODE] ...` | `Error`, message `[CODE] ...` | NULL + last error `[CODE] ...` | rejected with `[CODE] ...` (#673) |
| plain value of an unsupported type | language-layer `TypeError` / `IllegalArgumentException` / compile error (Swift) | `TypeError` | `API-017` | `TypeError` |
| bad UUID string, negative tx count, integer outside `i64` | `API-017` | `API-017` | `API-017` | `API-017` |
| prepared handle used after the database closed | n/a | `API-018` (`database is closed`) | NULL + last error `API-018` | n/a |

**UniFFI error variant.** `MiniGrafError` maps both `Api` and `Internal` to `Other`. So, for the change in decision 4 to reach UniFFI callers, `MiniGrafError` gains `Api { msg }`, and `ErrorCategory::Api` maps to it. This also moves the existing binding errors `API-017` and `API-018`, and every other `API` code, from `Other` to `Api`. Adding a variant breaks exhaustive `when`/`switch` in Kotlin and Swift. That is acceptable at v3.0.0 and is listed in the upgrade guide. `MinigrafKit`'s `MiniGrafError.message` gains the case.

## 6. Lifetime and threading

- **UniFFI.** `MiniGrafPreparedQuery { db: Arc<Mutex<minigraf::Minigraf>>, inner: minigraf::PreparedQuery }`. The handle holds the database's `Arc`, so the close-time checkpoint runs when the last of the database and its prepared handles is freed. `execute`/`query` do not lock `db`: `PreparedQuery` reads its own snapshot (#554/#555) and is `Send + Sync`. So prepared queries run concurrently with each other and with writes on other threads.
- **`BrowserDb`.** `BrowserPreparedQuery` holds the `Rc<RefCell<BrowserDbInner>>` and the `PreparedQuery`. `execute` is synchronous and returns the JSON string, as `query` is today (a read needs no IndexedDB flush).
- **Node.** `MiniGrafDb` gains a list of `Weak` slots, one per prepared handle it made. Each slot holds an `Option<PreparedQuery>`. `close()` empties every live slot, then drops the database, so the file lock is released and the documented reopen after `close()` works. A prepared handle whose slot is empty throws `MinigrafError::closed("database")` (`API-018`). Node's existing `CLOSED_MSG` error for the database's own methods is unchanged.
- **C.** The same as Node, with `minigraf_close` as the close. `minigraf_prepared_free` must still be called after `minigraf_close`: it frees the handle's own memory. Using a handle after `minigraf_prepared_free` is undefined behaviour, as for every C handle.
- Cursors from `query(bindings)` are the binding's existing cursor objects and follow their existing rules. The plan checks whether an open Node or C cursor or fact log already blocks a reopen after `close()` (each holds a snapshot). If it does, that is filed as its own issue, not fixed here.
- With #180, a UDF may call a prepared query (#180 §5).

## 7. API per binding

### 7.1 UniFFI (template, python, java, android, swift)

```rust
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum MiniGrafBindValue {
    Value { value: MiniGrafValue },
    Entity { uuid: String },
    TxCount { value: i64 },
    Timestamp { value: i64 },
    AnyValidTime,
}

#[uniffi::export]
impl MiniGrafDb {
    /// Parse and plan a `(query ...)` once. Slots are `$name`.
    pub fn prepare(&self, datalog: String) -> Result<Arc<MiniGrafPreparedQuery>, MiniGrafError>;
}

#[derive(uniffi::Object)]
pub struct MiniGrafPreparedQuery { /* §6 */ }

#[uniffi::export]
impl MiniGrafPreparedQuery {
    /// The query's result as JSON, encoded like `MiniGrafDb.execute`.
    pub fn execute(&self, bindings: HashMap<String, MiniGrafBindValue>) -> Result<String, MiniGrafError>;
    /// A cursor over the rows; its answer is fixed when it opens.
    pub fn query(&self, bindings: HashMap<String, MiniGrafBindValue>) -> Result<Arc<MiniGrafCursor>, MiniGrafError>;
}
```

The language layers (§4.2) wrap these. Written once in `minigraf-binding-template/src/lib.rs` and copied, as with existing items.

### 7.2 Node

```ts
type BindValue =
  | FactValue
  | { type: 'entity'; value: string }
  | { type: 'txCount'; value: bigint | number }
  | { type: 'timestamp'; value: bigint | number }
  | { type: 'anyValidTime' }
  | boolean | number | bigint | string | null;

class MiniGrafDb { prepare(datalog: string): PreparedQuery }
class PreparedQuery {
  execute(bindings: Record<string, BindValue>): string
  query(bindings: Record<string, BindValue>): Cursor
}
```

### 7.3 C

```c
typedef struct MiniGrafPrepared MiniGrafPrepared;
// NULL on error (minigraf_last_error on handle).
MiniGrafPrepared *minigraf_prepare(MiniGrafDb *handle, const char *datalog);
// bindings_json: a JSON object, slot name -> bind value (§4); NULL means {}.
// Returns JSON (free with minigraf_string_free), or NULL on error (minigraf_prepared_last_error).
char *minigraf_prepared_execute(MiniGrafPrepared *prepared, const char *bindings_json);
MiniGrafCursor *minigraf_prepared_query(MiniGrafPrepared *prepared, const char *bindings_json);
// The error of the last call on prepared, or NULL. Valid until the next call on it.
const char *minigraf_prepared_last_error(MiniGrafPrepared *prepared);
void minigraf_prepared_free(MiniGrafPrepared *prepared);
```

`include/minigraf.h` is regenerated by cbindgen.

### 7.4 `BrowserDb` (core, `browser` feature)

```rust
#[wasm_bindgen] impl BrowserDb {
    pub fn prepare(&self, datalog: &str) -> Result<BrowserPreparedQuery, JsValue>;
}
#[wasm_bindgen] impl BrowserPreparedQuery {
    pub fn execute(&self, bindings: JsValue) -> Result<String, JsValue>;
    pub fn query(&self, bindings: JsValue) -> Result<BrowserCursor, JsValue>;
}
```

`minigraf-wasm` re-exports core, so it only gets a pin bump, a test and a README section.

## 8. Tests

**Core**:
- `tests/prepared_test.rs` (or the existing prepared tests): each new code at each raise site, in a query, a `not` body, an `or` branch and a rule argument; `Entity` in value and rule-argument slots and `Val(Ref)` in an entity slot return the same rows as the matching variant.
- `src/error.rs` registry tests; `docs/ERROR_REFERENCE.md` sync test.
- `BrowserDb`: `wasm_bindgen_test` cases for prepare, execute and query with every plain and typed form, `bigint`, a `TypeError`, and repeated execution.

**Every binding** (Rust unit tests where the binding has them, plus language tests: pytest, Kotlin/JVM and Java interop, Kotlin on Android, XCTest, Node test runner, the C test program):
- one prepared query executed at least three times with different bindings, each giving the matching rows (#181 acceptance);
- every typed variant, and every plain host type of §4.2;
- `query(bindings)` read to the end;
- each error row of §5 that applies to the binding;
- Node and C: prepared handle after close fails with `API-018`, and the same path reopens after close while a prepared handle is still alive;
- UniFFI: the database freed before the prepared handle; the handle still runs.

The consumer-test contract (`consumer-test/run.sh`, or the binding's equivalent) gains one prepared query with two executions, so the published package is checked.

## 9. Interplay

- **#180.** Both change `MiniGrafDb`'s impl in the same files. The binding PRs of whichever lands second rebase onto the other. #180's PR 0 (#734) adds `INT-001` to `prepare`/`PreparedQuery`; nothing here depends on it.
- **#306.** Its `:limit`/`:offset` resolver raises `INT-033`/`INT-035`. Whichever PR lands second switches those raises to `API-022`/`API-024`. Plain integers bind there as `Val(Integer)`, as #306 expects.
- **#316.** §4.2's last paragraph. `API-021` is #316's.
- **Codes.** #180's and #316's specs both claim `QRY-014`. That is outside this spec but has to be settled before either merges.

## 10. Documentation

- `docs/ERROR_REFERENCE.md`, `src/error.rs`: §5's codes; `INT-033`–`INT-035` marked deprecated.
- `CHANGELOG.md` (v3.0.0): `Added` prepared queries in every binding and `BrowserDb.prepare`; `Changed` the three codes (a breaking change for anyone matching `INT-033`–`INT-035`), `Entity`/`Val(Ref)` interchangeable, `MiniGrafError::Api`.
- `ROADMAP.md`: #181 moves to "done on `v3`" when the last binding PR merges.
- `prepared.rs` doc comments: the `BindValue` table gains decision 6.
- Docs site (`../minigraf-docs`): `content/tutorial-08-prepared-queries.md` gains an example per binding, and `content/upgrading-to-v3.md` gains the code change and `MiniGrafError::Api`, both wrapped in `<!-- @since v3.0.0 -->`.
- Each binding README: a prepared-query section with the typed and plain forms, and §6's lifetime rule.

## 11. PRs

1. **Core, on `v3`**: §3, §5, §7.4, core and wasm tests, docs. Docs-site PR merged after it.
2. **`minigraf-binding-template`**, on `v3`: §7.1 and its Rust tests.
3. **`minigraf-python`, `minigraf-java`, `minigraf-android`, `minigraf-swift`**, each on `v3`: bump the pin to PR 1's merge; add §7.1, `MiniGrafError::Api`, the §4.2 language layer, tests, consumer test and README. One PR per binding.
4. **`minigraf-node`** and **`minigraf-c`**, each on `v3`: pin bump, §7.2 / §7.3, §6, tests, consumer test, README.
5. **`minigraf-wasm`**, on `v3`: pin bump, browser consumer test, README.

The size gate checks PR 1. No growth beyond a few KiB is expected: the core change is two resolver arms and three codes, and the `BrowserDb` code compiles only with `browser`.
