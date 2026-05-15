# Issue #208 Selective B+Tree Lookup — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unconditional `get_all_facts()` full scan in `filter_facts_for_query` with index-backed selective fetches when queries bind concrete entity literals or attributes, keeping a guard so we never pay more overhead than a plain full scan.

**Architecture:** Promote three index-backed methods out of `#[cfg(test)]` in `graph/storage.rs`, add a `selective_fact_fetch` helper on `DatalogExecutor` that inspects main-query patterns for bound entities/attributes and unions their results (up to a threshold of 4 total lookups), then wire it into the `(None, None)` arm of `filter_facts_for_query`. The `as_of` arms and the `facts_override` arm are untouched.

**Tech Stack:** Rust stable, existing EAVT/AEVT B+tree indexes already maintained in `FactStorage`, `criterion` for benchmarks.

---

## File Map

| Action | File | What changes |
|--------|------|--------------|
| Modify | `src/graph/storage.rs:618-823` | Split `#[cfg(test)] impl FactStorage` — promote 3 methods to production, leave 5 test-only methods behind |
| Modify | `src/query/datalog/executor.rs` | Add `selective_fact_fetch` method; modify `filter_facts_for_query` |
| Modify | `benches/minigraf_bench.rs` | Add `bench_btree_lookup` function and register it in `criterion_group!` |

---

### Task 1: Promote index methods to production

**Files:**
- Modify: `src/graph/storage.rs:618-823`

The current block starting at line 618 is one `#[cfg(test)] impl FactStorage { … }` containing eight methods. We need to split it: three methods become production code, five stay test-only.

- [ ] **Step 1: Split the impl block**

In `src/graph/storage.rs`, replace the single `#[cfg(test)] impl FactStorage` block (lines 618–823) with two blocks. The first block is new production code (no `#[cfg(test)]`), the second retains the remaining test-only methods.

The new production block goes right before the `#[cfg(test)] impl FactStorage` block:

```rust
/// Index-backed selective fetches. Available in production for the query executor.
impl FactStorage {
    /// Get all facts for a specific entity using the EAVT B+tree index.
    ///
    /// Falls back to a linear scan when no index is available (e.g. before the first
    /// checkpoint on a fresh in-memory DB).
    pub(crate) fn get_facts_by_entity(&self, entity_id: &EntityId) -> Result<Vec<Fact>> {
        use crate::storage::index::EavtKey;
        let d = self.data.read().unwrap();

        let start = EavtKey {
            entity: *entity_id,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let next_entity = uuid::Uuid::from_u128(entity_id.as_u128().wrapping_add(1));
        let end = EavtKey {
            entity: next_entity,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };

        // Fallback: no indexes built yet
        if d.pending_indexes.eavt.is_empty() && d.committed_index_reader.is_none() {
            if d.committed.is_none() {
                return Ok(d
                    .facts
                    .iter()
                    .filter(|f| &f.entity == entity_id)
                    .cloned()
                    .collect());
            }
            let mut result: Vec<Fact> = d
                .facts
                .iter()
                .filter(|f| &f.entity == entity_id)
                .cloned()
                .collect();
            if let Some(loader) = &d.committed {
                for fact in loader.stream_all()? {
                    if &fact.entity == entity_id {
                        result.push(fact);
                    }
                }
            }
            return Ok(result);
        }

        let mut facts = Vec::new();

        // Pending: in-memory BTreeMap bounded range.
        for (key, &fr) in d.pending_indexes.eavt.range(start.clone()..end.clone()) {
            if key.entity != *entity_id {
                break;
            }
            facts.push(resolve_fact_ref(&d, fr)?);
        }

        // Committed: on-disk B+tree range scan
        if let Some(reader) = &d.committed_index_reader {
            let committed_refs = reader.range_scan_eavt(&start, Some(&end))?;
            for fr in committed_refs {
                facts.push(resolve_fact_ref(&d, fr)?);
            }
        }

        Ok(facts)
    }

    /// Get all facts for a specific attribute using the AEVT B+tree index.
    pub(crate) fn get_facts_by_attribute(&self, attribute: &Attribute) -> Result<Vec<Fact>> {
        use crate::storage::index::AevtKey;
        let d = self.data.read().unwrap();

        // Fallback: no index
        if d.pending_indexes.aevt.is_empty() && d.committed_index_reader.is_none() {
            drop(d);
            return Ok(self
                .get_all_facts()?
                .into_iter()
                .filter(|f| &f.attribute == attribute)
                .collect());
        }

        let start = AevtKey {
            attribute: attribute.clone(),
            entity: uuid::Uuid::nil(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        };
        let end_opt: Option<AevtKey> = next_string_prefix(attribute).map(|next_attr| AevtKey {
            attribute: next_attr,
            entity: uuid::Uuid::nil(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
        });

        let mut facts = Vec::new();

        // Pending
        let pending_range: Vec<FactRef> = match &end_opt {
            Some(end) => d
                .pending_indexes
                .aevt
                .range(start.clone()..end.clone())
                .filter(|(k, _)| k.attribute == *attribute)
                .map(|(_, &r)| r)
                .collect(),
            None => d
                .pending_indexes
                .aevt
                .range(start.clone()..)
                .take_while(|(k, _)| k.attribute == *attribute)
                .map(|(_, &r)| r)
                .collect(),
        };
        for fr in pending_range {
            facts.push(resolve_fact_ref(&d, fr)?);
        }

        // Committed
        if let Some(reader) = &d.committed_index_reader {
            let committed_refs = reader.range_scan_aevt(&start, end_opt.as_ref())?;
            for fr in committed_refs {
                let fact = resolve_fact_ref(&d, fr)?;
                if &fact.attribute == attribute {
                    facts.push(fact);
                }
            }
        }

        Ok(facts)
    }

    /// Get all facts for a specific entity and attribute.
    pub(crate) fn get_facts_by_entity_attribute(
        &self,
        entity_id: &EntityId,
        attribute: &Attribute,
    ) -> Result<Vec<Fact>> {
        let all = self.get_all_facts()?;
        Ok(all
            .into_iter()
            .filter(|f| &f.entity == entity_id && &f.attribute == attribute)
            .collect())
    }
}
```

