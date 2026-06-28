# Magic Sets `fb` Adornment Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix two v1.2.0 regressions (#297, #298) in magic sets rewriting where literal keyword arguments at rule call sites produce wrong bindings (empty results or "Unbound variable" errors).

**Architecture:** All changes are in `src/query/datalog/magic_sets.rs`. A new `sentinel_entity` helper produces a deterministic UUID from the magic attribute name. For `fb` adornments (value-position bound), `build_seed_facts` uses the sentinel as the seed entity (keeping the keyword value in value position), and `inject_magic_guard` / `build_propagation_rules` emit a 2-arg guard `[sentinel, bound_var]` so pattern matching binds the variable from value position — preserving its `Keyword` type through the rule body.

**Tech Stack:** Rust, `uuid` crate (`Uuid::new_v5`, `Uuid::NAMESPACE_OID`), existing `magic_sets.rs` helpers (`magic_pred_name`, `has_bound_arg`, `edn_to_value`, `edn_to_entity_id`).

## Global Constraints

- All changes in `src/query/datalog/magic_sets.rs` only — no edits to `evaluator.rs`, `matcher.rs`, `executor.rs`, or `parser.rs`.
- `bf` adornment code path must remain functionally identical after the change.
- All 974 existing tests must continue to pass.
- New tests must NOT use `{:?}` debug format of `Result`, `Fact`, `Value`, `EdnValue`, or any type containing `Uuid` in `assert!`/`assert_eq!` message strings (CodeQL rule `rust/cleartext-logging`). Use plain string messages or `unwrap()`/`expect()` instead.
- Run `cargo test` (not `cargo test --test` alone) to catch both unit and integration failures.

---

## File Map

| File | Action | What changes |
|---|---|---|
| `tests/magic_sets_test.rs` | Modify | Add 2 failing integration tests (one per bug) |
| `src/query/datalog/magic_sets.rs` | Modify | Add `sentinel_entity` helper; fix `build_seed_facts`, `inject_magic_guard`, `build_propagation_rules` for `fb` case; add 3 unit tests |

---

### Task 1: Write failing integration tests for #298 and #297

**Files:**
- Modify: `tests/magic_sets_test.rs`

**Interfaces:**
- Consumes: existing `open_db()`, `exec()`, `extract_refs()` helpers already in `tests/magic_sets_test.rs`
- Produces: two `#[test]` functions that compile and fail at assertion time

- [ ] **Step 1: Write the two failing tests**

Open `tests/magic_sets_test.rs` and append the following at the bottom of the file (before the closing `}`-less end — this file has no module wrapper, just top-level items):

```rust
/// #298 — value-position keyword binding: (reports-to ?emp :alice) must return
/// employees whose manager is :alice. Previously returned empty due to broken
/// fb seed encoding.
#[test]
fn test_value_position_keyword_binding() {
    let db = open_db();
    let bob = uuid::Uuid::new_v4();
    let carol = uuid::Uuid::new_v4();

    exec(
        &db,
        &format!(
            r#"(transact [[#uuid "{}" :employee/manager :alice]
                           [#uuid "{}" :employee/manager :alice]])"#,
            bob, carol
        ),
    );
    exec(
        &db,
        r#"(rule [(reports-to ?emp ?mgr) [?emp :employee/manager ?mgr]])"#,
    );

    let result = exec(&db, r#"(query [:find ?emp :where (reports-to ?emp :alice)])"#);
    let targets = extract_refs(result);

    assert_eq!(targets.len(), 2, "expected 2 employees reporting to alice");
    assert!(targets.contains(&bob), "bob should report to alice");
    assert!(targets.contains(&carol), "carol should report to alice");
}

/// #297 — recursive rule with intermediate variable and entity-position keyword:
/// (reach :a ?y) must return the values reachable from :a via :edge.
/// Previously failed with "Unbound variable in rule head: ?mid".
#[test]
fn test_recursive_intermediate_variable_with_keyword_entity() {
    let db = open_db();

    exec(&db, r#"(transact [[:a :edge :b] [:b :edge :c]])"#);
    exec(&db, r#"(rule [(reach ?x ?y) [?x :edge ?y]])"#);
    exec(
        &db,
        r#"(rule [(reach ?x ?y) [?x :edge ?mid] (reach ?mid ?y)])"#,
    );

    let result = exec(&db, r#"(query [:find ?y :where (reach :a ?y)])"#);

    match result {
        minigraf::QueryResult::QueryResults { results, .. } => {
            let values: Vec<minigraf::Value> =
                results.into_iter().map(|row| row[0].clone()).collect();
            assert_eq!(values.len(), 2, "expected 2 results");
            assert!(
                values.contains(&minigraf::Value::Keyword(":b".to_string())),
                "expected :b in results"
            );
            assert!(
                values.contains(&minigraf::Value::Keyword(":c".to_string())),
                "expected :c in results"
            );
        }
        _ => panic!("expected QueryResults"),
    }
}
```

