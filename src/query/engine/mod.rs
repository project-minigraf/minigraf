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
mod join;
#[cfg(any(test, feature = "test-oracle"))]
pub(crate) mod oracle;
mod plan;
mod project;
mod scan;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;

use crate::error::{ErrorCode, err_coded};
use crate::graph::storage::Snapshot;
use crate::graph::types::Value;
use crate::query::datalog::executor::QueryResult;
use crate::query::datalog::functions::FunctionRegistry;
use crate::storage::{CommittedReader, KeySource};
use anyhow::Result;
use batch::Batch;
use ids::IdSpace;
pub(crate) use plan::{Plan, plan};
use std::sync::{Arc, RwLock};

#[cfg(test)]
thread_local! {
    /// Queries this thread ran on the engine (tests check routing with it).
    pub(crate) static RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Most rows in one internal batch (spec §5.1).
pub(crate) const BATCH_ROWS: usize = 1024;

/// What every operator of one query reads and shares, for one pull.
pub(crate) struct Ctx<'a> {
    snapshot: &'a Snapshot,
    /// Key-level reads of the committed generation; `None` for an in-memory
    /// database.
    keys: Option<&'a dyn KeySource>,
    ids: &'a mut IdSpace,
    /// Predicates for expression clauses.
    functions: &'a FunctionRegistry,
}

/// What one operator has done (spec §6.3): rows it read or received, rows
/// and batches it passed on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counters {
    pub(crate) op: &'static str,
    pub(crate) rows_in: u64,
    pub(crate) rows_out: u64,
    pub(crate) batches: u64,
}

impl Counters {
    fn new(op: &'static str) -> Self {
        Counters {
            op,
            ..Counters::default()
        }
    }

    /// Count `batch` as passed on.
    fn out(&mut self, batch: &Batch) {
        self.rows_out += batch.len as u64;
        self.batches += 1;
    }
}

/// One operator of a query (spec §6). `next_batch` never returns an empty
/// batch; after `close`, it returns `None` and reads nothing.
pub(crate) trait Operator: Send {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>>;
    /// Stop: release what the operator holds and close its children.
    fn close(&mut self);
    /// This operator's counters, then its children's.
    fn counters(&self, out: &mut Vec<Counters>);
}

/// A query on the engine: its operator tree over one snapshot, pulled one
/// batch at a time. Owned, so a cursor can hold it.
pub(crate) struct Stream {
    snapshot: Snapshot,
    ids: IdSpace,
    /// Read-locked for each pull. Predicates are never replaced once
    /// registered (`INT-040`), so the answer does not change between pulls.
    functions: Arc<RwLock<FunctionRegistry>>,
    root: Box<dyn Operator>,
    /// The slot of each `:find` variable.
    find: Vec<usize>,
    vars: Vec<String>,
    project: Counters,
    closed: bool,
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Stream")
            .field("vars", &self.vars)
            .field("closed", &self.closed)
            .field("counters", &self.counters())
            .finish_non_exhaustive()
    }
}

/// Open `plan` over `snapshot`, or `None` when the snapshot's committed
/// reader has no keys (a test reader) and the executor must run it.
pub(crate) fn open(
    snapshot: Snapshot,
    plan: Plan,
    functions: Arc<RwLock<FunctionRegistry>>,
) -> Option<Stream> {
    let keys = match &snapshot.committed {
        Some(reader) => Some(reader.key_source()?),
        None => None,
    };
    let ids = IdSpace::new(keys);
    let Plan {
        scan,
        steps,
        find,
        vars,
    } = plan;
    let mut root: Box<dyn Operator> = Box::new(scan::Scan::new(scan));
    for step in steps {
        root = match step {
            plan::Step::Join(spec) => Box::new(join::Join::new(root, spec)),
            step => Box::new(filter::Apply::new(root, step)),
        };
    }
    Some(Stream {
        snapshot,
        ids,
        functions,
        root,
        find,
        vars,
        project: Counters::new("Project"),
        closed: false,
    })
}

impl Stream {
    /// A stream of `root`'s batches, projecting slot 0 as `?x`.
    #[cfg(test)]
    pub(crate) fn for_test(snapshot: Snapshot, root: Box<dyn Operator>) -> Self {
        Stream {
            snapshot,
            ids: IdSpace::new(None),
            functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
            root,
            find: vec![0],
            vars: vec!["?x".to_string()],
            project: Counters::new("Project"),
            closed: false,
        }
    }

    /// The rows of the next batch, as values, or `None` once every row has
    /// been returned. Never an empty batch. After an error, or `close`, it
    /// returns `None`.
    pub(crate) fn next_rows(&mut self) -> Result<Option<Vec<Vec<Value>>>> {
        if self.closed {
            return Ok(None);
        }
        let rows = self.pull();
        self.ids.end_pull();
        if !matches!(rows, Ok(Some(_))) {
            self.close();
        }
        rows
    }

    fn pull(&mut self) -> Result<Option<Vec<Vec<Value>>>> {
        let functions = self
            .functions
            .read()
            .map_err(|_| err_coded!(ErrorCode::Qry008))?;
        let mut ctx = Ctx {
            snapshot: &self.snapshot,
            keys: self
                .snapshot
                .committed
                .as_deref()
                .and_then(CommittedReader::key_source),
            ids: &mut self.ids,
            functions: &functions,
        };
        let Some(batch) = self.root.next_batch(&mut ctx)? else {
            return Ok(None);
        };
        self.project.rows_in += batch.len as u64;
        let rows = project::project(&batch, &self.find, &mut ctx)?;
        self.project.out(&batch);
        Ok(Some(rows))
    }

    /// Stop every operator. Idempotent; dropping the stream does the same.
    pub(crate) fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.root.close();
        }
    }

    pub(crate) fn vars(&self) -> &[String] {
        &self.vars
    }

    /// The counters of every operator, from `Project` down to the scan.
    pub(crate) fn counters(&self) -> Vec<Counters> {
        let mut out = vec![self.project];
        self.root.counters(&mut out);
        out
    }

    /// Every row, as one result.
    pub(crate) fn collect(mut self) -> Result<QueryResult> {
        let mut results = Vec::new();
        while let Some(rows) = self.next_rows()? {
            results.extend(rows);
        }
        Ok(QueryResult::QueryResults {
            vars: std::mem::take(&mut self.vars),
            results,
        })
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.close();
    }
}
