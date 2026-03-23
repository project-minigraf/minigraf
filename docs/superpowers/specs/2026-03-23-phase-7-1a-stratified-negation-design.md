# Phase 7.1 — Stratified Negation (`not`) Design

## Goal

Add `not` (negation as failure) to the Datalog query language and rule bodies, with full stratification support including negation of derived/recursive predicates. Unstratifiable programs (negative cycles) are rejected at rule registration time.

## Scope

- `not` clause only — `not-join` is deferred to a future sub-phase
- `not` bodies may contain base fact patterns and rule invocations
- Hard error at rule registration when the program is unstratifiable
- Safety constraint: all variables in a `not` body must be bound by outer clauses (enforced at parse time)

## Syntax

```datalog
;; Exclude entities with a base fact
(query [:find ?person
        :where [?person :person/name ?name]
               (not [?person :person/banned true])])

;; Exclude entities matched by a derived rule
(query [:find ?person
        :where [?person :person/name ?name]
               (not (blocked ?person))])

;; Multiple clauses inside not (conjunction)
(query [:find ?person
        :where [?person :person/name ?name]
               (not [?person :role :admin]
                    [?person :active false])])

;; not in a rule body
(rule [(eligible ?x)
       [?x :applied true]
       (not (rejected ?x))])
```

## Architecture

Six components are touched or added:

```
src/query/datalog/types.rs           — add WhereClause::Not; Rule.body Vec<EdnValue> → Vec<WhereClause>
src/query/datalog/parser.rs          — parse (not ...) in :where clauses and rule bodies
src/query/datalog/stratification.rs  — NEW: DependencyGraph, stratify()
src/query/datalog/rules.rs           — call stratify() on register_rule; reject on negative cycle
src/query/datalog/evaluator.rs       — add StratifiedEvaluator; update RecursiveEvaluator::evaluate_rule
                                       to branch on WhereClause variants (was branching on EdnValue)
src/query/datalog/executor.rs        — execute_query_with_rules uses StratifiedEvaluator;
                                       execute_query handles not-only queries as filters
```

**Note on `RecursiveEvaluator`**: `evaluate_recursive_rules` (the public entry point and fixed-point loop) is unchanged. `evaluate_rule` (the private per-rule method) must be updated: it currently inspects raw `EdnValue` items in `rule.body` via `.as_vector()` / `.as_list()`. Since `Rule.body` changes to `Vec<WhereClause>`, `evaluate_rule` is updated to branch on `WhereClause::Pattern`, `WhereClause::RuleInvocation`, and — for mixed rules passed to it by `StratifiedEvaluator` — `WhereClause::Not` is handled outside `RecursiveEvaluator` (see §5). `RecursiveEvaluator` receives only positive-only rules; `WhereClause::Not` in a body passed to `evaluate_rule` returns an error.

### Data flow

```
register_rule  →  build DependencyGraph  →  stratify()  →  [Err if negative cycle]
execute_query  →  filter_facts_for_query (temporal)
               →  StratifiedEvaluator
                    stratum 0: RecursiveEvaluator (positive rules only)
                    stratum 1: RecursiveEvaluator (positive part) + not-filter
                    ...
               →  PatternMatcher (final query patterns)
               →  QueryResults
```

---

## Component Design

### 1. Type Changes (`types.rs`)

**Add `Not` variant to `WhereClause`:**

```rust
pub enum WhereClause {
    Pattern(Pattern),
    RuleInvocation { predicate: String, args: Vec<EdnValue> },
    Not(Vec<WhereClause>),  // (not clause1 clause2 ...)
}
```

The `not` body is itself `Vec<WhereClause>`, allowing patterns and rule invocations inside `not`.

**Change `Rule.body`:**

```rust
pub struct Rule {
    pub head: Vec<EdnValue>,    // unchanged
    pub body: Vec<WhereClause>, // was Vec<EdnValue>
}
```

This removes ad-hoc EDN inspection from `RecursiveEvaluator.evaluate_rule` and makes rule bodies the same typed representation as query `where_clauses`.

**Helper methods on `WhereClause`:**

```rust
impl WhereClause {
    /// Collect all rule invocation predicate names, recursively (including inside Not bodies).
    /// Used by stratification graph construction and executor routing.
    pub fn rule_invocations(&self) -> Vec<&str>;

    /// True if this clause contains at least one RuleInvocation anywhere (including inside Not).
    pub fn has_negated_invocation(&self) -> bool;
}
```

**Update `DatalogQuery::uses_rules()` and `get_rule_invocations()`:**

`uses_rules()` must recurse into `Not` bodies so that queries like `(not (blocked ?x))` — where the rule invocation is nested inside `Not` — route to `execute_query_with_rules` (and thus `StratifiedEvaluator`) rather than the simple `execute_query` path.

`get_rule_invocations()` must likewise collect predicates from inside `Not` bodies, so that `StratifiedEvaluator` evaluates the negated predicate (`blocked`) as part of a lower stratum.

