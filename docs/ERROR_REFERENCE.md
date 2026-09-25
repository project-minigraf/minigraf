# Minigraf Error Reference

This document covers every user-facing error produced by the core Minigraf Rust library.
Errors surface as `MinigrafError` values returned from `db.execute()`, `db.prepare()`,
and related API methods.

**Reference codes** (e.g. `PRS-001`) appear in runtime error output as
`[CODE] message`, and programmatically via `MinigrafError::code()` /
`MinigrafError::category()`. As of [#277](https://github.com/project-minigraf/minigraf/issues/277),
every `bail!`/`anyhow!` call site in the crate has been migrated to a specific
structured code — `INT-000` is now reserved for the rare case of a raw,
non-`anyhow` error (e.g. an `io::Error`) reaching the public API boundary
with no `CodedError` anywhere in its chain.

**Out of scope**: FFI/binding errors from `minigraf-python`, `minigraf-node`, and
`minigraf-wasm` are handled in those repos.

---

## Quick Reference Table

| Code | Error (prefix) | Category |
|------|----------------|----------|
| PRS-001 | Unexpected end of input | Parser |
| PRS-002 | Unexpected character | Parser |
| PRS-003 | Unexpected token | Parser |
| PRS-004 | Unclosed vector | Parser |
| PRS-005 | Unclosed list | Parser |
| PRS-006 | Unterminated map | Parser |
| PRS-007 | String exceeds maximum length | Parser |
| PRS-008 | Keyword exceeds maximum length | Parser |
| PRS-009 | Tagged literal exceeds maximum length | Parser |
| PRS-010 | Expected command symbol | Parser |
| PRS-011 | Unknown command | Parser |
| PRS-012 | Expected a list starting with command | Parser |
| PRS-013 | Query requires a map argument | Parser |
| PRS-014 | :as-of requires a value | Parser |
| PRS-015 | :as-of counter must be non-negative | Parser |
| PRS-016 | :as-of must be integer or ISO 8601 | Parser |
| PRS-017 | :valid-at requires a value | Parser |
| PRS-018 | :valid-at must be ISO 8601 or :any-valid-time | Parser |
| PRS-019 | :valid-from must be ISO 8601 | Parser |
| PRS-020 | :valid-to must be ISO 8601 | Parser |
| PRS-021 | :with requires aggregate in :find | Parser |
| PRS-022 | :with variable not bound in :where | Parser |
| PRS-023 | Aggregate variable not bound in :where | Parser |
| PRS-024 | Aggregate expression must have 2 elements | Parser |
| PRS-025 | Aggregate function name must be a symbol | Parser |
| PRS-026 | Aggregate argument must be a variable | Parser |
| PRS-027 | Window function requires :over clause | Parser |
| PRS-028 | Window expression cannot be empty | Parser |
| PRS-029 | Window function name must be a symbol | Parser |
| PRS-030 | lag/lead not supported in this version | Parser |
| PRS-031 | Function is not window-compatible | Parser |
| PRS-032 | Function requires variable before :over | Parser |
| PRS-033 | Function requires :over after variable | Parser |
| PRS-034 | Function requires :over after function name | Parser |
| PRS-035 | :over must be followed by a list | Parser |
| PRS-036 | Unexpected tokens after :over clause | Parser |
| PRS-037 | :partition-by requires a variable | Parser |
| PRS-038 | :order-by requires a variable | Parser |
| PRS-039 | Unknown option in :over clause | Parser |
| PRS-040 | Unexpected element in :over clause | Parser |
| PRS-041 | Transact requires a vector of facts | Parser |
| PRS-042 | Transact argument must be a vector | Parser |
| PRS-043 | Retract requires a vector of facts | Parser |
| PRS-044 | Retract argument must be a vector | Parser |
| PRS-045 | Each fact must be a vector [e a v] | Parser |
| PRS-046 | Fact must have at least 3 elements | Parser |
| PRS-047 | Optional 4th fact element must be a map | Parser |
| PRS-048 | Transact with options requires facts vector | Parser |
| PRS-049 | Unexpected end of fact vector | Parser |
| PRS-050 | Empty list in :where clause | Parser |
| PRS-051 | (not) cannot appear inside another (not) | Parser |
| PRS-052 | (not) requires at least one clause | Parser |
| PRS-053 | (or) requires at least one branch | Parser |
| PRS-054 | (or-join) requires join-vars and branch | Parser |
| PRS-055 | (or-join) first argument must be a vector | Parser |
| PRS-056 | (or-join) join variables must be logic variables | Parser |
| PRS-057 | (or) branches must introduce same variables | Parser |
| PRS-058 | (and) requires at least one clause | Parser |
| PRS-059 | (not-join) requires join-vars and clause | Parser |
| PRS-060 | Expected pattern or rule in :where clause | Parser |
| PRS-061 | Unexpected element in query | Parser |
| PRS-062 | Expression list cannot be empty | Parser |
| PRS-063 | Expression head must be a symbol | Parser |
| PRS-064 | Function takes exactly 1 argument | Parser |
| PRS-065 | Function takes exactly 2 arguments | Parser |
| PRS-066 | matches? second argument must be string literal | Parser |
| PRS-067 | Unknown expression operator | Parser |
| PRS-068 | Expression clause must be [(expr)] or [(expr) ?out] | Parser |
| PRS-069 | Expression output must be a ?variable | Parser |
| PRS-070 | Unsupported expression argument | Parser |
| PRS-071 | Expected UUID string after #uuid | Parser |
| PRS-072 | Invalid UUID | Parser |
| PRS-073 | Unknown tagged literal | Parser |
| PRS-074 | Bind slot name exceeds maximum length | Parser |
| PRS-075 | Transact rejects unexpected trailing argument(s) | Parser |
| PRS-076 | Retract rejects unexpected trailing argument(s) | Parser |
| PRS-077 | Unexpected trailing input after a complete form | Parser |
| PRS-078 | Query rejects unexpected trailing argument(s) | Parser |
| PRS-079 | Rule rejects unexpected trailing argument(s) | Parser |
| QRY-001 | Invalid entity | Query Execution |
| QRY-002 | Attribute must be a keyword | Query Execution |
| QRY-003 | Cannot transact a pseudo-attribute | Query Execution |
| QRY-004 | Invalid value | Query Execution |
| QRY-005 | Transaction failed | Query Execution |
| QRY-006 | Retraction failed | Query Execution |
| QRY-007 | Unknown predicate | Query Execution |
| QRY-008 | Functions lock poisoned | Query Execution |
| QRY-009 | Rules lock poisoned | Query Execution |
| STG-001 | Invalid header: too short | Storage |
| STG-002 | Invalid magic number: not a .graph file | Storage |
| STG-003 | Invalid v4/v5/v6 header too short | Storage |
| STG-004 | Invalid v6 header too short | Storage |
| STG-005 | Invalid v7 header too short | Storage |
| STG-006 | Unsupported format version | Storage |
| STG-007 | page_count must be greater than 0 | Storage |
| STG-008 | eavt_root_page must be less than page_count | Storage |
| STG-009 | fact_page_count cannot exceed page_count | Storage |
| STG-010 | Failed to read header from existing file | Storage |
| STG-011 | Internal page has no children | Storage |
| STG-012 | Expected index page at page N | Storage |
| STG-013 | range_scan expected leaf | Storage |
| STG-014 | Expected packed page type | Storage |
| STG-015 | Record extends beyond page boundary | Storage |
| STG-016 | Backend mutex poisoned | Storage |
| STG-017 | Page count overflow: index_start | Storage |
| STG-018 | Page count overflow: next_free | Storage |
| STG-019 | Page count overflow: new_fact_start | Storage |
| STG-020 | Fact index exceeds u16::MAX | Storage |
| STG-021 | Page id overflow in checksum computation | Storage |
| STG-022 | Page id overflow writing fact pages | Storage |
| STG-023 | Page index exceeds u64::MAX | Storage |
| STG-024 | Pending fact count exceeds u64::MAX | Storage |
| STG-025 | Database already open in this process | Storage |
| STG-026 | Database locked by another process | Storage |
| STG-027 | Filesystem does not support file locking | Storage |
| WAL-001 | Invalid WAL magic number | WAL |
| WAL-002 | Unsupported WAL version | WAL |
| WAL-003 | Fact serialised size exceeds maximum | WAL |
| WAL-004 | Fact serialised size exceeds u32 range | WAL |
| WAL-005 | WAL num_facts exceeds platform usize | WAL |
| WAL-006 | Failed to delete WAL file | WAL |
| API-001 | Write lock poisoned | Database API |
| API-002 | Unexpected command variant in write path | Database API |
| API-003 | Attribute must be a keyword | Database API |
| API-004 | Cannot transact a pseudo-attribute | Database API |
| API-005 | Only query commands can be prepared (transact) | Database API |
| API-006 | Only query commands can be prepared (retract) | Database API |
| API-007 | Only query commands can be prepared (rule) | Database API |
| API-008 | Function registry lock poisoned | Database API |
| API-009 | WAL not initialized | Database API |
| INT-000 | Unclassified internal error | Internal |
| INT-001 | WriteTransaction already in progress on this thread | Internal |
| INT-002 | Invalid entity (API layer) | Internal |
| INT-003 | Invalid value (API layer) | Internal |
| INT-004 | Invalid timestamp | Internal |
| INT-005 | Millisecond value outside supported datetime range | Internal |
| INT-006 | Internal parser error: expected token | Internal |
| INT-007 | Float literal out of range | Internal |
| INT-008 | Integer literal out of range | Internal |
| INT-009 | Symbol exceeds maximum length | Internal |
| INT-010 | Exceeded maximum recursion depth | Internal |
| INT-011 | Unexpected end of clause | Internal |
| INT-012 | Duplicate query option | Internal |
| INT-013 | Query option requires a value | Internal |
| INT-014 | Query option must be >= 1 | Internal |
| INT-015 | Query option must be a positive integer | Internal |
| INT-016 | (or)/(or-join) nested inside (not)/(not-join) | Internal |
| INT-017 | Pseudo-attribute not valid in this position | Internal |
| INT-018 | Variable not bound by any outer/earlier clause | Internal |
| INT-019 | Datalog parser internal error | Internal |
| INT-020 | Evaluator iteration/result limit exceeded | Internal |
| INT-021 | Evaluator: unsupported where-clause in evaluate_rule | Internal |
| INT-022 | Datalog evaluator internal error | Internal |
| INT-023 | Packed page: fact exceeds slot capacity | Internal |
| INT-024 | Packed page: malformed page data | Internal |
| INT-025 | Unsubstituted :valid-at bind slot reached executor | Internal |
| INT-026 | Temporal pseudo-attributes require :any-valid-time | Internal |
| INT-027 | Query has no :where clause, rules, or aggregates | Internal |
| INT-028 | Rule invocation argument count mismatch | Internal |
| INT-029 | Unknown aggregate function | Internal |
| INT-030 | Unknown window function | Internal |
| INT-031 | or-join variable not bound in incoming scope | Internal |
| INT-032 | Query executor internal error | Internal |
| INT-033 | Missing bind value for slot | Internal |
| INT-034 | Bind slot not permitted in attribute position | Internal |
| INT-035 | Bind slot type mismatch | Internal |
| INT-036 | Header too short: need N..M bytes | Internal |
| INT-037 | Header slice not the exact expected byte length | Internal |
| INT-038 | Header too short for a specific field | Internal |
| INT-039 | Invalid magic number | Internal |
| INT-040 | Function/predicate name already registered | Internal |
| INT-041 | sum: unsupported value type | Internal |
| INT-042 | min/max: no non-null values in group | Internal |
| INT-043 | Cannot compare values of different types | Internal |
| INT-044 | Aggregate: unsupported value type | Internal |
| INT-045 | Pending fact index out of bounds | Internal |
| INT-046 | Missing CommittedFactReader for a committed FactRef | Internal |
| INT-047 | into_backend: backend Arc has multiple owners | Internal |
| INT-048 | Storage: arithmetic overflow | Internal |
| INT-049 | Storage: internal invariant violation | Internal |
| INT-050 | Internal lock poisoned | Internal |
| INT-051 | Invalid page size | Internal |
| INT-052 | Page not found | Internal |
| INT-053 | Header checksum mismatch: possible file corruption | Internal |
| INT-054 | Unstratifiable negative recursion cycle | Internal |
| INT-055 | Rule predicate disappeared during rollback | Internal |

---

## PRS — Parser Errors

Parser errors occur when Minigraf cannot parse the Datalog/EDN input string.
They are returned immediately from `db.execute()` before any fact is read or written.

See the [Datalog Reference](../../.wiki/Datalog-Reference.md) for syntax guidance.

### PRS-001 Unexpected end of input

**Error text**: `Unexpected end of input`

**Cause**: Input cut off before parser completed an expression. Happens when `(`, `[`, or `{` opened but never closed, or empty string passed to `execute()`.

**Resolution**:
- Ensure every `(` → `)`, `[` → `]`, `{` → `}`.
- Use REPL multi-line mode for long queries.

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :name "alice"]
```
*(missing closing `]` and `}`)*

### PRS-002 Unexpected character

**Error text**: `Unexpected character: {}`

**Cause**: Tokeniser encountered a character not valid in Datalog/EDN. Common culprits: `@`, `#` outside a tagged literal, `\`, smart quotes.