Then update the `#[cfg(test)] impl FactStorage` block comment and remove the three promoted methods from it, leaving only:

```rust
/// Test-only helpers: for use in tests, not the production query path.
#[cfg(test)]
impl FactStorage {
    /// Return all asserted facts valid at the given timestamp. Test use only.
    pub(crate) fn get_facts_valid_at(&self, ts: i64) -> Result<Vec<Fact>> { … }

    /// Get the current value for an entity-attribute pair. Test use only.
    pub(crate) fn get_current_value(…) -> Result<Option<Value>> { … }

    /// Get the count of all facts in storage. Test use only.
    pub(crate) fn fact_count(&self) -> usize { … }

    /// Get the count of currently asserted facts. Test use only.
    pub(crate) fn asserted_fact_count(&self) -> usize { … }

    /// Returns (eavt_len, aevt_len, avet_len, vaet_len). Test use only.
    pub(crate) fn index_counts(&self) -> (usize, usize, usize, usize) { … }
}
```

(Keep the full bodies of all five methods unchanged — only move them into the new smaller `#[cfg(test)]` block.)

- [ ] **Step 2: Verify it compiles**

```bash
cargo build 2>&1 | head -20
```

Expected: no errors. If `resolve_fact_ref` or `next_string_prefix` are not in scope in the new block, check that they are defined as free functions in the same file (they are — search for `fn resolve_fact_ref` and `fn next_string_prefix` to confirm line numbers).

- [ ] **Step 3: Run existing storage tests**

```bash
cargo test --test '*' 2>&1 | grep -E "FAILED|error\[" | head -20
cargo test graph::storage 2>&1 | tail -5
```

Expected: all pass. The promoted methods were already tested; we haven't changed any logic.

- [ ] **Step 4: Commit**

```bash
git add src/graph/storage.rs
git commit -m "refactor: promote get_facts_by_entity/attribute to production code (#208)"
```

---

### Task 2: Write failing tests for selective lookup behaviour

**Files:**
- Modify: `src/query/datalog/executor.rs` (add test module entries)

These tests verify that entity-bound and attribute-bound queries return the right results after the optimisation is applied. They test behaviour, not internals — no spying on which method was called.

- [ ] **Step 1: Write the tests**

