# Plan-independent answers: pseudo-attribute sources and unifying expressions — Design

**Issues:** #594, #596
**Milestone:** v3.0.0, branch `v3` (no format change; breaking changes to query semantics)
**Builds on:** the #545 join rules (`docs/superpowers/plans/2026-10-14-545-joins.md`,
decisions 1, 2 and 4), the streaming engine design
(`2026-10-11-streaming-engine-design.md` §5.4, §6.3)

## 1. The requirement

Every plan must return the same rows the same number of times. Only the order of rows
may differ, and only when the query sets none. "Plan" covers the native build (which
reorders patterns, `not` clauses and `or` branches by selectivity), the `wasm` feature
(which keeps source order), the executor and the streaming engine, and any later
optimization. An answer that depends on the plan is a bug.

Two answers depend on it today:

- **#594.** `:db/valid-from`, `:db/valid-to`, `:db/tx-count` and `:db/tx-id` read the fact
  of the last pattern with a stored attribute that matched the same entity, in the order
  `optimizer::plan` runs the patterns (`matcher.rs`: the `__fvf_<uuid>` keys, overwritten
  by every match). When no such pattern matched the entity on that row, the
  pseudo-attribute pattern matches every visible fact of the entity instead.
- **#596.** An expression binding `[(f …) ?x]` overwrites `?x` where the planner places it
  (`executor.rs` plan loop, `apply_expr_clauses`, `evaluator.rs`
  `apply_expr_clauses_in_evaluator`, engine `Step::Bind`). If a pattern also binds `?x`,
  the answer depends on whether it runs before or after the expression.

## 2. Terms

- **Clause list.** The top-level `:where`, a rule body, one branch of an `or`/`or-join`,
  or the body of a `not`/`not-join`. Each clause list except the top-level `:where` and
  a rule body sits inside a **containing clause** (the `or`, `or-join`, `not` or
  `not-join`) in an enclosing clause list.
- **Per-fact pseudo-attribute pattern.** A pattern whose attribute is `:db/valid-from`,
  `:db/valid-to`, `:db/tx-count` or `:db/tx-id` (`PseudoAttr::is_per_fact`). `:db/valid-at`
  is not one (§3.6).
- **Source candidate.** A pattern whose attribute is stored: a keyword, or a variable
  (`[?e ?a ?v]`). Rule invocations, pseudo-attribute patterns and patterns whose entity
  term is `_` are never candidates.
- **Entity term.** The first position of a pattern. Two entity terms are **the same**
  when both are variables with the same name, both are `$slot`s with the same name, or
  both are constants (keyword or UUID) that resolve to the same entity id by
  `edn_to_entity_id`. So `:alice` and `#uuid "<the id :alice resolves to>"` are the same
  term; `?e` and `:alice` are not, even when `?e` is `:alice` on some row; `_` is never
  the same as anything, including another `_`.

## 3. Rules

### 3.1 Pseudo-attribute source (#594)

A per-fact pseudo-attribute pattern `P` reads the metadata of exactly one fact: the fact
matched, on the same row, by its **source**, a source candidate chosen once, when the
query or rule is parsed, as follows:

1. In `P`'s own clause list, the nearest source candidate before `P` whose entity term
   is the same as `P`'s.
2. If there is none and the clause list has a containing clause, the nearest source
   candidate before the containing clause, in the enclosing clause list, whose entity
   term is the same as `P`'s, subject to §3.2. Repeat outwards.
3. Stop at the top-level `:where` or at a rule body: a rule body never reads its
   caller's patterns.
4. If no source is found, the query, rule or prepared query is rejected with the new
   **PRS-081** (§5).

Text order decides. Where the planner runs `P` or its source does not change the answer.
The source runs before `P` in every plan (§4.2).

A pattern after `P` never counts, so `[?e :db/tx-count ?t] [?e :name ?n]` is PRS-081.
Two pseudo-attribute patterns may share a source. A sibling `or`/`or-join` branch is
never searched: its patterns are not before `P` in any clause list `P` can see.

