# Recursive rules on the streaming engine — Design

**Issue:** #548 (part 6 of 9 of #473)
**Milestone:** v3.0.0, branch `v3` (no format change; breaking changes to rule-query semantics, listed in §4)
**Builds on:** the streaming engine design (`2026-10-11-streaming-engine-design.md` §6.2, §7, §9),
plan-independent answers (`2026-10-15-plan-independent-answers-design.md`), the #546 and #547 designs
**Depends on:** #553 (one total `max_derived_facts` limit, PR #663) and #661 (pseudo-attributes in rule
bodies read the stored fact, PR #665), both before PR 1 here

## 1. Scope

Every query that invokes a rule runs on the executor today (`execute_query_with_rules`): it copies the
filtered facts into a `FactStorage`, runs `StratifiedEvaluator`, and matches the query body against base
and derived facts together. This work moves rule queries to the engine:

- a `Recursive` evaluation that derives each stratum's relations by semi-naive iteration over id rows
  (§5), replacing the `FactStorage` copy and `seen_facts`;
- `Scan` reading derived rows as a third source after the committed and pending ranges (§5.2);
- rule queries planned like any other engine query, with rule invocations as patterns (§5.4);
- three executor changes so that both engines give one answer (§4), made first (PR 1).

Magic-set rewriting (`magic_sets.rs`) and stratification (`stratification.rs`) stay as rewrites in front
of the planner, unchanged, on both engines.

Settled with the maintainer on 2026-10-20:

1. **Rule relations stay merged with stored facts.** The relation of predicate `p` is the visible stored
   facts with attribute `:p` together with the facts the rules derive; in a rule query every pattern sees
   derived facts as it sees stored ones (§3.1). Both engines keep today's behaviour.
2. **A pseudo-attribute in a rule body reads the stored fact** (#661, fixed on `v3` and `main` before this
   work).
3. **A rule head whose entity argument is not an entity is an error**, with a new user-facing code,
   `QRY-013`, on both paths (§3.4). Today a positive rule raises `INT-022` and a rule with `not`/`or`
   drops the row.
