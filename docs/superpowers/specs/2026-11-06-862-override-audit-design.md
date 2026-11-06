# #862: OpenOptions override audit and `allow_overrides`

Date: 2026-11-06. Target: v3.0.0 (#862's milestone). Base: `v3` at `472e3f25`.
Status: draft for maintainer review.
Issue: https://github.com/project-minigraf/minigraf/issues/862. Blocks #861.

Maintainer decisions (2026-11-06):

- `allow_overrides` replaces the default list rather than merging with it, and it governs query text only.
- Override direction (lower-only for unlisted settings) is deferred.
- `query_memory_limit` gets a gated keyword and an ungated setter.
- Per-transaction `synchronous` and chunk size ship in v3.0.0.
- Limit keywords take bind slots, and `PreparedQuery` gets setters.
- `Overridable::DEFAULTS` holds `MaxDerivedFacts`, `MaxResults` and `MaxIterations`.

## 1. Audit (v3 as of `472e3f25`)

"Text" means query text (keywords and bind slots), which is gated. "Code" means application API, which is not gated.

| `OpenOptions` field | Today | After #862 / #861 | Level | Semantics |
|---|---|---|---|---|
| `max_derived_facts` | `:max-derived-facts N` (literal only) | + `$slot`, + `PreparedQuery::with_max_derived_facts` | query (text, code) | Replaces the database value, up or down |
| `max_results` (deprecated) | `:max-results N` (literal only) | + `$slot`; no setter (deprecated) | query (text) | With both keywords present, the lower applies; alone, it replaces the database value (`executor.rs` `derived_limit`) |
| round limit (#861) | hard-coded 1000 | `OpenOptions::max_iterations`, `:max-iterations N`/`$slot`, `with_max_iterations` (all in #861) | open, query (text, code) | Replaces the database value |
| `query_memory_limit` | open-only | + `:query-memory-limit N`/`$slot` (gated, not in defaults), + `with_query_memory_limit` | open, query (text, code) | Replaces the database value |
| `transaction_chunk_facts` | open-only; no binding exposes it | + `TxOptions::chunk_facts`; exposed in every binding | open, transaction (code) | Replaces the database value for one write transaction; clamped to 1..=1,000,000 as today |
| `synchronous` | open-only | + `TxOptions::synchronous` | open, transaction (code) | Applies to this transaction's WAL write only |
| `wal_checkpoint_threshold` | open-only | open-only | open | Database-wide checkpoint policy |
| `page_cache_size` | open-only | open-only | open | Shared cache |
| `allow_unlocked` | open-only | open-only | open | Decided when the file is opened |
| `read_only` | open-only | open-only | open | Decided when the file is opened |
| `allow_overrides` (new) | — | open-only | open | Policy for query text |

Other findings:

- **Cursors.** `Minigraf::query` and `PreparedQuery::query` take the same keywords as `execute`. No cursor-specific override is needed.
- **BrowserDb.** BrowserDb takes no `OpenOptions` at all, so its defaults (and `Overridable::DEFAULTS`) always apply. This spec adds no options API to it.
- **Bindings.** On `v3`, all 7 bindings already expose `max_derived_facts`, `max_results`, `query_memory_limit`, `synchronous`, `wal_checkpoint_threshold`, `page_cache_size`, `read_only` and `allow_unlocked`. None exposes `transaction_chunk_facts`.

## 2. `OpenOptions::allow_overrides`

```rust
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overridable { MaxDerivedFacts, MaxResults, QueryMemoryLimit /* MaxIterations: #861 */ }

impl Overridable {
    pub const DEFAULTS: &'static [Overridable] = &[MaxDerivedFacts, MaxResults /* , MaxIterations: #861 */];
}

pub struct OpenOptions { /* … */ pub allow_overrides: Vec<Overridable> }  // default: DEFAULTS.to_vec()
impl OpenOptions { pub fn allow_overrides(self, list: impl IntoIterator<Item = Overridable>) -> Self }
```

- **Replace semantics.** The list replaces `DEFAULTS`; it is not merged with it. An empty list allows no override keywords, and duplicates have no effect. #861 adds `MaxIterations` both to the enum and to `DEFAULTS`.
- **What it gates.** Every limit keyword in query text: `:max-derived-facts`, `:max-results`, `:query-memory-limit` and (#861) `:max-iterations`. This holds whether the value is a literal or a `$slot`, and whatever the entry point: `Minigraf::execute`, `query` and `prepare`, and `WriteTransaction::execute`. The check depends on whether the keyword is present, never on its value.
- **When the check runs.** It runs after parsing, before execution. For `prepare`, it runs at prepare time, so a refused keyword never yields a `PreparedQuery`. The check reads the handle's own `allow_overrides`, which is fixed at open.
- **Refusal.** A new code, **API-025 "Query option override not allowed"** (category Database API), names the keyword and the `Overridable` variant to add. The override is never ignored or clamped. API is the right category because the refusal enforces embedder policy; the query itself is valid.
- **Setters are not gated.** `PreparedQuery::with_*` and `TxOptions` are application code (decision 2026-11-06).
- **Rule definitions.** `(rule …)` takes none of these keywords today, and it stays that way.

Philosophy check: zero-configuration holds, because the default list keeps every v2 query working and adds `:max-iterations`. The default list also guards the one setting that can exhaust memory (`QueryMemoryLimit`), so an embedder running LLM-written query text is safe without configuring anything.

Compatibility: an embedder that sets `allow_overrides(vec![])` breaks queries that use `:max-derived-facts`. That is the intent of setting it. The default changes no v2 behavior.

## 3. `:query-memory-limit`

- **Syntax.** `:query-memory-limit N` in the query vector, where `N` is a non-negative integer (bytes), or a `$slot`. Errors: duplicate is INT-012, missing value INT-013, a negative value or a non-integer INT-015.
- **Zero is allowed.** `0` means what `query_memory_limit(0)` means at open level: the query runs only if no operator keeps state, and otherwise fails at once with QRY-012. This asserts that a query streams in constant memory. `:max-derived-facts` and `:max-results` keep rejecting `0` with INT-014 (unchanged; relaxing that is outside #862).
- **Effect.** It replaces the effective `query_memory_limit` for this query only. It applies to the query's top-level engine plan, rule evaluation and every operator the query uses, the same as the open-level value.
- **Gate.** `Overridable::QueryMemoryLimit`, which is not in `DEFAULTS`.
- **Syntax-change rule.** The PR updates `tests/grammar/grammar.pest`, regenerates `GRAMMAR.md`, adds a corpus seed and adds the keyword to the verifier's TOKENS/SEMANTIC lists.

## 4. Bind slots on limit keywords

- `:max-derived-facts`, `:max-results` and `:query-memory-limit` (and `:max-iterations` in #861) accept `$name`, parsed into a `Literal | Slot` value as `:limit $n` is today (`types.rs` `LimitValue`).
- A bind value must be `Val(Value::Integer(n))`. The range follows the keyword's literal form: `n >= 1` for `:max-derived-facts`/`:max-results` (INT-014 otherwise), `n >= 0` for `:query-memory-limit` (INT-015 for a negative value). Any other type fails at execute time with API-024, as for `:limit $n`. A missing value is API-022.
- `Minigraf::execute` and `query` with an unbound limit slot are refused with API-010, as for any other slot.

## 5. `PreparedQuery` setters

```rust
impl PreparedQuery {
    pub fn with_max_derived_facts(self, n: usize) -> Self;
    pub fn with_query_memory_limit(self, bytes: usize) -> Self;
    // with_max_iterations: #861
}
```

- A setter replaces the database value that was copied at prepare time. It becomes the statement's base limit.
- An allowed keyword in the prepared text (literal or slot) still overrides that base for the query, the same way a keyword overrides the database value today.
- Setters take any `usize` and are infallible, matching the `OpenOptions` setters, which validate nothing. `usize::MAX` means no limit. `0` behaves as it does at open: no derived facts allowed, or any stateful query fails.
- `max_results` gets no setter, because it is deprecated.
- **Bindings.** UniFFI objects are behind `Arc`, so each binding exposes these as methods that return a new prepared object (Node/C: a new handle; Python: a new object; JVM/Swift/Android: a new instance).

## 6. Per-transaction options

```rust
#[non_exhaustive]
#[derive(Clone, Debug, Default)]
pub struct TxOptions { pub synchronous: Option<SyncMode>, pub chunk_facts: Option<usize> }

impl TxOptions {
    pub fn new() -> Self;
    pub fn synchronous(self, m: SyncMode) -> Self;
    pub fn chunk_facts(self, n: usize) -> Self;
}

impl Minigraf { pub fn begin_write_with(&self, opts: TxOptions) -> Result<WriteTransaction<'_>, MinigrafError> }
```

- `begin_write()` is `begin_write_with(TxOptions::default())`, and `None` inherits the database value.
- **`synchronous`.** It applies to this transaction's commit: the WAL append and the WAL guard of a large transaction. A `Full` commit after an earlier `Normal` one flushes the whole WAL file, which includes the earlier entry; that is today's on-disk behavior. `WalWriter` takes a per-append `SyncMode` instead of storing one per writer. Its stored mode stays the database default for implicit `execute` transactions. On an in-memory database the option has no effect, as at open.
- **`chunk_facts`.** It replaces `transaction_chunk_facts` for statements executed in this transaction, using the same clamp (`statement::tx_chunk_facts`).
- **Durability tests.** A `Normal` transaction followed by a `Full` one, a crash after each (fault-injection backend), and the SIGKILL test, extended with a per-transaction mode.
- **Bindings.** Every binding exposes `transaction_chunk_facts` on its open options now. `TxOptions` reaches a binding only where that binding exposes write transactions; that is #824's work, and #824 picks `TxOptions` up.

## 7. Documentation

- **Where the §1 table goes.**
  - In this repo, a new `docs/CONFIGURATION.md`, linked from `README.md`.
  - On the docs site, `performance-tuning.md`, with the keywords in `datalog-reference.md`, all wrapped in `<!-- @since v3.0.0 -->`.
- `docs/ERROR_REFERENCE.md`: API-025.
- `CHANGELOG.md`, `docs/TEST_COVERAGE.md` and `CLAUDE.md` (test count) are updated per PR.

## 8. PR split (all on `v3`)

1. **Gate.** Adds `Overridable`, `allow_overrides`, `DEFAULTS` and API-025, and gates the two existing keywords. Also tests, the §1 table and its docs-site PR.
2. **Query text and prepared.** Adds `:query-memory-limit`, bind slots on limit keywords, the `PreparedQuery` setters, and the grammar/verifier updates. Docs-site PR.
3. **Transactions.** Adds `TxOptions`, `begin_write_with` and the per-append `SyncMode`, plus durability tests.
4. **Bindings.** One PR per binding, 7 in all: `allow_overrides` (plus a `DEFAULTS` equivalent), `transaction_chunk_facts`, and the prepared setters where the binding has `PreparedQuery`. Each pins core past PRs 1–3.

After PR 1 merges, #861 is unblocked. It adds `MaxIterations` and `:max-iterations` using the same mechanisms.

## 9. Tests (minimum)

- **Gate.** A default handle accepts `:max-derived-facts` and `:max-results`. `allow_overrides([])` refuses both with API-025 on `execute`, `query`, `prepare` and `WriteTransaction::execute`, for both the literal and the slot form. `QueryMemoryLimit` is refused by default and accepted when listed. Replace semantics: `[QueryMemoryLimit]` alone refuses `:max-derived-facts`.
- **`:query-memory-limit`.** A query that passes at open-level `usize::MAX` fails with QRY-012 under a small keyword value. A query that fails at a small open-level limit passes under a larger keyword value. With `:query-memory-limit 0`, a scan-and-filter query returns its rows and a hash-join or aggregate query fails with QRY-012. Plus the validation codes.
- **Slots.** Different per-execution values change the outcome; type, range and missing-value errors.
- **Setters.** A setter replaces the database value, and a keyword overrides the setter. Setters work even with `allow_overrides([])`.
- **`TxOptions`.** Chunk size changes when a transaction becomes large. Durability as in §6.

## 10. Out of scope

- Lower-only semantics for unlisted settings (deferred).
- BrowserDb options.
- `TxOptions` in bindings (#824).
- `:max-iterations` (#861).
