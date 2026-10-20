# Parser vs documented grammar: a bounded-exhaustive verifier (#643) — Plan

Spec: `docs/superpowers/specs/2026-10-20-grammar-drift-verifier-design.md`. PRs 2–4 target
`v3`, PR 5 targets `main`. None uses a closing keyword; #643 closes by hand after PR 5,
and the drift fixes get their own issues (step 11).

| PR | Steps | Branch | Base |
|---|---|---|---|
| 2 | 1–5: error codes (spec §3) | `feat/643-error-codes` | `v3` |
| 3 | 6–13: the verifier (spec §4–§6) | `feat/643-verifier` | `v3` |
| 4 | 14–18: renderer, `docs/GRAMMAR.md`, docs-site PR (spec §7) | `feat/643-grammar-doc` | `v3` |
| 5 | 19–21: verifier backport and the scheduled job (spec §8) | `feat/643-verifier-v2` | `main` |

Each PR updates the test counts (`CLAUDE.md`, `docs/TEST_COVERAGE.md`) and
`CHANGELOG.md` for what it ships, and is reviewed with Codex before merge.

## Deviations from the spec (need approval)

1. **`grammar.pest`'s `fact` becomes exact** (PR 3). The spec classifies codes against
   the loose `fact = { "[" ~ edn_value+ ~ "]" }`. That rule accepts a 5+ element fact,
   and so does the parser, so the property holds and the verifier cannot see the bug
   that drops data (#643). It would also turn into a loose generated EBNF once PR 4
   lands. New rules:

   ```pest
   fact         = { "[" ~ edn_value ~ edn_value ~ edn_value ~ valid_time_map? ~ "]" }
   retract_fact = { "[" ~ edn_value ~ edn_value ~ edn_value ~ "]" }
   ```

   with `retract_cmd` using a `retract_fact` vector. Consequences:
   - `invalid/semantic/fact_too_few_elements.edn` and `retract_wrong_arity.edn` move to
     `invalid/syntax/`;
   - these codes become syntax: `PRS-046` (too few), `PRS-047` (4th not a map),
     `PRS-019`/`PRS-020` (non-string valid time), and from §3.1 the unknown valid-time
     key and retract arity codes (`PRS-088`, `PRS-090` below);
   - the 5+ element fact becomes a `KNOWN_DRIFT` entry.
2. **15 templates, not 12** (spec §5.2). A full clause does not fit in 3–5 tokens, so the
   templates put the hole where the clause is short (list in step 9).
3. **A no-separator variant replaces "gluing is the lexical level's job"** (spec §5.2).
   That sentence is wrong: `parse_edn` reads `:find?e` as one keyword, and so does pest,
   so the lexical level sees no difference. At bound `min(k, 3)` each filler is also
   joined with no separators, including at the template boundary.
4. **Known-drift entries carry a minimum bound** (spec §5.3). Some drift needs 5 tokens
   (for example `[ ( ?x ?y ) ]`). Its entry would fail the "matches at least one" rule
   at the default `k = 3`. An entry declares `min_k`, and that rule applies only when
   `k ≥ min_k`.
5. **Correction to spec §3.2: no binding impact.** `ErrorCode` is `pub(crate)`
   (`error.rs:49`). Bindings see only the code string, and none of the seven binding repos or the
   playground mentions `INT-019`.

## What the property cannot see

The property compares outcomes, not parse trees. `[:find?e :where [?e :a ?v]]` is
accepted by both. pest reads `:find` then `?e`; the parser reads an unknown keyword
`:find?e`, then an empty `:find`. Once the empty-`:find` drift is fixed, the parser
rejects it and the verifier reports it. The fix for pest's literal-keyword boundaries
(`":find" ~ !word_cont`, and the same for `":where"`, `":with"`, …) belongs with that fix.

## PR 2 — error codes

1. **Registry.** 25 codes, in source order of `parser.rs` on `v3` at 3f130235. The class
   column is after deviation 1.

   | Code | Site | Message template | Class |
   |---|---|---|---|
   | PRS-083 | 255 | `Bind slot '$' must be followed by an identifier (e.g. $entity)` | syntax |
   | PRS-084 | 684 | `':order-by' is required in the ':over' clause` | semantic |
   | PRS-085 | 712 | `Query argument must be a vector` | syntax |
   | PRS-086 | 853 | `':with' clause accepts only variables, got {}` | syntax |
   | PRS-087 | 879 | `Expected variable or aggregate expression in :find clause, got {}` | syntax |
   | PRS-088 | 1123 | `Unknown key in valid-time map: {}; expected :valid-from or :valid-to` | syntax |
   | PRS-089 | 1155 | `Each fact must be a vector [e a v]` (retract) | syntax |
   | PRS-090 | 1156 | `Retract fact must have exactly 3 elements (E A V), got {}` | syntax |
   | PRS-091 | 1236 | `invalid regex pattern {}: {}` | semantic |
   | PRS-092 | 1377 | `expected pattern or rule invocation inside (not), got {}` | syntax |
   | PRS-093 | 1387 | `(not-join ...) cannot appear inside another (not ...) or (not-join ...)` | semantic |
   | PRS-094 | 1399 | `(not-join) first argument must be a vector of join variables` | syntax |
   | PRS-095 | 1409 | `(not-join) join variables must be logic variables, got {}` | syntax |
   | PRS-096 | 1438 | `expected pattern or rule invocation inside (not-join), got {}` | syntax |
   | PRS-097 | 1508 | `Rule invocation must start with predicate name (symbol), got {}` | syntax |
   | PRS-098 | 1522 | `Pattern must have exactly 3 elements (E A V), got {}` | syntax |
   | PRS-099 | 1603 | `expected clause inside or branch, got {}` | syntax |
   | PRS-100 | 2086 | `Rule must have a body` | syntax |
   | PRS-101 | 2098 | `Rule body must be a vector` | syntax |
   | PRS-102 | 2101 | `Rule body cannot be empty` | syntax |
   | PRS-103 | 2109 | `Rule head must be a list: (predicate ?args)` | syntax |
   | PRS-104 | 2115 | `Rule head cannot be empty` | syntax |
   | PRS-105 | 2124 | `Rule head must start with a symbol (predicate name)` | syntax |
   | PRS-106 | 2148 | `Rule body clause must be a vector (pattern) or list (rule invocation / not), got {}` | syntax |
   | PRS-107 | 2158 | `Rule must have at least one pattern or rule invocation in body` | semantic |

   Message text is today's, without the `datalog parser: ` prefix that `INT-019`'s
   template adds (`error.rs:1409`). PRS-090 is new text: today's comes from
   `Pattern::from_edn` and says "Pattern". `parse_retract` checks the length and bails
   before calling `from_edn`.

   Each code goes into the four places `Prs082` appears in `error.rs` (enum `:131`,
   registry `:777`, category match `:2002`, all-codes list `:2230`).
2. **Call sites.** Replace each `err_coded!(ErrorCode::Int019, …)`/`bail_coded!` with
   the new code and its arguments. The four unreachable sites (739, 1018, 1303, 1310)
   keep `INT-019`.
3. **`docs/ERROR_REFERENCE.md`.** A quick-reference row and a section per code (cause,
   resolution, example), matching the existing PRS sections. In the `INT-019` section,
   say it is now raised only by unreachable invariants, and list the codes that replaced
   it from v3.0.0.
4. **Tests.** In `tests/error_codes_prs_test.rs`, one test per code through
   `assert_execute_code` (`prepare` for query codes where `execute` would need data),
   asserting the code and that the message holds its argument. Update any `parser.rs`
   unit test that matched the old `datalog parser:` text.
5. **Docs and counts.** `CHANGELOG.md` v3.0.0 "Changed": parse errors that were
   `INT-019` (and the retract arity `INT-000`) now carry `PRS-083`–`PRS-107`, so code
   matching on `INT-019` must change. Counts. No docs-site PR: `ERROR_REFERENCE.md` is
   read per tag.

## PR 3 — the verifier

6. **Grammar** (deviation 1). Add `edn_value_only = { SOI ~ edn_value ~ EOI }`, the exact
   `fact`, and `retract_fact`. Move the two corpus files. Run `grammar_conformance`.
7. **Module.** `src/query/datalog/parser/verifier.rs`, declared in `parser.rs` as
   `#[cfg(all(test, not(target_arch = "wasm32")))] mod verifier;`. It holds a
   `#[derive(pest_derive::Parser)] #[grammar = "tests/grammar/grammar.pest"] struct G;`.
   Its outcome type is `enum Outcome { Accept, Reject(&'static str) }`, taking the code
   from `MinigrafError::from(err).code()` (`error.rs:1802`).
8. **Lexical level.** The 102-character alphabet in the spec's order (ASCII `0x20`–`0x7E`,
   then `\t \n \r`, then `é 名 ٣ U+00A0`). Odometer enumeration for lengths 1–3. For each
   input: pest `edn_value_only` against `parser::parse_edn`.