**Resolution**:
- Use only plain ASCII in attribute names and keywords.
- String values may contain any UTF-8.
- For UUIDs use `#uuid "..."` tagged literal form.

**Example**:
```datalog
(transact [[@entity :name "alice"]])
```
*`@` is not a valid EDN character; use a UUID or string entity ID instead*

### PRS-003 Unexpected token

**Error text**: `Unexpected token: {}`

**Cause**: Parser encountered a token in a position where it cannot appear. Often caused by missing/misplaced delimiter, or keyword where symbol/value expected.

**Resolution**:
- Check surrounding delimiters for balance.
- Consult the [Datalog Reference](../../.wiki/Datalog-Reference.md) for expected syntax at that position.

**Example**:
```datalog
(query :find [?e] :where [[?e :name "alice"]])
```
*`:find` must be inside a map `{}`; use `(query {:find [?e] :where [...]})`*

### PRS-004 Unclosed vector

**Error text**: `Unclosed vector`

**Cause**: A `[` was opened but never closed with `]`. Check fact vectors in `transact` and pattern vectors in `:where`.

**Resolution**:
- Ensure every `[` is matched with `]`.

**Example**:
```datalog
(transact [[:alice :name "Alice"]
```
*(missing closing `]` for the outer vector)*

### PRS-005 Unclosed list

**Error text**: `Unclosed list`

**Cause**: A `(` was opened but never closed with `)`. Check command forms and `not`/`or` clauses.

**Resolution**:
- Ensure every `(` is matched with `)`.

**Example**:
```datalog
(query {:find [?e]
        :where [(not [?e :deleted true])
```
*(missing closing `)` for the `not` clause and `}`)*

### PRS-006 Unterminated map

**Error text**: `Unterminated map: missing '}'`

**Cause**: A `{` was opened but never closed with `}`. Check query maps and fact option maps.

**Resolution**:
- Ensure every `{` is matched with `}`.

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :name "alice"]]
```
*(missing closing `}`)*

### PRS-007 String exceeds maximum length

**Error text**: `String exceeds maximum length of {} bytes`

**Cause**: A string value in the input exceeds 4096 bytes. Minigraf limits string lengths to keep the parser bounded.

**Resolution**:
- Store large strings externally and reference them with a path or URL string attribute.
- Use `Value::Ref` to point to a dedicated entity.

**Example**:
```datalog
(transact [[#uuid "..." :doc/content "<string longer than 4096 bytes>"]])
```
*Truncate or externalise the value*

### PRS-008 Keyword exceeds maximum length

**Error text**: `Keyword exceeds maximum length of {} bytes`

**Cause**: An attribute keyword in the input exceeds 4096 bytes.

**Resolution**:
- Shorten the attribute name.
- Attribute names should be brief and namespaced, e.g. `:namespace/attr`.

**Example**:
```datalog
(transact [[#uuid "..." :<4097-character-keyword> "value"]])
```
*Shorten the attribute keyword*

### PRS-009 Tagged literal exceeds maximum length

**Error text**: `Tagged literal exceeds maximum length of {} bytes`

**Cause**: The string inside a `#uuid "..."` or other tagged literal exceeds 4096 bytes. UUIDs are 36 characters; anything longer indicates a malformed input.

**Resolution**:
- UUIDs must be 36 ASCII characters in the form `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
- Check for accidental concatenation or copy-paste errors.

**Example**:
```datalog
#uuid "<string longer than 4096 characters>"
```
*A valid UUID is exactly 36 characters*

### PRS-010 Expected command symbol

**Error text**: `Expected command symbol`

**Cause**: The top-level form must start with `transact`, `retract`, `query`, or `rule`. The form was empty or started with a non-symbol token.

**Resolution**:
- Ensure the input is a list beginning with a valid command symbol.

**Example**:
```datalog
(:transact [[:alice :name "Alice"]])
```
*`:transact` is a keyword, not a symbol; use `transact` (no colon)*

### PRS-011 Unknown command

**Error text**: `Unknown command: {}`

**Cause**: The opening symbol is not a recognised command. Check spelling.

**Resolution**:
- Valid commands are `transact`, `retract`, `query`, `rule`.
- Check for typos.

**Example**:
```datalog
(upsert [[:alice :name "Alice"]])
```
*`upsert` is not a command; use `transact`*

### PRS-012 Expected a list starting with a command symbol

**Error text**: `Expected a list starting with a command symbol`

**Cause**: The input is not a list form at all — a bare keyword, integer, map, or vector was passed to `execute()`.

**Resolution**:
- Wrap the command in a list: `(transact [...])`, `(query {...})`, etc.

**Example**:
```datalog
{:find [?e] :where [[?e :name "alice"]]}
```
*A bare map is not a valid command; use `(query {:find [?e] :where [...]})`*

### PRS-013 Query requires a map argument

**Error text**: `Query requires a map argument`

**Cause**: The `query` command expects its sole argument to be a map `{:find [...] :where [...]}`. A vector, symbol, or other form was passed instead.

**Resolution**:
- Wrap the query clauses in `{}`: `(query {:find [?e] :where [[?e :name "alice"]]})`.

**Example**:
```datalog
(query [:find ?e :where [?e :name "alice"]])
```
*The argument is a vector `[...]`; it must be a map `{...}`*

### PRS-014 :as-of requires a value

**Error text**: `:as-of requires a value`

**Cause**: The `:as-of` keyword appeared in the query map with no value following it.

**Resolution**:
- Add a value: `:as-of 5` (transaction count) or `:as-of "2024-01-01T00:00:00Z"` (ISO 8601 wall-clock time).
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :as-of})
```
*`:as-of` must be followed by an integer or ISO 8601 string*

### PRS-015 :as-of counter must be non-negative

**Error text**: `:as-of counter must be non-negative, got {}`

**Cause**: Transaction counters start at 1. A negative integer was passed to `:as-of`.

**Resolution**:
- Use a non-negative integer.
- `:as-of 0` returns the database before any transaction; `:as-of 1` returns the state after the first transaction.
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :as-of -1})
```
*`:as-of` accepts non-negative integers (transaction count) or ISO 8601 strings*

### PRS-016 :as-of must be integer or ISO 8601 string

**Error text**: `:as-of must be an integer (counter) or ISO 8601 string, got {}`

**Cause**: `:as-of` value is neither an integer transaction count nor an ISO 8601 timestamp string.

**Resolution**:
- Use `42` (tx-count) or `"2024-01-01T00:00:00Z"` (wall-clock).
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :as-of :now})
```
*`:now` is not a valid value; use an integer or ISO 8601 string*

### PRS-017 :valid-at requires a value

**Error text**: `:valid-at requires a value`

**Cause**: The `:valid-at` keyword appeared with no value following it.

**Resolution**:
- Add an ISO 8601 timestamp: `:valid-at "2024-06-01T00:00:00Z"`.
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :valid-at})
```
*`:valid-at` requires an ISO 8601 timestamp string or `:any-valid-time`*

### PRS-018 :valid-at must be ISO 8601 or :any-valid-time

**Error text**: `:valid-at must be an ISO 8601 string or :any-valid-time, got {}`

**Cause**: `:valid-at` value is not an ISO 8601 string or the special keyword `:any-valid-time`.

**Resolution**:
- Use `"2024-06-01T00:00:00Z"` or `:any-valid-time`.
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :valid-at 42})
```
*Use a string: `:valid-at "2024-06-01T00:00:00Z"`*

### PRS-019 :valid-from must be ISO 8601

**Error text**: `:valid-from must be an ISO 8601 string, got {}`

**Cause**: The `:valid-from` key in a fact's option map must be an ISO 8601 string, not an integer or keyword.

**Resolution**:
- Use `"2024-01-01T00:00:00Z"` format.
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(transact [[#uuid "..." :name "Alice" {:valid-from 1704067200000}]])
```
*Use `:valid-from "2024-01-01T00:00:00Z"` (ISO 8601), not a Unix timestamp integer*

### PRS-020 :valid-to must be ISO 8601

**Error text**: `:valid-to must be an ISO 8601 string, got {}`

**Cause**: The `:valid-to` key in a fact's option map must be an ISO 8601 string.

**Resolution**:
- Use `"2024-12-31T23:59:59Z"` format.
- To express "forever", omit `:valid-to` (the default is `VALID_TIME_FOREVER`).
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(transact [[#uuid "..." :name "Alice" {:valid-to :forever}]])
```
*Omit `:valid-to` for open-ended validity, or use an ISO 8601 string*

### PRS-021 :with requires aggregate in :find

**Error text**: `':with' clause requires at least one aggregate in :find`

**Cause**: `:with` is used to group result rows before aggregation, so it requires at least one aggregate function (`count`, `sum`, `avg`, etc.) in `:find`.

**Resolution**:
- Add an aggregate to `:find`, or remove `:with` if no aggregation is needed.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [?e ?name]
        :with [?order]
        :where [[?e :name ?name] [?e :order ?order]]})
```
*`:with` needs an aggregate in `:find`, e.g. `(count ?order)`*

### PRS-022 :with variable not bound in :where

**Error text**: `':with' variable {} not bound in :where`

**Cause**: A variable listed in `:with` does not appear in any `:where` pattern.

**Resolution**:
- Add a `:where` clause that binds the variable, or remove it from `:with`.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [(count ?e)]
        :with [?unbound]
        :where [[?e :name ?n]]})
```
*`?unbound` must be bound by a `:where` pattern*

### PRS-023 Aggregate variable not bound in :where

**Error text**: `Aggregate variable {} not bound in :where`

**Cause**: An aggregate's input variable (e.g. `(sum ?amount)`) is not bound by any `:where` pattern.

**Resolution**:
- Add a `:where` clause that binds the variable.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [(sum ?amount)]
        :where [[?e :name ?n]]})
```
*`?amount` must be bound: add `[?e :payment/amount ?amount]` to `:where`*

### PRS-024 Aggregate expression must have exactly 2 elements

**Error text**: `Aggregate expression must have exactly 2 elements (func ?var), got {}`

**Cause**: An aggregate expression in `:find` must be `(function ?variable)` — exactly two elements.

**Resolution**:
- Use `(count ?e)`, `(sum ?amount)`, `(avg ?score)` — one function name, one variable.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [(sum ?amount :distinct)]
        :where [[?e :payment/amount ?amount]]})
```
*`:distinct` is not part of the aggregate syntax; use `(sum ?amount)` alone*

### PRS-025 Aggregate function name must be a symbol

**Error text**: `Aggregate function name must be a symbol, got {}`

**Cause**: The aggregate function name must be an unqualified symbol, not a keyword.

**Resolution**:
- Use `sum`, not `:sum`.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [(:sum ?amount)]
        :where [[?e :payment/amount ?amount]]})
```
*Use `(sum ?amount)` not `(:sum ?amount)`*

### PRS-026 Aggregate argument must be a variable

**Error text**: `Aggregate argument must be a variable (starting with ?)`

**Cause**: The argument to an aggregate function must be a logic variable beginning with `?`.

**Resolution**:
- Use a `?variable`, not a literal value.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [(sum 42)]
        :where [[?e :payment/amount ?amount]]})
```
*Use `(sum ?amount)` with a variable, not a literal*

### PRS-027 Window function requires :over clause

**Error text**: `'{}' is a window function and requires an ':over (...)' clause`

**Cause**: Window functions (`rank`, `row-number`, `dense-rank`, `ntile`, `percent-rank`, `cume-dist`) must be accompanied by an `:over` clause specifying ordering and/or partitioning.

**Resolution**:
- Add an `:over` clause: `(rank :over (:order-by ?score))`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [?e (rank)]
        :where [[?e :score ?score]]})
```
*Missing `:over`; use `(rank :over (:order-by ?score))`*

### PRS-028 Window expression cannot be empty

**Error text**: `window expression cannot be empty`

**Cause**: An `:over ()` clause was provided with no options inside it.

**Resolution**:
- Provide at least `:order-by` or `:partition-by` inside the `:over` list.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [?e (rank :over ())]
        :where [[?e :score ?score]]})
```
*Add `:order-by`: `(rank :over (:order-by ?score))`*

### PRS-029 Window function name must be a symbol

**Error text**: `window function name must be a symbol`

**Cause**: The window function name token is not a symbol (e.g. a keyword was used).

**Resolution**:
- Use `rank`, not `:rank`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [?e (:rank :over (:order-by ?score))]
        :where [[?e :score ?score]]})
