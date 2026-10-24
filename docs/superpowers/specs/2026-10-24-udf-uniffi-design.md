# UDF registration over UniFFI (#180)

**Milestone:** v3.0.0, branch `v3` (moved from v3.next, 2026-10-24). No change to the `.graph` format or the WAL format. The public Rust API gains two methods and one type; nothing is removed or changed.
**Bindings:** every UniFFI binding: `minigraf-binding-template`, `minigraf-python`, `minigraf-java`, `minigraf-android`, `minigraf-swift`, each on its `v3` branch. `minigraf-c`, `minigraf-node` and `minigraf-wasm` do not use UniFFI and are not covered here.
**Status:** draft 2026-10-24; in review.

## 1. Decisions

1. **A failing user function fails the query.** A UDF can fail: a foreign callback throws, or a Rust closure returns `Err`. The query then returns `QRY-014` (new) at the first failing call. It is never read as `false` or `nil`, because that would give a wrong answer with no error.
2. **The fallible core API is additive.** `try_register_predicate` and `try_register_aggregate` take closures that return `Result<_, UdfError>`. `register_predicate` and `register_aggregate` keep their signatures and behaviour, and wrap their closures as ones that never fail.
3. **UDFs may call back into the database.** A query reads the function registry from a snapshot taken when it opens, and holds no lock while a UDF runs. So a UDF may query, write, register functions or rules, or checkpoint the same database (§5). Calls that are refused today stay refused: any database call made on a thread inside a `WriteTransaction` still returns `INT-001`.
4. **Foreign accumulators keep their own state.** A binding's aggregate is a factory that returns one accumulator object per group (or per window partition). The accumulator's methods take `&self`, so its state lives on the foreign side and no Rust closure or Rust value crosses the FFI.

## 2. Terms

- **UDF**: a registered predicate or aggregate (window functions included) that is not built in.
- **Call**: one call of a UDF closure: a predicate's test, or an aggregate's `init`, `step` or `finalise`.
- **Failing call**: a call that returns `Err(UdfError)`. Over the FFI, it is also a call that throws, whether or not the exception is declared, and a call that returns a value the core cannot use (a `Ref` that is not a UUID).
- **Registry snapshot**: the `Arc<FunctionRegistry>` a query reads all its functions from (§5).

## 3. Core API

```rust
/// Why a user-defined function failed. Its message is in the query's QRY-014 error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdfError { message: String }

impl UdfError {
    pub fn new(message: impl Into<String>) -> Self;
    pub fn message(&self) -> &str;
}
impl std::fmt::Display for UdfError { /* the message */ }
impl std::error::Error for UdfError {}

impl Minigraf {
    pub fn try_register_predicate(
        &self,
        name: &str,
        f: impl Fn(&Value) -> Result<bool, UdfError> + Send + Sync + 'static,
    ) -> Result<(), MinigrafError>;

    pub fn try_register_aggregate<Acc: Any + Send + 'static>(
        &self,
        name: &str,
        init: impl Fn() -> Result<Acc, UdfError> + Send + Sync + 'static,
        step: impl Fn(&mut Acc, &Value) -> Result<(), UdfError> + Send + Sync + 'static,
        finalise: impl Fn(&Acc, usize) -> Result<Value, UdfError> + Send + Sync + 'static,
    ) -> Result<(), MinigrafError>;
}
```

`UdfError` is exported at the crate root next to `MinigrafError`. Registration errors are unchanged: a name that is already taken (built-in or UDF) is `INT-040`, and a poisoned registry lock is `API-008`. The `try_` methods follow the same call semantics as today's methods (`db.rs` doc comments): an aggregate steps the group's non-`nil` values and gets `nil` with no values; a window steps every value and calls `finalise` once per set of peer rows.

Internally, `PredicateDesc::f`, `UdfOps::init`, `UdfStepFn` and `UdfFinaliseFn` (`functions.rs`) return `Result<_, UdfError>`. The existing methods wrap the user's closure in `Ok`. The failure path is the same for both kinds of registration.

A Rust closure that panics is not a failing call. It unwinds as it does today. This spec does not change that.

## 4. Errors

| Code | Name | Category | Message |
|------|------|----------|---------|
| `QRY-014` (new) | User function failed | Query | ``[QRY-014] user function `<name>` failed: <message>`` |

- The error is raised at the first failing call. No later call of any UDF is made for that query. `Minigraf::execute` and `PreparedQuery::execute` return it. A cursor treats it as any other read error (`cursor.rs` `next_batch`): rows gathered before the failure are returned first, the error comes on the next call, and after it the cursor returns `None`.
- Rows already returned stay returned. A cursor's earlier batches are not undone.
- Today, an unknown UDF predicate name gives no row. It stays that way, and `QRY-007` at open already rejects an unknown predicate. Today's other "no value" cases are also unchanged: unbound variable, type mismatch, division by zero.
- On a write, a query inside a `WriteTransaction` that returns `QRY-014` leaves the transaction open, as any query error does today.