9. **Structural level.** The 32 tokens in the spec's order, and these templates (`_` is
   the hole):

   | # | Template |
   |---|---|
   | T1 | `(query [ _ ])` |
   | T2 | `(query [:find _ :where [?x :a ?y]])` |
   | T3 | `(query [:find ?x :where [?x :a ?y] _ ])` |
   | T4 | `(query [:find ?x :where [?x :a ?y] (not _ )])` |
   | T5 | `(query [:find ?x :where [?x :a ?y] (not-join [?x] _ )])` |
   | T6 | `(query [:find ?x :where [?x :a ?y] (or _ )])` |
   | T7 | `(query [:find ?x :where [?x :a ?y] (or-join [?x] _ )])` |
   | T8 | `(query [:find ?x :where [?x :a ?y] (or (and _ ))])` |
   | T9 | `(query [:find ?x (sum ?y :over ( _ )) :where [?x :a ?y]])` |
   | T10 | `(transact [ _ ])` |
   | T11 | `(transact [[:a :b 1 _ ]])` |
   | T12 | `(transact { _ } [[:a :b 1]])` |
   | T13 | `(retract [ _ ])` |
   | T14 | `(rule [( _ ) [?x :a 1]])` |
   | T15 | `(rule [(foo ?x) _ ])` |

   - Balanced fillers come from a stack-tracking generator. The order is token count,
     then token index, so the output needs no sort.
   - Each filler is joined with single spaces. At `k ≤ 3` it is also joined with no
     separators, including at both template boundaries (deviation 3).
   - Oracle: pest `command` against `parser::parse_datalog_command`.
   - `k` comes from `MINIGRAF_GRAMMAR_BOUND` (default 3).
