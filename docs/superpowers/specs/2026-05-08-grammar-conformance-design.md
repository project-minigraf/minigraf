# Grammar Specification and Conformance Harness — Design

**Date:** 2026-05-08
**Issue:** [#233](https://github.com/project-minigraf/minigraf/issues/233) — EBNF grammar specification and semantics documentation (sub-issue of [#230](https://github.com/project-minigraf/minigraf/issues/230))

## Deliverables

This work produces three things:

1. **Conformance test harness** — `pest` shadow grammar + `.edn` corpus + `grammar_conformance.rs`. Verifies continuously that the grammar doc matches parser behaviour.
2. **EBNF grammar document** — human-readable formal grammar for the full Minigraf Datalog syntax, derived from `grammar.pest` and published in the wiki (`Datalog-Reference.md`).
3. **Semantics documentation** — prose covering all constraints enforced above the structural grammar layer (not-safety, expr binding, aggregate binding, window function rules, timestamp/UUID validation). Also published in `Datalog-Reference.md`.

Tutorials are tracked separately in [#234](https://github.com/project-minigraf/minigraf/issues/234).

---

## Problem

Minigraf's Datalog query language needs a formal grammar document (EBNF). A grammar that silently diverges from the real parser is worse than no grammar — it misleads users and breaks tooling built on the spec. The conformance harness makes divergence impossible to ignore: it fails CI the moment the grammar doc and the parser disagree on any corpus fixture.

---

## Architecture

Three artefacts:

1. **`tests/grammar/grammar.pest`** — the grammar encoded as an executable `pest` PEG grammar. Serves as the machine-checkable equivalent of the EBNF documentation. The EBNF doc is derived from this file; they must say the same thing.
2. **`tests/grammar/`** corpus — `.edn` fixture files partitioned into three buckets by expected behaviour. This is the shared ground truth that both parsers are checked against.
3. **`tests/grammar_conformance.rs`** — one `#[test]` per bucket; sweeps all files, runs both parsers, reports all failures before panicking.

`pest` and `pest_derive` are added to `[dev-dependencies]` only. Zero production-code impact, no binary size change.

---

## Corpus Layout

```
tests/grammar/
  grammar.pest                           ← pest shadow grammar
  valid/                                 ← pest ACCEPTS + real parser ACCEPTS
    transact_basic.edn
    transact_valid_time_tx_level.edn
    transact_valid_time_per_fact.edn
    transact_valid_time_both.edn
    retract_basic.edn
    query_basic.edn
    query_as_of_counter.edn
    query_as_of_timestamp.edn
    query_valid_at_timestamp.edn
    query_valid_at_any_valid_time.edn
    query_any_valid_time_shorthand.edn
    query_with_clause.edn
    query_aggregate_count.edn
    query_aggregate_sum.edn
    query_aggregate_udf.edn
    query_window_sum.edn
    query_window_rank.edn
    query_window_partition_by.edn
    query_not.edn
    query_not_join.edn
    query_or.edn
    query_or_join.edn
    query_expr_filter.edn
    query_expr_binding.edn
    query_expr_nested.edn
    query_prepared_bind_slot.edn
    rule_basic.edn
    rule_recursive.edn
    edn_uuid.edn
    edn_string_escapes.edn
    edn_all_value_types.edn
  invalid/
    syntax/                              ← pest REJECTS + real parser REJECTS
      unclosed_paren.edn
      unclosed_bracket.edn
      unknown_command.edn
      empty_command.edn
      unknown_tagged_literal.edn
      string_unterminated.edn
      bind_slot_empty.edn
      keyword_invalid_chars.edn
      unexpected_bare_char.edn
    semantic/                            ← pest ACCEPTS, real parser REJECTS
      not_safety_unbound_var.edn
      not_nested_inside_not.edn
      or_inside_not.edn
      not_join_insufficient_args.edn
      expr_unbound_var_filter.edn
      aggregate_var_unbound.edn
      with_without_aggregate.edn
      with_var_unbound.edn
      window_only_func_without_over.edn
      window_incompatible_func_with_over.edn
      fact_too_few_elements.edn
      retract_wrong_arity.edn
      invalid_uuid_format.edn
```

---

## Pest Grammar Scope

### Covered (structural acceptance)

- All EDN primitives: keywords, symbols, strings (with `\n \t \r \" \\` escapes), integers (signed), floats, booleans (`true`/`false`), `nil`, `#uuid "..."`, bind slots (`$name`)
- All EDN containers: list `(...)`, vector `[...]`, map `{k v ...}`
- Top-level command forms: `(transact ...)`, `(retract ...)`, `(query ...)`, `(rule ...)`
- Query vector structure: `:find` / `:where` / `:as-of` / `:valid-at` / `:with` / `:any-valid-time` — correct keyword placement and expected value types at each position
- Fact triple shape: `[e a v]` or `[e a v {map}]`; transaction-level options map `(transact {map} [facts])`
- `where` clause shapes: pattern vector, rule invocation list, `(not ...)`, `(not-join [...] ...)`, `(or ...)`, `(or-join [...] ...)`, expr vector `[(expr) ?out?]`
- Aggregate form: `(sym ?var)`; window form: `(sym ?var :over (...))` or `(sym :over (...))`
- Expression forms: unary `(op arg)`, binary `(op arg arg)`

### Not covered (semantic layer — `invalid/semantic/` only)

These are enforced by the real parser but are above the structural grammar layer. The `pest` grammar accepts them; the real parser rejects them.

| Constraint | Corpus file |
|---|---|
| Vars in `not`/`not-join` must be bound in outer clauses | `not_safety_unbound_var.edn` |
| `(not ...)` cannot nest inside `(not ...)` | `not_nested_inside_not.edn` |
| `(or)`/`(or-join)` cannot appear inside `(not)`/`(not-join)` | `or_inside_not.edn` |
| `not-join` requires join-vars vector + ≥1 clause | `not_join_insufficient_args.edn` |
| Filter expr vars must already be bound | `expr_unbound_var_filter.edn` |
| Aggregate variable must be bound in `:where` | `aggregate_var_unbound.edn` |
| `:with` requires ≥1 aggregate in `:find` | `with_without_aggregate.edn` |
| `:with` vars must be bound in `:where` | `with_var_unbound.edn` |
| `avg`/`rank`/`row-number` require `:over` clause | `window_only_func_without_over.edn` |
| `count-distinct`/`sum-distinct` not compatible with `:over` | `window_incompatible_func_with_over.edn` |
| Fact must have ≥3 elements | `fact_too_few_elements.edn` |
| Retract fact must be exactly `[e a v]` | `retract_wrong_arity.edn` |
| UUID string must be valid RFC 4122 format | `invalid_uuid_format.edn` |

Note: ISO 8601 timestamp parsing (`:as-of`, `:valid-at`, `:valid-from`, `:valid-to`) is also semantic — `pest` accepts any string in those positions.

---

## Conformance Test Logic

```
tests/grammar_conformance.rs
```

Three `#[test]` functions. Each sweeps its directory, accumulates all failures, then panics with the full list — no early exit on first failure.

```
#[test] fn valid_corpus()
  for each .edn in tests/grammar/valid/:
    pest ACCEPTS   → ok; failure means grammar.pest is too strict
    parser ACCEPTS → ok; failure means test fixture is wrong or parser regressed

#[test] fn invalid_syntax_corpus()
  for each .edn in tests/grammar/invalid/syntax/:
    pest REJECTS   → ok; failure means grammar.pest is too permissive
    parser REJECTS → ok; failure means parser is too permissive

#[test] fn invalid_semantic_corpus()
  for each .edn in tests/grammar/invalid/semantic/:
    pest ACCEPTS   → ok; failure means this should be in syntax/ instead
    parser REJECTS → ok; failure means parser is missing a semantic check
```

Failure output per file:
```
FAIL valid/query_not.edn: parser rejected (expected accept)
FAIL invalid/semantic/not_nested_inside_not.edn: pest rejected (expected accept — semantic bucket)
```

### Wiring pest

A `Grammar` struct with `#[derive(Parser)]` and `#[grammar = "tests/grammar/grammar.pest"]` lives in `tests/grammar_conformance.rs` (or a helper module `tests/grammar_helpers.rs` if the file grows large). The conformance test calls `Grammar::parse(Rule::command, input)` — `Ok` = accepted, `Err` = rejected.

---

## Cargo Changes

```toml
[dev-dependencies]
pest = "2"
pest_derive = "2"
```

No production dependencies added.

---

## What This Does Not Cover

- **Tutorials** — tracked in [#234](https://github.com/project-minigraf/minigraf/issues/234).
- **Round-trip / pretty-print testing** — not in scope.
- **Property-based generation** — the corpus is manually curated. Fuzzing is a possible future addition.
- **Executor correctness** — the harness tests parse acceptance/rejection only, not query result correctness.
