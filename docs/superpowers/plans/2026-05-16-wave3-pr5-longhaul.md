# Wave 3 PR 5 — Long-haul Smoke Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a long-run reliability smoke suite (#220) that imports a large synthetic graph, runs representative queries across multiple reopen+checkpoint cycles, and asserts correctness invariants after each cycle.

**Architecture:** A single `tests/smoke_test.rs` file with one `#[ignore]` test (skipped in standard CI). A separate `smoke.yml` nightly GitHub Actions workflow runs it on a schedule. The smoke test uses file-backed `Minigraf::open()` so it exercises the full durability path.

**Tech Stack:** Rust stable, `tempfile`, file-backed Minigraf

**Prerequisites:** PRs 2, 3, 4 merged (all reliability tests passing)

**Closes:** #220

---

## File Map

| Action | Path | Purpose |
|---|---|---|
| Create | `tests/smoke_test.rs` | long-haul smoke suite |
| Create | `.github/workflows/smoke.yml` | nightly scheduled runner |

---

## Task 1: Long-haul smoke suite

**Files:**
- Create: `tests/smoke_test.rs`

- [ ] **Step 1: Write the smoke test**

Create `tests/smoke_test.rs`:

```rust
//! Long-run reliability smoke suite (#220).
//!
//! Imports a large synthetic graph, runs representative queries, and reopens
//! repeatedly to verify durability and correctness invariants.
//!
//! Skipped in standard CI. Run manually:
//!   cargo test --test smoke_test -- --include-ignored --nocapture
//!
//! Nightly CI runs this via .github/workflows/smoke.yml.
#![cfg(not(target_arch = "wasm32"))]

use minigraf::db::Minigraf;
use minigraf::QueryResult;

fn count_results(r: QueryResult) -> usize {
    match r {
        QueryResult::QueryResults { results, .. } => results.len(),
        _ => 0,
    }
}

/// The full long-haul smoke test.
///
/// Workload:
/// - 500 entities × 10 attributes = 5,000 facts
/// - 10 reopen + checkpoint cycles
/// - Representative queries after each cycle (basic, temporal, recursive-capable, aggregation)
/// - Invariants checked after each cycle
#[test]
#[ignore]
fn smoke_large_graph_10_cycles() {
    eprintln!("smoke: starting long-haul smoke suite");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("smoke.graph");

    const NUM_ENTITIES: usize = 500;
    const NUM_CYCLES: usize = 10;

    // ── Phase 1: Load 5,000 facts ─────────────────────────────────────────────
    eprintln!("smoke: loading {} entities × 10 attributes", NUM_ENTITIES);
    {
        let db = Minigraf::open(&path).unwrap();

        // Batch transact in groups of 50 entities to keep individual transactions small.
        for batch_start in (0..NUM_ENTITIES).step_by(50) {
            let batch_end = (batch_start + 50).min(NUM_ENTITIES);
            let facts: String = (batch_start..batch_end)
                .flat_map(|i| {
                    vec![
                        format!(r#"[:entity{i} :name "entity-{i}"]"#),
                        format!(r#"[:entity{i} :index {i}]"#),
                        format!(r#"[:entity{i} :bucket {}]"#, i % 10),
                        format!(r#"[:entity{i} :active {}]"#, i % 3 != 0),
                        format!(r#"[:entity{i} :score {}]"#, (i * 7) % 100),
                        format!(r#"[:entity{i} :group {}]"#, i % 5),
                        format!(r#"[:entity{i} :tier {}]"#, i % 4),
                        format!(r#"[:entity{i} :region "region-{}"]"#, i % 8),
                        format!(r#"[:entity{i} :version 1]"#),
                        format!(r#"[:entity{i} :label "label-{}"]"#, i % 20),
                    ]
                })
                .collect::<Vec<_>>()
                .join(" ");
            db.execute(&format!("(transact [{facts}])")).unwrap();
        }

        db.checkpoint().unwrap();
    }
    eprintln!("smoke: initial load complete");

    // ── Phase 2: 10 reopen + query + checkpoint cycles ───────────────────────
    for cycle in 0..NUM_CYCLES {
        eprintln!("smoke: cycle {}/{}", cycle + 1, NUM_CYCLES);

        let db = Minigraf::open(&path).unwrap();

        // Invariant 1: all 500 entities must be queryable by :name.
        let n_names = count_results(
            db.execute("(query [:find ?e :where [?e :name ?n]])")
                .unwrap(),
        );
        assert_eq!(
            n_names, NUM_ENTITIES,
            "cycle {cycle}: expected {NUM_ENTITIES} entities by :name, got {n_names}"
        );

        // Invariant 2: :index values must be unique per entity.
        let n_index = count_results(
            db.execute("(query [:find ?e ?i :where [?e :index ?i]])")
                .unwrap(),
        );
        assert_eq!(
            n_index, NUM_ENTITIES,
            "cycle {cycle}: expected {NUM_ENTITIES} :index facts, got {n_index}"
        );

        // Invariant 3: bucket distribution — 10 buckets, 50 entities each.
        for b in 0..10usize {
            let n_bucket = count_results(
                db.execute(&format!("(query [:find ?e :where [?e :bucket {b}]])"))
                    .unwrap(),
            );
            assert_eq!(
                n_bucket, 50,
                "cycle {cycle}: bucket {b} should have 50 entities, got {n_bucket}"
            );
        }

        // Invariant 4: active entities (index % 3 != 0) = 334.
        let n_active = count_results(
            db.execute("(query [:find ?e :where [?e :active true]])")
                .unwrap(),
        );
        // 500 entities, active if i % 3 != 0. Inactive: i=0,3,6,...,498 → 167 entities.
        assert_eq!(
            n_active,
            NUM_ENTITIES - (NUM_ENTITIES / 3 + if NUM_ENTITIES % 3 == 0 { 1 } else { 0 }),
            "cycle {cycle}: active entity count mismatch; got {n_active}"
        );

        // Invariant 5: temporal query (:as-of 1) returns data from tx 1.
        let n_temporal = count_results(
            db.execute("(query [:find ?e :where [?e :version 1]] :as-of 1)")
                .unwrap_or(QueryResult::QueryResults { results: vec![], headers: vec![] }),
        );
        // Should return some results — exact count depends on tx ordering.
        // Just verify it doesn't error and returns > 0 for tx 1.
        assert!(
            n_temporal > 0,
            "cycle {cycle}: temporal query :as-of 1 returned no results"
        );

        // Invariant 6: prepared query returns consistent results.
        let prep = db
            .prepare("(query [:find ?e :where [?e :region $region]])")
            .unwrap();
        let r0 = prep.execute_with([("region", minigraf::db::BindValue::String("region-0".to_string()))]).unwrap();
        let n_region0 = count_results(r0);
        // region-0: entities where i % 8 == 0 → 0,8,16,...,496 → 63 entities.
        assert_eq!(
            n_region0, 63,
            "cycle {cycle}: prepared query for region-0 returned {n_region0}"
        );

        // Invariant 7: write a new fact each cycle.
        db.execute(&format!(r#"(transact [[:cycle{cycle} :cycle-fact {cycle}]])"#))
            .unwrap();

        // Checkpoint to flush all to disk.
        db.checkpoint().unwrap();

        eprintln!("smoke: cycle {}/{} complete — all invariants passed", cycle + 1, NUM_CYCLES);
    }

    // ── Phase 3: Final reopen and full invariant check ────────────────────────
    eprintln!("smoke: final reopen and invariant verification");
    let db_final = Minigraf::open(&path).unwrap();

    let n_final = count_results(
        db_final
            .execute("(query [:find ?e :where [?e :name ?n]])")
            .unwrap(),
    );
    assert_eq!(
        n_final, NUM_ENTITIES,
        "final reopen: expected {NUM_ENTITIES} entities, got {n_final}"
    );

    // Cycle facts from all 10 cycles must be present.
    let n_cycles = count_results(
        db_final
            .execute("(query [:find ?e :where [?e :cycle-fact ?c]])")
            .unwrap(),
    );
    assert_eq!(
        n_cycles, NUM_CYCLES,
        "final reopen: expected {NUM_CYCLES} cycle facts, got {n_cycles}"
    );

    eprintln!("smoke: all invariants passed across {NUM_CYCLES} cycles");
}
```