10. **Semantic codes.** `const SEMANTIC: &[Semantic]`, where
    `Semantic { code, witness, sites }` and `sites` is a comment-only reason per raising
    site.
    - To build it, run the verifier with an empty list and collect every code from a
      pest-accepted, parser-rejected input.
    - Review each one: list it if every site raising it is semantic (spec §4.2), else
      treat it as drift.
    - Expected semantic: `PRS-021`, `PRS-022`, `PRS-023`, `PRS-027`, `PRS-031`/`032`/`034`
      (window shape, checked against `over_option` and `window_expr`), `PRS-051`,
      `PRS-057`, `PRS-064`, `PRS-065`, `PRS-080`, `PRS-082`, `PRS-084`, `PRS-093`,
      `PRS-107`, `INT-016`, `INT-018`.
    - Codes that need inputs outside the alphabet (`PRS-015`, `PRS-066`, `PRS-072`, the
      length, range and depth limits, `INT-017`) are left out. Listing one requires a
      witness, and nothing generated can raise it.
    - Test: every witness is pest-accepted and parser-rejected with its code.
11. **Known drift.** `struct Drift { id, issue, level, min_k, matches: fn(&Input) ->
    bool }`, where `Input` carries the text and, at the structural level, the template
    and filler tokens. The predicates read tokens, not text. Initial entries for `v3`:

    | id | Level | min_k | Matches |
    |---|---|---|---|
    | `comment` | lexical | – | contains `;` |
    | `unicode-letter` | lexical | – | contains a non-ASCII alphanumeric |
    | `bare-colon` | lexical | – | `:` not followed by a keyword character |
    | `lone-minus` | lexical | – | `-` not followed by a word character or digit |
    | `empty-find` | structural | 1 | `:find` followed by a keyword or the vector's end |
    | `empty-with` | structural | 1 | `:with` followed by a keyword or the end |
    | `unknown-keyword` | structural | 1 | a keyword outside the query keywords at the query vector's top level, with no value after it |
    | `rule-head-args` | structural | 2 | T14 with a non-variable after the head symbol, or a variable as the head |
    | `call-head` | structural | 3 | a clause list whose first token is a variable or `<` |
    | `udf-var-head` | structural | 5 | `[ ( ?v … ) … ]` with a variable as the function |
    | `fact-arity` | structural | 3 | T11 with a filler of a map then one or more values |

    These are the ones I expect. The first run decides the real list. Each entry must
    reproduce an input from #643's audit, and overlaps are split until every failure
    matches exactly one entry. Before the PR merges, file the drift-fix issues and have
    every entry link one: comments and Unicode together as one documentation decision,
    the parser tightenings for v3.0.0, and the 5+ element fact as `data-integrity`
    (`v3`, then backport; listed in #421).
12. **Reports and timing.** On failure, print per entry the count and first input. Print
    unmatched inputs (first 20) and overlaps. Measure time per input on the
    development machine at `k = 3, 4, 5` (idle check first) and set the default and
    scheduled bound (spec §5.4). Record the numbers in the PR body.
13. **Docs and counts.** `CHANGELOG.md` v3.0.0 "Testing": the verifier. Update
    `docs/TEST_COVERAGE.md` and counts. Add a CLAUDE.md module-list line for `verifier.rs`.

## PR 4 — the EBNF

14. **Renderer.** `tests/grammar_doc.rs` (`#![cfg(not(target_arch = "wasm32"))]`), with
    a `pest_meta = "2"` dev-dependency (the same crate `pest_derive` already pulls in,
    so `cargo-deny`'s `multiple-versions` holds).
    - Parse the grammar with `pest_meta::parser::parse(Rule::grammar_rules, …)` and
      `consume_rules`, then walk `ParserExpr`.
    - Map constructs as spec §7.1 says, plus: `RepExact(e, n)` written as `n` copies;
      lookaheads as `(* not followed by x *)`; atomic rules marked `(* token *)`.
    - Skip `edn_value_only`, the verifier's entry rule.
    - Section headers come from the raw text: `// ── X ──` lines, placed before the first
      rule whose span starts after them.
15. **`docs/GRAMMAR.md`.** Contents:
    - title;
    - a note that the file is generated from `tests/grammar/grammar.pest` by
      `tests/grammar_doc.rs`, so edit the grammar and run it with
      `UPDATE_GRAMMAR_DOC=1`;
    - the whitespace note;
    - the EBNF block;
    - a sentence pointing to the Semantic Constraints section of the Datalog reference
      (plain text, so the docs link checker has nothing version-dependent to resolve).
16. **Staleness test.** Render and compare. On a mismatch, print the first differing line
    and the `UPDATE_GRAMMAR_DOC=1` command.
17. **Docs-site PR** (`../minigraf-docs`, branch `v3/643-grammar-page`):
    - a `[[snapshot]]` entry `slug = "grammar"`, Reference section, `path =
      "docs/GRAMMAR.md"`, `since = "v3.0.0"`;
    - in `content/datalog-reference.md`, the EBNF section in `<!-- @until v3.0.0 -->`,
      plus a `<!-- @since v3.0.0 -->` paragraph linking the Grammar page;
    - inside the `@until` text, fix #643's four EBNF mismatches with v2's `grammar.pest`
      (operators, escapes, `\r`, keyword boundaries).
18. **Docs and counts.** `CHANGELOG.md`, counts, `docs/TEST_COVERAGE.md`.

## PR 5 — backport and schedule (`main`)

19. **Verifier on `main`.** Port steps 6–12 to `main`'s parser (line numbers differ).
    - No new codes: `INT-019` and `INT-000` are semantic there (spec §8), each with a
      witness.
    - `KNOWN_DRIFT` is the `v3` list, re-checked on `main`. `empty-where` may appear
      there: `main` has no `PRS-080`.
    - Deviation 1 applies too: `main`'s `grammar.pest` is test-only.
20. **Scheduled job.** New `.github/workflows/grammar-verifier.yml`:
    - two crons, `main` and `v3`, picking the branch as `soak.yml` does;
    - `workflow_dispatch` with a `ref` input;
    - runs the verifier at the scheduled bound; on failure, opens or comments on an issue
      as `property-tests.yml` does.

    It lands on `main` in this PR and on `v3` as a separate small PR with the same file
    (shared CI rule).
21. **Docs and counts** for `main`: `CHANGELOG.md` unreleased v2.0.x "Testing", counts.
    No docs-site change.

## After PR 5

22. Link the drift-fix issues (step 11) from #643, close #643 by hand, and update the
    memory and issue roadmap.
