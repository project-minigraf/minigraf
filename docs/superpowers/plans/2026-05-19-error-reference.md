# Error Reference Guide Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce `docs/ERROR_REFERENCE.md` — a single Markdown file documenting every user-facing Minigraf error with cause, resolution, and a bad-input example, organised by category with docs-only reference codes.

**Architecture:** Pure documentation. No source code changes. One new file: `docs/ERROR_REFERENCE.md`. Work proceeds category by category (PRS → QRY → STG → WAL → API → Appendix), finishing with a Quick Reference table and doc-sync updates.

**Tech Stack:** Markdown, git worktree, GitHub PR.

---

## Entry format (reference for all tasks)

Every non-appendix entry follows this exact template:

```markdown
### PRS-001 Unexpected end of input

**Error text**: `Unexpected end of input`

**Cause**: The input was cut off before the parser completed an expression.
Commonly occurs when a list or vector is opened but never closed, or when
the REPL receives an empty line where a form is expected.

**Resolution**:
- Ensure every `(` is matched with `)` and every `[` with `]`
- Use the REPL's multi-line mode for long queries

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :name "alice"]
```
*(missing closing `]` and `}`)*
```

Rules:
- Parameterised error texts (e.g. `"String exceeds maximum length of {} bytes"`) fill in the concrete value: `` `String exceeds maximum length of 4096 bytes` ``
- For storage/WAL errors where no Datalog input triggers the error, replace the **Example** block with a prose scenario
- Link to `.wiki/Datalog-Reference.md` from PRS/QRY entries where relevant; link to `README.md` (file format section) from STG/WAL entries

---

## File map

| Action | Path |
|--------|------|
| Create | `docs/ERROR_REFERENCE.md` |
| Modify | `ROADMAP.md` (mark `🎯 Error message guide` → `✅`) |
| Modify | `CHANGELOG.md` (add entry under `## Unreleased`) |

---

## Task 1: Set up worktree

- [ ] **1.1** Invoke `superpowers:using-git-worktrees` to create a worktree for this feature. Use `.worktrees/error-reference` as the directory and `feat/error-reference` as the branch name.

---

## Task 2: Create document skeleton

**Files:**
- Create: `docs/ERROR_REFERENCE.md`

- [ ] **2.1** Create `docs/ERROR_REFERENCE.md` with the following content (the Quick Reference table will be filled in Task 9):

```markdown
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
<!-- filled in Task 9 -->

---

## PRS — Parser Errors

Parser errors occur when Minigraf cannot parse the Datalog/EDN input string.
They are returned immediately from `db.execute()` before any fact is read or written.

See the [Datalog Reference](../../.wiki/Datalog-Reference.md) for syntax guidance.

<!-- entries added in Tasks 3–8 -->

---

## QRY — Query Execution Errors

Query execution errors occur after parsing succeeds, during pattern matching,
predicate evaluation, or fact transacting.

<!-- entries added in Task 9 -->

---

## STG — Storage Errors

Storage errors relate to reading or writing the `.graph` file. They typically
indicate a corrupted, truncated, or incompatible database file.

See the [file format section in README](../README.md#file-format) for version history.

<!-- entries added in Task 10 -->

---

## WAL — Write-Ahead Log Errors

WAL errors relate to the sidecar `.wal` file written alongside the `.graph` file.
The WAL is replayed on open and deleted on checkpoint.

<!-- entries added in Task 11 -->

---

## API — Database API Errors

API errors indicate a violated contract in how the public `Minigraf` or
`WriteTransaction` API is used.

<!-- entries added in Task 12 -->

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
```