### 4.1 Where the error comes from

- **Predicates**: `eval_expr` (`executor.rs`) returns `Result<Value, EvalError>`. Here `EvalError::NoValue` is today's `Err(())` and `EvalError::Udf(anyhow::Error)` carries `QRY-014`. Every caller maps `NoValue` as it maps `Err(())` today and propagates `Udf`. The callers are the engine's `filter::eval` and whatever else the plan's grep finds.
- **Aggregates and windows**: `Udf::of` (init), `Agg::add`/`Running::add` (step), and `Agg::finish`/`Running::value` (finalise) return `Result`. Built-in arms are infallible and return `Ok`. Their callers in `query/engine/aggregate.rs`, `window.rs` and `aggregate.rs::finish_groups` propagate the error.

## 5. Registry snapshot and re-entry

`Inner::functions` becomes `RwLock<Arc<FunctionRegistry>>`, and `FunctionRegistry` derives `Clone`. Its maps hold `Arc`ed closures, so a clone copies only the maps.

This is new behaviour. Today `Stream::pull` (`engine/mod.rs` ~370) read-locks the shared registry for each batch, and rule-query opening reads it as well. So a callback that registers a function would try to take the write lock on a thread that already holds the read lock, and a cursor's later pulls see functions registered after it opened.

- **Query open** (`execute`, `query`, `PreparedQuery::execute`/`query`, rule queries including their stratum evaluation, `WriteTransaction::execute` of a query) takes the read lock only long enough to clone the `Arc`. The query keeps that registry snapshot until it closes. `Ctx::functions` borrows from it. The engine's `pull` and rule evaluation no longer take the lock.
- **Registration** takes the write lock, clones the registry, inserts the new function, and swaps in the new `Arc`. A query that is already open keeps its registry snapshot, so a function registered after a query opens is not visible to that query. It is visible to every query opened afterwards.
- **No lock is taken by the query while a UDF runs.** A plain read holds no database lock across a call: not the registry lock, not `rules`, not `write_lock`, not a `FactStorage` lock. The plan checks every lock on the query path. Any lock held across a call is either released before the call or replaced by a snapshot. The one exception is a query run by `WriteTransaction::execute`: the transaction holds `write_lock` for its whole lifetime, and so across the call. That is why every database call that could wait on `write_lock` must refuse on that thread (below).
- **Every public method that takes `write_lock`, or reads, refuses inside a `WriteTransaction` on the same thread** with `INT-001`. Today `execute`, `query`, `fact_log` and `begin_write` check this. `checkpoint`, `verify` and `rebuild_indexes` do not, and deadlock: that is a bug on its own, #734 (listed in #421), whose fix is a prerequisite of PR 1. `prepare`, `PreparedQuery::execute` and `PreparedQuery::query` take no write lock but also skip the check. They gain it with #734, for consistency with `execute`/`query`.

What a UDF may do from inside a call, on the thread running the query:

| Call from inside a UDF | Outer query is a plain read | Outer query runs in `WriteTransaction::execute` |
|---|---|---|
| `execute` (query), `query`, `prepare`, `PreparedQuery::execute`/`query`, `fact_log` | Runs. It reads its own snapshot. | `INT-001` (existing for `execute`/`query`/`fact_log`; #734 for the prepared ones) |
| `execute` (transact/retract/rule), `begin_write` | Runs. The outer query does not see the write (snapshot, #555). | `INT-001` (existing) |
| `checkpoint`, `verify`, `rebuild_indexes` | Runs. | `INT-001` (#734; deadlocks today) |
| `register_predicate`/`register_aggregate` (both kinds) | Runs. The outer query does not see the new function. | Runs (registration takes no write lock) |
| Calling a UDF in a nested query | Runs, to any depth the caller's stack allows. | n/a |

A checkpoint run from inside a UDF does not wait for the outer query's pinned committed generation. Checkpoints already quarantine pinned pages instead of waiting (#473 spec §4), so this cannot deadlock. The re-entry tests (§7) check it.

## 6. FFI

Every UniFFI binding gets the same Rust code. It is written once in `minigraf-binding-template/src/lib.rs` and copied into each binding, as with the existing items.

```rust
/// Why a user function failed. A callback that throws anything else is mapped to this.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MiniGrafUdfError {
    #[error("{msg}")]
    Failed { msg: String },
}
impl From<uniffi::UnexpectedUniFFICallbackError> for MiniGrafUdfError { /* Failed { msg: reason } */ }

#[uniffi::export]
pub trait MiniGrafPredicate: Send + Sync {
    fn test(&self, value: MiniGrafValue) -> Result<bool, MiniGrafUdfError>;
}

#[uniffi::export]
pub trait MiniGrafAggregate: Send + Sync {
    /// A new, empty accumulator: one per group, or per window partition.
    fn accumulator(&self) -> Result<Arc<dyn MiniGrafAccumulator>, MiniGrafUdfError>;
}

#[uniffi::export]
pub trait MiniGrafAccumulator: Send + Sync {
    fn step(&self, value: MiniGrafValue) -> Result<(), MiniGrafUdfError>;
    /// `count`: values stepped so far.
    fn finish(&self, count: i64) -> Result<MiniGrafValue, MiniGrafUdfError>;
}

#[uniffi::export]
impl MiniGrafDb {
    pub fn register_predicate(&self, name: String, predicate: Arc<dyn MiniGrafPredicate>) -> Result<(), MiniGrafError>;
    pub fn register_aggregate(&self, name: String, aggregate: Arc<dyn MiniGrafAggregate>) -> Result<(), MiniGrafError>;
}
```

- The traits are exported with plain `#[uniffi::export]`: UniFFI 0.31 lets foreign code implement an exported trait taken as `Arc<dyn Trait>`, and `with_foreign` is a deprecated alias for that. The plan checks that each target language (Python, Kotlin, Swift) generates an implementable interface.
- The binding implements these with `try_register_predicate`/`try_register_aggregate`. `Acc` is `Arc<dyn MiniGrafAccumulator>`. `MiniGrafUdfError::Failed { msg }` becomes `UdfError::new(msg)`.
- A `finish` that returns a `Ref` whose value is not a UUID is a failing call: ``ref value "<s>" is not a UUID`` (the same text `to_core` gives).
- `count` is `i64`, following the bindings' rule that counts cross the FFI signed (Kotlin's unsigned types cannot be called from Java).
- Names are passed through unchanged. A predicate is called in Datalog as `[(name ?v)]`, as in Rust.
- `QRY-014` reaches the language as `MiniGrafError::Query` with the coded message, as every Query-category error does.

### 6.1 Lifecycle and threading (documented in every binding README)

- The database keeps a strong reference to each registered object until the database and every cursor and prepared query opened from it are dropped. There is no unregister, as in the core.
- A callback runs on the thread that calls `execute` or `next_batch`. Queries on different threads may call the same object concurrently, so an implementation must be thread-safe. One accumulator is used by one query on one thread at a time.
- The language runtime's rules apply. Python: UniFFI's ctypes calls release the GIL and the callback re-acquires it. The plan checks this with a test that runs two threads.
- Each call crosses the FFI once, so a UDF over many rows costs one foreign call per value. The README says so.
- What a callback may call on the database is §5's table.

## 7. Tests

**Core (`tests/udf_test.rs`)**:
- a `try_` predicate that returns `Err` on one value: `execute` and `query`→`next_batch` both return `QRY-014` with the name and message, and no later call is made (checked with a call counter);
- a `try_` aggregate failing in `init`, in `step`, and in `finalise`, as an aggregate and as a window;
- a failing UDF in a rule body, a `not` body and an `or` branch;
- `PreparedQuery::execute` with a failing predicate;
- the `try_` methods with closures that never fail return the same rows as the existing methods;
- re-entry, one test per row of §5's table, both columns, including a nested query that itself calls a UDF and a checkpoint from inside a UDF on a file database;
- a function registered from inside a UDF is not visible to the outer query and is visible to the next one.

**Each binding**: Rust unit tests in `src/lib.rs` with Rust implementations of the three traits. Language tests in pytest, Kotlin/JVM (java and android) and XCTest (swift), covering at least: one predicate, one aggregate (`:find` and `:over`), a callback that throws (→ `Query` error with `QRY-014`), and a callback that runs a query on the same database. The consumer-test contract (`consumer-test/run.sh`) gains one predicate and one aggregate, so the published package's generated callback code is exercised.

## 8. Documentation

- `docs/ERROR_REFERENCE.md`: `QRY-014`. `src/error.rs` registry and tests.
- `CHANGELOG.md` (v3.0.0 `Added`): the `try_` methods, `UdfError`, `QRY-014`, re-entrant UDFs, and the registry snapshot rule (a cursor sees the functions registered when it opened).
- `ROADMAP.md`, `CLAUDE.md`: #180 moves from v3.next to v3.0.0. Add #180 to tracker #383.
- `db.rs` doc comments on all four registration methods: the failure rule, the snapshot rule, and re-entry.
- Docs site (`../minigraf-docs`): the UDF section of `content/datalog-reference.md` and the bindings pages, wrapped in `<!-- @since v3.0.0 -->`.
- Each binding README: a predicate and an aggregate example in the language, plus §6.1.

## 9. PRs

0. **#734, on `v3` with a `main` backport**: the `INT-001` guards (§5).
1. **Core, on `v3`**: §3–§5, core tests, `QRY-014`, docs. Docs-site PR merged after it.
2. **`minigraf-binding-template`**, on `v3`: §6 and its Rust tests.
3. **`minigraf-python`, `minigraf-java`, `minigraf-android`, `minigraf-swift`**, each on `v3`: bump the git-rev pin to PR 1's merge, add §6, the language tests and the README section. One PR per binding.

No change to `.graph`, WAL or the binary-size budget is expected beyond a few KiB. The size gate checks it.