```
*Use `(rank :over ...)` not `(:rank :over ...)`*

### PRS-030 lag/lead not supported in this version

**Error text**: `'{}' is not supported in this version; lag/lead are planned for a future release`

**Cause**: The `lag` and `lead` window functions are not yet implemented in this version.

**Resolution**:
- Remove `lag`/`lead` from the query.
- Follow [#182](https://github.com/project-minigraf/minigraf/issues/182) for progress on `lag`/`lead` implementation.

**Example**:
```datalog
(query {:find [?e (lag ?score :over (:order-by ?ts))]
        :where [[?e :score ?score] [?e :ts ?ts]]})
```
*`lag` is not yet available; restructure the query without it*

### PRS-031 Function is not window-compatible

**Error text**: `'{}' is not window-compatible and cannot be used with ':over'`

**Cause**: The named function is a plain aggregate, not a window function. Only `rank`, `row-number`, `dense-rank`, `ntile`, `percent-rank`, and `cume-dist` support `:over`.

**Resolution**:
- Drop the `:over` clause and use the function as a plain aggregate.
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates).

**Example**:
```datalog
(query {:find [(sum ?amount :over (:order-by ?ts))]
        :where [[?e :amount ?amount] [?e :ts ?ts]]})
```
*`sum` is an aggregate; use `(sum ?amount)` without `:over`*

### PRS-032 Function requires variable argument before :over

**Error text**: `'{}' requires a variable argument (starting with ?) before ':over'`

**Cause**: `ntile` requires a variable and then an `:over` clause: `(ntile ?bucket :over (:order-by ?score))`.

**Resolution**:
- Add the variable argument before `:over`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(ntile :over (:order-by ?score))]
        :where [[?e :score ?score]]})
```
*Use `(ntile ?bucket :over (:order-by ?score))`*

### PRS-033 Function requires :over after variable argument

**Error text**: `'{}' requires ':over' after the variable argument`

**Cause**: `ntile` was given a variable but no `:over` clause followed.

**Resolution**:
- Add `:over` after the variable.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(ntile ?bucket)]
        :where [[?e :score ?score]]})
```
*Add `:over`: `(ntile ?bucket :over (:order-by ?score))`*

### PRS-034 Function requires :over immediately after function name

**Error text**: `'{}' requires ':over' immediately after the function name (no variable argument)`

**Cause**: `rank`, `row-number`, `dense-rank`, `percent-rank`, and `cume-dist` take no variable — they take `:over` directly. A variable was placed between the function name and `:over`.

**Resolution**:
- Remove the variable: `(rank :over (:order-by ?score))`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank ?score :over (:order-by ?score))]
        :where [[?e :score ?score]]})
```
*Use `(rank :over (:order-by ?score))` — no variable argument*

### PRS-035 :over must be followed by a list

**Error text**: `':over' must be followed by a list, e.g., (:order-by ?var)`

**Cause**: `:over` was not followed by a list `(...)`.

**Resolution**:
- Always follow `:over` with a list: `(rank :over (:order-by ?score))`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank :over :order-by)]
        :where [[?e :score ?score]]})
```
*Use `(rank :over (:order-by ?score))`*

### PRS-036 Unexpected tokens after :over clause

**Error text**: `unexpected tokens after ':over' clause in window expression`

**Cause**: Extra tokens appear after the `:over (...)` clause inside a window expression.

**Resolution**:
- The window expression must end after the `:over` clause. Remove trailing tokens.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank :over (:order-by ?score) :extra)]
        :where [[?e :score ?score]]})
```
*Remove `:extra`; the expression must end after `(:order-by ?score)`*

### PRS-037 :partition-by requires a variable

**Error text**: `':partition-by' requires a variable (starting with ?)`

**Cause**: The value supplied to `:partition-by` inside an `:over` clause is not a logic variable.

**Resolution**:
- Use a `?variable`: `(rank :over (:partition-by ?dept :order-by ?score))`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank :over (:partition-by :dept :order-by ?score))]
        :where [[?e :dept ?d] [?e :score ?score]]})
```
*Use `?dept` not `:dept`*

### PRS-038 :order-by requires a variable

**Error text**: `':order-by' requires a variable (starting with ?)`

**Cause**: The value supplied to `:order-by` inside an `:over` clause is not a logic variable.

**Resolution**:
- Use a `?variable`: `(rank :over (:order-by ?score))`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank :over (:order-by "score"))]
        :where [[?e :score ?score]]})
```
*Use `?score` not the string `"score"`*

### PRS-039 Unknown option in :over clause

**Error text**: `unknown option in ':over' clause: '{}'`

**Cause**: An unrecognised keyword appeared inside the `:over` list. Valid options are `:order-by` and `:partition-by`.

**Resolution**:
- Use only `:order-by` and `:partition-by`.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank :over (:order-by ?score :ascending))]
        :where [[?e :score ?score]]})
```
*`:ascending` is not a valid option; ordering direction is not yet configurable*

### PRS-040 Unexpected element in :over clause

**Error text**: `unexpected element in ':over' clause: {}`

**Cause**: A non-keyword, non-variable element appeared inside the `:over` list.

**Resolution**:
- The `:over` list must contain only `:order-by` / `:partition-by` followed by variables.
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions).

**Example**:
```datalog
(query {:find [(rank :over (:order-by ?score 5))]
        :where [[?e :score ?score]]})
```
*Remove `5`; `:over` options take only variables*

### PRS-041 Transact requires a vector of facts

**Error text**: `Transact requires a vector of facts`

**Cause**: The `transact` command was called with no argument or with a non-vector argument.

**Resolution**:
- Wrap facts in a vector: `(transact [[#uuid "..." :name "Alice"]])`.

**Example**:
```datalog
(transact)
```
*Missing the facts vector*

### PRS-042 Transact argument must be a vector of facts

**Error text**: `Transact argument must be a vector of facts`

**Cause**: The argument to `transact` is present but is not a vector (e.g. a map or keyword was passed). PRS-041 covers the case where no argument is provided at all.

**Resolution**:
- Use `(transact [[#uuid "..." :attr value]])`.

**Example**:
```datalog
(transact {:entity #uuid "..." :name "Alice"})
```
*Pass a vector `[...]`, not a map*

### PRS-043 Retract requires a vector of facts

**Error text**: `Retract requires a vector of facts`

**Cause**: The `retract` command was called with no argument or with a non-vector argument.

**Resolution**:
- Wrap facts in a vector: `(retract [[#uuid "..." :name "Alice"]])`.

**Example**:
```datalog
(retract)
```
*Missing the facts vector*

### PRS-044 Retract argument must be a vector of facts

**Error text**: `Retract argument must be a vector of facts`

**Cause**: The argument to `retract` is present but is not a vector (e.g. a map or keyword was passed). PRS-043 covers the case where no argument is provided at all.

**Resolution**:
- Use `(retract [[#uuid "..." :attr value]])`.

**Example**:
```datalog
(retract {:entity #uuid "..." :name "Alice"})
```
*Pass a vector `[...]`, not a map*

### PRS-045 Each fact must be a vector [e a v]

**Error text**: `Each fact must be a vector [e a v] or [e a v {opts}]`

**Cause**: A non-vector element appeared inside the facts vector (e.g. a keyword or integer instead of a `[...]` fact).

**Resolution**:
- Every element of the outer facts vector must itself be a vector `[entity :attribute value]`.

**Example**:
```datalog
(transact [:alice :name "Alice"])
```
*Outer `[...]` must contain inner `[...]` facts: `(transact [[:alice :name "Alice"]])`*

### PRS-046 Fact must have at least 3 elements (E A V)

**Error text**: `Fact must have at least 3 elements (E A V), got {}`

**Cause**: Each fact must supply at minimum an entity, an attribute, and a value.

**Resolution**:
- Ensure every fact has the form `[entity :attribute value]`.
- Optionally add a 4th map for temporal options: `[entity :attribute value {:valid-from "..."}]`.

**Example**:
```datalog
(transact [[:alice :name]])
```
*Only 2 elements; add the value: `[:alice :name "Alice"]`*

### PRS-047 Optional 4th fact element must be a map

**Error text**: `Optional 4th element of a fact must be a map {:valid-from ... :valid-to ...}, got {}`

**Cause**: The 4th element of a fact vector is not a map.

**Resolution**:
- The 4th element must be a map: `{:valid-from "ISO" :valid-to "ISO"}`.
- Omit it entirely if no temporal options are needed.

**Example**:
```datalog
(transact [[#uuid "..." :name "Alice" "2024-01-01"]])
```
*Use a map: `{:valid-from "2024-01-01T00:00:00Z"}`*

### PRS-048 Transact with options requires facts vector after the map

**Error text**: `Transact with options requires a facts vector after the map`

**Cause**: The `(transact {opts} [...])` form was used but the facts vector is missing after the options map.

**Resolution**:
- Provide the facts vector after the options map: `(transact {:tx-time "..."} [[...]])`.

**Example**:
```datalog
(transact {:tx-time "2024-01-01T00:00:00Z"})
```
*Add the facts vector: `(transact {:tx-time "2024-01-01T00:00:00Z"} [[...]])`*

### PRS-049 Unexpected end of fact vector

**Error text**: `unexpected end of fact vector`

**Cause**: The parser ran out of tokens while reading a fact vector; a closing `]` is missing.

**Resolution**:
- Ensure every `[` in the facts vector is closed with `]`.

**Example**:
```datalog
(transact [[:alice :name "Alice"
```
*(missing closing `]` for the inner fact and outer vector)*

### PRS-050 Empty list in :where clause

**Error text**: `Empty list in :where clause`

**Cause**: An empty `()` appears in `:where`. Every list in `:where` must be a pattern vector, `not`, `not-join`, `or`, `or-join`, or an expression clause.

**Resolution**:
- Remove the empty list or replace it with a valid clause.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

**Example**:
```datalog
(query {:find [?e]
        :where [() [?e :name "alice"]]})
```
*Remove `()`*

### PRS-051 (not) cannot be nested inside another (not)

**Error text**: `(not ...) cannot appear inside another (not ...)`

**Cause**: Minigraf's Datalog does not support double-negation via nested `not` clauses.

**Resolution**:
- Double negation is logically equivalent to the positive pattern; use the pattern directly.
- For complex negation use `not-join` with explicit join variables.
- See the [Datalog Reference — Negation](../../.wiki/Datalog-Reference.md#negation).

**Example**:
```datalog
(query {:find [?e]
        :where [(not (not [?e :active true]))]})
```
*Remove the outer `(not ...)`; match `[?e :active true]` directly*

### PRS-052 (not) requires at least one clause

**Error text**: `(not) requires at least one clause`

**Cause**: `(not)` has no clauses inside it.

**Resolution**:
- Add at least one pattern inside `(not ...)`.
- See the [Datalog Reference — Negation](../../.wiki/Datalog-Reference.md#negation).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :name ?n] (not)]})
```
*Add a pattern: `(not [?e :deleted true])`*

### PRS-053 (or) requires at least one branch

**Error text**: `(or) requires at least one branch`

**Cause**: `(or)` has no branches inside it.

**Resolution**:
- Add at least one branch.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md#disjunction).

**Example**:
```datalog
(query {:find [?e]
        :where [(or)]})
```
*Add a branch: `(or [?e :type :admin] [?e :type :superuser])`*

### PRS-054 (or-join) requires join-vars vector and at least one branch

**Error text**: `(or-join) requires a join-vars vector and at least one branch`

**Cause**: `(or-join)` must have a `[join-vars]` vector followed by at least one branch.

**Resolution**:
- Use the form `(or-join [?e] [[?e :a 1]] [[?e :b 2]])`.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md#disjunction).

**Example**:
```datalog
(query {:find [?e]
        :where [(or-join)]})
```
*Add join vars and branches: `(or-join [?e] [[?e :type :a]] [[?e :type :b]])`*

### PRS-055 (or-join) first argument must be a vector of join variables

**Error text**: `(or-join) first argument must be a vector of join variables`

**Cause**: The first argument to `(or-join)` is not a vector.

**Resolution**:
- The form is `(or-join [?var1 ?var2] ...)`.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md#disjunction).

**Example**:
```datalog
(query {:find [?e]
        :where [(or-join ?e [[?e :type :a]])]})
```
*Wrap join vars in a vector: `(or-join [?e] ...)`*

### PRS-056 (or-join) join variables must be logic variables

**Error text**: `(or-join) join variables must be logic variables, got {}`

**Cause**: Join variables in `(or-join [?e ...])` must start with `?`.

**Resolution**:
- Use `?variable` not `:keyword` or a string.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md#disjunction).

**Example**:
```datalog
(query {:find [?e]
        :where [(or-join [:e] [[?e :type :a]])]})
```
*Use `[?e]` not `[:e]`*

### PRS-057 (or) branches must introduce the same set of new variables

**Error text**: `all branches of (or ...) must introduce the same set of new variables`

**Cause**: Each branch of `(or ...)` must bind the same set of new logic variables. If one branch binds `?status` and another doesn't, the result is undefined.

**Resolution**:
- Restructure branches so they all bind the same new variables.
- Or use `or-join` to specify exactly which variables to share.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md#disjunction).

**Example**:
```datalog
(query {:find [?e ?status]
        :where [(or [?e :active true]
                    [?e :status ?status])]})
```
*Both branches must bind `?status` or neither should*

### PRS-058 (and) inside or/or-join requires at least one clause

**Error text**: `(and) inside or/or-join requires at least one clause`

**Cause**: An `(and ...)` inside `or`/`or-join` has no clauses.

**Resolution**:
- Add at least one clause inside `(and ...)`.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md#disjunction).

**Example**:
```datalog
(query {:find [?e]
        :where [(or (and) [?e :name "alice"])]})
```
*Add a clause: `(and [?e :active true] [?e :name "alice"])`*

### PRS-059 (not-join) requires join-vars vector and at least one clause

**Error text**: `(not-join) requires a join-vars vector and at least one clause`

**Cause**: `(not-join)` must have a `[join-vars]` vector followed by at least one clause.

**Resolution**:
- Use `(not-join [?e] [?e :deleted true])`.
- See the [Datalog Reference — Negation](../../.wiki/Datalog-Reference.md#negation).

**Example**:
```datalog
(query {:find [?e]
        :where [(not-join)]})
```
*Add join vars and clause: `(not-join [?e] [?e :deleted true])`*

### PRS-060 Expected pattern vector or rule invocation in :where clause

**Error text**: `Expected pattern vector or rule invocation in :where clause, got {}`

**Cause**: Something other than a pattern vector `[...]` or a list-form clause appeared directly in `:where`.

**Resolution**:
- Each `:where` element must be a vector `[e a v]`, a `(not ...)`, `(not-join ...)`, `(or ...)`, `(or-join ...)`, or expression `[(expr) ?out]`.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

**Example**:
```datalog
(query {:find [?e]
        :where [:name [?e :name ?n]]})
```
*`:name` is not a clause; use `[?e :name ?n]`*

### PRS-061 Unexpected element in query

**Error text**: `Unexpected element in query: {}`

**Cause**: An unrecognised key appeared at the top level of the query map.

**Resolution**:
- Valid query map keys are `:find`, `:where`, `:with`, `:as-of`, `:valid-at`.
- Check spelling.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :limit 10})
```
*`:limit` is not supported; results are not paginated in this version*

### PRS-062 Expression list cannot be empty

**Error text**: `expression list cannot be empty`

**Cause**: An expression clause `[()]` contains an empty list.

**Resolution**:
- Provide a function: `[(+ ?a ?b) ?sum]`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :a ?a] [()]]})
```
*Add a function: `[(> ?a 0)]`*

### PRS-063 Expression head must be a symbol

**Error text**: `expression head must be a symbol, got {}`

**Cause**: The first element of an expression must be a symbol naming a function, not a keyword.

**Resolution**:
- Use `+` not `:+`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :amount ?a] [(:+ ?a 10) ?total]]})
```
*Use `(+ ?a 10)` not `(:+ ?a 10)`*

### PRS-064 Function takes exactly 1 argument

**Error text**: `{} takes exactly 1 argument`

**Cause**: A built-in operator that takes 1 argument was given a different number.

**Resolution**:
- Check the argument count.
- Single-argument operators: `abs`, `not`, `str` (for coercion).
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :amount ?a] [(abs ?a ?a) ?pos]]})
```
*Use `[(abs ?a) ?pos]` — one argument only*

### PRS-065 Function takes exactly 2 arguments

**Error text**: `{} takes exactly 2 arguments`

**Cause**: A built-in operator that takes 2 arguments was given a different number.

**Resolution**:
- Check the argument count.
- Two-argument operators: `+`, `-`, `*`, `/`, `mod`, `quot`, `=`, `!=`, `<`, `<=`, `>`, `>=`, `starts-with?`, `ends-with?`, `contains?`, `matches?`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :a ?a] [?e :b ?b] [(+ ?a ?b ?c) ?sum]]})
```
*Use `[(+ ?a ?b) ?sum]` — two arguments only*