Routing rule (in `execute_query`):
- If any `WhereClause` in `where_clauses` (recursively including inside `Not`) contains a `RuleInvocation` → route to `execute_query_with_rules`
- Otherwise → handle `Not` as a pure post-filter in `execute_query` (no stratification needed)

---

### 2. Parser Changes (`parser.rs`)

When parsing a `(list ...)` in a `:where` clause or rule body, check the first token:

- Symbol `not` → parse remaining items as `Vec<WhereClause>` → `WhereClause::Not(...)`
- Any other symbol → existing rule invocation path (unchanged)

```
(not [?x :banned true])         →  WhereClause::Not([Pattern(...)])
(not (blocked ?x))              →  WhereClause::Not([RuleInvocation { "blocked", [?x] }])
(not [?x :a ?v] (blocked ?x))  →  WhereClause::Not([Pattern(...), RuleInvocation(...)])
```

**Safety validation at parse time:**

After parsing a `not` body, verify that every variable appearing in it is also mentioned in at least one non-`not` clause in the same scope. For queries, "same scope" means the `:where` clause list. For rule bodies, "same scope" includes both the non-`Not` body clauses **and** the rule head arguments — a variable bound by the head is considered bound for safety purposes:

```datalog
;; SAFE: ?x is bound by the head and by [?x :applied true]
(rule [(eligible ?x) [?x :applied true] (not (rejected ?x))])

;; UNSAFE: ?y appears only in (not ...), never in a non-not clause or head
(rule [(eligible ?x) [?x :applied true] (not [?y :banned true])])
;;                                             ^^ error: ?y is unbound
```

If the safety check fails:

```
error: variable ?y in (not ...) is not bound by any outer clause
```

**Error cases:**

```
(not)         →  parse error: (not) requires at least one clause
(not :foo)    →  parse error: expected pattern or rule invocation inside (not)
```

Rule body parsing uses the same list-parsing logic — `(not ...)` in a rule body is identical to in a query.

---

### 3. Stratification (`src/query/datalog/stratification.rs` — new file)

**Dependency graph:**

```rust
pub struct DependencyGraph {
    // head_predicate → Vec<(dependency_predicate, is_negative)>
    edges: HashMap<String, Vec<(String, bool)>>,
}

impl DependencyGraph {
    /// Build from all rules in the registry.
    pub fn from_rules(registry: &RuleRegistry) -> Self;

    /// Assign stratum numbers to all predicates.
    /// Returns Err if a negative cycle is detected.
    pub fn stratify(&self) -> Result<HashMap<String, usize>>;
}
```

**Graph construction** — for each rule `head_pred :- body`:

- `WhereClause::RuleInvocation { predicate, .. }` → positive edge: `head_pred →⁺ predicate`
- `WhereClause::Not(clauses)` → for each `RuleInvocation` inside (one level only — see note below): negative edge: `head_pred →⁻ predicate`
- `WhereClause::Pattern` → no edges (base facts carry no predicate dependency)

**Note on nested `Not`**: `WhereClause::Not(Vec<WhereClause>)` could syntactically contain another `Not`. The parser **must reject nested `not`** with a parse error (`(not ...) cannot appear inside another (not ...)`). This keeps the stratification algorithm simple (one level of negation per clause) and avoids semantically ambiguous double-negation.

**Stratification algorithm:**

Initialise all predicates at stratum 0. Propagate constraints iteratively:

- Positive edge `P →⁺ Q`: if `stratum[Q] > stratum[P]`, set `stratum[P] = stratum[Q]`
- Negative edge `P →⁻ Q`: if `stratum[Q] >= stratum[P]`, set `stratum[P] = stratum[Q] + 1`

Repeat until no stratum changes. A negative cycle is detected when any predicate's stratum reaches `>= N` (where `N` = number of distinct predicates in the graph) — because a stratifiable program with N predicates needs at most N-1 strata (0-indexed):

```rust
// Cycle detection: if stratum[P] >= n_predicates after propagation → unstratifiable
// Error message format:
"unstratifiable: predicate 'p' is involved in a negative cycle through 'q'"
```

---

### 4. Rule Registration Check (`rules.rs`)

In `RuleRegistry::register_rule`, after inserting the new rule:

1. Rebuild `DependencyGraph::from_rules(&self)`
2. Call `stratify()`
3. If `Err` → remove the just-inserted rule, return the error

The rule is never committed to the registry if it makes the program unstratifiable.

---

### 5. `StratifiedEvaluator` (`evaluator.rs`)

```rust
pub struct StratifiedEvaluator {
    storage: FactStorage,
    rules: Arc<RwLock<RuleRegistry>>,
    max_iterations: usize,
}

impl StratifiedEvaluator {
    pub fn new(storage: FactStorage, rules: Arc<RwLock<RuleRegistry>>, max_iterations: usize) -> Self;
    pub fn evaluate(&self, predicates: &[String]) -> Result<FactStorage>;
}
```