Locate the `#[cfg(test)]` module near the bottom of `src/query/datalog/executor.rs`. Add these tests:

```rust
#[cfg(test)]
mod selective_lookup_tests {
    use crate::graph::FactStorage;
    use crate::query::datalog::executor::DatalogExecutor;
    

    fn make_db_with_entities(n: usize) -> DatalogExecutor {
        let storage = FactStorage::new();
        let exec = DatalogExecutor::new(storage);
        for batch_start in (0..n).step_by(50) {
            let batch_end = (batch_start + 50).min(n);
            let mut cmd = String::from("(transact [");
            for i in batch_start..batch_end {
                cmd.push_str(&format!(r#"[:e{i} :name "entity{i}"]"#, i = i));
                cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
            }
            cmd.push_str("])");
            exec.execute(crate::query::datalog::parser::parse_datalog_command(&cmd).unwrap()).unwrap();
        }
        exec
    }

    #[test]
    fn entity_bound_query_returns_correct_results() {
        let exec = make_db_with_entities(100);
        // Point lookup: only entity :e5
        let result = exec
            .execute(crate::query::datalog::parser::parse_datalog_command(r#"(query [:find ?n :where [:e5 :name ?n]])"#).unwrap())
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 1, "expected exactly 1 result for entity :e5");
            assert_eq!(
                results[0][0],
                crate::graph::types::Value::String("entity5".to_string())
            );
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn attribute_bound_query_returns_correct_results() {
        let exec = make_db_with_entities(100);
        // All entities with :val
        let result = exec
            .execute(crate::query::datalog::parser::parse_datalog_command("(query [:find ?e ?v :where [?e :val ?v]])").unwrap())
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert_eq!(results.len(), 100, "expected 100 results for :val attribute scan");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn query_with_many_bound_entities_returns_correct_results() {
        // 6 entity literals → exceeds threshold of 4 → falls back to full scan
        // Result must still be correct
        let exec = make_db_with_entities(100);
        let result = exec
            .execute(crate::query::datalog::parser::parse_datalog_command(
                "(query [:find ?n :where [?e :name ?n] \
                 (or [:e0 :val ?v0] [:e1 :val ?v1] [:e2 :val ?v2] \
                     [:e3 :val ?v3] [:e4 :val ?v4] [:e5 :val ?v5])])",
            ).unwrap())
            .unwrap();
        // We just need it to not panic and return a non-empty result
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            assert!(!results.is_empty(), "expected non-empty results");
        } else {
            panic!("expected QueryResults");
        }
    }

    #[test]
    fn as_of_query_still_works_after_change() {
        let exec = make_db_with_entities(10);
        // as_of query must work — it takes the full-scan path regardless
        let result = exec
            .execute(crate::query::datalog::parser::parse_datalog_command("(query [:find ?n :where [?e :name ?n] :as-of 1])").unwrap())
            .unwrap();
        if let crate::query::datalog::executor::QueryResult::QueryResults { results, .. } = result {
            // as-of 1 (first tx) should return the first batch
            assert!(!results.is_empty(), "expected results from as-of 1 query");
        } else {
            panic!("expected QueryResults");
        }
    }
}
```

