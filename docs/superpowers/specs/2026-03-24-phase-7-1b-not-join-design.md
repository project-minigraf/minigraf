# Phase 7.1b — `not-join` Design

## Goal

Add `(not-join [?join-vars…] clauses…)` to the Datalog query language and rule bodies — existentially-quantified negation that explicitly declares which variables are shared with the outer query context. Unstratifiable programs (negative cycles through `not-join`) are rejected at rule registration time.

## Scope

- `not-join` only — `not` was implemented in Phase 7.1a
- `not-join` bodies may contain base fact patterns and rule invocations
- Only the explicitly listed `join_vars` are substituted from the outer binding; all other body variables are existentially quantified (fresh/unbound, matched against the fact store)
- Safety constraint: every variable listed in `join_vars` must be bound by an outer clause — enforced at parse time
- Variables appearing only inside the `not-join` body (not in `join_vars`) are unconstrained — no error
- Nesting constraint: `(not-join ...)` cannot appear inside `(not ...)` or another `(not-join ...)` — rejected at parse time

## Syntax

```datalog
;; not-join: only join_vars must be pre-bound; body vars are existentially quantified.
(query [:find ?e
        :where [?e :name ?n]
               (not-join [?e]                 ;; ?e shared from outer
                         [?e :has-tag ?tag]   ;; ?tag is local/fresh
                         [?tag :is-bad true])])
;; Semantics: reject ?e for which ∃?tag s.t. (?e :has-tag ?tag) ∧ (?tag :is-bad true)

;; Multiple join variables
(query [:find ?e
        :where [?e :name ?n]
               [?e :role ?r]
               (not-join [?e ?r]
                         [?e :has-role ?r]
                         [?r :is-admin true])])

;; not-join in a rule body
(rule [(eligible ?x)
       [?x :applied true]
       (not-join [?x]
                 [?x :dep ?d]
                 [?d :status :rejected])])
;; Semantics: ?x is eligible if it applied and has no dependency with status :rejected

;; Contrast with not — not requires ALL body vars to be pre-bound:
(query [:find ?e
        :where [?e :name ?n]
               (not [?e :banned true])])   ;; ?e must be bound — no fresh vars allowed
               ;; (not [?e :tag ?tag]) would ERROR — ?tag unbound
```

## Architecture

Five files are modified; no new files are created. `not-join` builds entirely on the infrastructure established in Phase 7.1a (`WhereClause::Not`, `StratifiedEvaluator`, `substitute_pattern`, `DependencyGraph`).

```
src/query/datalog/types.rs           — add WhereClause::NotJoin { join_vars, clauses }; update helpers
src/query/datalog/stratification.rs  — traverse NotJoin bodies (same as Not) in DependencyGraph::from_rules
src/query/datalog/parser.rs          — parse (not-join [?v…] clauses…); safety check for join vars
src/query/datalog/evaluator.rs       — StratifiedEvaluator handles NotJoin; add evaluate_not_join helper
src/query/datalog/executor.rs        — both not-post-filter sites handle NotJoin in query bodies
```

### Data flow

```
register_rule  →  DependencyGraph (includes NotJoin negative edges)  →  stratify()  →  [Err if negative cycle]
execute_query  →  filter_facts_for_query (temporal)
               →  StratifiedEvaluator
                    stratum 0: RecursiveEvaluator (positive rules only)
                    stratum 1+: positive part + not/not-join filter (evaluate_not_join)
               →  PatternMatcher (final query patterns)
               →  QueryResults
```

---

## Component Design

### 1. Type Changes (`types.rs`)

**Add `NotJoin` variant to `WhereClause`:**

```rust
pub enum WhereClause {
    Pattern(Pattern),
    RuleInvocation { predicate: String, args: Vec<EdnValue> },
    Not(Vec<WhereClause>),
    /// not-join: explicit join variables + existentially quantified body.
    /// Succeeds (outer binding survives) when no assignment to non-join variables
    /// satisfies all inner clauses when join variables are substituted from the outer binding.
    NotJoin {
        join_vars: Vec<String>,
        clauses: Vec<WhereClause>,
    },
}
```