### 3.2 Crossing `not-join` and `or-join`

`not` and `or` bodies see every outer variable, so step 2 crosses them for any entity
term. `not-join` and `or-join` share only their join variables. Step 2 crosses them only
when `P`'s entity term is a variable listed in the join variables. A variable of the
same name that is not listed is a fresh variable inside the body, and a constant entity
term does not cross either.

### 3.3 What `P` matches

`P` binds or checks its value term against the source fact's metadata, with the same
equality any already-bound variable uses (§3.5). It never scans facts. It adds no rows,
and it removes a row only when its value term is a constant, or a variable already bound
elsewhere, that differs from the metadata. The old fallback (one row per visible fact of
the entity when no earlier pattern matched it) is gone. A query that needs every fact of
an entity writes `[?e ?a _] [?e :db/tx-count ?t]`.

### 3.4 Unchanged

- **INT-026**: a per-fact pseudo-attribute still requires `:valid-at :any-valid-time`.
- **INT-017**: `:db/*` in entity or value position is still rejected.
- **`:db/valid-at`** (§3.6).

### 3.5 Expression bindings unify (#596)

An expression clause `[(f …) ?x]` whose output variable is bound by any other clause in
scope binds it only if no other clause does. Otherwise it **unifies**: it keeps the rows
where its result equals `?x`'s value, wherever it runs. "Any other clause" means a
pattern, a pseudo-attribute pattern, a rule invocation, an `or`/`or-join` that binds it,
or another expression. So:

- `[(+ ?v 1) ?v]` never matches (its input and output are one variable, so the result
  must equal the value it was computed from).
- Two expressions binding `?x` must agree.
- The issue's example, `[?e :age ?a] [:p1 :score ?x] [(+ ?a 1) ?x]`, returns `[40 41]`
  natively and under `wasm`.

**Equality** is the join equality of #545, the one every bound variable already uses:
`Value`'s `PartialEq` (floats by IEEE `==`; `Value`'s `Hash` treats `-0.0` as `0.0`), a
keyword equal to a ref to the entity it names (`PatternMatcher::normalize_join_value`;
engine `cell::join_eq` with normalisation), and NaN equal to nothing. An integer never
equals a float. An expression that fails (type mismatch, division by zero) still drops
the row.