- [ ] **Step 2: Run the tests to verify they compile and pass (they should pass even before the optimisation, since correctness is what we're testing)**

```bash
cargo test selective_lookup_tests 2>&1 | tail -10
```

Expected: all 4 tests pass. These tests verify existing behaviour — confirming the baseline before we touch executor.rs.

- [ ] **Step 3: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "test: add selective lookup correctness tests (#208)"
```

---

### Task 3: Implement `selective_fact_fetch`

**Files:**
- Modify: `src/query/datalog/executor.rs` — add method on `DatalogExecutor`

- [ ] **Step 1: Add `selective_fact_fetch` as a private method on `DatalogExecutor`**

Find the `impl DatalogExecutor` block (starts around line 96). Add this method after `filter_facts_for_query` (after line ~338):

```rust
/// Attempt a selective index-backed fact fetch for the given patterns.
///
/// Inspects `patterns` for bound entity literals (UUID or keyword → UUID) and bound
/// attribute strings. If the total distinct lookup count is 0 (no bound values) or
/// exceeds `threshold` (too many, full scan is cheaper), returns `None`.
/// Otherwise returns `Some(facts)` — the union of all selectively fetched facts,
/// deduplicated by `(entity, attribute, tx_count)`.
fn selective_fact_fetch(
    &self,
    patterns: &[Pattern],
    threshold: usize,
) -> Option<Vec<Fact>> {
    use crate::query::datalog::matcher::edn_to_entity_id;
    use std::collections::HashSet;

    let mut entity_ids: HashSet<uuid::Uuid> = HashSet::new();
    let mut attributes: HashSet<String> = HashSet::new();

    for pattern in patterns {
        // Bound entity literal (UUID or keyword that resolves deterministically)
        match &pattern.entity {
            EdnValue::Uuid(u) => {
                entity_ids.insert(*u);
            }
            EdnValue::Keyword(_) => {
                if let Ok(uid) = edn_to_entity_id(&pattern.entity) {
                    entity_ids.insert(uid);
                }
            }
            _ => {}
        }
        // Bound attribute (non-variable Real attribute)
        if let AttributeSpec::Real(EdnValue::Keyword(attr)) = &pattern.attribute {
            attributes.insert(attr.clone());
        }
    }

    let total = entity_ids.len() + attributes.len();
    if total == 0 || total > threshold {
        return None;
    }

    // Dedup key: (entity uuid, attribute string, tx_count) — no Value formatting needed.
    let mut seen: HashSet<(uuid::Uuid, String, u64)> = HashSet::new();
    let mut all_facts: Vec<Fact> = Vec::new();

    for uid in &entity_ids {
        match self.storage.get_facts_by_entity(uid) {
            Ok(facts) => {
                for fact in facts {
                    let key = (fact.entity, fact.attribute.clone(), fact.tx_count);
                    if seen.insert(key) {
                        all_facts.push(fact);
                    }
                }
            }
            Err(_) => return None, // storage error → fall back to full scan
        }
    }

    for attr in &attributes {
        match self.storage.get_facts_by_attribute(attr) {
            Ok(facts) => {
                for fact in facts {
                    let key = (fact.entity, fact.attribute.clone(), fact.tx_count);
                    if seen.insert(key) {
                        all_facts.push(fact);
                    }
                }
            }
            Err(_) => return None,
        }
    }

    Some(all_facts)
}
```

- [ ] **Step 2: Wire it into `filter_facts_for_query`**

Find the `(None, None)` arm of the `match` statement in `filter_facts_for_query` (around line 308):

```rust
// BEFORE:
(None, None) => self.storage.get_all_facts()?,
```

Replace with:

```rust
// AFTER:
(None, None) => {
    let patterns = query.get_patterns();
    self.selective_fact_fetch(&patterns, 4)
        .unwrap_or_else(|| self.storage.get_all_facts().unwrap_or_default())
}
```

Wait — the surrounding `match` arm is inside a `Result` context. Look at the exact shape:

```rust
let source_facts: Vec<Fact> = match (&self.facts_override, query.as_of.as_ref()) {
    (Some(facts), Some(as_of)) => { … }
    (Some(facts), None) => facts.iter().cloned().collect(),
    (None, Some(as_of)) => self.storage.get_facts_as_of(as_of)?,
    (None, None) => self.storage.get_all_facts()?,
};
```

Replace only the last arm:

```rust
    (None, None) => {
        let patterns = query.get_patterns();
        match self.selective_fact_fetch(&patterns, 4) {
            Some(facts) => facts,
            None => self.storage.get_all_facts()?,
        }
    }
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo build 2>&1 | head -20
```

Expected: no errors.

- [ ] **Step 4: Run the tests from Task 2**

```bash
cargo test selective_lookup_tests 2>&1 | tail -10
```

Expected: all 4 pass.

- [ ] **Step 5: Run the full test suite**

```bash
cargo test 2>&1 | tail -15
```

Expected: all 795+ tests pass. Zero failures.

- [ ] **Step 6: Commit**

```bash
git add src/query/datalog/executor.rs
git commit -m "feat: selective B+Tree lookup in filter_facts_for_query (#208)"
```

---

### Task 4: Add benchmarks

**Files:**
- Modify: `benches/minigraf_bench.rs` — add `bench_btree_lookup` function
- Modify: `benches/helpers/mod.rs` — add `populate_with_point_lookups` helper

- [ ] **Step 1: Add a helper to `benches/helpers/mod.rs`**

After `populate_with_dept`, add:

```rust
/// In-memory DB with `n` entities, each with `:val` and `:name` attributes.
/// Suitable for point-lookup and attribute-scan benchmarks.
///
/// Schema:
///   `:e{i} :val {i}` for i in 0..n
///   `:e{i} :name "entity{i}"` for i in 0..n
pub fn populate_with_names(n: usize) -> Arc<Minigraf> {
    let db = Minigraf::in_memory().unwrap();
    for batch_start in (0..n).step_by(50) {
        let batch_end = (batch_start + 50).min(n);
        let mut cmd = String::from("(transact [");
        for i in batch_start..batch_end {
            cmd.push_str(&format!("[:e{i} :val {i}]", i = i));
            cmd.push_str(&format!(r#"[:e{i} :name "entity{i}"]"#, i = i));
        }
        cmd.push_str("])");
        db.execute(&cmd).unwrap();
    }
    Arc::new(db)
}
```

- [ ] **Step 2: Add `bench_btree_lookup` to `benches/minigraf_bench.rs`**

Add this function before the `criterion_group!` macro at the bottom:

```rust
// ── B+Tree selective lookup (Issue #208) ─────────────────────────────────────
// Baseline benchmarks for point-entity and attribute-scan queries.
// Used to measure the effect of selective vs full-scan fetch.

fn bench_btree_lookup(c: &mut Criterion) {
    const SCALES: &[(&str, usize)] = &[("1k", 1_000), ("10k", 10_000), ("100k", 100_000)];

    // entity_point: query a single known entity literal.
    // Should be O(1) in entity count with selective lookup.
    {
        let mut group = c.benchmark_group("btree_lookup/entity_point");
        group.sample_size(10);
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let db = helpers::populate_with_names(n);
                b.iter(|| {
                    db.execute(r#"(query [:find ?n :where [:e0 :name ?n]])"#)
                        .unwrap()
                });
            });
        }
        group.finish();
    }

    // attribute_scan: scan all entities via a single bound attribute.
    // Should be O(matching facts) rather than O(all facts) with selective lookup.
    {
        let mut group = c.benchmark_group("btree_lookup/attribute_scan");
        group.sample_size(10);
        for &(label, n) in SCALES {
            group.bench_with_input(BenchmarkId::from_parameter(label), &n, |b, &n| {
                let db = helpers::populate_with_names(n);
                b.iter(|| {
                    db.execute("(query [:find ?e ?n :where [?e :name ?n]])")
                        .unwrap()
                });
            });
        }
        group.finish();
    }
}
```

- [ ] **Step 3: Register the new benchmark group**

In the `criterion_group!` macro at the bottom of `benches/minigraf_bench.rs`, add `bench_btree_lookup` to the list:

```rust
criterion_group!(
    benches,
    bench_insert,
    bench_insert_file,
    bench_query,
    bench_time_travel,
    bench_recursion,
    bench_negation,
    bench_disjunction,
    bench_aggregation,
    bench_expr,
    bench_window,
    bench_temporal_metadata,
    bench_udf,
    bench_aggregation_extras,
    bench_query_extras,
    bench_open,
    bench_checkpoint,
    bench_concurrent,
    bench_concurrent_file,
    bench_concurrent_btree_scan,
    bench_prepared,
    bench_retract,
    bench_btree_lookup,   // ← new
);
criterion_main!(benches);
```

- [ ] **Step 4: Verify benchmarks compile**

```bash
cargo bench --no-run 2>&1 | tail -5
```

Expected: `Compiling minigraf …` then `Finished`. No errors.

- [ ] **Step 5: Run just the new benchmarks (quick smoke check)**

```bash
cargo bench btree_lookup -- --sample-size 3 2>&1 | tail -20
```

Expected: two groups run, numbers printed, no panics.

- [ ] **Step 6: Run full test suite one last time**

```bash
cargo test 2>&1 | tail -5
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add benches/minigraf_bench.rs benches/helpers/mod.rs
git commit -m "bench: add btree_lookup/entity_point and attribute_scan benchmarks (#208)"
```