### PRS-066 matches? second argument must be a string literal

**Error text**: `matches? second argument must be a string literal`

**Cause**: The second argument to `matches?` must be a string literal (the regex pattern), not a variable.

**Resolution**:
- Pass the pattern as a string literal: `[(matches? ?name "alice.*")]`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :name ?n] [(matches? ?n ?pattern)]]})
```
*The pattern must be a literal: `[(matches? ?n "alice.*")]`*

### PRS-067 Unknown expression operator

**Error text**: `unknown expression operator: {}`

**Cause**: An expression clause used a function name that Minigraf does not recognise. Built-in operators include: `+`, `-`, `*`, `/`, `mod`, `quot`, `abs`, `min`, `max`, `str`, `not`, `=`, `!=`, `<`, `<=`, `>`, `>=`, `matches?`, `starts-with?`, `ends-with?`, `contains?`.

**Resolution**:
- Check spelling against the supported operator list above.
- For missing operators, register a custom predicate via `db.register_predicate()`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :amount ?a] [(floor-div ?a 100) ?bucket]]})
```
*`floor-div` is not built-in; use `(quot ?a 100)` instead*

### PRS-068 Expression clause must be [(expr)] or [(expr) ?out]

**Error text**: `expression clause must be [(expr)] or [(expr) ?out], got {} elements`

**Cause**: An expression clause in `:where` must be a 1- or 2-element outer vector: `[(predicate)]` or `[(expr) ?out]`. A 3+ element outer vector is invalid.

**Resolution**:
- Ensure the expression is wrapped in exactly one outer `[...]`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :a ?a] [(+ ?a 1) ?b :extra]]})
```
*Remove `:extra`; the form must be `[(+ ?a 1) ?b]`*

### PRS-069 Expression output must be a ?variable

**Error text**: `expression output must be a ?variable, got {}`

**Cause**: The output binding in `[(expr) ?out]` is not a logic variable starting with `?`.

**Resolution**:
- Use a `?variable`: `[(+ ?a ?b) ?sum]`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :a ?a] [(+ ?a 1) :result]]})
```
*Use `?result` not `:result`*

### PRS-070 Unsupported expression argument

**Error text**: `unsupported expression argument: {}`

**Cause**: An argument inside an expression is of a type that the expression engine cannot accept (e.g. a nested map or list).

**Resolution**:
- Expressions accept variables (`?x`), integers, floats, strings, booleans, and keywords.
- Restructure to avoid passing complex types.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :a ?a] [(+ ?a {:x 1}) ?b]]})
```
*Maps are not valid expression arguments*

### PRS-071 Expected UUID string after #uuid tag

**Error text**: `Expected UUID string after #uuid tag`

**Cause**: `#uuid` was used but not followed by a string token.

**Resolution**:
- Use the form `#uuid "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"`.

**Example**:
```datalog
(transact [[#uuid :some-id :name "Alice"]])
```
*`#uuid` must be followed by a string: `#uuid "550e8400-e29b-41d4-a716-446655440000"`*

### PRS-072 Invalid UUID

**Error text**: `Invalid UUID`

**Cause**: The string after `#uuid` is not a valid UUID. UUIDs must match `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` (32 hex digits in 5 groups).

**Resolution**:
- Verify the UUID format.
- All 32 characters must be hex digits (0–9, a–f).
- The groups must be separated by hyphens at the correct positions.

**Example**:
```datalog
(transact [[#uuid "not-a-valid-uuid" :name "Alice"]])
```
*Use a properly formatted UUID: `#uuid "550e8400-e29b-41d4-a716-446655440000"`*

### PRS-073 Unknown tagged literal

**Error text**: `Unknown tagged literal: #{}`

**Cause**: A `#tag "..."` form used an unrecognised tag. Only `#uuid` is supported.

**Resolution**:
- Remove the unrecognised tagged literal.
- If you need to store binary data, encode it as a string attribute value.

**Example**:
```datalog
(transact [[#uuid "..." :data #base64 "SGVsbG8="]])
```
*`#base64` is not supported; store the string value directly*

### PRS-074 Bind slot name exceeds maximum length

**Error text**: `Bind slot name exceeds maximum length of {} bytes`

**Cause**: A prepared-query bind slot name `$name` in a `prepare()` call exceeds 4096 bytes.

**Resolution**:
- Use shorter bind slot names.
- Bind slot names should be brief and descriptive, e.g. `$name`, `$entity-id`.

**Example**:
```datalog
$<4097-char-slot-name>
```
*Shorten the bind slot name; `$name` is sufficient for most use cases*

### PRS-075 Transact rejects unexpected trailing argument(s)

**Error text**: `transact takes (transact [facts]) or (transact {opts} [facts]); found {} unexpected trailing argument(s) — the valid-time options map must come BEFORE the facts vector, not after`

**Cause**: `(transact ...)` was called with more top-level arguments than it accepts — either extra tokens after the facts vector, or the valid-time options map placed after the facts vector instead of before it. Earlier parser versions silently dropped these extra arguments (and any `:valid-from`/`:valid-to` in a misplaced map); this is now a hard error.

**Resolution**:
- Use exactly one of `(transact [facts])` or `(transact {:valid-from "..." :valid-to "..."} [facts])`.
- If you meant to set valid-time options, move the map before the facts vector.
- Remove any stray trailing tokens.

**Example**:
```datalog
(transact [[:alice :employment/status :active]] {:valid-from "2023-01-01"})
```
*The options map comes after the facts vector; move it before: `(transact {:valid-from "2023-01-01"} [[:alice :employment/status :active]])`*

### PRS-076 Retract rejects unexpected trailing argument(s)

**Error text**: `retract takes (retract [facts]); found {} unexpected trailing argument(s)`

**Cause**: `(retract ...)` was called with more than one top-level argument. `retract` only accepts a single facts vector — unlike `transact`, it does not accept a leading options map.

**Resolution**:
- Use exactly `(retract [facts])`.
- Remove any trailing tokens after the facts vector.

**Example**:
```datalog
(retract [[:alice :employment/status :active]] {:valid-from "2023-01-01"})
```
*`retract` takes only a facts vector; drop the trailing map*

### PRS-077 Unexpected trailing input after a complete form

**Error text**: `unexpected trailing input after a complete form: {}`

**Cause**: The top-level input contains a syntactically complete EDN form (e.g. a fully closed list or vector) followed by additional tokens. Earlier parser versions silently stopped after the first complete form instead of rejecting the extra input.

**Resolution**:
- Ensure the input contains exactly one top-level form.
- Remove any text, delimiters, or extra forms following the first complete form.

**Example**:
```datalog
(query [:find ?e :where [?e :entity-type :type/commit]]) garbage-trailing-tokens
```
*Only one top-level form is allowed; remove everything after the closing `)`*

### PRS-078 Query rejects unexpected trailing argument(s)

**Error text**: `query takes (query [...]); found {} unexpected trailing argument(s)`

**Cause**: `(query ...)` was called with more than one top-level argument. All query options (`:find`, `:where`, `:as-of`, `:max-results`, etc.) must appear as keywords *inside* the query vector, not as sibling arguments after it. Earlier parser versions silently dropped these trailing arguments.

**Resolution**:
- Put every query option inside the single query vector: `(query [:find ?e :where [...] :max-results 5])`.
- Remove any arguments after the closing `]` of the query vector.

**Example**:
```datalog
(query [:find ?e :where [?e :a :b]] :max-results 5)
```
*`:max-results 5` must be inside the vector: `(query [:find ?e :where [?e :a :b] :max-results 5])`*

### PRS-079 Rule rejects unexpected trailing argument(s)

**Error text**: `rule takes (rule [...]); found {} unexpected trailing argument(s)`

**Cause**: `(rule ...)` was called with more than one top-level argument. A rule definition is a single vector containing the rule head and body patterns; nothing may follow it.

**Resolution**:
- Keep the rule head and all body patterns inside one vector: `(rule [(reachable ?a ?b) [?a :edge ?b]])`.
- Remove any arguments after the closing `]` of the rule vector.

**Example**:
```datalog
(rule [(reachable ?a ?b) [?a :edge ?b]] :unexpected-extra)
```
*Nothing may follow the rule vector; remove `:unexpected-extra`*

---

## QRY — Query Execution Errors

Query execution errors occur after parsing succeeds, during pattern matching,
predicate evaluation, or fact transacting.

### QRY-001 Invalid entity

**Error text**: `Invalid entity: {}`

**Cause**: An entity ID in a `transact` fact could not be resolved. Entity IDs must be UUIDs (as `#uuid "..."` tagged literals), existing entity symbols, or values that can be resolved to a UUID at execution time.

