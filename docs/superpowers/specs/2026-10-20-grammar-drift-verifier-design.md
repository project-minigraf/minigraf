# Parser vs documented grammar: a bounded-exhaustive verifier — Design

**Issue:** #643
**Milestone:** v3.0.0, branch `v3`; a test-only backport to `main` (§8)
**Builds on:** the grammar conformance harness (#233,
`2026-05-08-grammar-conformance-design.md`); the bounded approach of #627

## 1. Scope

Three documents describe the Datalog syntax: the real parser
(`src/query/datalog/parser.rs`), the shadow grammar (`tests/grammar/grammar.pest`), and
the EBNF on the docs site (`content/datalog-reference.md` in minigraf-docs). The #233
harness checks a hand-picked corpus, and an audit on 2026-10-20 found drift on both `main`
and `v3` that no corpus file covers (listed in #643). This work:

- re-codes the parser errors that share `INT-019` (or fall to `INT-000`) so every
  user-facing parse error has its own code (§3);
- adds a verifier that checks the parser against `grammar.pest` for every input up to a
  bound, deterministically (§4–§6);
- makes `grammar.pest` the only hand-written grammar: the EBNF is generated from it (§7).

Fixing the drift itself is out of scope. Each fix is its own PR afterwards and removes its
known-drift entry (§5.3).

Settled with the maintainer on 2026-10-20:

1. **The syntax/semantic classification of error codes is test-only** (§4.2): a list in
   the verifier, not a public `ErrorCode` method.
2. **The EBNF is a generated file in this repo** (§7): `docs/GRAMMAR.md`, rendered from
   `grammar.pest`, checked for staleness by a test, and published by the docs site as a
   snapshot page like `docs/ERROR_REFERENCE.md`.
3. **`main` gets the verifier only** (§8): no new error codes on v2.x.

## 2. Terms

- **pest accepts `s`**: `grammar.pest`'s rule parses all of `s` (the rule is wrapped in
  `SOI ~ … ~ EOI`).
- **The parser accepts `s`**: the parse entry point returns `Ok` (§4.1). It does not run
  anything, so errors raised only while executing (for example `INT-003`, "Cannot convert
  Symbol to Value") are not parser rejections.
- **Syntax code / semantic code**: an error code is *semantic* when an input that
  `grammar.pest` is meant to accept can raise it: the rules in the reference's "Semantic
  Constraints" section, value checks (UUID, timestamp, regex), resource limits (length,
  range, depth), and arity or shape checks that `grammar.pest` deliberately leaves loose
  (for example `fact = { "[" ~ edn_value+ ~ "]" }`). Every other parse error code is a
  *syntax* code.
- **Drift**: an input on which the verifier's property (§4) fails.

## 3. Error codes

### 3.1 Sites that move off `INT-019` and `INT-000`

`parser.rs` on `v3` raises `INT-019` ("Datalog parser internal error") at 28 sites. Four
of them cannot be reached from input and stay `INT-019`:

| Line (v3, 2026-10-20) | Message |
|---|---|
| 739 | `:as-of counter must be non-negative` (guarded by `n >= 0` just above) |
| 1018 | `internal error: is_map() true but as_map() returned None` |
| 1303 | `parse_expr_clause called with empty vector` |
| 1310 | `parse_expr_clause called with non-list element 0` |

The other 24 are user errors and each gets a new `PRS` code, numbered from `PRS-083` in
source order. The plan lists the numbers. The class each will have:

| Message (abridged) | Class | Why |
|---|---|---|
| Bind slot `$` must be followed by an identifier | syntax | `bind_slot` needs one character |
| `:order-by` is required in `:over` | semantic | `over_option*` allows none |
| Query argument must be a vector | syntax | |
| `:with` accepts only variables | syntax | |
| Expected variable or aggregate in `:find` | syntax | |
| Unknown key in valid-time map | semantic | reachable through a per-fact map, which `fact` leaves loose |
| Each fact must be a vector (retract) | syntax | |
| Invalid regex pattern | semantic | value check |
| Expected pattern or rule invocation inside `(not)` | syntax | |
| `(not-join)` inside `(not)`/`(not-join)` | semantic | nesting rule in Semantic Constraints |
| `(not-join)` first argument must be a vector | syntax | |
| `(not-join)` join variables must be logic variables | syntax | |
| Expected pattern or rule invocation inside `(not-join)` | syntax | |
| Rule invocation must start with a symbol | syntax | |
| Pattern must have exactly 3 elements | syntax | `pattern_clause` is exactly 3 |
| Expected clause inside or branch | syntax | |
| Rule must have a body | syntax | |
| Rule body must be a vector | syntax | |
| Rule body cannot be empty | syntax | `rule_vector` needs a head |
| Rule head must be a list | syntax | |
| Rule head cannot be empty | syntax | |
| Rule head must start with a symbol | syntax | |
| Rule body clause must be a vector or list | syntax | |
| Rule must have at least one clause in its body | semantic | `rule_vector` allows `where_clause*` |