**Returned value.** A variable bound by several clauses returns the value of the first of
them in text order (`Binders`, #545 decision 2). Expressions become ordinary binders in
that order: `Binders::expression` records the expression's text position instead of
marking the variable as overwritten, and the representative is kept in both directions:

- A binder earlier in the text that runs later replaces the value when it matches. This
  is the `rebind` list patterns already get from `Binders::pattern`. An expression gets
  the same treatment.
- A binder later in the text that runs later only checks.

This matters only for a keyword and a ref that are equal. Within rule bodies, patterns
keep today's representative rule until #548 (`evaluator.rs` passes no `rebind`), and
expressions there follow the same rule: they unify, and the value a rule body returns
for an equal keyword/ref pair is #548's to fix.

### 3.6 `:db/valid-at`

Unchanged. It is the query's valid-time, the same on every row, and the pattern still
joins with every visible fact of its entity, as `[e _ _]` does. That answer does not
depend on the plan.

## 4. Design

### 4.1 Resolution at parse time

A new pass, `resolve_pseudo_sources` in `src/query/datalog/parser.rs`, runs on every
parsed `query` (`:where`), every `rule` body, and every prepared query (which is parsed
the same way). It walks each clause list with a stack of enclosing lists (§3.1, §3.2)
and, for each per-fact pseudo-attribute pattern:

- finds the source, or fails with PRS-081 (for a prepared query, on the template, before
  any `$slot` is substituted: slot entity terms compare by slot name, never by the value
  bound later, so one template resolves the same for every execution);
- gives the source a **hidden metadata variable** for that pseudo-attribute, recorded on
  the source pattern (`Pattern::meta: Vec<(PseudoAttr, String)>`, one entry per distinct
  pseudo-attribute that reads it);
- rewrites `P` to read that variable (`Pattern::pseudo_source: Option<String>`, the hidden
  variable's name).

Both fields are new on the crate-internal `Pattern` (`query` is `pub(crate)`). Its
constructors (`Pattern::new`, `Pattern::pseudo`, `Pattern::from_edn`) leave them empty
until the pass fills them.

A hidden variable's name is `?#<n>.<pseudo-attribute>`, with `<n>` the source's position
in a walk of the whole query. `#` and `.` cannot appear in a symbol the tokenizer produces
(it accepts letters, digits, `?`, `_`, `-`, `/`), so a hidden name never collides with a
user variable, and it is never in `:find`. It starts with `?`, so every path that treats
`?`-names as variables handles it without change.

The pass runs before `check_not_join_safety` and the other parse-time checks, so those
checks see the hidden variables as ordinary variables. Tests and internal callers that
build a `DatalogQuery` by hand go through the same function.

After the pass, a pseudo-attribute reference is an ordinary variable reference:

- The source **binds** the hidden variable to its fact's metadata whenever it matches a
  fact, like any other variable it binds.
- `P` is an equality between its value term and the hidden variable (§3.3).
- Inside `not`/`or`, the hidden variable is an outer variable the body references. It
  flows in like any other outer variable. The paths that evaluate a body apart from the
  outer row cannot see it:
  - `apply_not_clause`/`apply_not_join_clause` match body patterns once, independently
    (the exclusion set).
  - The `or` fast path in `apply_or_clauses` evaluates each branch from an empty seed
    and joins back on shared variables.

  A body or branch whose pseudo-attribute pattern reads an outer source therefore takes
  the seeded, per-row path each of them already has: `not_body_matches` for `not`, the
  seeded `evaluate_branch` loop for `or`. The seeded loop is used today when a branch
  holds a `not`.
- Inside a `not-join`/`or-join` crossed by §3.2, the pass appends the hidden variable to
  the join variables. Only variables the user listed are visible to the rest of the
  query. The added ones change nothing else, because they are functionally dependent on
  the source row.

This replaces the per-entity `__fvf_`/`__fvt_`/`__ftc_`/`__fti_` keys and the fast-path and
fallback code in `PatternMatcher::match_pattern_with_bindings`. It also replaces the
`retain(|k, _| !k.starts_with("__f"))` filters in `apply_not_clause` and
`apply_not_join_clause`.

**Row identity.** Results are a bag: `extract_variables` keeps every binding. Only `or`
deduplicates, keeping the first binding per key, and its key skips names starting with
`__`.

- Hidden variables start with `?`, so they are part of that key. With the `__f` keys
  gone, the key covers the whole binding.
- So which duplicate survives cannot change the answer, whatever order the branches run
  in (natively they are sorted by cost).
- Two incoming rows that differ only in a source's metadata stay two rows. Today the
  `__f` keys are excluded and they collapse. This is part of breaking change 1 (§6).

### 4.2 Ordering

Every planner already places a clause after the clauses that bind its variables:

- `optimizer::plan` places an expression, a `not` and a `not-join` at the earliest point
  where their required variables are bound.
- `or`/`or-join` run after the planned clauses (`apply_or_clauses`).
- The evaluator matches rule-body patterns before their expressions.
- The engine's planner orders steps by bound slots.

`P` and every containing clause it is read through now require the hidden variable, so
each runs after its source with no new ordering rule. Two cases need a change:

- **Pseudo-attribute patterns are no longer patterns in `optimizer::plan`.** They are
  placed the way expressions are: after the patterns binding their hidden variable and,
  when the value term is a variable another pattern binds, as a check after it. Their
  selectivity score is no longer used.
- **A `not` whose body variables are all `?_` or constants** is placed at position 0
  today. With a pseudo-attribute pattern inside, its body references the hidden variable,
  so it now waits for the source.

### 4.3 Executor (`src/query/datalog/`)

- `matcher.rs`:
  - A source pattern binds its `meta` variables from the matched fact.
  - A pattern with `pseudo_source` matches no facts. It unifies its value term with the
    hidden variable's value in the row: bind if unbound, else compare with
    `normalize_join_value` equality.
  - `PseudoAttr::ValidAt` keeps the current scan.
- `executor.rs`:
  - The plan loop, `apply_expr_clauses` and `evaluate_branch` unify an expression whose
    output is already bound in the row: compare with the same equality, keep or drop the
    row.
  - `Binders::expression` takes the clause's text position.
  - `query_uses_per_fact_pseudo_attr` (INT-026) is unchanged.
- `evaluator.rs`: every expression path unifies the same way:
  - `apply_expr_clauses_in_evaluator`, used by `RecursiveEvaluator` after it matches all
    body patterns;
  - the planned loop of the stratified evaluator for rules with negation (the
    `WhereClause::Expr` arm next to `match_with_hint_seeded(..., &[])`);
  - the expression handling in `evaluate_not_join`.

  Rule-body patterns carry their own hidden variables. The rule head never includes them.
  Magic-sets rewriting copies body clauses with their `meta`/`pseudo_source` fields
  unchanged; it adds no pattern before a pseudo-attribute pattern that could become its
  source, because sources are fixed before it runs.

### 4.4 Streaming engine (`src/query/engine/`)

- `plan.rs`:
  - A source pattern's scan fills a metadata column per `meta` entry, a slot for the
    hidden variable (the #545 PR 3 metadata columns, now keyed by source rather than
    picked per row).
  - A pseudo-attribute pattern becomes a `Filter`/`Bind`-like step on that slot: bind its
    value variable if new, else compare.
  - The per-row `Step::Pseudo` choice by `join_eq` entity goes away, and so does the
    `[e _ _]` fallback for per-fact attributes.
  - So do the planner's refusals that existed only because of plan-order dependence: an
    expression rebinding an entity variable used by a pseudo-attribute pattern
    (`plan.rs` `pseudo_forms`), and pseudo-attribute patterns with no stored-attribute
    pattern before them, which are now PRS-081.
  - `:db/valid-at` keeps its `[e _ _]` join.
- `filter.rs`: `Step::Bind` on a slot already bound becomes an equality check with
  `cell::join_eq` (normalising).
- Which queries the engine runs is unchanged by this work: no `not`, `or`, `or-join` or
  rule invocations, and plain `:find` variables only. Every other shape, and every
  prepared query of those shapes, runs on the executor. There, §3–4.3 apply, and the
  differential test (§7) covers it through the same `Minigraf::query` calls.
- Engine support for `not`, `or` and rules is unchanged by this work. When #546 adds
  `not`, the hidden variables are ordinary outer variables of the body.

### 4.5 Test-only reorder switch

A thread-local `optimizer::REORDER` flag, compiled under `cfg(test)` or the `test-oracle`
feature. When off, every place the native build reorders behaves as under `wasm`:

- pattern selectivity sort;
- `not`/`not-join` cost sort (planned and deferred);
- `or`/`or-join` branch cost sort;
- the engine's build-side choice (which affects only row order).

Integration tests reach it through a `#[doc(hidden)]` function under the `test-oracle`
feature, which CI already enables (`cargo test --features test-oracle`). Test files that
use it are `#![cfg(feature = "test-oracle")]`. A query runs on the thread that calls
`query`/`execute` or pulls the cursor, so a thread-local flag reaches every planner the
query uses. Release builds compile the reordering unconditionally, as today.

## 5. Errors

**PRS-081** "Pseudo-attribute has no source pattern". Text:
`{} on {} has no pattern before it, with the same entity, that reads a stored attribute;
add one (e.g. [{} ?a _]) before it`. The arguments are the pseudo-attribute keyword and
the entity term (twice). Category `Parser`. Added to `src/error.rs` and
`docs/ERROR_REFERENCE.md`. No code is deprecated: the old fallback raised no error.

## 6. Breaking changes (CHANGELOG, v3.0.0)

1. A per-fact pseudo-attribute pattern reads the nearest pattern before it in `:where`
   text order with the same entity term (lexical scope, §3.1–3.2), not the last pattern
   the planner ran.
2. A per-fact pseudo-attribute pattern with no such pattern is PRS-081 instead of
   matching every fact of its entity. This covers `[?e :db/tx-count ?t]` before
   `[?e :name ?n]`, `_` entities, and a pseudo-attribute inside a rule body whose only
   candidate is in the caller.
3. Inside `not`/`not-join`/`or`/`or-join`, a per-fact pseudo-attribute reads the outer
   source fact (§3.2). Previously a `not` body ignored the outer fact.
4. An expression binding a variable that another clause binds keeps only the rows where
   the two agree (§3.5). Previously it replaced the value.

The docs site (`content/datalog-reference.md`, pseudo-attributes and expression clauses)
documents rules 1–4 under `<!-- @since v3.0.0 -->`, keeping the old text under
`<!-- @until v3.0.0 -->`.

## 7. Tests

### PR 1 (#596 and the differential test)

- `tests/plan_independence_test.rs`:
  - Runs a fixed matrix of queries with `REORDER` on and off. The matrix covers patterns,
    expressions, rule invocations, `or`/`or-join`, `not`/`not-join`, pseudo-attributes,
    keyword and ref joins, and floats.
  - Runs it on the executor and on the engine (through `Minigraf::query`).
  - Requires bag equality: rows sorted, multiplicities compared.
  - Includes #596's example and the expression cases of §3.5.
- The #385 model-based test and the #386 property tests: each generated query also runs
  with `REORDER` off and must give the same bag.
- Unit tests for unification in all four expression paths: plan loop,
  `apply_expr_clauses`, the evaluator, and the engine `Step::Bind`. They cover a
  keyword/ref pair, `-0.0`/`0.0`, NaN, int/float, and two expressions on one variable.
- Pseudo-attribute cases in the differential matrix are added in PR 2. Until then, #594's
  shapes are excluded from it, with a comment naming #594.
- Any other plan-dependent answer the differential test finds gets its own issue,
  milestoned v3.0.0, unless it is one of §1's.

### PR 2 (#594)

- Parser:
  - PRS-081 for each §6 rule 2 shape.
  - Resolution through `not`, `or`, `not-join` (listed and unlisted join variable), and
    `or-join`.
  - Rule bodies closed.
  - Constants by entity id (`:alice` and its UUID).
  - `$slot` by name.
  - Sibling branches never searched.
- Executor and engine:
  - #594's examples give the same answer with `REORDER` on and off, natively and under
    `--features wasm`.
  - A pseudo-attribute in a `not` body reads the outer fact.
  - Two pseudo-attributes share a source.
  - A pseudo value that is a constant, and one that is a variable bound elsewhere.
  - The pseudo-attribute cases join the differential matrix.
- The existing pseudo-attribute tests (`tests/bitemporal_test.rs`,
  `tests/join_rules_test.rs`, the matcher and executor unit tests) are updated where they
  relied on the fallback or plan order. Each change is listed in the PR.
- Envelope: `join_2_pseudo` at 1M is no slower than after #545 PR 3.

## 8. Delivery

Two PRs to `v3`, each with its CHANGELOG entries and a docs-site PR:

1. **PR 1:** `REORDER` switch, the differential test, and #596 (§3.5, §4.3–4.4
   expression parts).
2. **PR 2:** #594 (§3.1–3.4, §3.6, §4.1–4.4 pseudo-attribute parts, PRS-081).

#594 and #596 close by hand on merge (`v3` does not fire closing keywords). #546 (`not`/`not-join` on the engine) follows. Its `not` bodies treat hidden variables
as outer variables (§4.4).

## 9. Deviations from the issues

- **#594** proposed choosing between text-order sources and pseudo-attributes as a
  property of a pattern. This design keeps the surface syntax (text-order sources) and
  makes the property internal: the source carries hidden metadata variables (§4.1).
- **#594** left the scope of rule, `or` and `not` bodies open. This design uses lexical
  scope with closed rule bodies and join-variable-only crossing for `-join` forms
  (§3.1–3.2).