**Resolution**:
- Use a `#uuid "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"` literal for a specific entity.
- Use a new unique symbol if creating a new entity and let Minigraf assign a UUID.

**Example**:
```datalog
(transact [["my-entity" :name "Alice"]])
```
*`"my-entity"` is a plain string, not a UUID; use `#uuid "..."` or an entity variable*

### QRY-002 Attribute must be a keyword

**Error text**: `Attribute must be a keyword`

**Cause**: An attribute in a `transact` fact is not a keyword (does not start with `:`). Attributes must always be namespaced keywords.

**Resolution**:
- Use the `:namespace/attr` form, e.g. `:person/name`, `:app/status`.

**Example**:
```datalog
(transact [[#uuid "..." name "Alice"]])
```
*`name` is a symbol, not a keyword; use `:name` or `:person/name`*

### QRY-003 Cannot transact a pseudo-attribute

**Error text**: `Cannot transact a pseudo-attribute`

**Cause**: A pseudo-attribute (an internal Minigraf metadata attribute used for system bookkeeping) was used as a fact attribute in `transact`. Pseudo-attributes are reserved and cannot be written by user code.

**Resolution**:
- Use only user-defined attributes (e.g. `:person/name`, `:app/status`).
- Avoid attribute names that begin with `db/` or other system namespaces.

**Example**:
```datalog
(transact [[#uuid "..." :db/id #uuid "..."]])
```
*`:db/id` is a pseudo-attribute; use only user-defined attributes*

### QRY-004 Invalid value

**Error text**: `Invalid value: {}`

**Cause**: A value in a fact is of a type that Minigraf cannot store. Supported value types are: string, integer (i64), float (f64), boolean, UUID ref (`Value::Ref`), and keyword.

**Resolution**:
- Convert the value to a supported type.
- For lists or maps, serialise to a string or model them as separate entities with `:ref` attributes.

**Example**:
```datalog
(transact [[#uuid "..." :tags ["a" "b" "c"]]])
```
*Vectors are not a valid value type; represent tags as separate facts or a comma-joined string*

### QRY-005 Transaction failed

**Error text**: `Transaction failed: {}`

**Cause**: The batch of facts could not be committed. This is a wrapper error — the nested reason message identifies the root cause, which is typically a storage error, lock poisoning, or WAL write failure.

**Resolution**:
- Check the nested error message.
- Common sub-causes are covered by API-001 (lock poisoned) and STG/WAL errors.
- Restart the process if the database is in an inconsistent state.

**Scenario**: A `transact` call fails because an earlier operation panicked while holding the write lock. The nested message reads "write lock is poisoned".

### QRY-006 Retraction failed

**Error text**: `Retraction failed: {}`

**Cause**: A retraction could not be applied. The nested reason message identifies the specific cause — the fact may not exist at the given transaction time, or a storage error occurred.

**Resolution**:
- Check the nested error message.
- Verify the entity, attribute, and value exactly match an asserted fact before retracting.
- A retraction of a non-existent fact is not a no-op — it is an error.

**Scenario**: `(retract [[#uuid "..." :name "alice"]])` fails because no fact `[:name "alice"]` was ever asserted for that entity.

### QRY-007 Unknown predicate

**Error text**: `unknown predicate: '{}'`

**Cause**: A `:where` clause invoked a predicate (expression function) that is not built-in and has not been registered via `db.register_predicate()`.

**Resolution**:
- Check spelling against built-in predicates: `=`, `!=`, `<`, `<=`, `>`, `>=`, `matches?`, `starts-with?`, `ends-with?`, `contains?`.
- Register a custom predicate: `db.register_predicate("between?", |args| { ... })?;`.

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :age ?a]
                [(between? ?a 18 65)]]})