**`evaluate` algorithm:**

1. Build `DependencyGraph` from registry; call `stratify()` (defensive — should always succeed post-registration check).
2. Collect all transitive predicate dependencies of `predicates`; group by stratum.
3. Initialise `accumulated: FactStorage` = `self.storage` (base facts).
4. For each stratum in ascending order:
   a. Collect rules for predicates in this stratum.
   b. Partition rules into **positive-only** (no `WhereClause::Not` in body) and **mixed** (contain `Not`).
   c. Run `RecursiveEvaluator::new(accumulated.clone(), rules_subset, max_iterations).evaluate_recursive_rules(&stratum_predicates)` for positive-only rules. The returned `FactStorage` contains all base + newly derived facts for this stratum.
   d. For mixed rules: match all non-`Not` body clauses via `PatternMatcher` against `accumulated` to get candidate bindings; apply not-filter (see below); instantiate rule head from surviving bindings; collect derived facts.
   e. **Merge new derived facts into `accumulated`**: call `get_asserted_facts()` on the `FactStorage` returned from step (c), then for each fact not already in `accumulated`, call `accumulated.load_fact(fact)`. For mixed-rule derived facts (step d), call `accumulated.transact(...)` for each newly derived head fact. No dedicated `merge` API is needed.
5. Return `accumulated`.

**`not` filter (step 4d):**

For each candidate binding `b` and each `WhereClause::Not(clauses)` in the rule body:

- Substitute bound variables from `b` into each clause in `clauses`.
- Check if `PatternMatcher` finds **any** satisfying match in `accumulated`.
- If yes → discard `b` (the `not` condition is violated).
- If no → keep `b` (the `not` condition holds).

Repeat for each `Not` clause in the rule body; a binding survives only if all `Not` conditions hold.

---

### 6. Executor Changes (`executor.rs`)

**Queries with rule invocations** — switch to `StratifiedEvaluator`:

```rust
// execute_query_with_rules: replace
let evaluator = RecursiveEvaluator::new(filtered_storage, self.rules.clone(), 1000);
let derived_storage = evaluator.evaluate_recursive_rules(&predicates)?;

// with
let evaluator = StratifiedEvaluator::new(filtered_storage, self.rules.clone(), 1000);
let derived_storage = evaluator.evaluate(&predicates)?;
```

**Queries without rule invocations (pure `not` filter)** — `execute_query` (the non-rules path) handles `WhereClause::Not` after `PatternMatcher` runs: filter candidate bindings using the same not-filter logic, against `filtered_storage`. No stratification overhead.

---

## Error Reference

| Situation | Error type | Message |
|---|---|---|
| Unbound variable in `not` body | Parse error | `variable ?y in (not ...) is not bound by any outer clause` |
| Empty `not` body | Parse error | `(not) requires at least one clause` |
| Invalid item in `not` body | Parse error | `expected pattern or rule invocation inside (not)` |
| Negative cycle at registration | Runtime error | `unstratifiable: predicate 'p' is involved in a negative cycle through 'q'` |

---

## Testing Plan

### Unit tests

**`stratification.rs`:**
- Positive-only rules → all stratum 0
- Single negative edge → head in strictly higher stratum
- Two-stratum chain `p →⁻ q →⁺ base` → correct strata
- Negative cycle `p →⁻ q`, `q →⁻ p` → `Err`
- Self-negative cycle `p →⁻ p` → `Err`
- Disconnected predicates → each gets stratum 0

**`parser.rs`:**
- `(not [?x :banned true])` → `WhereClause::Not([Pattern])`
- `(not (blocked ?x))` → `WhereClause::Not([RuleInvocation])`
- Unbound variable in `not` → parse error
- Empty `not` → parse error

**`evaluator.rs`:**
- `StratifiedEvaluator` with no negation → same results as `RecursiveEvaluator`
- `not` filter removes binding when body is satisfied
- `not` filter keeps binding when body is not satisfied
- Multi-stratum: lower stratum fully computed before upper stratum `not` filter runs

### Integration tests (`tests/negation_test.rs`)

1. Simple `not` on base fact — exclude banned entities
2. `not` with multiple clauses in body (conjunction)
3. `not` negating a derived rule — `not (blocked ?x)` where `blocked` is a user-defined rule
4. Multi-stratum chain: `eligible` uses `not (rejected)`, `rejected` is itself derived
5. `not` combined with `:as-of` time travel
6. `not` combined with `:valid-at`
7. Negative cycle at rule registration → error, rule not registered
8. Recursive rule + `not` coexist in same query (different predicates)
9. `not` in a rule body
10. Safety violation — unbound variable in `not` → parse error

---

## Non-goals for this sub-phase

- `not-join` (explicit variable sharing) — deferred
- Aggregation — Phase 7.2
- Disjunction (`or`) — Phase 7.3
- Optimizer awareness of `not` clauses — Phase 7.4