**Update `WhereClause::rule_invocations()`** — recurse into `NotJoin` bodies identically to `Not`:

```rust
WhereClause::NotJoin { clauses, .. } => {
    clauses.iter().flat_map(|c| c.rule_invocations()).collect()
}
```

**Update `WhereClause::has_negated_invocation()`** — treat `NotJoin` the same as `Not`:

```rust
WhereClause::Not(clauses) | WhereClause::NotJoin { clauses, .. } => {
    clauses.iter().any(|c| matches!(c, WhereClause::RuleInvocation { .. }))
}
```

**Update `collect_rule_invocations_recursive()` in `DatalogQuery`** — recurse into `NotJoin` bodies:

```rust
WhereClause::Not(inner) | WhereClause::NotJoin { clauses: inner, .. } => {
    result.extend(Self::collect_rule_invocations_recursive(inner));
}
```

**`get_top_level_rule_invocations()`** — already correct (only matches `RuleInvocation`; `NotJoin` bodies are implicitly skipped). No change needed, but verify it compiles after `NotJoin` is added.

---

### 2. Stratification (`stratification.rs`)

`not-join` creates the same negative dependency edges as `not` — wherever a rule uses `(not-join [?x] (blocked ?x) ...)`, there is a negative edge from the rule's head predicate to `blocked`.

**Update `DependencyGraph::from_rules`** — extend the `Not` arm to also handle `NotJoin`:

```rust
WhereClause::Not(inner) | WhereClause::NotJoin { clauses: inner, .. } => {
    for inner_clause in inner {
        if let WhereClause::RuleInvocation { predicate: dep, .. } = inner_clause {
            graph.add_negative_edge(head_pred, dep);
        }
    }
}
```

No other changes to stratification logic — cycle detection and stratum assignment are unchanged.

---

### 3. Parser Changes (`parser.rs`)

**Syntax:**

```
(not-join [?v1 ?v2 ...] clause1 clause2 ...)
```

- First argument: a vector `[?v …]` of logic variables (the join variables)
- Remaining arguments: one or more patterns or rule invocations

**Parse path:** inside `parse_list_as_where_clause`, add a new branch after `"not"`:

```
EdnValue::Symbol(s) if s == "not-join" →
  - if allow_not is false → error: "(not-join ...) cannot appear inside another (not ...) or (not-join ...)"
  - if list.len() < 3 → error: "(not-join) requires a join-vars vector and at least one clause"
  - list[1] must be a vector of logic variables → join_vars: Vec<String>
  - list[2..] parsed recursively with allow_not=false → inner clauses
  → WhereClause::NotJoin { join_vars, clauses }
```

**Safety validation — `check_not_join_safety`:**

After the whole `:where` clause list (or rule body) is parsed, validate that every variable listed in `join_vars` is bound by at least one non-`NotJoin`/non-`Not` clause in the same scope. Variables that appear only inside the body but are not in `join_vars` are existentially quantified — no error.

For rule bodies, the rule head arguments are also considered outer-bound (same convention as `check_not_safety`).

```
error format: "join variable ?x in (not-join ...) is not bound by any outer clause"
```

**`outer_vars_from_clause`** — `NotJoin` body variables are local and must not be harvested as outer-bound. Add arm:

```rust
WhereClause::NotJoin { .. } => vec![],
```

**Error cases:**

```
(not-join)                 → parse error: requires a join-vars vector and at least one clause
(not-join [?e])            → parse error: requires at least one clause
(not-join ?e [...])        → parse error: first argument must be a vector of join variables
(not-join [?unbound] [...])→ safety error: join variable ?unbound not bound by any outer clause
(not (not-join [...] ...)) → parse error: cannot appear inside (not ...)
(not-join [...] (not-join [...] ...)) → parse error: cannot nest
```

---

### 4. Evaluator — `rule_invocation_to_pattern` refactor + `evaluate_not_join` helper (`evaluator.rs`)