`retract`'s fact arity error comes from `Pattern::from_edn` as a plain string and is
reported as `INT-000` (`parser.rs:1156`). It gets a new `PRS` code, class semantic (the
loose `fact` rule accepts any arity; the corpus file `retract_wrong_arity.edn` is already
in `invalid/semantic/`).

### 3.2 What does not change

- `INT-019` stays registered for the four unreachable sites; no code is removed or
  recycled.
- The other `INT` codes the parser raises (`INT-006`–`INT-018`) each have one meaning, so
  the verifier can classify them as they are. Re-coding them to `PRS` is out of scope.
- Message text stays the same; only the code changes. Each new code gets a row and a
  section in `docs/ERROR_REFERENCE.md` and a test in `tests/error_codes_prs_test.rs`.
- The UniFFI bindings' `v3` branches map core error codes; a new `ErrorCode` variant may
  need a binding update (cascade check after merge).

## 4. The property

### 4.1 Oracles

| Level | pest rule | Parser entry point |
|---|---|---|
| Lexical (§5.1) | new `edn_value_only = { SOI ~ edn_value ~ EOI }` | `parse_edn` |
| Structural (§5.2) | `command` | `parse_datalog_command` |

Both parser functions are crate-internal, so the verifier is a unit test module:
`src/query/datalog/parser/verifier.rs`, declared as a module in `parser.rs` (the
`aggregate.rs` / `aggregate/tests.rs` pattern; the cfg is in §5.4). `pest_derive` finds
`tests/grammar/grammar.pest` relative to `CARGO_MANIFEST_DIR`, as the #233 harness does. `pest` and `pest_derive` are already
dev-dependencies.

### 4.2 Statement

For every generated input `s`:

1. pest rejects `s` ⇒ the parser rejects `s`;
2. pest accepts `s` ⇒ the parser accepts `s`, or rejects it with a semantic code.

The semantic codes are a `const` list in the verifier. Each entry carries a **witness**:
an input that pest accepts and the parser rejects with exactly that code. A test checks
every witness, so a code cannot sit in the list without proof that the grammar accepts
something that raises it. A syntax code wrongly marked semantic would hide drift, and the
witness rule is what guards against that.

The class of a code that is reached from both a pest-accepted and a pest-rejected input
(for example the valid-time map key check, §3.1) is semantic.

## 5. Generated inputs

All generation is deterministic: fixed alphabets, fixed templates, fixed bounds, and
inputs produced shortest first then in alphabet order, so the first failure reported is a
smallest one. No randomness.

### 5.1 Lexical level

Every string of length 1 to 3 over:

- printable ASCII (`0x20`–`0x7E`), `\t`, `\n`, `\r`;
- non-ASCII stand-ins: `é`, `名`, `٣` (Arabic-Indic digit), U+00A0.

That is 102 characters, about 1.07M strings. This level covers comments, Unicode
letters, the bare `:`, the lone `-`, number forms and string escapes.

### 5.2 Structural level

Templates with holes, each hole filled by every bracket-balanced sequence of up to `k`
tokens from a fixed token alphabet:

- templates: `(query [ _ ])`, `(transact [ _ ])`, `(transact { _ } [ _ ])`,
  `(retract [ _ ])`, `(rule [ _ ])`, and a clause hole inside `:where`, `(not _)`,
  `(not-join [?x] _)`, `(or _)`, `(or-join [?x] _)`, `(and _)` and `:over ( _ )`, each
  inside an otherwise valid query;
- tokens (about 25): `(` `)` `[` `]` `{` `}`, the heads `query` `transact` `rule` `not`
  `not-join` `or` `and`, the keywords `:find` `:where` `:with` `:as-of` `:valid-at`
  `:any-valid-time` `:over` `:order-by` `:valid-from`, an unknown keyword `:foo`, `?x`,
  `?y`, `foo`, `1`, `1.5`, `"2024-01-01T00:00:00Z"`, `<`, `$s`, and
  `#uuid "550e8400-e29b-41d4-a716-446655440000"`.

Tokens are joined with one space. Gluing (`:find?e`) is the lexical level's job.

### 5.3 Known drift

