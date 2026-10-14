//! Streaming query engine (#473): operators that pass batches of id cells
//! over one [`Snapshot`].
//!
//! Built beside the existing executor (spec §9): [`plan`] says which queries
//! run here, and the executor runs the rest. Under `cfg(test)` and the
//! `test-oracle` feature, every query run here is also run by the executor
//! and the answers are compared ([`oracle`]).

mod batch;
mod cell;
mod filter;
mod ids;
#[cfg(any(test, feature = "test-oracle"))]
pub(crate) mod oracle;
mod plan;
mod project;
mod scan;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

use crate::graph::storage::Snapshot;
use crate::query::datalog::executor::QueryResult;
use crate::query::datalog::functions::FunctionRegistry;
use crate::storage::KeySource;
use anyhow::Result;
use ids::IdSpace;
pub(crate) use plan::{Plan, plan};

#[cfg(test)]
thread_local! {
    /// Queries this thread ran on the engine (tests check routing with it).
    pub(crate) static RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Most rows in one internal batch (spec §5.1).
pub(crate) const BATCH_ROWS: usize = 1024;

/// What every operator of one query reads and shares.
pub(crate) struct Ctx<'a> {
    snapshot: &'a Snapshot,
    /// Key-level reads of the committed generation; `None` for an in-memory
    /// database.
    keys: Option<&'a dyn KeySource>,
    ids: IdSpace,
    /// Predicates for expression clauses.
    functions: &'a FunctionRegistry,
}

/// Run `plan` over `snapshot`, or `None` when the snapshot's committed
/// reader has no keys (a test reader) and the executor must run it.
pub(crate) fn run(
    snapshot: &Snapshot,
    plan: Plan,
    functions: &FunctionRegistry,
) -> Option<Result<QueryResult>> {
    let keys = match &snapshot.committed {
        Some(reader) => Some(reader.key_source()?),
        None => None,
    };
    #[cfg(test)]
    RUNS.with(|r| r.set(r.get() + 1));
    Some(execute(snapshot, keys, plan, functions))
}

fn execute(
    snapshot: &Snapshot,
    keys: Option<&dyn KeySource>,
    plan: Plan,
    functions: &FunctionRegistry,
) -> Result<QueryResult> {
    let Plan {
        scan,
        steps,
        find,
        vars,
    } = plan;
    let mut results = Vec::new();
    let mut ctx = Ctx {
        snapshot,
        keys,
        ids: IdSpace::new(keys),
        functions,
    };
    let mut scan = scan::Scan::new(scan);
    while let Some(mut batch) = scan.next_batch(&mut ctx)? {
        for step in &steps {
            if batch.len == 0 {
                break;
            }
            batch = filter::apply(step, batch, &mut ctx)?;
        }
        results.extend(project::project(&batch, &find, &mut ctx)?);
    }
    Ok(QueryResult::QueryResults { vars, results })
}