```
*`between?` is not built-in; register it or use `[(>= ?a 18)] [(<= ?a 65)]`*

### QRY-008 Functions lock poisoned

**Error text**: `functions lock poisoned`

**Cause**: An internal Rust mutex guarding the custom function registry was poisoned — a previous operation panicked while holding the lock.

**Resolution**:
- Restart the process.
- If panics recur when calling custom registered functions, investigate the function implementations for panics.
- If this occurs without any custom functions, file a bug.

**Scenario**: `db.register_predicate()` or a query invoking a custom predicate panics; subsequent calls to any query return this error.

### QRY-009 Rules lock poisoned

**Error text**: `rules lock poisoned`

**Cause**: An internal Rust mutex guarding the Datalog rule registry was poisoned — a previous operation panicked while adding or evaluating rules.

**Resolution**:
- Restart the process.
- If panics recur when using `(rule ...)` forms, investigate whether the rule input is triggering a bug.
- File a bug with the rule text if the input appears valid.

**Scenario**: `db.execute("(rule ...)")` panics; subsequent queries that reference rules return this error.

---

## STG — Storage Errors

Storage errors relate to reading or writing the `.graph` file. They typically
indicate a corrupted, truncated, or incompatible database file.

See the [file format section in README](../README.md#file-format) for version history.

### STG-001 Invalid header: too short

**Error text**: `Invalid header: too short (got {} bytes, need 64)`

**Cause**: The `.graph` file is truncated — shorter than the minimum header size. Happens if the file was partially written (e.g. a crash during the initial `save()`) or if a non-Minigraf file was passed by mistake.

**Resolution**:
- Restore the file from a backup. If no backup exists and the file was newly created, delete it and let Minigraf create a fresh one. See the [file format section in README](../README.md#file-format).

**Scenario**: Opening a `.graph` file that was truncated by a disk-full condition during the first `db.save()` or `db.checkpoint()` call.

### STG-002 Invalid magic number: not a .graph file

**Error text**: `Invalid magic number: not a .graph file`

**Cause**: The first 4 bytes of the file are not the Minigraf magic bytes `MGRF`. The path points to a non-Minigraf file, or the file header was overwritten by another process.

**Resolution**:
- Verify the file path is correct and points to a `.graph` file created by Minigraf. Do not open SQLite databases, JSON files, or other formats with Minigraf.

**Scenario**: `Minigraf::open("config.json")` — a wrong file path was passed.

### STG-003 Invalid v4/v5/v6 header too short

**Error text**: `Invalid v4/v5/v6 header: expected at least 72 bytes, got {}`

**Cause**: A file identified as format version 4, 5, or 6 is too short to hold a valid header of that version. The file is truncated at the header.

**Resolution**:
- Restore from backup. If the file is newly created, delete it. See the [file format section in README](../README.md#file-format).

**Scenario**: A v5-format `.graph` file was corrupted by a partial write and is missing the latter part of its header.

### STG-004 Invalid v6 header too short

**Error text**: `Invalid v6 header: expected 80 bytes, got {}`

**Cause**: A file identified as format version 6 is shorter than the required 80-byte v6 header. The file is truncated.

**Resolution**:
- Restore from backup. If the file is newly created, delete it. See the [file format section in README](../README.md#file-format).

**Scenario**: A v6-format `.graph` file was written by an older pre-release version and its header is incomplete.

### STG-005 Invalid v7 header too short

**Error text**: `Invalid v7 header: expected 84 bytes, got {}`

**Cause**: A file identified as format version 7 (current format) is shorter than the required 84-byte header. The file is truncated.

**Resolution**:
- Restore from backup. If the file is newly created, delete it. See the [file format section in README](../README.md#file-format).

**Scenario**: A v7-format `.graph` file was partially written by a crash during header initialisation.

### STG-006 Unsupported format version

**Error text**: `Unsupported format version: {} (supported: 1-{})`

**Cause**: The file was written by a newer version of Minigraf than is currently installed. The format version number in the header is outside the range this library can read.

**Resolution**:
- Upgrade the Minigraf library to a version that supports the file's format. Do not downgrade a database file to an older format — upgrade the library instead. See the [file format section in README](../README.md#file-format).

**Scenario**: A `.graph` file created with a future version of Minigraf is opened with the current library.

### STG-007 page_count must be greater than 0

**Error text**: `page_count must be greater than 0`

**Cause**: The `page_count` field in the file header is zero, which is invalid for any non-empty database.

**Resolution**:
- The file header is corrupted. Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: The header page of a `.graph` file was zeroed out by a storage hardware error.

### STG-008 eavt_root_page must be less than page_count

**Error text**: `eavt_root_page ({}) must be less than page_count ({})`

**Cause**: The EAVT B+tree root page index in the header points beyond the file's page count. The header is internally inconsistent — a sign of file corruption.

**Resolution**:
- Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: Partial overwrite of the header during an interrupted `checkpoint()` left the root page pointer pointing past the end of the file.

### STG-009 fact_page_count cannot exceed page_count

**Error text**: `fact_page_count ({}) cannot exceed page_count ({})`

**Cause**: The fact page count in the header is larger than the total page count. The header is internally inconsistent.

**Resolution**:
- Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: Bit-flip corruption in the header fact_page_count field produced an out-of-range value.

### STG-010 Failed to read header from existing file

**Error text**: `Failed to read header from existing file: {}`

**Cause**: A low-level I/O error prevented reading the file header. Common causes: file permissions, file moved or deleted between open and read, disk I/O error.

**Resolution**:
- Check file system permissions (the process must have read access). Verify the file path still exists. Check for disk errors with your OS's filesystem check tool.

**Scenario**: `Minigraf::open("/data/app.graph")` fails because the process does not have read permission on the file.

### STG-011 Internal page has no children

**Error text**: `internal page has no children`

**Cause**: An internal B+tree page was found with an empty children list, which violates B+tree invariants. This indicates index corruption.

**Resolution**:
- Restore from backup. If the corruption occurred after a recent write, the WAL file may contain a recoverable state — delete the `.graph` file and try opening from the WAL only (by restoring the last good checkpoint and replaying the WAL). See the [file format section in README](../README.md#file-format).

**Scenario**: A crash mid-checkpoint left an internal B+tree page in an inconsistent state.

### STG-012 Expected index page at page N

**Error text**: `Expected index page at page {}`

**Cause**: The B+tree traversal expected an index page at a specific page number, but the page found has a different type tag. This indicates index corruption or file truncation.

**Resolution**:
- Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: A packed-fact page and an index page were written to overlapping positions due to a page allocation bug in a pre-release version.

### STG-013 range_scan expected leaf

**Error text**: `range_scan: expected leaf at page_id={}`

**Cause**: A range scan of the B+tree expected a leaf page at a given position but found a non-leaf page. Indicates corruption of the B+tree structure.

**Resolution**:
- Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: A partial checkpoint left the B+tree leaf and internal pages inconsistent.

### STG-014 Expected packed page type

**Error text**: `Expected packed page (0x02), got 0x{}`

**Cause**: A page expected to contain packed facts has a different page type tag. Indicates that the page layout in the file does not match the header's page allocation records.

**Resolution**:
- Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: A fact page and an index page swapped positions in the file due to a storage driver bug.

### STG-015 Record extends beyond page boundary

**Error text**: `Record at slot {} extends beyond page boundary`

**Cause**: A fact record at the given slot in a packed-facts page extends past the 4KB page boundary. This indicates corruption of the page's internal offset table.

**Resolution**:
- Restore from backup. See the [file format section in README](../README.md#file-format).

**Scenario**: A disk write was interrupted, leaving a partial fact page with a corrupt slot table.

### STG-016 Backend mutex poisoned

**Error text**: `backend mutex poisoned`

**Cause**: An internal Rust mutex guarding the storage backend was poisoned — a previous operation panicked while holding it.

**Resolution**:
- Restart the process. The WAL will be replayed on the next `Minigraf::open()` to recover committed facts. If panics recur, investigate the application code for panics occurring inside write operations.

**Scenario**: A write closure panics mid-transaction, poisoning the backend mutex and preventing all subsequent reads and writes.

### STG-017 Page count overflow: index_start

**Error text**: `page count overflow computing index_start`

**Cause**: Computing the starting page of the index section caused an integer overflow in the page count. This occurs only if the database has grown to an extreme size (billions of pages) or if the page count field in the header is corrupted.

**Resolution**:
- If the database file size is normal (under several terabytes), the header is likely corrupted — restore from backup. If the file is genuinely enormous, file a bug with the database size.

**Scenario**: The `page_count` header field was corrupted to `u64::MAX`, causing an overflow when computing derived positions.

### STG-018 Page count overflow: next_free

**Error text**: `page count overflow computing next_free`

**Cause**: Computing the next free page position caused an integer overflow. Same root cause as STG-017 — extreme database size or corrupted header.

**Resolution**:
- Same as STG-017. Restore from backup if the database size is normal.

**Scenario**: Same as STG-017.

### STG-019 Page count overflow: new_fact_start

**Error text**: `page count overflow computing new_fact_start`

**Cause**: Computing the starting position for new fact pages caused an integer overflow. Same root cause as STG-017.

**Resolution**:
- Same as STG-017. Restore from backup if the database size is normal.

**Scenario**: Same as STG-017.

### STG-020 Fact index exceeds u16::MAX

**Error text**: `fact index {} exceeds u16::MAX`

**Cause**: The number of facts on a single packed page exceeded 65535 (u16::MAX). This is a theoretical limit that should not be reached in practice — a single 4KB page holds roughly 20–30 facts.

**Resolution**:
- This should not occur under normal operation. If it does, file a bug with the fact serialisation size of the triggering fact.

**Scenario**: A bug in the page packer caused more facts to be written to a single page than the slot index can represent.

### STG-021 Page id overflow in checksum computation

**Error text**: `page id overflow in checksum computation`

**Cause**: A page ID exceeded the range that can be represented during checksum computation. Indicates an extremely large database or a corrupted page count.

**Resolution**:
- If the database file is unexpectedly large, file a bug. Otherwise restore from backup.

**Scenario**: Same as STG-017 — corrupted `page_count` triggers overflow during the checksum phase.

### STG-022 Page id overflow writing fact pages

**Error text**: `page id overflow writing fact pages`

**Cause**: A page ID exceeded the representable range when writing fact pages during a checkpoint. Indicates an extremely large database or corrupted header state.

**Resolution**:
- Same as STG-021. Restore from backup if the database size is normal.

**Scenario**: Same as STG-017.

### STG-023 Page index exceeds u64::MAX

**Error text**: `page index {} exceeds u64::MAX`

**Cause**: A page index computation produced a value larger than u64::MAX. This is practically unreachable — would require a database with more pages than u64 can represent.

**Resolution**:
- This should never occur under normal operation. File a bug.

**Scenario**: An extreme edge case in page count arithmetic triggered an overflow that wrapped past u64::MAX.

### STG-024 Pending fact count exceeds u64::MAX

**Error text**: `pending fact count exceeds u64::MAX`

**Cause**: The number of pending (unflushed) facts in the WAL exceeded u64::MAX. Practically unreachable.

**Resolution**:
- This should never occur under normal operation. File a bug.

**Scenario**: An extremely long-running write session accumulated more unflushed facts than u64 can count.

---

### STG-025 Database already open in this process

**Error text**: `Database is already open in this process ({}). A second handle on one file would give each its own page table and corrupt both — reuse the existing handle instead. `Minigraf` is cheap to clone and all clones share the same database.`

**Cause**: This process already holds an open handle on this file. Two `FileBackend` instances on one file each cache their own `header.page_count`, allocate new pages from that count, and bounds-check reads against it, so the two page tables diverge and produce intermittent `Page N out of bounds` errors.

**Resolution**:
- Reuse the handle you already have. `Minigraf` is cheap to clone and all clones share one database.
- If you cannot find the other handle, it is usually held by a longer-lived object than you expect — a cache, a registry, or a background task.

**Scenario**: A request handler calls `Minigraf::open` per request while a connection pool already holds the same path open.

---

### STG-026 Database locked by another process

**Error text**: `Database is locked by another process ({}). The lock is held on the file itself and is released automatically when the holding process exits, so there is no lock file to clean up.`

**Cause**: Another process holds the kernel file lock on this `.graph` file. Minigraf is single-writer: one process at a time. This is reported only after a bounded retry (up to ~375ms) rules out a transient `fork`-related false positive — see the v2.0.0 CHANGELOG entry on the bounded retry.

**Resolution**:
- Wait for the other process to exit; the kernel releases the lock automatically, including when the process is killed.
- There is no lock file to delete. If you find a stale `.graph.lock`, it is a leftover from a version before 2.0 and has no effect — it is not read or deleted, and should not be removed by hand in case an old process still depends on it.
- If two services genuinely need the same file, put one in front of the other. Minigraf is embedded, not a server.

**Scenario**: A rolling deployment starts the new pod before the old one has exited, and both mount the same volume.

---

### STG-027 Filesystem does not support file locking

**Error text**: `Failed to lock database at {}: {}. This filesystem does not support file locking (common on NFSv3 without lockd, and on some FUSE mounts). Set `allow_unlocked` in `OpenOptions` to open anyway — that accepts the risk that concurrent writers corrupt the file.`

**Cause**: The underlying filesystem rejected the lock outright. Common on NFSv3 mounts with no `lockd` running, and on some FUSE filesystems. Because the file must exist before it can be locked, this refusal can leave a 0-byte `.graph` file behind; the next open sees `is_new` and initialises normally.

**Resolution**:
- Prefer moving the database to a filesystem that supports locking — this is the safe fix.
- On NFS, ensure NFSv4, or start `lockd` for NFSv3.
- If you are certain there is exactly one writer, set `OpenOptions::allow_unlocked(true)`. This accepts the risk that concurrent writers corrupt the file; Minigraf cannot detect them without a working lock.

**Scenario**: A `.graph` file placed on an NFSv3 export whose `lockd` is not running.

**NFSv3 `nolock` is not covered by this error — it is a silent, undetectable gap (#334).** An export explicitly mounted with `-o nolock` behaves differently from plain "no `lockd`": the Linux NFS client serves `flock`/OFD locks out of its own local, in-kernel lock table instead of going over NLM to the server. `try_lock` sees an ordinary local success, `classify` takes the same branch it would on a filesystem where locking genuinely works, and this error is never raised — `allow_unlocked` is never consulted because the code never learns the mount can't really lock. Two separate client hosts writing the same `nolock` export each get a false `Ok(())` from their own kernel with zero cross-host coordination, and can silently corrupt the file. There is no way to detect this from inside `try_lock`'s result, so `nolock` NFSv3 exports are an unsupported deployment for multi-writer use, the same way running mixed Minigraf versions against one file is unsupported (see the kernel-locking CHANGELOG entry) — avoid `nolock` exports rather than relying on this check to catch them.

---

## WAL — Write-Ahead Log Errors

WAL errors relate to the sidecar `.wal` file written alongside the `.graph` file.
The WAL is replayed on open and deleted on checkpoint.

### WAL-001 Invalid WAL magic number

**Error text**: `Invalid WAL magic number: not a .wal file`

**Cause**: The sidecar `.wal` file does not start with the expected WAL magic bytes. The file may have been replaced, corrupted, or created by an incompatible tool.

**Resolution**:
- If the WAL file is stale or corrupt, delete `<dbname>.wal` and reopen the database — Minigraf will replay only from the committed state in the `.graph` file.
- Do not manually create or edit `.wal` files.

**Scenario**: `my-db.wal` was accidentally replaced with an empty file before `Minigraf::open("my-db.graph")` was called.

### WAL-002 Unsupported WAL version

**Error text**: `Unsupported WAL version: {} (expected {})`

**Cause**: The `.wal` file was written by a version of Minigraf with a different WAL format. This can occur when downgrading the library after a WAL was written by a newer version.

**Resolution**:
- Delete the `.wal` file if it is from an incomplete or stale session (no data is lost — committed facts are in the `.graph` file).
- If the WAL contains uncommitted in-flight data you need to recover, upgrade the library to the version that wrote the WAL before reopening.

**Scenario**: A `.wal` file written by a pre-release version of Minigraf is opened with the stable release, which uses a different WAL version number — e.g. `Unsupported WAL version: 3 (expected 2)`.

### WAL-003 Fact serialised size exceeds maximum

**Error text**: `Fact serialised size {} bytes exceeds maximum {} bytes. Store large payloads externally and reference them with a Value::String URL/path or Value::Ref entity ID.`

**Cause**: A single fact's serialised size exceeds the WAL entry limit (~512 KB). This typically means a `Value::String` attribute value contains very large content such as raw document text, a base64-encoded image, or binary data.

**Resolution**:
- Store large payloads in an external file or object store.
- Store the file path or URL as a `Value::String` attribute on the entity.
- Or create a dedicated entity for the content and reference it with `Value::Ref`.
- See [BENCHMARKS.md](../BENCHMARKS.md) for size guidance.

**Example**:
```datalog
(transact [[#uuid "..." :document/body "<50000-word essay...>"]])
```
*The `:document/body` value is too large; store it in a file and use `:document/path` instead*

### WAL-004 Fact serialised size exceeds u32 range

**Error text**: `fact serialised size {} exceeds u32 range`

**Cause**: The serialised size of a single fact exceeds `u32::MAX` (~4 GB). This is practically unreachable — WAL-003's ~512 KB limit fires first.

**Resolution**:
- This should not occur under normal operation.
- If it does, file a bug.

**Scenario**: An extreme edge case where the fact serialisation path bypassed the WAL-003 limit check, producing an impossibly large entry — e.g. `fact serialised size 4294967297 exceeds u32 range`.

### WAL-005 WAL num_facts exceeds platform usize

**Error text**: `WAL num_facts exceeds platform usize`

**Cause**: The number of facts recorded in the WAL header exceeds the platform's `usize` maximum. Practically unreachable on 64-bit platforms.

**Resolution**:
- This should not occur under normal operation.
- File a bug.

**Scenario**: A corrupted WAL header field contains a fact count larger than `usize::MAX`, triggering an overflow during replay.

### WAL-006 Failed to delete WAL file

**Error text**: `failed to delete WAL file {}: {}`

**Cause**: After a successful `checkpoint()`, Minigraf could not delete the sidecar `.wal` file. This is typically a file system permissions issue.

**Resolution**:
- Check that the process has write access to the directory containing the `.graph` file (WAL deletion requires directory write permission, not just file write permission).
- The `.wal` file is safe to delete manually — Minigraf will create a new one on the next write.

**Scenario**: `db.checkpoint()` succeeds but the process lacks directory write permission, preventing deletion of `my-db.wal` — e.g. `failed to delete WAL file my-db.wal: permission denied`.

---

## API — Database API Errors

API errors indicate a violated contract in how the public `Minigraf` or
`WriteTransaction` API is used.

### API-001 Write lock poisoned

**Error text**: `write lock is poisoned; database may be in an inconsistent state`

**Cause**: A previous `WriteTransaction` panicked while holding the write lock. Rust's mutex poisoning mechanism prevents further writes to protect data integrity.

**Resolution**:
- Restart the process — the WAL will be replayed on the next `Minigraf::open()` call to recover any committed facts. If panics are occurring regularly, investigate the root cause in your application code before retrying writes.

**Scenario**: `db.begin_write()` is called after a previous write closure panicked mid-transaction, poisoning the lock.

### API-002 Unexpected command variant in write path

**Error text**: `unexpected command variant in write path`

**Cause**: An internal routing error — a command type not expected in the write path was dispatched there. This indicates a bug in the Minigraf library, not a user mistake.

**Resolution**:
- File a bug report with the exact input string that triggered this error.

**Scenario**: A newly added command type was not handled in the write path dispatcher, causing an unexpected variant to arrive.

### API-003 Attribute must be a keyword (API layer)

**Error text**: `attribute must be a keyword`

**Cause**: An attribute validation check in the database API layer (mirroring QRY-002) found a non-keyword attribute. This fires for facts that pass parsing but fail validation at execution time.

**Resolution**:
- Ensure all attribute names are keywords starting with `:`, e.g. `:person/name`, `:app/status`.

**Example**:
```rust
// Wrong — attribute is a plain string
db.execute("(transact [[#uuid \"...\" \"name\" \"Alice\"]])")?;

// Right
db.execute("(transact [[#uuid \"...\" :name \"Alice\"]])")?;
```

### API-004 Cannot transact a pseudo-attribute (API layer)

**Error text**: `cannot transact a pseudo-attribute`

**Cause**: A pseudo-attribute (reserved internal attribute) was used in a `transact` call. This mirrors QRY-003 but fires at the API layer.

**Resolution**:
- Use only user-defined attributes. Avoid attribute names in system-reserved namespaces such as `db/`.

**Example**:
```rust
// Wrong
db.execute("(transact [[#uuid \"...\" :db/id #uuid \"...\"]])")?;

// Right — use user-defined attributes
db.execute("(transact [[#uuid \"...\" :person/name \"Alice\"]])")?;
```

### API-005 Only query commands can be prepared (got transact)

**Error text**: `only (query ...) commands can be prepared; got transact`

**Cause**: `db.prepare()` only accepts `(query ...)` forms. Passing a `(transact ...)` command to `prepare()` is not supported.

**Resolution**:
- Use `db.execute()` for `transact` commands. Only use `db.prepare()` for `(query ...)` commands that will be executed repeatedly with different bind slot values.

**Example**:
```rust
// Wrong
let pq = db.prepare("(transact [[#uuid \"...\" :name \"Alice\"]])")?;

// Right — use execute() for transact
db.execute("(transact [[#uuid \"...\" :name \"Alice\"]])")?;

// Right — prepare() is for repeated queries
let pq = db.prepare("(query {:find [?e] :where [[?e :name $name]]})")?;
```

### API-006 Only query commands can be prepared (got retract)

**Error text**: `only (query ...) commands can be prepared; got retract`

**Cause**: `db.prepare()` does not accept `(retract ...)` commands.

**Resolution**:
- Use `db.execute()` for `retract` commands.

**Example**:
```rust
// Wrong
let pq = db.prepare("(retract [[#uuid \"...\" :name \"Alice\"]])")?;

// Right
db.execute("(retract [[#uuid \"...\" :name \"Alice\"]])")?;
```

### API-007 Only query commands can be prepared (got rule)

**Error text**: `only (query ...) commands can be prepared; got rule`

**Cause**: `db.prepare()` does not accept `(rule ...)` commands.

**Resolution**:
- Use `db.execute()` for `rule` commands. Rules are registered once and then available in all subsequent queries — they do not need to be prepared.

**Example**:
```rust
// Wrong
let pq = db.prepare("(rule [(ancestor ?x ?y) [?x :parent ?y]])")?;

// Right
db.execute("(rule [(ancestor ?x ?y) [?x :parent ?y]])")?;
```

### API-008 Function registry lock poisoned

**Error text**: `function registry lock poisoned: PoisonError { .. }`

**Cause**: An internal Rust mutex guarding the custom function/predicate registry was poisoned — a previous operation panicked while registering or invoking a custom function.

**Resolution**:
- Restart the process. If panics recur during `db.register_predicate()` or `db.register_aggregate()` calls, investigate the closure implementations for panics.

**Scenario**: A custom predicate closure registered via `db.register_predicate()` panics during a query, poisoning the function registry mutex.

### API-009 WAL not initialized

**Error text**: `WAL not initialized`

**Cause**: `db.execute()` or `db.checkpoint()` was called in a state where the WAL subsystem has not been initialised. This indicates an internal sequencing bug in the library.

**Resolution**:
- File a bug report with the sequence of API calls that produced this error.

**Scenario**: A code path in the library called a write operation before the WAL was set up during `Minigraf::open()`.

---

## INT — Internal Errors

Internal errors are invariant violations or edge cases that are not expected
to occur through normal use of the public API, or (for a handful of codes
inherited from before every call site was audited — see
[#277](https://github.com/project-minigraf/minigraf/issues/277)) genuine
user-reachable mistakes that were never given a category-specific `PRS-`/
`QRY-`/`API-` code of their own. If you see one of these in practice and
cannot explain it from your own input, it likely indicates a bug in
Minigraf itself; please
[file an issue](https://github.com/project-minigraf/minigraf/issues) with
the full error message and the input that triggered it.

Many `INT-` codes below are intentionally shared by several call sites in
the same subsystem (mirroring how, e.g., `QRY-005`/`QRY-006` already wrap
an arbitrary underlying failure) — the `{}` placeholder in the "Error text"
carries the specific underlying detail. This keeps the registry a
manageable size while still giving every call site a real, matchable code
instead of falling through to the generic `INT-000` fallback.

### INT-000 Unclassified internal error

**Error text**: `unclassified internal error: {}`

**Cause**: A raw, non-`anyhow` error (e.g. `io::Error`, a panic caught at a
FFI boundary) reached the public API boundary with no `CodedError` anywhere
in its `anyhow::Error` chain. As of [#277](https://github.com/project-minigraf/minigraf/issues/277)
every `bail!`/`anyhow!` call site in the crate has been migrated to a
specific code, so this should now be rare.

**Resolution**:
- Read the wrapped message text (the `{}` above) for the underlying cause.
- If you see this in practice, please file an issue — it likely means a new
  call site was added without going through `bail_coded!`/`err_coded!`.

**Scenario**: An I/O error from the underlying filesystem propagates through
`?` without ever being wrapped in a `CodedError`.

### INT-001 WriteTransaction already in progress on this thread

**Error text**: `a WriteTransaction is already in progress on this thread; use tx.execute() instead`

**Cause**: `Minigraf::execute()` or `Minigraf::begin_write()` was called on
the same thread that already holds an active `WriteTransaction`. Acquiring
the write lock again on that thread would deadlock, so this is detected and
rejected instead.

**Resolution**:
- Use the existing `WriteTransaction`'s `tx.execute()` for further writes
  instead of calling `db.execute()` again on the same thread.

**Scenario**: Code inside a `WriteTransaction` write closure calls
`db.execute("(transact ...)")` on the outer `db` handle instead of
`tx.execute(...)`.

### INT-002 Invalid entity (API layer)

**Error text**: `invalid entity: {}`

**Cause**: `Minigraf::materialize_transaction`/`materialize_retraction`
(the API-layer write path used by `db.execute()` and `WriteTransaction`)
could not resolve an entity ID in a fact. This mirrors `QRY-001` but fires
at the API layer's own materialization step, which runs before the WAL
write for correct WAL-first ordering.

**Resolution**:
- Ensure entity IDs are UUIDs (`#uuid "..."` tagged literals) or values
  resolvable to a UUID.

### INT-003 Invalid value (API layer)

**Error text**: `invalid value: {}`

**Cause**: `Minigraf::materialize_transaction`/`materialize_retraction`
found a fact value of a type Minigraf cannot store. This mirrors `QRY-004`
but fires at the API layer's own materialization step.

**Resolution**:
- Use only supported value types: string, integer, float, boolean, UUID
  ref, or keyword.

### INT-004 Invalid timestamp

**Error text**: `invalid timestamp: {}`

**Cause**: `parse_timestamp` (used for `:as-of`, `:valid-at`, `:valid-from`,
and `:valid-to` date strings) rejected the input — either a timezone offset
was present (only UTC `Z` timestamps are supported, to avoid chrono's local
timezone handling, GHSA-wcg3-cvx6-7396), or the string did not parse as an
RFC 3339 datetime or `YYYY-MM-DD` date.

**Resolution**:
- Use a UTC timestamp like `"2024-01-15T10:00:00Z"` or a bare date like
  `"2024-01-15"`. Do not include a timezone offset such as `+05:30`.

### INT-005 Millisecond value outside supported datetime range

**Error text**: `millisecond value {} is outside the supported datetime range`

**Cause**: `millis_to_timestamp_string` was given a millisecond value
outside the range chrono can represent as a UTC datetime. `i64::MAX`
(`VALID_TIME_FOREVER`) must never be passed to this function directly —
callers should check for the sentinel before formatting.

**Resolution**:
- Check for `VALID_TIME_FOREVER` before formatting a `valid_to` value.

### INT-006 Internal parser error: expected token

**Error text**: `internal parser error: expected {} token`

**Cause**: The Datalog/EDN parser's second pass expected a specific
lexer-token kind (keyword, symbol, string, integer, float, boolean, tagged
literal, or bind slot) at a position the first pass had already validated
would hold that kind. This indicates the parser's two passes have gone out
of sync — a bug in Minigraf, not a user mistake.

**Resolution**:
- File a bug report with the exact input string that triggered this error.

### INT-007 Float literal out of range

**Error text**: `Float literal out of range: {}`

**Cause**: A numeric literal in the input parsed as a float token but
overflowed Rust's `f64` range during conversion.

**Resolution**:
- Use a value within `f64`'s representable range.

### INT-008 Integer literal out of range

**Error text**: `Integer literal out of range: {}`

**Cause**: A numeric literal in the input parsed as an integer token but
overflowed Rust's `i64` range during conversion.

**Resolution**:
- Use a value within `i64`'s representable range (roughly ±9.2×10¹⁸).

### INT-009 Symbol exceeds maximum length

**Error text**: `Symbol exceeds maximum length of {} bytes`

**Cause**: A Datalog symbol (variable name, predicate name, etc.) exceeded
the parser's maximum symbol length.

**Resolution**:
- Shorten the symbol name.

### INT-010 Exceeded maximum recursion depth

**Error text**: `Exceeded maximum recursion depth of {}`

**Cause**: The input's nested list/vector structure exceeded the parser's
maximum recursion depth — a guard against stack overflow on deeply nested
or adversarial input.

**Resolution**:
- Flatten the query structure; deeply nested `(and ...)`/`(or ...)` forms
  are rarely necessary.

### INT-011 Unexpected end of clause

**Error text**: `unexpected end of {}`

**Cause**: The parser reached the end of the input while still expecting
more elements inside a clause (e.g. `:over`, a query vector, an expression
clause, `not`/`not-join`/`or`/`or-join`, a rule invocation, or `and`). The
`{}` names which clause was left incomplete.

**Resolution**:
- Check the input for a missing closing `]`/`)` or a truncated clause.

### INT-012 Duplicate query option

**Error text**: `duplicate {}`

**Cause**: A query specified the same top-level option (`:max-derived-facts`
or `:max-results`) more than once.

**Resolution**:
- Specify each query option at most once.

### INT-013 Query option requires a value

**Error text**: `{} requires a value`

**Cause**: A query option keyword (`:max-derived-facts` or `:max-results`)
was present with no following value.

**Resolution**:
- Supply a positive integer value after the option keyword.

### INT-014 Query option must be >= 1

**Error text**: `{} must be >= 1`

**Cause**: `:max-derived-facts` or `:max-results` was given a value less
than 1.

**Resolution**:
- Use a value of 1 or greater.

### INT-015 Query option must be a positive integer

**Error text**: `{} must be a positive integer`

**Cause**: `:max-derived-facts` or `:max-results` was given a non-integer
value.

**Resolution**:
- Supply a positive integer literal.

### INT-016 (or)/(or-join) nested inside (not)/(not-join)

**Error text**: `(or)/(or-join) cannot appear inside (not)/(not-join)`

**Cause**: A `(not ...)` or `(not-join ...)` clause's body contained a
nested `(or ...)`/`(or-join ...)`, which is not supported.

**Resolution**:
- Restructure the query to avoid nesting `or`/`or-join` inside negation.

### INT-017 Pseudo-attribute not valid in this position

**Error text**: `pseudo-attribute {} is not valid in {} position`

**Cause**: A pseudo-attribute (e.g. `:db/valid-from`) was used in the
entity or value position of a pattern, where only real attributes and
plain values are permitted.

**Resolution**:
- Use pseudo-attributes only in the attribute position of a pattern.

### INT-018 Variable not bound by any outer/earlier clause

**Error text**: `{} {} in {} is not bound by any {} clause`

**Cause**: A variable referenced inside `(not ...)`, `(not-join ...)`, an
expression clause, or `(or-join ...)` was never bound by an outer or
earlier clause in the query.

**Resolution**:
- Ensure every variable used inside negation, expressions, or `or-join`
  also appears in a preceding pattern that binds it.

### INT-019 Datalog parser internal error

**Error text**: `datalog parser: {}`

**Cause**: A shared catch-all for the parser's remaining long-tail syntax
and structural errors (malformed `:with`/`:find` clauses, malformed
fact/rule/pattern vectors, invalid regex literals in `matches?`, and
similar). The `{}` carries the specific underlying message.

**Resolution**:
- Read the wrapped message text for the specific syntax problem and correct
  the input accordingly.

### INT-020 Evaluator iteration/result limit exceeded

**Error text**: `evaluator: iteration or result limit exceeded: {}`

**Cause**: The recursive rule evaluator (semi-naive evaluation) exceeded its
maximum iteration count, maximum derived facts per iteration, or maximum
query result count — guards against infinite recursion/cycles or
runaway-generating rules.

**Resolution**:
- Check the rule for an unintended cycle (e.g. `(ancestor ?x ?y) [?x
  :parent ?y]` combined with a cyclic `:parent` graph). Raise the relevant
  limit via query options if the workload is legitimately large.

### INT-021 Evaluator: unsupported where-clause in evaluate_rule

**Error text**: `evaluator: unsupported where-clause in evaluate_rule: {}`

**Cause**: A rule body contained a `not`, `not-join`, `or`, or `or-join`
clause, but was evaluated through the non-stratified `RecursiveEvaluator`
instead of `StratifiedEvaluator`, which is required for rules with
negation/disjunction. This indicates an internal dispatch bug — the
executor should route negation-containing rules to the stratified
evaluator.

**Resolution**:
- File a bug report with the rule definition that triggered this error.

### INT-022 Datalog evaluator internal error

**Error text**: `datalog evaluator: {}`

**Cause**: A shared catch-all for the recursive rule evaluator's remaining
internal invariant checks around rule invocation/head parsing (e.g. a rule
invocation or head with an unexpected shape, an unbound variable in a rule
head, or a window-function row-index bound check). These conditions should
already be rejected by the parser and are not expected to reach the
evaluator through normal use.

**Resolution**:
- File a bug report with the rule/query that triggered this error.

### INT-023 Packed page: fact exceeds slot capacity

**Error text**: `packed page: fact record exceeds slot capacity: {}`

**Cause**: A fact's postcard-encoded size exceeds the maximum a packed fact
page slot can hold.

**Resolution**:
- Store large payloads externally and reference them with a `Value::String`
  URL/path or `Value::Ref` entity ID, the same guidance as `WAL-003`.

### INT-024 Packed page: malformed page data

**Error text**: `packed page: malformed page data: {}`

**Cause**: A shared catch-all for packed-fact-page invariant violations —
a page shorter than expected, a directory entry or record slice out of
bounds, or an offset/length computation that would overflow. These
indicate on-disk corruption or a bug in the packed-page encoder, not a
user mistake.

**Resolution**:
- If this occurs on a database file that was not modified outside
  Minigraf, please file a bug report with the file (or a minimal
  reproduction) attached.

### INT-025 Unsubstituted :valid-at bind slot reached executor

**Error text**: `internal: unsubstituted :valid-at bind slot reached the executor`

**Cause**: A prepared query's `:valid-at` clause still contained an
unresolved `$slot` bind placeholder when it reached the query executor.
`PreparedQuery::execute` is expected to substitute all bind slots before
handing the query to the executor.

**Resolution**:
- File a bug report — this indicates a gap in `PreparedQuery`'s bind-slot
  substitution.

### INT-026 Temporal pseudo-attributes require :any-valid-time

**Error text**: `temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and :db/tx-id require :any-valid-time; add :any-valid-time to your query`

**Cause**: A query referenced a temporal pseudo-attribute (`:db/valid-from`,
`:db/valid-to`, `:db/tx-count`, `:db/tx-id`) without also specifying
`:valid-at :any-valid-time`. These pseudo-attributes expose per-fact
temporal metadata that only makes sense when the query is not filtering to
a single point in valid time.

**Resolution**:
- Add `:valid-at :any-valid-time` to the query.

### INT-027 Query has no :where clause, rules, or aggregates

**Error text**: `query has no :where clause, rules, or aggregates — nothing binds the variables. Add a :where clause (e.g., [:find ?e ?a ?v :where [?e ?a ?v]]) or use an aggregate.`

**Cause**: A `(query ...)` command's `:find` variables have nothing to
bind them — no `:where` clause, no rule invocation, and no aggregate.

**Resolution**:
- Add a `:where` clause that binds every variable in `:find`.

### INT-028 Rule invocation argument count mismatch

**Error text**: `Rule invocation '{}' must have 1 or 2 arguments, got {}`

**Cause**: A rule was invoked with an argument count other than 1 (entity
only) or 2 (entity and value). Minigraf's rule invocations only support
these two arities.

**Resolution**:
- Invoke the rule with 1 or 2 arguments, matching its definition.

### INT-029 Unknown aggregate function

**Error text**: `unknown aggregate function: '{}'`

**Cause**: A `:find` or `:with` clause referenced an aggregate function
name that is neither a built-in (`count`, `sum`, `avg`, `min`, `max`, etc.)
nor a user-defined function registered via `db.register_aggregate()`.

**Resolution**:
- Check the function name for typos, or register it first with
  `db.register_aggregate()`.

### INT-030 Unknown window function

**Error text**: `unknown window function '{}' — register it with register_aggregate() before querying`

**Cause**: An `:over` (window) clause referenced a function name that is
not a recognized window-compatible built-in and has not been registered.

**Resolution**:
- Register the function with `db.register_aggregate()` before using it in
  an `:over` clause.

### INT-031 or-join variable not bound in incoming scope

**Error text**: `or-join variable {} is not bound in the incoming scope`

**Cause**: An `(or-join [vars] ...)` clause's declared join variable was
not actually bound in the scope the `or-join` was evaluated in.

**Resolution**:
- Ensure every variable listed in `or-join`'s join-vars vector is bound by
  a clause preceding it in the query.

### INT-032 Query executor internal error

**Error text**: `query executor: {}`

**Cause**: A shared catch-all for a handful of query executor invariant
checks that are not expected to be user-reachable (an empty rule head
reaching execution, a rule head not starting with a predicate symbol, or a
window function's row-number position overflowing).

**Resolution**:
- File a bug report with the query that triggered this error.

### INT-033 Missing bind value for slot

**Error text**: `missing bind value for slot '${}'`

**Cause**: `PreparedQuery::execute()` was called without supplying a bind
value for every `$slot` the prepared query declared.

**Resolution**:
- Supply a `BindValue` for every named slot before calling `execute()`.

### INT-034 Bind slot not permitted in attribute position

**Error text**: `bind slot '${}' is not permitted in attribute position; the query optimizer selects an index based on the attribute at prepare time and cannot handle a parameterised attribute`

**Cause**: A `$slot` bind placeholder appeared in the attribute position of
a pattern in a prepared query. The query optimizer picks an index (EAVT,
AEVT, AVET, VAET) based on the attribute at `prepare()` time, which is not
possible if the attribute itself is a runtime-bound parameter.

**Resolution**:
- Use a literal attribute keyword in patterns; reserve bind slots for the
  entity or value position.

### INT-035 Bind slot type mismatch

**Error text**: `slot '${}' in {} position requires {}, got {}`

**Cause**: A bind value supplied to `PreparedQuery::execute()` was of the
wrong `BindValue` variant for the position its slot appears in (entity
position requires `Entity`, value/expression position requires `Val`,
`:as-of` position requires `TxCount` or `Timestamp`, `:valid-at` position
requires `Timestamp` or `AnyValidTime`).

**Resolution**:
- Supply a `BindValue` of the variant the slot's position requires.

### INT-036 Header too short: need N..M bytes

**Error text**: `header too short: need {}..{}, got {} bytes`

**Cause**: A file-header field read (a 4-byte or 8-byte fixed-width field)
extended past the end of the available header bytes.

**Resolution**:
- The file is likely truncated or corrupted. Restore from a backup.

### INT-037 Header slice not the exact expected byte length

**Error text**: `header: slice at {} not exactly {} bytes`

**Cause**: An internal invariant check found a header field slice that was
not exactly the expected width (4 or 8 bytes) after a bounds check that
should have guaranteed it. Indicates a bug in the header-parsing slicing
logic.

**Resolution**:
- File a bug report with the file header bytes attached.

### INT-038 Header too short for a specific field

**Error text**: `header too short for {}`

**Cause**: The v7 header parser ran out of bytes while reading a specific
field (the magic bytes, `fact_page_format`, or one of the reserved padding
bytes).

**Resolution**:
- The file is likely truncated or corrupted. Restore from a backup.

### INT-039 Invalid magic number

**Error text**: `Invalid magic number`

**Cause**: A legacy-format header parsing path found a magic number that
does not match `"MGRF"`. This mirrors `STG-002` but fires on a different
internal parsing path.

**Resolution**:
- The file is not a Minigraf `.graph` file, or is corrupted.

### INT-040 Function/predicate name already registered

**Error text**: `{} '{}' is already registered`

**Cause**: `db.register_aggregate()` or `db.register_predicate()` was
called with a name that is already registered — either a built-in or a
previously registered user-defined function.

**Resolution**:
- Choose a unique name, or only register the function once.

### INT-041 sum: unsupported value type

**Error text**: `sum: expected Integer, Float, or Null, got {}`

**Cause**: The built-in `sum` aggregate encountered a value in its input
group that is not an `Integer`, `Float`, or `Null`.

**Resolution**:
- Ensure the attribute being summed only ever holds numeric or null
  values.

### INT-042 min/max: no non-null values in group

**Error text**: `min/max: no non-null values in group`

**Cause**: The built-in `min`/`max` aggregate was applied to a group whose
values were all `Null`, leaving nothing to compare.

**Resolution**:
- Filter out rows with a null value before aggregating, or handle the
  all-null-group case in your query.

### INT-043 Cannot compare values of different types

**Error text**: `{}: cannot compare {} and {} values`

**Cause**: An aggregate that orders values (`min`, `max`) encountered two
values of different, non-comparable `Value` types within the same group.

**Resolution**:
- Ensure the attribute being aggregated holds a consistent value type
  across all matching facts.

### INT-044 Aggregate: unsupported value type

**Error text**: `{}: expected Integer, Float, String, or Null, got {}`

**Cause**: An aggregate that requires an orderable scalar type (e.g. `min`,
`max`) encountered a value type it does not support (such as a `Boolean`
or `Ref`).

**Resolution**:
- Ensure the aggregated attribute only holds Integer, Float, String, or
  Null values.

### INT-045 Pending fact index out of bounds

**Error text**: `pending fact index {} out of bounds`

**Cause**: A `FactRef` pointed at a pending (not-yet-committed) fact slot
index that no longer exists in the pending fact buffer.

**Resolution**:
- File a bug report — this indicates a lifecycle bug between fact
  materialization and lookup.

### INT-046 Missing CommittedFactReader for a committed FactRef

**Error text**: `no CommittedFactReader but got committed FactRef (page_id={})`

**Cause**: A `FactRef` pointing at a committed (on-disk) fact was resolved
without a `CommittedFactReader` configured to read it.

**Resolution**:
- File a bug report — this indicates the storage backend was constructed
  without the reader it needs for the facts it is indexing.

### INT-047 into_backend: backend Arc has multiple owners

**Error text**: `into_backend: backend Arc has multiple owners`

**Cause**: `PersistentFactStorage::into_backend()` requires exclusive
ownership of its internal `Arc<Mutex<Backend>>` to unwrap it, but another
clone of the `Arc` was still alive when it was called.

**Resolution**:
- Ensure no other handle (e.g. a second `Minigraf` clone) is holding a
  reference to the same backend when converting it back into a raw
  backend.

### INT-048 Storage: arithmetic overflow

**Error text**: `storage: arithmetic overflow: {}`

**Cause**: A shared catch-all for arithmetic-overflow guards throughout the
storage layer (B+tree page/entry-length computations, page-count/page-id
arithmetic, packed-page offset arithmetic) — a checked or `saturating_*`
computation would have overflowed its target integer width. Only reachable
on databases far larger than any realistic deployment.

**Resolution**:
- File a bug report if this occurs on a database of reasonable size — it
  likely indicates a corrupt page count or index in the file header.

### INT-049 Storage: internal invariant violation

**Error text**: `storage: internal invariant violation: {}`

**Cause**: A shared catch-all for storage-layer invariant violations —
malformed or corrupted B+tree/packed-fact page bytes, an out-of-bounds
page/entry index, an unexpected page type where a specific type was
required, or an "impossible" empty internal structure (e.g. `BUG:
cur_first_key empty when writing leaf page`). These indicate either
on-disk corruption or a bug in Minigraf's B+tree/page-encoding logic.

**Resolution**:
- If this occurs on a database file that was not modified outside
  Minigraf, please file a bug report with the file (or a minimal
  reproduction) attached.

### INT-050 Internal lock poisoned

**Error text**: `{} lock poisoned`

**Cause**: One of Minigraf's internal `Mutex`/`RwLock` guards (the shared
fact-storage data lock, the page cache lock, the in-memory backend's page
table lock, or the on-disk `MutexStorageBackend`'s lock) was poisoned by a
previous operation panicking while holding it.

**Resolution**:
- Restart the process. If panics recur, investigate the root cause before
  retrying — a poisoned lock is permanent for the life of the process.

### INT-051 Invalid page size

**Error text**: `Invalid page size: {} bytes (expected {})`

**Cause**: A page passed to a storage backend's `write_page` was not
exactly `PAGE_SIZE` (4096) bytes.

**Resolution**:
- This is an internal contract violation between the storage layer and its
  backend, not a user mistake. File a bug report.

### INT-052 Page not found

**Error text**: `Page {} not found`

**Cause**: A storage backend was asked to read a page ID that does not
exist in its page table (in-memory or browser IndexedDB-backed backends).

**Resolution**:
- File a bug report — this indicates a page was referenced (e.g. by a
  B+tree pointer) without having first been written.

### INT-053 Header checksum mismatch: possible file corruption

**Error text**: `Header checksum mismatch: possible file corruption. Database may be damaged.`

**Cause**: The v7 file header's stored checksum does not match a freshly
computed checksum of the header bytes — the header was modified outside
Minigraf, corrupted by a partial write, or damaged by the underlying
storage medium.

**Resolution**:
- Restore the database file from a backup. If the file was open at the
  time of a crash, check for a `.wal` sidecar next to it — replaying it on
  the last-known-good copy may recover recent writes.

### INT-054 Unstratifiable negative recursion cycle

**Error text**: `unstratifiable: predicate '{}' is involved in a negative cycle through '{}'`

**Cause**: A set of rules contains a predicate that depends negatively
(through `not`/`not-join`) on itself, directly or transitively — this has
no well-defined semantics under stratified negation and cannot be
evaluated.

**Resolution**:
- Restructure the rules so that no predicate's definition negatively
  depends on itself, even indirectly through other rules.

### INT-055 Rule predicate disappeared during rollback

**Error text**: `rule predicate '{}' disappeared during rollback`

**Cause**: `WriteTransaction::rollback()` (or an implicit rollback on
drop) attempted to remove a rule that had been staged for registration in
this transaction, but the rule registry no longer contained it.

**Resolution**:
- File a bug report — this indicates a lifecycle bug in
  `WriteTransaction`'s rule-staging/rollback logic.