4. **One limit on derived facts** (#553 first): `max_derived_facts` counts every fact a query's rules
   derive; the 1,000-iteration limit is unchanged (§3.5).
5. **A derived fact has no metadata**: a per-fact pseudo-attribute whose source matched a derived fact
   binds `nil` (§3.3), on both engines.
6. **One fixpoint per stratum.** Positive rules and rules with `not`/`not-join`/`or`/`or-join` iterate
   together (§5.3), which subsumes the #633 repeat loop on the engine.
7. **Naive fallback.** A rule that reads a predicate of its own stratum other than through a top-level
   rule invocation is evaluated in full every round (§5.3).
8. **`or` branches in rule bodies get the database's function registry**, not the built-ins only (§4).
9. Delivery: this spec, a plan, then three PRs (§8).

## 2. Terms

- **Predicate**: a rule name. A rule query evaluates the predicates its `:where` invokes at any depth
  (inside `not`, `not-join`, `or`, `or-join` too) and every predicate their rules invoke, transitively
  (`StratifiedEvaluator::evaluate`'s `all_preds`). A predicate reached only through a pattern such as
  `[?x :p ?y]` is not evaluated.
- **Invocation**: `(p a)` or `(p a b)` as a clause. It reads `p`'s relation as the pattern `[a :p ?_rule_value]`
  or `[a :p b]` would (`rule_invocation_to_pattern`). Three or more arguments are `INT-028`, as today.
- **Stored fact**: a fact of the snapshot that passes the query's `:as-of` and `:valid-at` filters, or a
  magic-set seed fact (`magic_sets::build_seed_facts`, query-local).
- **Derived fact**: an `(entity, :p, value)` that a rule of `p` produced and that is not already a stored
  fact or an earlier derived fact (§3.2). A one-argument head derives the value `true`.
- **Relation of `p`**: the stored facts with attribute `:p` and the derived facts of `p`.
- **Stratum**: the number `stratify()` gives a predicate. Strata are evaluated in ascending order.
- **Round**: one evaluation of every rule of a stratum. **Delta**: the facts first derived in the previous
  round.
- **Same-stratum read**: a clause of a rule body that can read a relation of the rule's own stratum.

## 3. What a rule query returns (both engines)

### 3.1 Visibility

In a query that invokes a rule, at any depth, every pattern — in the query body, in rule bodies, in
`not`/`not-join` bodies and in `or`/`or-join` branches — matches derived facts as it matches stored facts
with the same attribute, once those facts exist. Concretely, as today:

- `(reach ?x ?y)` returns the stored `[:x :reach :y]` too;
- `[?x ?a ?v]` returns the attribute of each derived fact of `?x` (including a magic-set predicate's,
  `:__magic_<p>_<adornment>`, when the rewrite runs) as well as stored ones;
- a rule body that reads `[?x :q ?y]`, with `q` a predicate of a lower stratum, sees `q`'s derived facts;
  with `q` in a higher stratum it sees none (that stratum has not run).

When the magic-set rewrite runs (a top-level invocation with a literal argument), predicates derive only
the facts their callers demand, so the second and third cases see fewer facts than in the same query
without a literal. This is kept as it is on both engines, and `CHANGELOG.md` and the docs site's rule
section say so; changing it is out of scope (the rewrite stays as is).

### 3.2 Identity of a derived fact

A derived fact is new when no stored fact and no earlier derived fact has the same entity, attribute and
value, values compared as `encode_value` encodes them: kinds distinct (a keyword is not a ref to the
entity it names), integers and floats distinct, `-0.0` distinct from `0.0`, every NaN one value, strings
by content. This is `seen_facts`'s key today. Relations are sets: a fact derived twice is one fact.

Rows of the query body are a bag, as on the rest of the engine: a stored fact visible in several
valid-time windows (`:any-valid-time`) is matched once per window, as today.

### 3.3 Metadata

A stored fact's `:db/valid-from`, `:db/valid-to`, `:db/tx-count` and `:db/tx-id` are its own (#661).
A derived fact is not a stored fact and has none: a per-fact pseudo-attribute whose source matched a
derived fact binds `nil`, the row is kept, and a constant (`[?x :db/tx-count 1]`) never matches it.
`:db/valid-at` binds the query's valid time for a derived fact as for a stored one (#617), and a
`:db/valid-at` pattern, which joins with every visible fact of its entity, joins with derived facts too.

Today a derived fact carries the metadata of the evaluator's `transact` call (the time the query ran,
forever, a working counter); §4 changes the executor to `nil`.

### 3.4 Heads

A head's entity argument must be an entity: a ref, or a keyword, which names the entity
`uuidv5(keyword)` (`edn_to_entity_id`; a keyword whose name parses as a UUID names that UUID). Any other
value fails the query with **`QRY-013` rule head entity is not an entity**, naming the predicate. Its
value argument may be any value. A head variable that the body does not bind (the parser accepts
`(rule [(p ?x ?y) [?x :e 1]])`) fails the query with `INT-022` on both paths; the mixed pass drops the row
today. Rejecting it when the rule is defined is a follow-up, not part of this work.

### 3.5 Limits

- **`INT-020`, iterations**: a stratum that is still deriving facts after `max_iterations` (1,000) rounds
  fails. The round that derives nothing counts, as `RecursiveEvaluator`'s iteration does today, so a
  positive recursion of depth `d` needs `d + 1` rounds on both engines. In a stratum with `not`/`or`
  rules the two engines count differently (one fixpoint against today's positive fixpoints and mixed
  passes), so the depth at which `INT-020` is raised can differ near the limit; below it the answers are
  the same.
- **`INT-020`, derived facts** (#553): more than `max_derived_facts` derived facts in total, across strata
  and rounds, fails. Stored facts and magic seeds do not count. Both engines count the same facts.

## 4. Executor changes (PR 1)

So that the oracle (spec §9 of the engine design) compares one answer, the executor changes first:

1. **`QRY-013`** for a head whose entity is not an entity, in `RecursiveEvaluator::instantiate_head`
   (positive rules) and in `StratifiedEvaluator`'s mixed pass, which today skips the row
   (`if let Ok(fact) = temp_eval.instantiate_head_public(…)`); the mixed pass also raises `INT-022` for an
   unbound head variable instead of skipping the row (§3.4). Breaking: such a query used to fail with
   `INT-022` or drop the row.
2. **Derived facts have no metadata.** Derived facts are marked (tx-count 0, which no stored fact has:
   the counter starts at 1; magic seeds already have tx-count 0 from `Fact::new`) and the matcher binds `nil` for a per-fact pseudo-attribute whose source
   matched one. Breaking only in that the values were meaningless before.
3. **`or` / `or-join` branches in rule bodies get the database's `FunctionRegistry`.** `StratifiedEvaluator`
   passes `FunctionRegistry::with_builtins()` to `apply_or_clauses` today, so a registered predicate in
   such a branch is unknown. A fix.

Each change gets tests on the executor path, `CHANGELOG.md` entries and, for `QRY-013`,
`docs/ERROR_REFERENCE.md`. A new `ErrorCode` variant breaks the UniFFI bindings until they regenerate, so
the binding PRs follow (engine design §7).

## 5. Engine design (PR 2)

### 5.1 Relations

A rule query owns one **relation store**, query state like the provisional id map (engine design §5.3):

- per evaluated predicate, an append-only `Vec` of rows of two cells, entity then value;
- a hash set of the rows by identity (§3.2: `Cell` compared with floats by bits, `-0.0` ≠ `0.0`, NaNs
  equal; a keyword `Iid` is not the `Eid` it names), used to decide whether a head is new;
- per predicate, a hash index from the entity cell's join key to row positions, for entity probes.

Rows are appended at the end of each round, so the relation as of round `k` is a prefix of the `Vec`
and the delta of round `k` is a range of it: `old = ..start_k`, `delta = start_k..end_k`,
`full = ..end_k`. No row is copied for the delta.

Before a stratum's first round, the identity set of each of its predicates `p` is seeded with the stored
facts with attribute `:p` (scanned once), so that a head equal to a stored fact is not derived (§3.2).
Magic seeds are rows of their magic predicate, appended before the first round. Both are held for the
rest of the query and counted by the memory accounting of #549.

### 5.2 Scans in a rule query

`ScanSpec` gains a derived source: which predicates' rows it reads, and which range — `Full`, `Old` or
`Delta` — for a same-stratum relation. A scan reads the committed range, then the pending range, then the
rows of:

- the predicate `p` for attribute `:p`, if `p` is evaluated;
- every evaluated predicate, for a variable or `_` attribute, with the attribute cell the ident of `:p`.

A `Delta` read reads derived rows only (stored facts are never in a delta). Derived rows satisfy the
`:as-of`/`:valid-at` test by definition and fill every metadata column with `nil` (§3.3). An entity probe
(`IndexNestedLoopJoin`) reads the relation's entity index; constants and bound values filter as on the
other sources (§5.4 of the engine design: a keyword constant matches a keyword or a ref to the entity it
names, a float within `f64::EPSILON`, and so on).

### 5.3 `Recursive`

`Recursive` evaluates every evaluated predicate, stratum by stratum, before the query body's plan opens.
For each rule it builds a body plan with the existing `Builder` (patterns, expressions, `not`/`not-join`,
`or`/`or-join`, pseudo-attributes, in `optimizer::plan` order) whose last step projects the head's two
cells. Within a stratum:

- **Round 1** runs every rule once, every relation read `Full` (stored facts only, or lower strata).
- **Round `k > 1`**, for a rule whose same-stratum reads are all top-level invocations `o_1 … o_m`: one
  variant per `o_i`, reading `o_i` as `Delta`, `o_j` for `j < i` as `Old` and for `j > i` as `Full`. A rule
  with no same-stratum read runs only in round 1.
- **Naive rules** run every round with every read `Full`: a rule with a same-stratum read inside an `or`
  / `or-join` branch, or through a pattern (`[?x :p ?y]` with `p` in the stratum, or a variable or `_`
  attribute while the stratum has predicates). `not` / `not-join` bodies only read lower strata
  (`stratify()` guarantees it), so they never make a rule naive.
- Each head row is checked against the identity set; new rows are appended at the end of the round, and
  each one counts against `max_derived_facts`. A round that appends nothing ends the stratum.

The variants compute exactly the heads derivable from round `k`'s relations that use at least one delta
fact, so the fixpoint is the one today's naive iteration reaches, and the relations are the same sets.
Rows from rounds and variants are deduplicated by the identity set, never by the query body.

Head cells: the entity cell must be an `Eid`, or an `Iid` converted with `IdSpace::keyword_eid`;
anything else is `QRY-013`. A one-argument head's value is `true`.

### 5.4 The query body

After `Recursive`, the query body is planned as any engine query, each invocation replaced by its pattern
(`rule_invocation_to_pattern`) and every scan reading the derived source (§5.2). All existing operators
apply: joins, `not`, `or`, expressions, aggregates, windows. The query body runs only after every stratum
is complete, so its reads are all `Full`.

### 5.5 Routing

`DatalogExecutor::open_query` sends a rule query to the engine when the query body and the body of every
evaluated rule can be planned (the limits of `plan` today: no `:as-of` timestamp, no slot left
unresolved) and the query is not inside a write transaction (`facts_override`, #550). Otherwise it runs on the executor,
unchanged. Under `cfg(test)` / `test-oracle` the oracle runs it on both and compares (rows as bags, or
both errors).

## 6. Memory

`Recursive` holds the relations (rows, identity set, entity index), each predicate's seeded stored keys,
and one round's new rows before they are appended. Nothing is proportional to the base facts the rules
read: they are scanned per variant, as any pattern is. The acceptance test (§7) checks that peak memory
follows the derived relation. Byte counts for `QRY-012` are #549's.

## 7. Tests and measurement

- **Executor (PR 1):** `QRY-013` on both paths; `nil` metadata of a derived fact under a pseudo-attribute
  (variable and constant); a registered predicate in an `or` branch of a rule body.
- **Engine (PR 2):** `tests/recursive_engine_test.rs`, every query also checked by the oracle: linear and
  non-linear transitive closure, mutual recursion, a stratum with `not` and `or` rules recursing through
  each other (#633's cases), naive rules (same-stratum read in an `or` branch, through a pattern, through
  `[?x ?a ?v]`), merged visibility (§3.1 cases, magic predicates included), identity (§3.2: keyword vs
  ref, `-0.0`/`0.0`, NaN, long strings), metadata `nil`, `QRY-013`, both `INT-020` limits, one-argument
  heads, constants in invocations (magic-set path) and the routing rules (§5.5). Every existing suite with
  rules (`recursive_rules_test`, `negation_test`, `disjunction_test`, `magic_sets_test`, the model-based
  and property tests) now runs rule queries on the engine with the oracle.
- **Acceptance (PR 3):** a recursive rule deriving 5M facts completes with peak RSS proportional to the
  derived relation (the #473 acceptance test; a layered graph, e.g. 63 layers of 50 nodes with complete
  bipartite edges between neighbours, ≈ 5M `reach` facts in 63 rounds, under the 1,000-round limit); and
  `max_derived_facts` stops a runaway rule with `INT-020`.
- **Envelope:** a `recursive` shape (transitive closure at 100K, 1M and 10M base facts) on the bench
  machine, engine against `origin/v3`, wall time and peak RSS; the #379 point queries must not regress.
- **Binary size:** the engine and the executor coexist until #551, under the temporary 1,230 KiB gate
  (v3 is at ~1,211 KiB). If PR 2 exceeds it, the PR says by how much and why before the gate moves.

## 8. Delivery

Each PR targets `v3` and uses no closing keyword for #473; #548 is closed by hand after PR 3.

1. **PR 1 — executor rule fixes** (§4): `QRY-013`, `nil` metadata for derived facts, the function registry
   in rule-body `or` branches. Docs site: the rule-head error and the metadata rule, `@since v3.0.0`.
2. **PR 2 — engine `Recursive` and rule queries** (§5): relation store, derived scan source, semi-naive
   rounds with the naive fallback, routing, `tests/recursive_engine_test.rs`.
3. **PR 3 — acceptance and measurement** (§7): the 5M-fact acceptance test, the envelope shape and its
   results in `CHANGELOG.md`.