**Prerequisite refactor — extract `rule_invocation_to_pattern` as a free function:**

`RecursiveEvaluator::rule_invocation_to_pattern` is currently a private method. Extract it as a `pub(super)` free function so that `evaluate_not_join` (which lives in the same module) can call it. The logic is unchanged:

- 1-arg invocation `(blocked ?x)` → `Pattern [?x :blocked ?_rule_value]`
- 2-arg invocation `(reachable ?from ?to)` → `Pattern [?from :reachable ?to]`

Derived facts are stored as regular EAV facts in `accumulated` with the predicate name as the attribute (`:predicate`), so this translation is sufficient to query them via `PatternMatcher`.

**New public helper function:**

```rust
/// Test whether a `not-join` body is satisfiable given a current binding.
///
/// Returns `true` if the body IS satisfiable → outer binding should be **rejected**.
/// Returns `false` if the body cannot be satisfied → outer binding survives.
///
/// Algorithm:
/// 1. Build a partial binding containing only the `join_vars` entries.
/// 2. For each clause in `clauses`:
///    - Pattern → substitute join_vars via substitute_pattern
///    - RuleInvocation → convert to Pattern via rule_invocation_to_pattern,
///      then substitute join_vars. Rule-derived facts are already present in
///      `storage` (accumulated) by the time this is called (lower strata ran first).
/// 3. Run PatternMatcher::match_patterns on all substituted patterns against `storage`.
/// 4. Any complete match → body is satisfiable → return true (reject outer binding).
pub fn evaluate_not_join(
    join_vars: &[String],
    clauses: &[WhereClause],
    binding: &Bindings,
    storage: &FactStorage,
) -> bool;
```

Implementation: build a partial `Bindings` containing only the `join_vars` entries. For each clause, convert to a `Pattern` (via `substitute_pattern` for `Pattern` clauses; via `rule_invocation_to_pattern` + `substitute_pattern` for `RuleInvocation` clauses). Run `PatternMatcher::match_patterns` on all resulting patterns. If any matches exist → return `true`.

**Update `StratifiedEvaluator::evaluate`** — mixed-rules processing:

1. Rules with `NotJoin` in the body are classified as `mixed_rules` (same as `Not`):
   ```rust
   let has_not = rule.body.iter().any(|c| {
       matches!(c, WhereClause::Not(_) | WhereClause::NotJoin { .. })
   });
   ```

2. Positive-patterns extraction skips `NotJoin` (same as `Not`):
   ```rust
   WhereClause::Not(_) | WhereClause::NotJoin { .. } => None,
   ```

3. Per-binding not-join filter (applied after the existing `not` filter):
   ```rust
   let not_join_clauses: Vec<(Vec<String>, Vec<WhereClause>)> = rule
       .body.iter()
       .filter_map(|c| match c {
           WhereClause::NotJoin { join_vars, clauses } => Some((join_vars.clone(), clauses.clone())),
           _ => None,
       })
       .collect();
   // ... in the binding loop:
   for (join_vars, nj_clauses) in &not_join_clauses {
       if evaluate_not_join(join_vars, nj_clauses, &binding, &accumulated) {
           continue 'binding;
       }
   }
   ```

---

### 5. Executor Changes (`executor.rs`)

There are two not-post-filter sites. Both need `NotJoin` handling via `evaluate_not_join`.

**`execute_query` (pure pattern queries, no rule invocations)** — after `PatternMatcher` produces candidate bindings, apply `not` post-filters. Extend with `NotJoin` handling:

```rust
// existing not filter
for not_clauses in &not_clause_groups { ... }
// add: not-join filter
for (join_vars, nj_clauses) in &not_join_groups {
    if evaluate_not_join(join_vars, nj_clauses, &binding, &filtered_storage) {
        rejected = true;
        break;
    }
}
```

**`execute_query_with_rules` (queries with rule invocations)** — same pattern, applied after `StratifiedEvaluator` runs and before final `PatternMatcher` pass.

