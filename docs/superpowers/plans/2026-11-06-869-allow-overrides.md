# #869 plan: `OpenOptions::allow_overrides` gate

Spec: `docs/superpowers/specs/2026-11-06-862-override-audit-design.md` (§1, §2, §7, §9 gate tests), merged in #868.
Branch `feat/869-allow-overrides` off `v3` at `70ed3f21`. One PR to `v3`, with `Refs #869, #862` in its body. Closing keywords do not fire on `v3`, so close #869 with `gh issue close` after the merge.

## 1. Public API

Add a new module `src/overrides.rs`, re-exported from `lib.rs` as `minigraf::Overridable`:

```rust
/// A setting that query text may override (#862).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overridable { MaxDerivedFacts, MaxResults }

impl Overridable {
    /// The list a database uses when `OpenOptions::allow_overrides` is not set.
    pub const DEFAULTS: &'static [Overridable] = &[Overridable::MaxDerivedFacts, Overridable::MaxResults];
    /// The query keyword this variant gates, e.g. ":max-derived-facts".
    pub fn keyword(self) -> &'static str;
}
```

- `QueryMemoryLimit` comes in #870, and `MaxIterations` in #861, together with its entry in `DEFAULTS`. Adding a variant later is not a breaking change, because the enum is `#[non_exhaustive]`.
- `OpenOptions` gains a field `pub allow_overrides: Vec<Overridable>`, which `Default` sets to `DEFAULTS.to_vec()`.
- It also gains a builder, `pub fn allow_overrides(mut self, list: impl IntoIterator<Item = Overridable>) -> Self`, which replaces the list. Its doc explains replace semantics and gives the `DEFAULTS` concat example.
- `OpenOptions` derives only `Debug` and `Clone`, not `Copy`, so a `Vec` field is fine.
- The `OpenOptionsWithPath` builder forwards `allow_overrides` like the other options.
- `cargo semver-checks` runs against the PR base. Adding a field to a `#[non_exhaustive]` struct and adding a new enum are both non-breaking.

## 2. Error code API-025

- Add `ErrorCode::Api025` with text `query option {} is not allowed on this database; OpenOptions::allow_overrides does not list Overridable::{}` (category Api).
- Add it to the registry tables in `src/error.rs`, including the category tests that list API codes.
- Add it to `docs/ERROR_REFERENCE.md`: in the table, "Query option override not allowed", and as a section with cause, resolution and scenario.

## 3. The check

- `pub(crate) fn check_overrides(query: &DatalogQuery, allowed: &[Overridable]) -> Result<()>` lives in `overrides.rs`. A keyword is present when its parsed field is `Some(_)`: `max_derived_facts` and `max_results`.
- The check runs in a fixed order, `MaxDerivedFacts` then `MaxResults`, and reports the first keyword that is present but not listed.
- It reads only the top-level query. Rule bodies take no keywords, and subqueries don't exist.

Call it right after parsing, before `reject_unbound_slots`, at every query entry point:

| Site | List |
|---|---|
| `Minigraf::execute`, Query arm (`db.rs` ~761) | `self.inner.options.allow_overrides` |
| `Minigraf::query_inner` (~982) | same |
| `Minigraf::prepare_inner` (~1347), before `prepare_query` | same |
| `WriteTransaction::execute`, Query arm (~1770) | `self.inner.options.allow_overrides` |
| `BrowserDb` `execute`, `query`, `prepare` (`browser/mod.rs`) | `Overridable::DEFAULTS` (BrowserDb takes no options) |

Since `DEFAULTS` allows both of today's keywords, this PR changes nothing for BrowserDb. Calling the check there now means #870's gated keyword is refused in BrowserDb without another change.

## 4. Tests (`tests/allow_overrides_test.rs`, `#![cfg(not(target_arch = "wasm32"))]`)

1. **Defaults.** A default handle runs a rule query with `:max-derived-facts` and one with `:max-results`. Both return their rows.
2. **Empty list.** Under `allow_overrides([])`, each keyword is refused with API-025 by `execute`, `query`, `prepare` and `WriteTransaction::execute`. Assert on `code()`, and check the message names the keyword. The same queries without a keyword run.
3. **Replace semantics.** `allow_overrides([MaxResults])` refuses `:max-derived-facts` and allows `:max-results`.
4. **Refusal happens before execution.** A refused `WriteTransaction::execute` leaves the transaction usable: a later transact and commit succeed.
5. **Refused at prepare.** A refused `prepare` returns no `PreparedQuery`.
6. **Defaults read back.** `OpenOptions::default().allow_overrides == Overridable::DEFAULTS`. `DEFAULTS` concatenated with another list works, and duplicates in the list have no effect.
7. **File databases.** The option holds on a file database (`OpenOptions::new().allow_overrides([]).path(..).open()`), and it isn't persisted: reopening with defaults allows the keyword again.
8. **Registry.** API-025 is registered, documented and in the Api category, through the existing registry/doc consistency tests (`error_codes_foundation_test.rs`).

Plus a unit test of `check_overrides` order (both keywords present, both unlisted → `:max-derived-facts` is reported), and a doc test on `OpenOptions::allow_overrides`.

BrowserDb: a `#[wasm_bindgen_test]` in `browser/mod.rs` confirms `:max-derived-facts` still runs. That is the only behavior there.

## 5. Docs (same PR)

- **`docs/CONFIGURATION.md` (new).** The spec's §1 table, trimmed to what is true after this PR. Rows not done yet are marked "planned (#870/#871/#861)". It also explains `allow_overrides` with Rust examples. Link it from `README.md`.
- **`CHANGELOG.md` `[Unreleased]`.** Under Added: `allow_overrides` / `Overridable`. Under Documentation: `CONFIGURATION.md`. API-025 is noted under Added.
- **Test counts.** Update `docs/TEST_COVERAGE.md` and the `CLAUDE.md` count. The `CLAUDE.md` module list gains `overrides.rs`, and the `db.rs` line gains `allow_overrides`.
- **`ROADMAP.md`.** Check whether #862 is listed; update it if so.
- **Docs site PR** (`../minigraf-docs`): put the table and the `allow_overrides` section in `performance-tuning.md` under `<!-- @since v3.0.0 -->`. In `datalog-reference.md`, note under the limit keywords that the embedder can refuse them (API-025). Docs CI checks links.

## 6. Verification before the PR

- `cargo fmt`, then `cargo clippy --all-targets -- -D warnings`.
- `cargo test`: the full suite, on the bench machine first.
- `cargo test --doc`.
- `cargo build --target wasm32-unknown-unknown --features browser` to check the BrowserDb call sites.
- The size gate (`--profile dist`) shouldn't move much. Record the delta in the PR body.
- Review: `codex exec -m gpt-6-astra` (skip if it fails), then CI until it is green.

After merge: #861 is unblocked (`MaxIterations`), then #870.
