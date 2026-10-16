# Negation and disjunction on the streaming engine — Design

**Issue:** #546 (part 4 of 9 of #473)
**Milestone:** v3.0.0, branch `v3` (no format change; breaking changes to query semantics)
**Builds on:** the streaming engine design (`2026-10-11-streaming-engine-design.md` §6.1,
§6.2, §9), the #545 join rules (`docs/superpowers/plans/2026-10-14-545-joins.md`,
decisions 1 and 2), plan-independent answers (`2026-10-15-plan-independent-answers-design.md`)

## 1. Scope

The engine runs queries whose `:where` holds patterns and expression clauses (#544, #545,
#594, #596). This work moves `not`, `not-join`, `or` and `or-join` to it:

- `AntiJoin` for `not` and `not-join`, hash-built or correlated (§4.2, §4.3);
- `Union` for `or` and `or-join`, followed by `Distinct` (§5);
- one comparison rule inside these clauses, on the executor too, so that both agree and
  neither depends on the path it takes (§3).

Rule invocations, aggregates, windows and `:with` stay on the executor (#547, #548). So
does any query holding a rule invocation at any depth, including inside a body or branch.

Settled with the maintainer on 2026-10-16:

1. One spec, two PRs: PR 1 `not`/`not-join`, PR 2 `or`/`or-join`.
2. Every comparison of a value from outside a body or branch with a value inside it uses
   the #545 join equality, on the executor and the engine.
3. Bodies that need the outer row run correlated on the engine; the others may build a
   hash set. Every `not` shape moves.
4. `or` keeps today's placement and deduplication.

## 2. Terms

- **Body.** The clause list of a `not` or `not-join`. The parser allows only patterns,
  expression clauses and rule invocations in it: no `not`, `not-join`, `or` or `or-join`
  (`parser.rs:1339–1412`).
- **Branch.** One clause list of an `or` or `or-join` (`and` groups several clauses into
  one). A branch may hold any clause, including `not`, `or` and another `or-join`.
- **Outer row.** A row of the clauses before the `not`/`or` in plan order.
- **Imported variable.** A variable whose value a body or branch takes from the outer
  row:
  - in a `not` body, every variable it names except `_` and `?_`-prefixed ones (the parser
    requires each to be bound outside, `check_not_safety`);
  - in a `not-join`, the join variables;
  - in an `or` branch, every variable the outer row binds;
  - in an `or-join` branch, the join variables;
  - in every case, the hidden metadata variables (`?#<n>.<attr>`, #594) of an outer
    source that a pseudo-attribute pattern inside reads.
- **Join equality.** Two values are join-equal when their `JoinKey`s are equal
  (`engine/cell.rs`, #473 spec §5.4): floats by IEEE `==` (`0.0` equals `-0.0`, NaN
  equals nothing), a keyword equals a ref to the entity it names, other values when
  equal. Two attribute-position values compare without the keyword-to-ref step, as joins
  do (`Key::normalise`).
- **Closed body.** A body whose expression clauses and pseudo-attribute patterns read
  only variables bound by stored-attribute patterns of the same body. A closed body has
  the same rows for every outer row; the others do not.

## 3. Rules

### 3.1 `not`

An outer row is kept when no assignment of the body's own variables makes every clause
of the body true, with each imported variable fixed to the outer row's value. A body
pattern's position holding an imported variable matches a fact's value when the two are
join-equal. A literal constant in the body keeps today's matching
(`PatternMatcher::match_component`; floats within `f64::EPSILON`).

The outer rows' multiplicity is kept: `not` filters, it never deduplicates.

### 3.2 `not-join`

As `not`, with only the join variables imported. A join variable no body pattern binds
is read by the body's expressions from the outer row. When the body binds none of the
join variables and is satisfiable, every outer row is removed (today's
`key_vars.is_empty()` case in `apply_not_join_clause`).

### 3.3 Expressions and errors in bodies

An expression that fails on a body row (type mismatch, division by zero) drops that body
row, so it cannot exclude the outer row (`executor.rs:1124–1127`). Unchanged from today:
`check_udf_predicates` checks only top-level clauses, so an unknown predicate in a body
is not reported; it fails on every body row, and the `not` keeps every outer row. The
engine reproduces this. Reporting it (`QRY-007`) would be a separate change.

### 3.4 `:db/valid-at` in bodies

A `:db/valid-at` pattern in a body or branch binds the query's valid time, as at the top
level. On the engine, a body or branch plan is built with the query's `as_of` and
`Valid` (`plan.rs`), as every scan and join of the query is, so its `:db/valid-at` join
reads the same valid time. Today `evaluate_not_join` matches with `PatternMatcher::from_slice`, whose valid
time is `nil`, so the answer changes with the path taken (§3.6).

### 3.5 `or` and `or-join` (PR 2)

- **`or`.** For each outer row and each branch, every extension of the outer row by the
  branch, where the branch's imported variables are fixed to the outer row's values and
  compared by join equality. The merged row keeps the outer row's cell for every
  imported variable. The rows are then deduplicated over the user-visible variables
  (§5.2).
- **`or-join`.** An outer row is kept when some branch has a row with the join variables
  fixed to the outer row's values. The kept rows are deduplicated as for `or`.
- The #250 early stop (skip later branches once earlier ones cover every outer row)
  remains an optimisation and does not change answers.

### 3.6 Answers that change

All are breaking changes for v3.0.0 and listed in `CHANGELOG.md`:

1. **Floats imported into a `not`/`not-join` body.** The executor's per-row path
   (`not_body_matches`, `evaluate_not_join`, and their rule-query variants) substitutes
   the outer value as a literal, so a float matches within `f64::EPSILON`; its
   exclusion-set path (`apply_not_clause`, `apply_not_join_clause`) compares
   `normalize_value` keys, so floats by IEEE `==` and keywords as the refs they name.
   The path depends on whether the body holds an expression, so
   on `v3` today

   ```
   (transact [[:e1 :x 0.00000000000000000001] [:g :y 0.00000000000000000002]])
   [:find ?e :where [?e :x ?f] (not-join [?f] [?g :y ?f])]                  ; :e1
   [:find ?e :where [?e :x ?f] (not-join [?f] [?g :y ?f] [(> ?f 0.0)])]     ; nothing
   ```

   Both now return `:e1`.
2. **`:db/valid-at` in a `not-join` body with an expression** read `nil`; it now reads the
   query's valid time. On `v3` today, with `:valid-at "2030-01-01"` and two entities with
   `:n`, `(not-join [?e] [?e :db/valid-at ?t])` removes both and
   `(not-join [?e] [?e :db/valid-at ?t] [(> ?t 0)])` keeps both; both now remove both.
3. **`or` (PR 2).** The unseeded path (`apply_or_clauses`) joins branch rows to outer rows
   by exact `Value` equality, so a keyword never meets a ref to the entity it names; the
   seeded path (a branch with `not`) matches them. Both now use join equality. The exact
   cases are recorded by PR 2's tests.

## 4. `not` and `not-join` on the engine (PR 1)

### 4.1 Which queries

`plan::plan` accepts `WhereClause::Not` and `WhereClause::NotJoin` whose bodies hold no
rule invocation. Queries with `or`/`or-join` stay on the executor until PR 2.

Clauses run in the order `optimizer::plan` gives the executor: each `not` at the position
it chooses, then the deferred ones (`optimizer::plan`'s second result) after every other
clause, sorted by `optimizer::clause_cost` when reordering, as `execute_query_on_executor`
runs them. Without `or`, a `not` is deferred only when an expression binds one of its
required variables.

### 4.2 Hash `AntiJoin`

For a closed body that the planner does not run correlated (§4.4):

1. The body is planned as a query of its own (`plan`), its rows streamed once, on the
   first pull of the outer side.
2. Each body row's **key** is its cells of the imported variables the body binds, as
   `JoinKey`s; a row with a cell that equals nothing (NaN) is skipped.
3. The keys form a hash set. Each outer row is kept when its key is absent. When the key
   is empty (a `not-join` whose body binds no join variable), a non-empty body removes
   every outer row.

The set is the only state that grows with the data (§6.2 of the engine design; counted
by #549).

### 4.3 Correlated `AntiJoin`

For every other body:

1. The body's plan starts from a **seed**: a one-row source holding the outer row's cells
   of the imported variables, in the slots of those variables. Its patterns join to the
   seed as later patterns join today: an `IndexNestedLoopJoin` when the entity is bound,
   otherwise a `HashJoin` built on the seed side.
2. For each outer row, the seed is set to that row, the body is opened, and the first
   body row decides: the outer row is removed when there is one. The body is then closed.
3. The state held is one outer batch and the body's operators for one row.

The same plan is reused for every outer row; only the seed changes.

### 4.4 Choosing hash or correlated

The planner runs a body correlated when either holds:

- the body is not closed; or
- the body's first pattern in `optimizer::plan` order has a constant entity or an entity
  that is an imported variable, so each outer row costs one index probe and nothing is
  held.

Otherwise it builds the hash set. Both give the same rows (§3.1, §3.2); the choice
affects time and memory only. The differential test (§6) runs every negation query with
both choices forced, under a test-only switch beside `optimizer::REORDER`.

### 4.5 Variables and kinds

- An imported variable keeps its outer slot kind (`Slots::kinds`), so its comparisons in
  the body normalise as a join with the outer pattern would (#545 decision 2).
- A body's `Binders` start from the outer variables bound before the `not` in text
  order, so a body expression that rebinds an imported variable checks it (#596).
- Hidden metadata variables of outer sources are imported variables (#594 spec §4.4).

## 5. `or` and `or-join` on the engine (PR 2)

### 5.1 `Union`

`plan::plan` accepts `Or` and `OrJoin`. Each runs after every pattern, expression and
placed `not`, and before the deferred `not`s, in text order, as `apply_or_clauses` does
today. Each branch is planned as in §4: closed branches once, keyed on the imported
variables they bind, and hash-joined with the outer rows; the others correlated from a
seed. `Union` concatenates the branches' merged rows.

### 5.2 `Distinct`

`Union` is followed by `Distinct`, a hash set of rows over every variable of the merged
row, the hidden metadata variables included. Today's paths deduplicate on the variables
not starting with `__` (`or`: `seen`, `seen_keys`) or on all of them (`seen_result`,
`or-join`: `seen_proj`); since #594 no `:where` binding starts with `__` (only
aggregate results do, after `:where`), so these are the same set. Duplicate outer rows therefore collapse to one, as
today. No other operator deduplicates; `:find` stays a bag.

### 5.3 Executor

`apply_or_clauses` compares branch rows with outer rows by join equality in both paths
(`normalize_join_value` keys, IEEE floats), so its answer no longer depends on whether a
branch holds `not` (§3.6 item 3).

## 6. Testing

- **Oracle.** Every engine query also runs on the executor and must agree (`oracle.rs`),
  so the executor fixes in §3 land with the engine work, in the same PR.
- **`tests/plan_independence_test.rs`** gains negation (PR 1) and disjunction (PR 2)
  queries over a schema with floats near zero, `-0.0`, NaN, keywords and refs to the
  same entities, and `:db/valid-at` in bodies; each runs with reordering on and off and,
  for negation, with hash and correlated forced (§4.4).
- **Answer-change tests** for each item of §3.6, asserting the new rows.
- **Routing tests** in `engine/tests.rs` (the `RUNS` counter): `not`/`not-join` queries run
  on the engine; bodies with rule invocations do not; PR 2 the same for `or`.
- **Property test** (`tests/property_test.rs`): generated queries gain `not` and `not-join`
  bodies (PR 1) and `or`/`or-join` branches (PR 2) over its existing schema.
- **Envelope** (#394): PR 1 adds `not_probe` (an entity-bound `not`, correlated) and
  `not_join_hash` (a closed `not-join` keyed on a value); PR 2 adds `or_2`. Measured at
  100K, 1M and 10M facts on the bench machine against `v3` before the PR; the existing
  workloads must not regress.

## 7. Documentation

- `CHANGELOG.md`: breaking entries for §3.6; internal entry for the engine operators.
- `CLAUDE.md`, `docs/TEST_COVERAGE.md`: engine module description, test counts.
- Docs site: `content/datalog-reference.md` (comparison inside `not`/`or`, `:db/valid-at`
  in bodies, `@since v3.0.0`), `content/architecture.md` (engine operators).

## 8. Delivery

Two PRs to `v3`, each with its CHANGELOG entries, tests and a docs-site PR:

1. **PR 1:** §3.1–3.4, §3.6 items 1–2, §4, the hash/correlated switch, PR 1 tests and
   envelope workloads.
2. **PR 2:** §3.5, §3.6 item 3, §5, PR 2 tests and the `or_2` workload.

Neither uses a closing keyword for #473. #546 closes by hand after PR 2.

## 9. Deviations

- **Decision 3** was "closed bodies use hash `AntiJoin`". §4.4 also runs a closed body
  correlated when its first pattern is entity-bound: one probe per outer row holds
  nothing, where the hash set holds every body row. The executor always builds the set,
  because its per-row matching scans facts; the engine's probes do not.
- **Engine design §6.1** describes `Union` as reproducing each of the executor's two
  paths. With §3.5 both paths give the same rows, so the engine has one `Union`, with
  hash or correlated branches chosen as in §4.4.
- **§3.4** (`:db/valid-at` in `not-join` bodies) is not in #546; it is a disagreement
  between executor paths found while writing this design, fixed here because the oracle
  must agree with the engine.