- [ ] **Step 2: Run the tests and confirm both fail**

```bash
cargo test --test magic_sets_test test_value_position_keyword_binding -- --nocapture
cargo test --test magic_sets_test test_recursive_intermediate_variable_with_keyword_entity -- --nocapture
```

Expected: both tests FAIL. `test_value_position_keyword_binding` should fail with an assertion about `targets.len()` being 0. `test_recursive_intermediate_variable_with_keyword_entity` should fail with either an "Unbound variable" error or a wrong-count assertion.

If either test PASSES at this step, stop and report — the bug may already be fixed or the test is wrong.

- [ ] **Step 3: Commit the failing tests**

```bash
git add tests/magic_sets_test.rs
git commit -m "test(magic-sets): add failing tests for #297 and #298"
```

---

### Task 2: Add `sentinel_entity` helper and fix `fb` encoding

**Files:**
- Modify: `src/query/datalog/magic_sets.rs`

**Interfaces:**
- Consumes: `Uuid::new_v5`, `Uuid::NAMESPACE_OID` (already imported via `uuid::Uuid` at top of file); `EntityId` (alias for `Uuid`); `EdnValue::Uuid`
- Produces:
  - `fn sentinel_entity(magic_attr: &str) -> EntityId` — private helper, deterministic UUID
  - Modified `build_seed_facts` — `fb` branch uses sentinel instead of `Uuid::new_v4()`
  - Modified `inject_magic_guard` — `fb` branch emits 2-arg guard `[Uuid(sentinel), bound_var]`
  - Modified `build_propagation_rules` — `fb` branch uses 2-arg guard in body and 2-element head

- [ ] **Step 1: Add `sentinel_entity` helper**

In `src/query/datalog/magic_sets.rs`, add the following function immediately before `build_seed_facts` (around line 73):

```rust
/// Return a deterministic sentinel entity UUID for an `fb`-adorned magic predicate.
///
/// Using a stable UUID (v5, OID namespace, keyed on the magic attribute name)
/// means the seed fact entity and the guard literal always match, without
/// requiring the bound variable to be coerced through `edn_to_entity_id`.
fn sentinel_entity(magic_attr: &str) -> EntityId {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, magic_attr.as_bytes())
}
```

- [ ] **Step 2: Fix `build_seed_facts` for the `fb` branch**

Find the `fb` branch in `build_seed_facts` (currently around line 100–107):

```rust
        } else if adornment.get(1) == Some(&'b') {
            // arg1-only bound (fb) — ephemeral carrier UUID
            if let Some(arg1) = args.get(1)
                && let Ok(value) = edn_to_value(arg1)
            {
                seeds.push((Uuid::new_v4(), magic_attr, value));
            }
        }
```

Replace with:

```rust
        } else if adornment.get(1) == Some(&'b') {
            // arg1-only bound (fb) — deterministic sentinel entity so the guard
            // can match via entity position while the bound value stays in value
            // position, preserving its original type (e.g. Keyword).
            if let Some(arg1) = args.get(1)
                && let Ok(value) = edn_to_value(arg1)
            {
                seeds.push((sentinel_entity(&magic_attr), magic_attr, value));
            }
        }
```

- [ ] **Step 3: Fix `inject_magic_guard` for the `fb` branch**

Find the guard construction in `inject_magic_guard` (currently around lines 134–145):

```rust
    let magic_name = magic_pred_name(predicate, adornment);
    let bound_head_args: Vec<EdnValue> = adornment
        .iter()
        .enumerate()
        .filter(|&(_, &ch)| ch == 'b')
        // rule.head[0] is the predicate name; args start at index 1
        .filter_map(|(i, _)| rule.head.get(i + 1).cloned())
        .collect();
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name,
        args: bound_head_args,
    };
```

Replace with:

```rust
    let magic_name = magic_pred_name(predicate, adornment);
    // For fb adornments (entity position free, value position bound): emit a
    // 2-arg guard [sentinel, bound_var] so the pattern matches value position,
    // keeping the bound variable's original type (e.g. Keyword) intact.
    // For bf/bb adornments: 1-arg guard with entity-position bound vars (unchanged).
    let guard = if adornment.first() == Some(&'f') && has_bound_arg(adornment) {
        let magic_attr = format!(":{}", magic_name);
        let sentinel = EdnValue::Uuid(sentinel_entity(&magic_attr));
        let bound_var = adornment
            .iter()
            .enumerate()
            .filter(|&(_, &ch)| ch == 'b')
            // rule.head[0] is the predicate name; args start at index 1
            .filter_map(|(i, _)| rule.head.get(i + 1).cloned())
            .next();
        WhereClause::RuleInvocation {
            predicate: magic_name,
            args: bound_var
                .map(|v| vec![sentinel, v])
                .unwrap_or_default(),
        }
    } else {
        let bound_head_args: Vec<EdnValue> = adornment
            .iter()
            .enumerate()
            .filter(|&(_, &ch)| ch == 'b')
            // rule.head[0] is the predicate name; args start at index 1
            .filter_map(|(i, _)| rule.head.get(i + 1).cloned())
            .collect();
        WhereClause::RuleInvocation {
            predicate: magic_name,
            args: bound_head_args,
        }
    };
```

- [ ] **Step 4: Fix `build_propagation_rules` for the `fb` branch**

In `build_propagation_rules`, find the guard construction (currently around lines 196–200):

```rust
    // Guard clause reused in every propagation rule body.
    let guard = WhereClause::RuleInvocation {
        predicate: magic_name,
        args: bound_head_vars,
    };
```

Replace with:

```rust
    // Guard clause reused in every propagation rule body.
    // fb adornment: 2-arg sentinel guard (mirrors inject_magic_guard fix).
    // bf/bb: 1-arg entity-position guard (unchanged).
    let guard = if adornment.first() == Some(&'f') && has_bound_arg(adornment) {
        let magic_attr_str = format!(":{}", magic_name);
        let sentinel = EdnValue::Uuid(sentinel_entity(&magic_attr_str));
        let bound_var = adornment
            .iter()
            .enumerate()
            .filter(|&(_, &ch)| ch == 'b')
            .filter_map(|(i, _)| rule.head.get(i + 1).cloned())
            .next();
        WhereClause::RuleInvocation {
            predicate: magic_name,
            args: bound_var
                .map(|v| vec![sentinel, v])
                .unwrap_or_default(),
        }
    } else {
        WhereClause::RuleInvocation {
            predicate: magic_name,
            args: bound_head_vars,
        }
    };
```

Then find the propagation rule head construction further down in the same function (currently around lines 247–257):

```rust
        // Fix 1: Head must include ALL bound args, not just the first.
        let mut head = Vec::with_capacity(1 + new_magic_args.len());
        head.push(EdnValue::Symbol(called_magic_name.clone()));
        head.extend(new_magic_args);
        result.push((
            called_magic_name,
            Rule {
                head,
                body: prop_body,
            },
        ));
```