Drift that exists when the verifier lands is listed in `KNOWN_DRIFT`. An entry is an id,
the issue that tracks it, and a predicate over the input (for example "contains `;`" or
"contains a non-ASCII character"), because one drift shows up in many generated inputs.

- Every failing input must match some entry, or the test fails and prints the input.
- Every entry must match at least one failing input, or the test fails and names the
  entry. A fix PR therefore has to remove its entry.

The initial entries are the drift in #643 that the generated inputs reach.

### 5.4 Bounds and CI

- The lexical level always runs in full.
- The structural bound `k` comes from `MINIGRAF_GRAMMAR_BOUND`, with a small default for
  the normal `cargo test` run.
- A scheduled job runs a deeper bound. Scheduled workflows run from the default branch,
  so the job picks its branch from the cron entry, as `soak.yml` does: one cron for
  `main`, one for `v3`. It is a shared CI change, so it lands on both branches as
  separate PRs.
- Targets: under a minute in a debug `cargo test` by default; under 30 minutes for the
  scheduled bound. The first implementation PR measures and sets both values.
- The module is `#[cfg(all(test, not(target_arch = "wasm32")))]`. `wasm.yml` runs
  `cargo test --target wasm32-wasip1 --lib --tests` under Wasmtime, and the parser is the
  same code on every target.

## 6. Relation to the #233 harness

`tests/grammar_conformance.rs` and its corpus stay. The corpus pins named, readable
examples, and `db.execute`/`db.prepare` cover the full path. The verifier covers
everything small. A failure the verifier finds and a fix closes can also get a corpus
file.

## 7. The EBNF

### 7.1 Renderer

A test, `tests/grammar_doc.rs`, reads `grammar.pest` with `pest_meta` (already in the
dependency tree through `pest_derive`; added as a dev-dependency) and renders EBNF:

- `a ~ b` → `a b`; `|`, `*`, `+`, `?`, grouping and string literals unchanged;
- rule names from `snake_case` to `kebab-case`; `SOI`, `EOI` and the `_{}`, `@{}`, `${}`
  modifiers dropped;
- `ASCII_DIGIT` → `digit`, `ASCII_ALPHA` → `letter`, `ASCII_ALPHANUMERIC` →
  `letter | digit`, `ANY` → `any-char`;
- `!x` (negative lookahead, which EBNF lacks) → an EBNF comment, `(* not followed by x *)`;
- `WHITESPACE` → the note on which characters separate tokens;
- grammar comments that start a section (`// ── … ──`) → section comments.

The test compares its output with `docs/GRAMMAR.md` and fails when they differ. Running it
with `UPDATE_GRAMMAR_DOC=1` rewrites the file.

### 7.2 Docs site

- minigraf-docs adds a `[[snapshot]]` page for `docs/GRAMMAR.md` (Reference section),
  `since = "v3.0.0"`.
- In `content/datalog-reference.md`, the hand-written EBNF is wrapped in
  `<!-- @until v3.0.0 -->`, with a link to the generated page under
  `<!-- @since v3.0.0 -->`.
- The v2.x EBNF stays hand-written, and that docs PR corrects the four mismatches with
  `grammar.pest` that #643 lists.

## 8. Backport to `main`

`main` gets the verifier only: no new codes and no `docs/GRAMMAR.md`. `INT-019` still
mixes syntax and semantic errors there, so on `main` the verifier counts `INT-019` (and
`INT-000`) as semantic. Clause 1 of §4.2 is checked in full. Clause 2 is weaker: a syntax
error coded `INT-019` that pest accepts is not caught. The v2.x `KNOWN_DRIFT` list starts
as the same list as v3.

## 9. Pull requests

1. This spec (`v3`).
2. Error codes, §3 (`v3`).
3. The verifier, §4–§6 (`v3`).
4. Renderer and `docs/GRAMMAR.md`, §7.1 (`v3`), and the minigraf-docs PR, §7.2.
5. The verifier backport, §8 (`main`).

Then the drift fixes, each removing its `KNOWN_DRIFT` entry. The 5+ element fact (#643)
silently drops data, so it goes to `v3` and is backported to `main`. The other fixes
change which input is accepted and stay on `v3`.

## 10. Deviations from #643

- #643 proposed checking "pest rejects ⇒ parser rejects" and "parser accepts ⇒ pest
  accepts" on `main` as two directions. They are the same statement. On `main` the
  verifier checks it in full and checks clause 2 in the weaker form of §8.
- #643's lexical level compared acceptance exactly. Pest accepts some inputs the parser
  rejects on value checks (an invalid UUID, `PRS-072`; an out-of-range integer,
  `INT-008`), so the lexical level uses the same two clauses as the structural one.