In both sites, collect `not_join_groups` from `query.where_clauses` with:

```rust
let not_join_groups: Vec<(Vec<String>, Vec<WhereClause>)> = query
    .where_clauses.iter()
    .filter_map(|c| match c {
        WhereClause::NotJoin { join_vars, clauses } => Some((join_vars.clone(), clauses.clone())),
        _ => None,
    })
    .collect();
```

---

## Error Reference

| Situation | Error type | Message |
|---|---|---|
| Unbound join var in `not-join` | Parse error | `join variable ?x in (not-join ...) is not bound by any outer clause` |
| No join-vars vector | Parse error | `(not-join) first argument must be a vector of join variables` |
| No clauses after join-vars | Parse error | `(not-join) requires a join-vars vector and at least one clause` |
| Invalid item in body | Parse error | `expected pattern or rule invocation inside (not-join), got ...` |
| Nested inside `not` or `not-join` | Parse error | `(not-join ...) cannot appear inside another (not ...) or (not-join ...)` |
| Negative cycle via `not-join` | Runtime error | `unstratifiable: predicate 'p' is involved in a negative cycle through 'q'` |

---

## Testing Plan

### Unit tests

**`types.rs`:**
- `WhereClause::NotJoin` variant exists and matches
- `rule_invocations()` recurses into `NotJoin` body
- `has_negated_invocation()` returns true when `NotJoin` body contains `RuleInvocation`
- `collect_rule_invocations_recursive` recurses into `NotJoin`
- `get_top_level_rule_invocations` excludes `NotJoin` body invocations

**`stratification.rs`:**
- `not-join` containing `RuleInvocation` creates negative dependency edge → head in higher stratum
- Negative cycle via `not-join` (`p not-join→ q`, `q not-join→ p`) → `Err`

**`parser.rs`:**
- `(not-join [?e] [?e :banned true])` → `WhereClause::NotJoin { join_vars: ["?e"], clauses: [Pattern] }`
- Multiple join vars parsed correctly
- Inner-only variable (not in `join_vars`) allowed without error
- Unbound join var → parse error naming the variable
- Non-vector first arg → parse error
- No clauses → parse error
- `(not (not-join [...] ...))` → parse error
- `not-join` in rule body → parsed correctly
- Unbound join var in rule body → parse error

**`evaluator.rs`:**
- `rule_invocation_to_pattern` extracted as free function — 1-arg and 2-arg cases
- `evaluate_not_join` rejects binding when inner patterns match (existential satisfied)
- `evaluate_not_join` keeps binding when inner patterns do not match
- `evaluate_not_join` with `RuleInvocation` in body — derived facts in `accumulated` are queried correctly
- `StratifiedEvaluator` classifies rules with `NotJoin` as mixed
- `StratifiedEvaluator` with `NotJoin` rule correctly excludes entities with matching dependencies

**`executor.rs`:**
- `execute_query` with `not-join` in `:where` excludes entities with matching existential
- `execute_query_with_rules` with `not-join` works alongside rule invocations

### Integration tests (`tests/not_join_test.rs` — 10 tests)

1. Simple `not-join` — exclude entities where an existentially quantified dependency exists
2. Multiple join variables in `not-join`
3. `not-join` with multi-hop inner body (two inner patterns linked by a local var)
4. `not-join` where no entities match the body — all outer bindings survive
5. `not-join` combined with `:as-of` time travel
6. `not-join` combined with `:valid-at`
7. Negative cycle via `not-join` at rule registration → error, rule not registered
8. `not-join` in a rule body — derived predicate respects existential negation
9. `not-join` and `not` coexist in the same query
10. `not-join` body contains a `RuleInvocation` — derived rule facts in `accumulated` are correctly negated (end-to-end through `Minigraf::execute`)

---

## Non-goals for this sub-phase

- Aggregation — Phase 7.2
- Disjunction (`or` / `or-join`) — Phase 7.3
- Optimizer awareness of `not-join` clauses — Phase 7.4
