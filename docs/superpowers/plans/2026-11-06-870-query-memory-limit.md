# #870 plan: `:query-memory-limit`, limit bind slots, `PreparedQuery` setters

Spec: `docs/superpowers/specs/2026-11-06-862-override-audit-design.md` §3–§5, §9 (PR 2 of §8), merged in #868.
Branch `feat/870-memory-limit` off `v3` at `b79ff1d6` (after #869). One PR to `v3`, with `Refs #870, #862` in its body. Closing keywords do not fire on `v3`, so close #870 with `gh issue close` after the merge.

## 1. Types (`types.rs`)

- `DatalogQuery::max_derived_facts` and `max_results` become `Option<Count>`. A new field, `query_memory_limit: Option<Count>`, is added. `Count` keeps its two variants (`Literal(usize)`, `Slot(String)`), and its doc now lists every keyword that uses it.
- `Count::literal(&self, option) -> Result<usize>` returns the literal, or the invariant error `plan.rs` uses today when a slot reaches the planner. `plan.rs`'s `count` closure calls it.

## 2. Parser (`parser.rs`)

- `:max-derived-facts` / `:max-results`: as today (INT-012 duplicate, INT-013 missing, INT-014 for `< 1`, INT-015 for a non-integer), plus `EdnValue::BindSlot` → `Count::Slot`.
- `:query-memory-limit`: the same arm shape. `Integer(n) if n >= 0` → `Literal`, `BindSlot` → `Slot`, a negative integer or a non-integer → INT-015. INT-012/013 as above.
- **Deviation 1.** INT-015's text is `{} must be a positive integer`. That is wrong for `:query-memory-limit`, where 0 is allowed. Change the text to `{} must be a non-negative integer`, both in `error.rs` and in `ERROR_REFERENCE.md`. The heading becomes "Query option must be a non-negative integer", and the cause lists all three keywords. The code is unchanged, and for the two older keywords the new text is still true (INT-014 then rejects 0).

## 3. Gate (`overrides.rs`)

- Add `Overridable::QueryMemoryLimit` (keyword `:query-memory-limit`, name `QueryMemoryLimit`). It is not added to `DEFAULTS`.
- `check_overrides` checks in a fixed order: `MaxDerivedFacts`, `MaxResults`, `QueryMemoryLimit`. Presence is `is_some()`, so a slot counts as presence.
- BrowserDb calls the check with `DEFAULTS` (#869), so it refuses `:query-memory-limit` with API-025 and needs no other change.

## 4. Executor (`executor.rs`)

- `derived_limit` reads the literals through `Count::literal`, and so returns `Result`.
- Add `memory_limit(query) -> Result<usize>`, which returns the keyword's literal if the query has one and otherwise the executor's `query_memory_limit`. Both `open` (passed to `engine::open`) and `open_rule_query` (`Limits::query_memory`) use it.
- No `db.rs` change is needed: every entry point (`execute`, `query`, `WriteTransaction::execute`, prepared) ends up in these two functions.

## 5. Slots (`prepared.rs`)

- `collect_slot_names` also collects the three limit slots. That way `reject_unbound_slots` gives API-010 on `execute`/`query`, and `bind` gives API-022 for a missing value.
- `substitute` resolves each one with `resolve_limit_slot(name, keyword, min, bindings)`:
  - `Val(Integer(n))` with `n >= min` → `Literal`.
  - `n < min`: INT-014 for `min == 1`, INT-015 for `min == 0`, matching the literal form.
  - Any other type → API-024 (`Val(Integer)` expected).
  - Missing → API-022.
- `min` is 1 for `:max-derived-facts`/`:max-results` and 0 for `:query-memory-limit`.

## 6. Setters (`prepared.rs`)

```rust
impl PreparedQuery {
    /// Replace the database's `max_derived_facts` for this statement. An allowed
    /// `:max-derived-facts` / `:max-results` in the query still overrides it.
    pub fn with_max_derived_facts(mut self, n: usize) -> Self { self.limits.0 = n; self }
    /// Replace the database's `query_memory_limit` for this statement.
    pub fn with_query_memory_limit(mut self, bytes: usize) -> Self { self.limits.1 = bytes; self }
}
```

Both are infallible and ungated. `usize::MAX` means no limit, and `0` behaves as at open. Doc tests on both.

## 7. Grammar and verifier

- `grammar.pest`: add `kw_query_memory_limit = @{ ":query-memory-limit" ~ !word_cont }`. `max_derived_facts_section` and `max_results_section` take `(integer_lit | bind_slot)`. Add `query_memory_limit_section = { kw_query_memory_limit ~ (integer_lit | bind_slot) }` to the `query_section` alternatives. If the grammar has a keyword list for `word` exclusion, add it there too.
- Regenerate `docs/GRAMMAR.md` (`tests/grammar_doc.rs` bless mode).
- Corpus: `tests/grammar/valid/query_memory_limit.edn` and `query_limit_slots.edn` (slots on all three keywords), plus `tests/grammar/invalid/` seeds if that directory takes them (e.g. `:query-memory-limit :x`). Add a fuzz seed in `fuzz/corpus/datalog_parser/`.
- Verifier: add `":query-memory-limit"` to `TOKENS` (40 → 41). Add `SEMANTIC` INT-015 with the witness `(query [:find ?x :where [?x :a ?y] :query-memory-limit -1])`.
- Run bound 4 on the bench machine. Record the case count and time in the PR body (+~10% fillers).

## 8. Tests

`tests/limit_keywords_test.rs` (`#![cfg(not(target_arch = "wasm32"))]`). Assertions are on `code()` only.

1. **Memory keyword lowers.** A hash-join query passes at the open default, and `:query-memory-limit 1024` makes it fail with QRY-012 (handle allows `QueryMemoryLimit`).
2. **Memory keyword raises.** At open `query_memory_limit(1024)` the same query fails, and with `:query-memory-limit 100000000` it passes.
3. **Zero.** With `:query-memory-limit 0`, a single-pattern scan with a `[(> ?v 1)]` filter returns its rows, while a hash-join query and an aggregate (`count` grouped) fail with QRY-012. A rule query under 0 fails with QRY-012.
4. **Gate.** By default `:query-memory-limit` is refused with API-025 (`execute`, `query`, `prepare`, `WriteTransaction::execute`). It is allowed when listed. `allow_overrides([])` refuses `:max-derived-facts $n` at prepare.
5. **Validation.** Duplicate → INT-012, missing → INT-013, `-1` → INT-015, `"x"` → INT-015.
6. **Slots.** One prepared rule query run with `n` = 1 and with `n` = 1000 for `:max-derived-facts $n` gives INT-020 then rows. The same holds for `:max-results $n`. With `:query-memory-limit $m`, `m` = 0 fails and `m` = `i64::MAX` passes.
   - Errors: `Val(String)` → API-024; missing → API-022; `n = 0` on `:max-derived-facts` → INT-014; `m = -1` → INT-015.
   - `execute` with a limit slot → API-010.
7. **Setters.** `with_max_derived_facts(1)` makes a rule query fail (INT-020), and `with_max_derived_facts(1000)` on a database opened at `max_derived_facts(1)` passes. A keyword overrides the setter: setter 1 with `:max-derived-facts 1000` passes. `with_query_memory_limit(1024)` makes a join fail and `with_query_memory_limit(usize::MAX)` on a 1024 database passes. Setters work under `allow_overrides([])`, and they also apply to `PreparedQuery::query` (cursor).
8. **Unit.** `check_overrides` order with all three keywords present; parser unit tests for the slot forms and INT codes; `Count::literal` on a slot.
9. **BrowserDb.** A `#[wasm_bindgen_test]` showing `:query-memory-limit` is refused with API-025.

TDD: write each group failing first, then implement.

## 9. Docs (same PR)

- `docs/CONFIGURATION.md`: the `query_memory_limit` row is now done (keyword + setter); `max_derived_facts` gets `$slot` and the setter; a `:query-memory-limit` example; the "planned (#870)" notes are removed.
- `docs/ERROR_REFERENCE.md`: the new INT-015 text (deviation 1), and the INT-012..015 causes mention `:query-memory-limit`.
- `CHANGELOG.md [Unreleased]` Added: `:query-memory-limit`, limit slots, the setters, `Overridable::QueryMemoryLimit`. Changed: the INT-015 text.
- Test counts in `docs/TEST_COVERAGE.md` and `CLAUDE.md`. In `CLAUDE.md`, the `prepared.rs`/`types.rs` lines mention the limit slots and setters.
- Docs site (`../minigraf-docs`), under `<!-- @since v3.0.0 -->`: `datalog-reference.md` gets the keyword and slot forms, and `performance-tuning.md` gets the setters and the table row.

## 10. Verification

- `cargo fmt`; `cargo clippy --all-targets -- -D warnings`.
- Full `cargo test` on the bench machine, plus `--features test-hooks`.
- `cargo build --target wasm32-unknown-unknown --features browser`.
- **Size gate.** Only ~360 B of headroom is left under 1,234 KiB. Measure `--profile dist`, and if it is over, ask the maintainer before raising the gate (with the measured delta).
- Codex review is skipped (quota out until 2026-11-11). Then watch CI until it is green.

After merge: #861 (`MaxIterations`, reusing `resolve_limit_slot` and the setter pattern), then #872.