Replace with:

```rust
        // Build the propagation rule head.
        // For fb-adorned called predicates: head = [pred, sentinel, bound_val_var]
        // so the derived magic fact lands in value position.
        // For bf/bb: head = [pred, bound_entity_vars...] (unchanged).
        let head = if called_adornment.first() == Some(&'f') && has_bound_arg(called_adornment) {
            let called_magic_attr = format!(":{}", called_magic_name);
            let sentinel = EdnValue::Uuid(sentinel_entity(&called_magic_attr));
            let mut h = Vec::with_capacity(3);
            h.push(EdnValue::Symbol(called_magic_name.clone()));
            h.push(sentinel);
            h.extend(new_magic_args);
            h
        } else {
            let mut h = Vec::with_capacity(1 + new_magic_args.len());
            h.push(EdnValue::Symbol(called_magic_name.clone()));
            h.extend(new_magic_args);
            h
        };
        result.push((
            called_magic_name,
            Rule {
                head,
                body: prop_body,
            },
        ));
```

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -20
```

Expected: both new integration tests pass. All existing 974 tests pass. If `test_recursive_intermediate_variable_with_keyword_entity` still fails, read the exact error message — it will tell you which assertion failed and with what values. Report the error before proceeding.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "fix(magic-sets): fix fb adornment seed encoding and guard injection (#297, #298)"
```

---

### Task 3: Add unit tests for the new sentinel/guard behavior

**Files:**
- Modify: `src/query/datalog/magic_sets.rs` (the `#[cfg(test)]` block at the bottom)

**Interfaces:**
- Consumes: `sentinel_entity`, `build_seed_facts`, `inject_magic_guard`, `make_rule`, `pat`, `rule_inv` (all already available in the test module)
- Produces: 3 new `#[test]` functions in the existing `mod tests` block

- [ ] **Step 1: Write the three unit tests**

Inside the existing `#[cfg(test)] mod tests { ... }` block in `src/query/datalog/magic_sets.rs`, append after the last existing test function:

```rust
    #[test]
    fn test_fb_seed_uses_sentinel_entity() {
        // Same keyword arg must produce the same entity UUID on repeated calls
        // (deterministic sentinel, not random UUID).
        let clauses = vec![WhereClause::RuleInvocation {
            predicate: "reports-to".to_string(),
            args: vec![
                EdnValue::Symbol("?emp".to_string()),
                EdnValue::Keyword(":alice".to_string()),
            ],
        }];
        let adornments: HashMap<String, Vec<char>> =
            [("reports-to".to_string(), vec!['f', 'b'])].into_iter().collect();

        let seeds1 = build_seed_facts(&clauses, &adornments);
        let seeds2 = build_seed_facts(&clauses, &adornments);

        assert_eq!(seeds1.len(), 1, "expected 1 seed");
        assert_eq!(seeds2.len(), 1, "expected 1 seed");
        assert_eq!(
            seeds1[0].0, seeds2[0].0,
            "sentinel entity must be deterministic"
        );
    }

    #[test]
    fn test_fb_guard_is_two_arg() {
        // inject_magic_guard for ['f','b'] must emit a 2-arg RuleInvocation:
        // [Uuid(sentinel), Symbol("?mgr")]
        // so the pattern matches value position rather than entity position.
        let rule = make_rule(
            "reports-to",
            &["?emp", "?mgr"],
            vec![pat("?emp", ":employee/manager", "?mgr")],
        );
        let adornment = vec!['f', 'b'];
        let rewritten = inject_magic_guard(&rule, "reports-to", &adornment);
        let guard = rewritten.body.first().expect("guard must be first body clause");
        match guard {
            WhereClause::RuleInvocation { predicate, args } => {
                assert_eq!(predicate, "__magic_reports-to_fb");
                assert_eq!(args.len(), 2, "fb guard must be 2-arg");
                assert!(
                    matches!(args[0], EdnValue::Uuid(_)),
                    "first arg must be sentinel UUID"
                );
                assert_eq!(args[1], EdnValue::Symbol("?mgr".to_string()));
            }
            _ => panic!("expected RuleInvocation guard"),
        }
    }

    #[test]
    fn test_fb_sentinel_matches_seed() {
        // The sentinel UUID in the guard must equal the entity in the seed fact,
        // so that pattern matching can find the seed.
        let clauses = vec![WhereClause::RuleInvocation {
            predicate: "reports-to".to_string(),
            args: vec![
                EdnValue::Symbol("?emp".to_string()),
                EdnValue::Keyword(":alice".to_string()),
            ],
        }];
        let adornments: HashMap<String, Vec<char>> =
            [("reports-to".to_string(), vec!['f', 'b'])].into_iter().collect();

        let seeds = build_seed_facts(&clauses, &adornments);
        assert_eq!(seeds.len(), 1, "expected 1 seed");
        let seed_entity = seeds[0].0;

        let rule = make_rule(
            "reports-to",
            &["?emp", "?mgr"],
            vec![pat("?emp", ":employee/manager", "?mgr")],
        );
        let adornment = vec!['f', 'b'];
        let rewritten = inject_magic_guard(&rule, "reports-to", &adornment);
        let guard = rewritten.body.first().expect("guard must be first body clause");
        match guard {
            WhereClause::RuleInvocation { predicate: _, args } => {
                match &args[0] {
                    EdnValue::Uuid(u) => {
                        assert_eq!(*u, seed_entity, "sentinel in guard must match seed entity");
                    }
                    _ => panic!("expected UUID as first guard arg"),
                }
            }
            _ => panic!("expected RuleInvocation guard"),
        }
    }
```

- [ ] **Step 2: Run all tests**

```bash
cargo test 2>&1 | tail -20
```

Expected: all tests pass, including the 3 new unit tests. Total should be ≥ 977 (974 existing + 2 integration + 3 unit, minus any previously counted).

- [ ] **Step 3: Commit**

```bash
git add src/query/datalog/magic_sets.rs
git commit -m "test(magic-sets): add unit tests for fb sentinel entity and guard format"
```

---

## Self-Review

**Spec coverage:**
- ✅ `sentinel_entity` helper — Task 2 Step 1
- ✅ `build_seed_facts` fb fix — Task 2 Step 2
- ✅ `inject_magic_guard` fb fix — Task 2 Step 3
- ✅ `build_propagation_rules` fb fix — Task 2 Step 4
- ✅ Integration test #298 — Task 1
- ✅ Integration test #297 — Task 1
- ✅ Unit test `test_fb_seed_uses_sentinel_entity` — Task 3
- ✅ Unit test `test_fb_guard_is_two_arg` — Task 3
- ✅ Unit test `test_fb_sentinel_matches_seed` — Task 3
- ✅ `bf` path unchanged — guard construction uses `else` branch with original code
- ✅ Edge case: `edn_to_value` failure — `fb` branch still gated on `let Ok(value) = edn_to_value(arg1)`

**Placeholder scan:** No TBDs, no "similar to above", no vague steps. Every code step shows exact replacement text.

**Type consistency:**
- `sentinel_entity` returns `EntityId` (= `Uuid`) — used as `EdnValue::Uuid(sentinel_entity(...))` in Task 2 Steps 3 and 4, and compared directly to `seeds[0].0` (also `EntityId`) in Task 3 Step 1.
- `magic_name` is a `String` — moved into `WhereClause::RuleInvocation { predicate: magic_name, ... }` in exactly one branch in both Task 2 Steps 3 and 4; `called_magic_name.clone()` used correctly where needed after the move.
- `new_magic_args: Vec<EdnValue>` — consumed by `.extend()` in the head construction; not used again after the head is built.
