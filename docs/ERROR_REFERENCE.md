# Minigraf Error Reference

This document covers every user-facing error produced by the core Minigraf Rust library.
Errors surface as `anyhow::Error` values returned from `db.execute()`, `db.prepare()`,
and related API methods.

**Reference codes** (e.g. `PRS-001`) are documentation-only identifiers. They do not
appear in runtime output today — runtime codes are tracked in
[#277](https://github.com/project-minigraf/minigraf/issues/277).

**Out of scope**: FFI/binding errors from `minigraf-python`, `minigraf-node`, and
`minigraf-wasm` are handled in those repos.

---

## Quick Reference Table

| Code | Error (prefix) | Category |
|------|----------------|----------|
<!-- filled in Task 12 -->

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

**Error text**: `Unexpected character: @`

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

**Error text**: `Unexpected token: Keyword(":find")`

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

**Error text**: `String exceeds maximum length of 4096 bytes`

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

**Error text**: `Keyword exceeds maximum length of 4096 bytes`

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

**Error text**: `Tagged literal exceeds maximum length of 4096 bytes`

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

**Error text**: `Unknown command: upsert`

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

**Error text**: `:as-of counter must be non-negative, got -1`

**Cause**: Transaction counters start at 1. A negative integer was passed to `:as-of`.

**Resolution**:
- Use a non-negative integer.
- `:as-of 0` returns the database before any transaction; `:as-of 1` returns the state after the first transaction.
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel).

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :as-of -1})
```

### PRS-016 :as-of must be integer or ISO 8601 string

**Error text**: `:as-of must be an integer (counter) or ISO 8601 string, got :now`

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

### PRS-018 :valid-at must be ISO 8601 or :any-valid-time

**Error text**: `:valid-at must be an ISO 8601 string or :any-valid-time, got 42`

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

**Error text**: `:valid-from must be an ISO 8601 string, got 42`

**Cause**: The `:valid-from` key in a fact's option map must be an ISO 8601 string, not an integer or keyword.

**Resolution**:
- Use `"2024-01-01T00:00:00Z"` format.
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel)

**Example**:
```datalog
(transact [[#uuid "..." :name "Alice" {:valid-from 1704067200000}]])
```
*Use `:valid-from "2024-01-01T00:00:00Z"` (ISO 8601), not a Unix timestamp integer*

### PRS-020 :valid-to must be ISO 8601

**Error text**: `:valid-to must be an ISO 8601 string, got :forever`

**Cause**: The `:valid-to` key in a fact's option map must be an ISO 8601 string.

**Resolution**:
- Use `"2024-12-31T23:59:59Z"` format.
- To express "forever", omit `:valid-to` (the default is `VALID_TIME_FOREVER`).
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel)

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

**Error text**: `':with' variable ?x not bound in :where`

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

**Error text**: `Aggregate variable ?amount not bound in :where`

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

**Error text**: `Aggregate expression must have exactly 2 elements (func ?var), got 3`

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

**Error text**: `Aggregate function name must be a symbol, got Keyword(":sum")`

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

**Error text**: `'rank' is a window function and requires an ':over (...)' clause`

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

**Error text**: `'lag' is not supported in this version; lag/lead are planned for a future release`

**Cause**: The `lag` and `lead` window functions are not yet implemented in this version.

**Resolution**:
- Remove `lag`/`lead` from the query.
- Follow [#277](https://github.com/project-minigraf/minigraf/issues/277) for progress on additional window functions.

**Example**:
```datalog
(query {:find [?e (lag ?score :over (:order-by ?ts))]
        :where [[?e :score ?score] [?e :ts ?ts]]})
```
*`lag` is not yet available; restructure the query without it*

### PRS-031 Function is not window-compatible

**Error text**: `'sum' is not window-compatible and cannot be used with ':over'`

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

**Error text**: `'ntile' requires a variable argument (starting with ?) before ':over'`

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

**Error text**: `'ntile' requires ':over' after the variable argument`

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

**Error text**: `'rank' requires ':over' immediately after the function name (no variable argument)`

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

**Error text**: `unknown option in ':over' clause: ':ascending'`

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

**Error text**: `unexpected element in ':over' clause: Integer(5)`

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

**Cause**: The argument to `transact` is not a vector. This variant fires on a different call path than PRS-041 but has the same meaning.

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

**Cause**: The argument to `retract` is not a vector. Same meaning as PRS-043 on a different code path.

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

**Error text**: `Fact must have at least 3 elements (E A V), got 2`

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

**Error text**: `Optional 4th element of a fact must be a map {:valid-from ... :valid-to ...}, got "2024-01-01"`

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
        :where [(not (not [[?e :active true]]))]})
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
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

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
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

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
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

**Example**:
```datalog
(query {:find [?e]
        :where [(or-join ?e [[?e :type :a]])]})
```
*Wrap join vars in a vector: `(or-join [?e] ...)`*

### PRS-056 (or-join) join variables must be logic variables

**Error text**: `(or-join) join variables must be logic variables, got Keyword(":e")`

**Cause**: Join variables in `(or-join [?e ...])` must start with `?`.

**Resolution**:
- Use `?variable` not `:keyword` or a string.
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

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
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

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
- See the [Datalog Reference](../../.wiki/Datalog-Reference.md).

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

**Error text**: `Expected pattern vector or rule invocation in :where clause, got Keyword(":name")`

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

**Error text**: `Unexpected element in query: Keyword(":limit")`

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

**Error text**: `expression head must be a symbol, got Keyword(":+")`

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

**Error text**: `abs takes exactly 1 argument`

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

**Error text**: `+ takes exactly 2 arguments`

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

**Error text**: `unknown expression operator: floor-div`

**Cause**: An expression clause used a function name that Minigraf does not recognise. Built-in operators include: `+`, `-`, `*`, `/`, `mod`, `quot`, `abs`, `min`, `max`, `str`, `not`, `=`, `!=`, `<`, `<=`, `>`, `>=`, `matches?`, `starts-with?`, `ends-with?`, `contains?`.

**Resolution**:
- Check spelling against the supported operator list above.
- For missing operators, register a custom function via `db.register_function()`.
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions).

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :amount ?a] [(floor-div ?a 100) ?bucket]]})
```
*`floor-div` is not built-in; use `(quot ?a 100)` instead*

### PRS-068 Expression clause must be [(expr)] or [(expr) ?out]

**Error text**: `expression clause must be [(expr)] or [(expr) ?out], got 3 elements`

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

**Error text**: `expression output must be a ?variable, got Keyword(":result")`

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

**Error text**: `unsupported expression argument: Map({...})`

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

**Error text**: `Unknown tagged literal: #base64`

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

**Error text**: `Bind slot name exceeds maximum length of 4096 bytes`

**Cause**: A prepared-query bind slot name `$name` in a `prepare()` call exceeds 4096 bytes.

**Resolution**:
- Use shorter bind slot names.
- Bind slot names should be brief and descriptive, e.g. `$name`, `$entity-id`.

**Example**:
```rust
db.prepare("(query {:find [?e] :where [[?e :name $<4097-char-name>]]})")
```
*Shorten the bind slot name*

---

## QRY — Query Execution Errors

Query execution errors occur after parsing succeeds, during pattern matching,
predicate evaluation, or fact transacting.

<!-- entries added in Task 8 -->

---

## STG — Storage Errors

Storage errors relate to reading or writing the `.graph` file. They typically
indicate a corrupted, truncated, or incompatible database file.

See the [file format section in README](../README.md#file-format) for version history.

<!-- entries added in Task 9 -->

---

## WAL — Write-Ahead Log Errors

WAL errors relate to the sidecar `.wal` file written alongside the `.graph` file.
The WAL is replayed on open and deleted on checkpoint.

<!-- entries added in Task 10 -->

---

## API — Database API Errors

API errors indicate a violated contract in how the public `Minigraf` or
`WriteTransaction` API is used.

<!-- entries added in Task 11 -->

---

## Appendix: Internal Errors

The following error strings indicate a bug in the Minigraf library itself, not a
user mistake. If you encounter one, please
[open a GitHub issue](https://github.com/project-minigraf/minigraf/issues/new)
with the full error message and the input that triggered it.

- `internal parser error: expected keyword token`
- `internal parser error: expected symbol token`
- `internal parser error: expected string token`
- `internal parser error: expected integer token`
- `internal parser error: expected float token`
- `internal parser error: expected boolean token`
- `internal parser error: expected tagged literal token`
- `internal parser error: expected bind slot token`