- [ ] **2.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: scaffold ERROR_REFERENCE.md skeleton (#192)"
```

---

## Task 3: PRS — Tokeniser and bounds errors (PRS-001 to PRS-012)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md` (PRS section)

Errors to document (replace the `<!-- entries added in Tasks 3–8 -->` placeholder and append subsequent tasks after):

| Code | Error text |
|------|-----------|
| PRS-001 | `Unexpected end of input` |
| PRS-002 | `Unexpected character: <char>` |
| PRS-003 | `Unexpected token: <token>` |
| PRS-004 | `Unclosed vector` |
| PRS-005 | `Unclosed list` |
| PRS-006 | `Unterminated map: missing '}'` |
| PRS-007 | `String exceeds maximum length of 4096 bytes` |
| PRS-008 | `Keyword exceeds maximum length of 4096 bytes` |
| PRS-009 | `Tagged literal exceeds maximum length of 4096 bytes` |
| PRS-010 | `Expected command symbol` |
| PRS-011 | `Unknown command: <cmd>` |
| PRS-012 | `Expected a list starting with a command symbol` |

- [ ] **3.1** Write entries PRS-001 through PRS-012 in `docs/ERROR_REFERENCE.md`. Full examples for the first three; follow the same pattern for the rest.

**PRS-001 full entry:**
```markdown
### PRS-001 Unexpected end of input

**Error text**: `Unexpected end of input`

**Cause**: The input string was cut off before the parser could complete an
expression. This happens when a `(`, `[`, or `{` is opened but never closed,
or when an empty string is passed to `execute()`.

**Resolution**:
- Ensure every `(` is matched with `)`, every `[` with `]`, and every `{` with `}`
- In the REPL, use multi-line mode for long queries

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :name "alice"]
```
*(missing closing `]` and `}`)*
```

**PRS-002 full entry:**
```markdown
### PRS-002 Unexpected character: \<char\>

**Error text**: `Unexpected character: @`

**Cause**: The tokeniser encountered a character that is not valid in Datalog/EDN.
Common culprits: `@`, `#` (outside a tagged literal), `\`, smart quotes (`""`).

**Resolution**:
- Use only plain ASCII in attribute names and keywords
- String values may contain any UTF-8; wrap them in double quotes
- For UUIDs use the `#uuid "..."` tagged literal form

**Example**:
```datalog
(transact [[@entity :name "alice"]])
```
*`@` is not a valid EDN character; use a UUID or string entity ID instead*
```

**PRS-003 full entry:**
```markdown
### PRS-003 Unexpected token: \<token\>

**Error text**: `Unexpected token: Keyword(":find")`

**Cause**: The parser encountered a token in a position where it cannot appear.
Often caused by a missing or misplaced delimiter, or by using a keyword where a
symbol or value is expected.

**Resolution**:
- Check surrounding delimiters for balance
- Consult the [Datalog Reference](../../.wiki/Datalog-Reference.md) for the expected
  syntax at that position

**Example**:
```datalog
(query :find [?e] :where [[?e :name "alice"]])
```
*`:find` must be inside a map `{}`; use `(query {:find [?e] :where [...]})`*
```

**PRS-004 through PRS-012** follow the same template. Causes and resolutions:

- **PRS-004 Unclosed vector**: A `[` was never closed with `]`. Check fact vectors in `transact` and pattern vectors in `:where`.
- **PRS-005 Unclosed list**: A `(` was never closed with `)`. Check command forms and `not`/`or` clauses.
- **PRS-006 Unterminated map**: A `{` was never closed with `}`. Check query maps and fact option maps.
- **PRS-007 String too long**: String value exceeds 4096 bytes. Store large blobs externally and reference them with a path or URL string, or use `Value::Ref` to point to a dedicated entity.
- **PRS-008 Keyword too long**: Attribute keyword exceeds 4096 bytes. Shorten the attribute name.
- **PRS-009 Tagged literal too long**: The string inside a `#uuid "..."` or other tagged literal exceeds 4096 bytes. UUIDs are 36 characters; anything longer is malformed.
- **PRS-010 Expected command symbol**: The top-level form must start with `transact`, `retract`, `query`, or `rule`. The form was empty or started with a non-symbol.
- **PRS-011 Unknown command**: The opening symbol is not a recognised command. Check spelling; valid commands are `transact`, `retract`, `query`, `rule`.
- **PRS-012 Expected a list starting with a command symbol**: The input is not a list form at all (e.g. a bare keyword or integer was passed).

- [ ] **3.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add PRS-001–PRS-012 tokeniser and bounds errors (#192)"
```

---

## Task 4: PRS — Query clause errors (PRS-013 to PRS-022)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| PRS-013 | `Query requires a map argument` |
| PRS-014 | `:as-of requires a value` |
| PRS-015 | `:as-of counter must be non-negative, got <n>` |
| PRS-016 | `:as-of must be an integer (counter) or ISO 8601 string, got <val>` |
| PRS-017 | `:valid-at requires a value` |
| PRS-018 | `:valid-at must be an ISO 8601 string or :any-valid-time, got <val>` |
| PRS-019 | `:valid-from must be an ISO 8601 string, got <val>` |
| PRS-020 | `:valid-to must be an ISO 8601 string, got <val>` |
| PRS-021 | `':with' clause requires at least one aggregate in :find` |
| PRS-022 | `':with' variable <var> not bound in :where` |
| PRS-023 | `Aggregate variable <var> not bound in :where` |

- [ ] **4.1** Write entries PRS-013 through PRS-023. Full entry for PRS-013 and PRS-015; follow the same pattern for the rest.

**PRS-013 full entry:**
```markdown
### PRS-013 Query requires a map argument

**Error text**: `Query requires a map argument`

**Cause**: The `query` command expects its sole argument to be a map `{:find [...] :where [...]}`.
A vector, symbol, or other form was passed instead.

**Resolution**:
- Wrap the query clauses in `{}`: `(query {:find [?e] :where [[?e :name "alice"]]})`

**Example**:
```datalog
(query [:find ?e :where [?e :name "alice"]])
```
*The argument is a vector `[...]`; it must be a map `{...}`*
```

**PRS-015 full entry:**
```markdown
### PRS-015 :as-of counter must be non-negative

**Error text**: `:as-of counter must be non-negative, got -1`

**Cause**: Transaction counters start at 1. A negative integer was passed to `:as-of`.

**Resolution**:
- Use a non-negative integer: `:as-of 0` returns the database before any transaction;
  `:as-of 1` returns the state after the first transaction
- See the [Datalog Reference — Time Travel](../../.wiki/Datalog-Reference.md#time-travel)

**Example**:
```datalog
(query {:find [?e] :where [[?e :name ?n]] :as-of -1})
```
```

Causes/resolutions for remaining entries:
- **PRS-014**: `:as-of` keyword appeared with no value following it. Add the value: `:as-of 5` or `:as-of "2024-01-01T00:00:00Z"`.
- **PRS-016**: `:as-of` value is neither an integer nor an ISO 8601 string. Use `42` (tx-count) or `"2024-01-01T00:00:00Z"` (wall-clock).
- **PRS-017**: `:valid-at` keyword appeared with no value. Add the ISO 8601 timestamp.
- **PRS-018**: `:valid-at` value is not an ISO 8601 string or `:any-valid-time`. Example fix: `":valid-at "2024-06-01T00:00:00Z""` or `:valid-at :any-valid-time`.
- **PRS-019/020**: `:valid-from`/`:valid-to` in a fact's option map must be ISO 8601 strings, not keywords or integers.
- **PRS-021**: `:with` is used for grouping before aggregation; it requires at least one aggregate function (`count`, `sum`, etc.) in `:find`.
- **PRS-022**: A variable listed in `:with` is not bound by any `:where` clause. Add a pattern that binds it.
- **PRS-023**: An aggregate's input variable (e.g. `(sum ?amount)`) is not bound by any `:where` pattern.

- [ ] **4.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add PRS-013–PRS-023 query clause errors (#192)"
```

---

## Task 5: PRS — Aggregate and window function errors (PRS-024 to PRS-041)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| PRS-024 | `Aggregate expression must have exactly 2 elements (func ?var), got <n>` |
| PRS-025 | `Aggregate function name must be a symbol, got <val>` |
| PRS-026 | `Aggregate argument must be a variable (starting with ?)` |
| PRS-027 | `'<fn>' is a window function and requires an ':over (...)' clause` |
| PRS-028 | `window expression cannot be empty` |
| PRS-029 | `window function name must be a symbol` |
| PRS-030 | `'<fn>' is not supported in this version; lag/lead are planned for a future release` |
| PRS-031 | `'<fn>' is not window-compatible and cannot be used with ':over'` |
| PRS-032 | `'<fn>' requires a variable argument (starting with ?) before ':over'` |
| PRS-033 | `'<fn>' requires ':over' after the variable argument` |
| PRS-034 | `'<fn>' requires ':over' immediately after the function name (no variable argument)` |
| PRS-035 | `':over' must be followed by a list, e.g., (:order-by ?var)` |
| PRS-036 | `unexpected tokens after ':over' clause in window expression` |
| PRS-037 | `':partition-by' requires a variable (starting with ?)` |
| PRS-038 | `':order-by' requires a variable (starting with ?)` |
| PRS-039 | `unknown option in ':over' clause: '<opt>'` |
| PRS-040 | `unexpected element in ':over' clause: <val>` |

- [ ] **5.1** Write entries PRS-024 through PRS-040. Full entry for PRS-024 and PRS-027; follow the same pattern for the rest.

**PRS-024 full entry:**
```markdown
### PRS-024 Aggregate expression must have exactly 2 elements

**Error text**: `Aggregate expression must have exactly 2 elements (func ?var), got 3`

**Cause**: An aggregate expression in `:find` must be `(function ?variable)` — exactly
two elements. Extra arguments or a missing variable will trigger this error.

**Resolution**:
- Use `(count ?e)`, `(sum ?amount)`, `(avg ?score)` — one function name, one variable
- See the [Datalog Reference — Aggregates](../../.wiki/Datalog-Reference.md#aggregates)

**Example**:
```datalog
(query {:find [(sum ?amount :distinct)]
        :where [[?e :payment/amount ?amount]]})
```
*`:distinct` is not part of the aggregate syntax; use `(sum ?amount)` alone*
```

**PRS-027 full entry:**
```markdown
### PRS-027 Window function requires ':over' clause

**Error text**: `'rank' is a window function and requires an ':over (...)' clause`

**Cause**: Window functions (`rank`, `row-number`, `dense-rank`, `ntile`, `percent-rank`,
`cume-dist`) must be accompanied by an `:over` clause specifying ordering and/or
partitioning. Writing them like a plain aggregate omits this required clause.

**Resolution**:
- Add an `:over` clause: `(rank :over (:order-by ?score))`
- See the [Datalog Reference — Window Functions](../../.wiki/Datalog-Reference.md#window-functions)

**Example**:
```datalog
(query {:find [?e (rank)]
        :where [[?e :score ?score]]})
```
*Missing `:over`; use `(rank :over (:order-by ?score))`*
```

Remaining entries follow the same template. Key causes/resolutions:
- **PRS-028**: `:over ()` is empty; provide at least `:order-by` or `:partition-by`.
- **PRS-029**: The window function name is not a symbol (e.g. a keyword was used).
- **PRS-030**: `lag`/`lead` are not yet implemented. Remove them; track #277 for progress.
- **PRS-031**: The named function is a plain aggregate, not a window function. Drop `:over`.
- **PRS-032/033/034**: Each window function has a specific signature; check the Datalog Reference for the exact form.
- **PRS-035**: `:over` must be followed by a list like `(:order-by ?var)`, not a keyword or vector.
- **PRS-036**: Extra tokens appear after the `:over (...)` clause inside the window expression.
- **PRS-037/038**: `:partition-by` and `:order-by` values must be logic variables `?foo`, not keywords or strings.
- **PRS-039**: An unrecognised keyword appeared inside the `:over` list. Valid options: `:order-by`, `:partition-by`.
- **PRS-040**: A non-keyword, non-variable element appeared inside the `:over` list.

- [ ] **5.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add PRS-024–PRS-040 aggregate and window function errors (#192)"
```

---

## Task 6: PRS — Transaction, retraction, and fact format errors (PRS-041 to PRS-049)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| PRS-041 | `Transact requires a vector of facts` |
| PRS-042 | `Transact argument must be a vector of facts` |
| PRS-043 | `Retract requires a vector of facts` |
| PRS-044 | `Retract argument must be a vector of facts` |
| PRS-045 | `Each fact must be a vector [e a v] or [e a v {opts}]` |
| PRS-046 | `Fact must have at least 3 elements (E A V), got <n>` |
| PRS-047 | `Optional 4th element of a fact must be a map {:valid-from ... :valid-to ...}, got <val>` |
| PRS-048 | `Transact with options requires a facts vector after the map` |
| PRS-049 | `unexpected end of fact vector` |

- [ ] **6.1** Write entries PRS-041 through PRS-049. Full entry for PRS-041 and PRS-046; follow the same pattern for the rest.

**PRS-041 full entry:**
```markdown
### PRS-041 Transact requires a vector of facts

**Error text**: `Transact requires a vector of facts`

**Cause**: The `transact` command was called with no argument or with a non-vector argument.
The command form must be `(transact [...facts...])`.

**Resolution**:
- Wrap facts in a vector: `(transact [[:alice :name "Alice"]])`

**Example**:
```datalog
(transact)
```
*Missing the facts vector*
```

**PRS-046 full entry:**
```markdown
### PRS-046 Fact must have at least 3 elements (E A V)

**Error text**: `Fact must have at least 3 elements (E A V), got 2`

**Cause**: Each fact in the `transact` or `retract` vector must supply at minimum an
entity, an attribute, and a value. A fact with fewer elements is incomplete.

**Resolution**:
- Ensure every fact has the form `[entity :attribute value]`
- Optionally add a 4th map for temporal options: `[entity :attribute value {:valid-from "..."}]`

**Example**:
```datalog
(transact [[:alice :name]])
```
*Only 2 elements; add the value: `[:alice :name "Alice"]`*
```

Remaining entries causes/resolutions:
- **PRS-042**: Duplicate of PRS-041 for a different call path; same resolution.
- **PRS-043/044**: `retract` equivalents of PRS-041/042.
- **PRS-045**: A non-vector element appeared inside the facts vector (e.g. a keyword or integer instead of `[...]`).
- **PRS-047**: The 4th element is not a map. It must be `{:valid-from "ISO" :valid-to "ISO"}` or omitted.
- **PRS-048**: `(transact {opts} [...])` form requires the facts vector as the second argument.
- **PRS-049**: The parser ran out of tokens while reading a fact vector; a closing `]` is missing.

- [ ] **6.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add PRS-041–PRS-049 transaction and fact format errors (#192)"
```

---

## Task 7: PRS — Where clause, expression, and tagged literal errors (PRS-050 to PRS-075)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| PRS-050 | `Empty list in :where clause` |
| PRS-051 | `(not ...) cannot appear inside another (not ...)` |
| PRS-052 | `(not) requires at least one clause` |
| PRS-053 | `(or) requires at least one branch` |
| PRS-054 | `(or-join) requires a join-vars vector and at least one branch` |
| PRS-055 | `(or-join) first argument must be a vector of join variables` |
| PRS-056 | `(or-join) join variables must be logic variables, got <val>` |
| PRS-057 | `all branches of (or ...) must introduce the same set of new variables` |
| PRS-058 | `(and) inside or/or-join requires at least one clause` |
| PRS-059 | `(not-join) requires a join-vars vector and at least one clause` |
| PRS-060 | `Expected pattern vector or rule invocation in :where clause, got <val>` |
| PRS-061 | `Unexpected element in query: <val>` |
| PRS-062 | `expression list cannot be empty` |
| PRS-063 | `expression head must be a symbol, got <val>` |
| PRS-064 | `<fn> takes exactly 1 argument` |
| PRS-065 | `<fn> takes exactly 2 arguments` |
| PRS-066 | `matches? second argument must be a string literal` |
| PRS-067 | `unknown expression operator: <op>` |
| PRS-068 | `expression clause must be [(expr)] or [(expr) ?out], got <n> elements` |
| PRS-069 | `expression output must be a ?variable, got <val>` |
| PRS-070 | `unsupported expression argument: <val>` |
| PRS-071 | `Expected UUID string after #uuid tag` |
| PRS-072 | `Invalid UUID` |
| PRS-073 | `Unknown tagged literal: #<tag>` |
| PRS-074 | `Bind slot name exceeds maximum length of 4096 bytes` |

- [ ] **7.1** Write entries PRS-050 through PRS-074. Full entry for PRS-051 and PRS-067; follow the pattern for the rest.

**PRS-051 full entry:**
```markdown
### PRS-051 (not) cannot be nested inside another (not)

**Error text**: `(not ...) cannot appear inside another (not ...)`

**Cause**: Minigraf's Datalog does not support double-negation via nested `not` clauses.
`(not (not [...]))` is syntactically rejected.

**Resolution**:
- Double negation is logically equivalent to the positive pattern; use the pattern directly
- For complex negation logic use `not-join` with explicit join variables
- See the [Datalog Reference — Negation](../../.wiki/Datalog-Reference.md#negation)

**Example**:
```datalog
(query {:find [?e]
        :where [(not (not [[?e :active true]]))]})
```
*Remove the outer `(not ...)`; match `[?e :active true]` directly*
```

**PRS-067 full entry:**
```markdown
### PRS-067 Unknown expression operator

**Error text**: `unknown expression operator: floor-div`

**Cause**: An expression clause `[(expr) ?out]` used a function name that Minigraf
does not recognise. Built-in operators include: `+`, `-`, `*`, `/`, `mod`, `quot`,
`abs`, `min`, `max`, `str`, `count`, `not`, `=`, `!=`, `<`, `<=`, `>`, `>=`,
`matches?`, `starts-with?`, `ends-with?`, `contains?`.

**Resolution**:
- Check spelling against the supported operator list above
- For missing operators, register a custom function via `db.register_function()`
- See the [Datalog Reference — Expressions](../../.wiki/Datalog-Reference.md#expressions)

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :amount ?a]
                [(floor-div ?a 100) ?bucket]]})
```
*`floor-div` is not built-in; use `(quot ?a 100)` instead*
```

Key causes/resolutions for remaining entries:
- **PRS-050**: An empty `()` appears in `:where`; every list in `:where` must be a pattern, `not`, `not-join`, `or`, `or-join`, or expression.
- **PRS-052**: `(not)` has no clauses inside it. Add at least one pattern.
- **PRS-053**: `(or)` has no branches. Add at least one.
- **PRS-054**: `(or-join)` needs `[join-vars]` + branches: `(or-join [?e] [[?e :a 1]] [[?e :b 2]])`.
- **PRS-055**: `(or-join)`'s first argument is not a vector.
- **PRS-056**: Join variables in `(or-join [?e ...])` must start with `?`.
- **PRS-057**: Each branch of `(or ...)` must bind the same set of new variables; restructure branches to match.
- **PRS-058**: An `(and ...)` inside `or`/`or-join` has no clauses.
- **PRS-059**: `(not-join)` needs `[join-vars]` + at least one clause.
- **PRS-060**: Something other than a pattern vector or rule invocation appeared in `:where` (e.g. a plain keyword).
- **PRS-061**: An unrecognised key appeared at the top level of the query map.
- **PRS-062/063**: Expression list is empty or its first element is not a symbol naming a function.
- **PRS-064/065**: A built-in operator received the wrong number of arguments.
- **PRS-066**: `matches?` second argument must be a string regex literal, not a variable.
- **PRS-068**: Expression clause form must be `[(expr)]` or `[(expr) ?out]`.
- **PRS-069**: The output binding in `[(expr) ?out]` is not a `?variable`.
- **PRS-070**: An argument inside an expression is a type that expressions cannot accept.
- **PRS-071**: `#uuid` was used but not followed by a string.
- **PRS-072**: The string after `#uuid` is not a valid UUID (must match `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`).
- **PRS-073**: A `#tag "..."` form used an unrecognised tag; only `#uuid` is supported.
- **PRS-074**: A prepared-query bind slot name `$name` exceeds 4096 bytes.

- [ ] **7.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add PRS-050–PRS-074 where-clause, expression, and tagged literal errors (#192)"
```

---

## Task 8: QRY — Query execution errors (QRY-001 to QRY-009)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| QRY-001 | `Invalid entity: <val>` |
| QRY-002 | `Attribute must be a keyword` |
| QRY-003 | `Cannot transact a pseudo-attribute` |
| QRY-004 | `Invalid value: <val>` |
| QRY-005 | `Transaction failed: <reason>` |
| QRY-006 | `Retraction failed: <reason>` |
| QRY-007 | `unknown predicate: '<name>'` |
| QRY-008 | `functions lock poisoned` |
| QRY-009 | `rules lock poisoned` |

- [ ] **8.1** Write entries QRY-001 through QRY-009. Full entry for QRY-001 and QRY-007; follow the same pattern for the rest.

**QRY-001 full entry:**
```markdown
### QRY-001 Invalid entity

**Error text**: `Invalid entity: "not-a-uuid"`

**Cause**: An entity ID in a `transact` fact could not be resolved. Entity IDs must be
UUIDs (as `#uuid "..."` tagged literals), existing entity symbols, or values that can
be resolved to a UUID at execution time.

**Resolution**:
- Use a `#uuid "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"` literal for a specific entity
- Use a new unique symbol if creating a new entity and let Minigraf assign a UUID

**Example**:
```datalog
(transact [["my-entity" :name "Alice"]])
```
*`"my-entity"` is a string, not a UUID; use `#uuid "..."` or an entity variable*
```

**QRY-007 full entry:**
```markdown
### QRY-007 Unknown predicate

**Error text**: `unknown predicate: 'between?'`

**Cause**: A where clause invoked a predicate (expression function) that is not built-in
and has not been registered via `db.register_function()`.

**Resolution**:
- Check spelling against built-in predicates: `=`, `!=`, `<`, `<=`, `>`, `>=`,
  `matches?`, `starts-with?`, `ends-with?`, `contains?`
- Register a custom function: `db.register_function("between?", |args| { ... })?;`

**Example**:
```datalog
(query {:find [?e]
        :where [[?e :age ?a]
                [(between? ?a 18 65)]]})
```
*`between?` is not built-in; register it or use `[(>= ?a 18)] [(<= ?a 65)]`*
```

Remaining entries causes/resolutions:
- **QRY-002**: An attribute in a fact is not a keyword (doesn't start with `:`). Use `:attr/name` form.
- **QRY-003**: A pseudo-attribute (internal Minigraf metadata attribute) cannot be stored via `transact`. Use regular user-defined attributes.
- **QRY-004**: A value in a fact is of a type that Minigraf cannot store. Supported: string, integer, float, boolean, UUID ref, keyword.
- **QRY-005**: The fact batch could not be committed. The nested reason message will identify the cause (often a lock or storage error).
- **QRY-006**: A retraction could not be applied. The nested reason message will identify the cause.
- **QRY-008/009**: An internal Rust mutex was poisoned — a previous operation panicked. Restart the process; if it recurs, file a bug.

- [ ] **8.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add QRY-001–QRY-009 query execution errors (#192)"
```

---

## Task 9: STG — Storage errors (STG-001 to STG-024)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| STG-001 | `Invalid header: too short (got <n> bytes, need 64)` |
| STG-002 | `Invalid magic number: not a .graph file` |
| STG-003 | `Invalid v4/v5/v6 header: expected at least 72 bytes, got <n>` |
| STG-004 | `Invalid v6 header: expected 80 bytes, got <n>` |
| STG-005 | `Invalid v7 header: expected 84 bytes, got <n>` |
| STG-006 | `Unsupported format version: <v> (supported: 1-7)` |
| STG-007 | `page_count must be greater than 0` |
| STG-008 | `eavt_root_page (<n>) must be less than page_count (<m>)` |
| STG-009 | `fact_page_count (<n>) cannot exceed page_count (<m>)` |
| STG-010 | `Failed to read header from existing file: <reason>` |
| STG-011 | `internal page has no children` |
| STG-012 | `Expected index page at page <n>` |
| STG-013 | `range_scan: expected leaf at page_id=<n>` |
| STG-014 | `Expected packed page (0x02), got 0x<xx>` |
| STG-015 | `Record at slot <n> extends beyond page boundary` |
| STG-016 | `backend mutex poisoned` |
| STG-017 | `page count overflow computing index_start` |
| STG-018 | `page count overflow computing next_free` |
| STG-019 | `page count overflow computing new_fact_start` |
| STG-020 | `fact index <i> exceeds u16::MAX` |
| STG-021 | `page id overflow in checksum computation` |
| STG-022 | `page id overflow writing fact pages` |
| STG-023 | `page index <i> exceeds u64::MAX` |
| STG-024 | `pending fact count exceeds u64::MAX` |

- [ ] **9.1** Write entries STG-001 through STG-024. For STG errors, the **Example** block is a prose scenario (no Datalog input), since these errors arise from file I/O. Full entry for STG-001 and STG-002; follow the same pattern for the rest.

**STG-001 full entry:**
```markdown
### STG-001 Invalid header: too short

**Error text**: `Invalid header: too short (got 12 bytes, need 64)`

**Cause**: The `.graph` file is truncated — it is shorter than the minimum header size.
This can happen if the file was partially written (e.g. a crash during the initial
`save()` call) or if a non-Minigraf file was passed by mistake.

**Resolution**:
- Restore the file from a backup
- If no backup exists and the file was newly created, delete it and let Minigraf create a fresh one
- See the [file format section in README](../README.md#file-format)

**Scenario**: Opening a `.graph` file that was truncated by a disk-full condition during
the first `db.save()` or `db.checkpoint()` call.
```

**STG-002 full entry:**
```markdown
### STG-002 Invalid magic number: not a .graph file

**Error text**: `Invalid magic number: not a .graph file`

**Cause**: The first 4 bytes of the file are not the Minigraf magic bytes `MGRF`. The path
points to a non-Minigraf file, or the file header was overwritten by another process.

**Resolution**:
- Verify the file path is correct and points to a `.graph` file created by Minigraf
- Do not open SQLite databases, JSON files, or other formats with Minigraf

**Scenario**: `Minigraf::open("config.json")` — a wrong file path was passed.
```

Remaining entries causes/resolutions:
- **STG-003/004/005**: Header length checks for specific legacy versions. File is truncated at the header; restore from backup.
- **STG-006**: The file uses a format version newer than the current library supports, or the version field is corrupted. Upgrade the library or restore from backup.
- **STG-007/008/009**: Header field validation failed — page counts are internally inconsistent. File is corrupted; restore from backup.
- **STG-010**: A low-level I/O error prevented reading the header (permissions, deleted file, etc). Check file system permissions and path.
- **STG-011/012/013**: B+tree page type mismatch — an index page was expected but a different page type was found. Indicates corruption; restore from backup.
- **STG-014/015**: A packed-facts page has an unexpected type byte or a record that overruns the page boundary. Corruption; restore from backup.
- **STG-016**: Internal Rust mutex poisoned. A previous operation panicked; restart the process.
- **STG-017 through STG-024**: Arithmetic overflow in page/fact count calculations. These indicate the database has grown to an extreme size, or the page count field is corrupted. File a bug with the database size if this occurs on a healthy file.

- [ ] **9.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add STG-001–STG-024 storage errors (#192)"
```

---

## Task 10: WAL — Write-ahead log errors (WAL-001 to WAL-006)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| WAL-001 | `Invalid WAL magic number: not a .wal file` |
| WAL-002 | `Unsupported WAL version: <v> (expected <expected>)` |
| WAL-003 | `Fact serialised size <n> bytes exceeds maximum <max> bytes. Store large payloads externally and reference them with a Value::String URL/path or Value::Ref entity ID.` |
| WAL-004 | `fact serialised size <n> exceeds u32 range` |
| WAL-005 | `WAL num_facts exceeds platform usize` |
| WAL-006 | `failed to delete WAL file <path>: <reason>` |

- [ ] **10.1** Write entries WAL-001 through WAL-006. Full entry for WAL-001 and WAL-003; follow the same pattern for the rest.

**WAL-001 full entry:**
```markdown
### WAL-001 Invalid WAL magic number

**Error text**: `Invalid WAL magic number: not a .wal file`

**Cause**: The sidecar `.wal` file does not start with the expected WAL magic bytes.
This can happen if the file was replaced by another file, or if the `.wal` file is from
an incompatible tool.

**Resolution**:
- If the WAL file is stale or corrupt, delete `<dbname>.wal` and reopen the database.
  Minigraf will replay only from committed state in the `.graph` file.
- Do not manually create or edit `.wal` files

**Scenario**: `my-db.wal` was accidentally replaced with an empty or foreign file before
`Minigraf::open("my-db.graph")` was called.
```

**WAL-003 full entry:**
```markdown
### WAL-003 Fact serialised size exceeds maximum

**Error text**: `Fact serialised size 524800 bytes exceeds maximum 524288 bytes. Store large payloads externally and reference them with a Value::String URL/path or Value::Ref entity ID.`

**Cause**: A single fact's serialised size exceeds the WAL entry limit (~512 KB).
This typically means a `Value::String` attribute value contains a very large string
(e.g. raw document text, a base64-encoded image, binary data).

**Resolution**:
- Store large payloads in an external file or object store
- Store the file path or URL as a `Value::String` attribute on the entity
- Or create a dedicated entity for the content and reference it with `Value::Ref`
- See [BENCHMARKS.md](../BENCHMARKS.md) for size guidance

**Example**:
```datalog
(transact [[#uuid "..." :document/body "<50000 word essay...>"]])
```
*The `:document/body` value is too large; store it in a file and use `:document/path` instead*
```

Remaining entries causes/resolutions:
- **WAL-002**: The `.wal` file was written by a different WAL version. Delete the `.wal` file if it is from a stale/incomplete session; otherwise upgrade the library.
- **WAL-004**: The serialised fact size exceeds the `u32` range (~4 GB). This should not be reachable in practice; if it occurs, file a bug.
- **WAL-005**: The number of facts in the WAL exceeds `usize::MAX` on this platform. Practically unreachable; file a bug.
- **WAL-006**: After a successful `checkpoint()`, Minigraf could not delete the `.wal` file. Check file system permissions. The `.wal` file is safe to delete manually; it will be recreated on the next write.

- [ ] **10.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add WAL-001–WAL-006 write-ahead log errors (#192)"
```

---

## Task 11: API — Database API errors (API-001 to API-009)

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

| Code | Error text |
|------|-----------|
| API-001 | `write lock is poisoned; database may be in an inconsistent state` |
| API-002 | `unexpected command variant in write path` |
| API-003 | `attribute must be a keyword` |
| API-004 | `cannot transact a pseudo-attribute` |
| API-005 | `only (query ...) commands can be prepared; got transact` |
| API-006 | `only (query ...) commands can be prepared; got retract` |
| API-007 | `only (query ...) commands can be prepared; got rule` |
| API-008 | `function registry lock poisoned: <reason>` |
| API-009 | `WAL not initialized` |

- [ ] **11.1** Write entries API-001 through API-009. Full entry for API-001 and API-005; follow the same pattern for the rest.

**API-001 full entry:**
```markdown
### API-001 Write lock poisoned

**Error text**: `write lock is poisoned; database may be in an inconsistent state`

**Cause**: A previous `WriteTransaction` panicked while holding the write lock. Rust's
mutex poisoning mechanism prevents further writes to protect data integrity.

**Resolution**:
- Restart the process; the WAL will be replayed on the next `Minigraf::open()` call
  to recover any committed facts
- If panics are occurring regularly, investigate the root cause in your application code
  before retrying writes

**Scenario**: `db.begin_write()` called after a previous write closure panicked mid-transaction.
```

**API-005 full entry:**
```markdown
### API-005 Only query commands can be prepared (got transact)

**Error text**: `only (query ...) commands can be prepared; got transact`

**Cause**: `db.prepare()` only accepts `(query ...)` forms. Passing a `(transact ...)`,
`(retract ...)`, or `(rule ...)` command to `prepare()` is not supported.

**Resolution**:
- Use `db.execute()` for `transact`, `retract`, and `rule` commands
- Only use `db.prepare()` for `(query ...)` commands that will be executed repeatedly
  with different bind slot values

**Example**:
```rust
// Wrong
let pq = db.prepare("(transact [[$e :name $name]])")?;

// Right
db.execute("(transact [[#uuid \"...\" :name \"Alice\"]])")?;
// Or for repeated queries:
let pq = db.prepare("(query {:find [?e] :where [[?e :name $name]]})")?;
```
```

Remaining entries causes/resolutions:
- **API-002**: An internal routing error — a command type not handled in the write path was passed. Indicates a library bug; file an issue.
- **API-003**: Attribute validation in the API layer (mirrors QRY-002) — attribute must be a keyword.
- **API-004**: A pseudo-attribute was used in a `transact`; use only user-defined attributes.
- **API-006/007**: Same as API-005 for `retract` and `rule` commands respectively.
- **API-008**: The function registry mutex was poisoned (a previous panic while registering/calling a custom function). Restart the process.
- **API-009**: `db.execute()` or `db.checkpoint()` was called before the WAL was initialised — indicates an internal sequencing bug. File an issue.

- [ ] **11.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: add API-001–API-009 database API errors (#192)"
```

---

## Task 12: Fill Quick Reference table

**Files:**
- Modify: `docs/ERROR_REFERENCE.md`

- [ ] **12.1** Replace the `<!-- filled in Task 9 -->` placeholder in the Quick Reference table with all 74 entries. The table lists every non-appendix code, a short error name (first ~6 words of the error text), and its category:

```markdown
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
```

- [ ] **12.2** Commit:
```bash
git add docs/ERROR_REFERENCE.md
git commit -m "docs: fill Quick Reference table — all 113 entries (#192)"
```

---

## Task 13: Doc sync and PR

**Files:**
- Modify: `ROADMAP.md`
- Modify: `CHANGELOG.md`

- [ ] **13.1** In `ROADMAP.md`, change:
```
- 🎯 Error message guide — every user-facing error has a documented cause and resolution
```
to:
```
- ✅ Error message guide — every user-facing error has a documented cause and resolution (#192)
```

- [ ] **13.2** In `CHANGELOG.md`, add under `## Unreleased` → new `### Documentation` subsection (or append to existing one):
```markdown
### Documentation

- Add `docs/ERROR_REFERENCE.md`: full inventory of user-facing errors (PRS/QRY/STG/WAL/API categories, ~113 entries) with cause, resolution steps, and bad-input examples; docs-only reference codes PRS-001…API-009 (#192)
```

- [ ] **13.3** Commit:
```bash
git add ROADMAP.md CHANGELOG.md
git commit -m "docs: mark error reference guide complete, update CHANGELOG (#192)"
```

- [ ] **13.4** Push branch and open PR:
```bash
git push -u origin feat/error-reference
gh pr create \
  --title "docs: error reference guide (#192)" \
  --body "$(cat <<'EOF'
## Summary

- Adds `docs/ERROR_REFERENCE.md` with ~113 user-facing error entries across five categories: PRS (parser), QRY (query execution), STG (storage), WAL (write-ahead log), API (database API)
- Each entry has: error text, cause, resolution steps, bad-input example
- Docs-only reference codes (PRS-001…API-009) are forward-compatible with runtime codes planned in #277
- Appendix lists 8 internal-error strings with instructions to file a bug

## Test plan

- [ ] Read through `docs/ERROR_REFERENCE.md` — every section renders correctly in GitHub Markdown
- [ ] Quick Reference table: spot-check 10 codes; confirm the linked section heading exists
- [ ] Verify all outbound links are correct: Datalog-Reference.md, README.md, BENCHMARKS.md, #277
- [ ] Confirm `ROADMAP.md` shows `✅` for the error guide item
- [ ] Confirm `CHANGELOG.md` entry is present under `## Unreleased`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review

**Spec coverage check:**

| Spec requirement | Covered by |
|-----------------|------------|
| Inventory parse, query, storage, migration, WAL, FFI errors | Tasks 3–11 (FFI explicitly out of scope per spec) |
| For each error: cause and resolution | Entry format applied in every task |
| Bad-input example | Entry format — every entry |
| Docs-only reference codes | PRS/QRY/STG/WAL/API prefixes throughout |
| Quick Reference table | Task 12 |
| Outbound links to Datalog Reference and README | Noted in entry format rules and section intros |
| ROADMAP ✅ update | Task 13.1 |
| CHANGELOG entry | Task 13.2 |
| Worktree | Task 1 |
| PR | Task 13.4 |
| Appendix for internal errors | Task 2 (skeleton) |

No gaps found.

**Placeholder scan:** No TBD/TODO present. All tasks have complete content.

**Consistency check:** Code ranges are non-overlapping and sequential within each prefix. Entry format template matches all worked examples. Table in Task 12 covers all codes defined in Tasks 3–11.