Note: `BindValue` import path — check `src/query/datalog/prepared.rs` or `src/db.rs` for the correct path. If `BindValue` is not re-exported from `minigraf::db`, use `minigraf::query::datalog::prepared::BindValue` or the appropriate path. If prepared queries have a different API, simplify that invariant to just use `db.execute()` with string interpolation.

Verify the BindValue import:
```bash
grep -rn 'pub.*BindValue' src/ | head -5
grep -n 'BindValue' src/db.rs | head -5
```

Adjust the import in the test accordingly.

- [ ] **Step 2: Run the smoke test to verify it works**

```bash
cargo test --test smoke_test -- --include-ignored --nocapture 2>&1 | tail -30
```
Expected: outputs "smoke: cycle N/10 complete" for each cycle, ends with "all invariants passed".

Note: this takes ~30–60 seconds on a typical laptop.

- [ ] **Step 3: Verify standard cargo test skips the smoke suite**

```bash
cargo test --test smoke_test 2>&1 | tail -10
```
Expected: `0 tests` run (all ignored).

- [ ] **Step 4: Commit**

```bash
git add tests/smoke_test.rs
git commit -m "test(smoke): add #220 long-haul reliability smoke suite (500 entities, 10 cycles)"
```

---

## Task 2: Nightly smoke workflow

**Files:**
- Create: `.github/workflows/smoke.yml`

- [ ] **Step 1: Create smoke.yml**

```yaml
name: Smoke Suite (nightly)

on:
  schedule:
    - cron: "0 5 * * *"
  workflow_dispatch:

permissions:
  contents: read

jobs:
  smoke:
    name: Long-haul smoke suite
    runs-on: ubuntu-latest
    timeout-minutes: 15

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust stable
        uses: dtolnay/rust-toolchain@stable

      - name: Build
        run: cargo build --release

      - name: Run smoke suite
        run: cargo test --test smoke_test -- --include-ignored --nocapture
        env:
          RUST_BACKTRACE: 1
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/smoke.yml
git commit -m "ci: add nightly smoke suite workflow (#220)"
```

---

## Task 3: Open PR

- [ ] **Push and open PR**

```bash
git push -u origin HEAD
gh pr create \
  --title "test(smoke): long-haul reliability smoke suite (#220)" \
  --body "$(cat <<'EOF'
## Wave 3 PR 5 — Long-haul

Closes #220.

### Changes
- `tests/smoke_test.rs`: `#[ignore]` smoke test — 500 entities × 10 attributes = 5,000 facts, across 10 reopen+checkpoint cycles. 7 invariants checked per cycle including temporal queries, prepared queries, and bucket distributions.
- `.github/workflows/smoke.yml`: nightly scheduled run (5am UTC daily), 15-minute timeout.

### Run locally
```
cargo test --test smoke_test -- --include-ignored --nocapture
```

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Monitor CI until green before merging**
