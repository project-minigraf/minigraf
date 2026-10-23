//! Streaming query engine (#473): operators that pass batches of id cells
//! over one [`Snapshot`].
//!
//! Built beside the existing executor (spec §9): [`plan`] says which queries
//! run here, and the executor runs the rest. Under `cfg(test)` and the
//! `test-oracle` feature, every query run here is also run by the executor
//! and the answers are compared ([`oracle`]).

mod aggregate;
mod anti;
mod batch;
mod cell;
mod distinct;
mod filter;
mod ids;
mod join;
mod meter;
#[cfg(any(test, feature = "test-oracle"))]
pub(crate) mod oracle;
mod plan;
mod project;
mod recursive;
mod relation;
mod scan;
mod seed;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
mod union;
mod window;

use crate::error::{ErrorCode, err_coded};
use crate::graph::storage::Snapshot;
use crate::graph::types::{EntityId, Value};
use crate::query::datalog::executor::QueryResult;
use crate::query::datalog::functions::FunctionRegistry;
use crate::storage::{CommittedReader, KeySource};
use anyhow::Result;
use batch::Batch;
use ids::IdSpace;
pub(crate) use meter::Meter;
pub(crate) use plan::{Cause, Plan, Refusal, plan, plan_rules};
pub(crate) use recursive::Limits;
use std::sync::{Arc, RwLock};

#[cfg(test)]
thread_local! {
    /// Queries this thread ran on the engine (tests check routing with it).
    pub(crate) static RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Why each query this thread ran on the executor did not run here.
    pub(crate) static FALLBACKS: std::cell::RefCell<Vec<Cause>> =
        const { std::cell::RefCell::new(Vec::new()) };
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
    /// In a rule query, the relations its rules derived (#548).
    derived: Option<&'a relation::Relations>,
}

/// What one operator has done (spec §6.3): rows it read or received, rows
/// and batches it passed on, and the bytes it holds (#549).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counters {
    pub(crate) op: &'static str,
    pub(crate) rows_in: u64,
    pub(crate) rows_out: u64,
    pub(crate) batches: u64,
    pub(crate) bytes_held: u64,
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
    /// Start again from the beginning, reopening a closed operator, with
    /// `seed` as the row of the plan's [`seed::Seed`], if it has one (#546).
    /// Only a correlated body's plan is rewound.
    fn rewind(&mut self, seed: &[cell::Cell]);
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
    /// The bytes the query's holders count (#549); each holder keeps its
    /// own reference, so the stream keeps one for tests only.
    #[cfg(test)]
    meter: Arc<Meter>,
    /// The column of each `:find` element in the root's rows.
    find: Vec<usize>,
    vars: Vec<String>,
    project: Counters,
    /// Keep the DICT lookups until the stream ends: a `Window` turns every
    /// row's ids into values in its first pull, and the rows it returns
    /// later read the same ids again (#547).
    hold_ids: bool,
    /// In a rule query, the relations its rules derived (#548).
    relations: Option<relation::Relations>,
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

/// The key-level reads of `snapshot`'s committed generation, if it has one.
fn keys(snapshot: &Snapshot) -> std::result::Result<Option<&dyn KeySource>, Refusal> {
    match &snapshot.committed {
        Some(reader) => match reader.key_source() {
            Some(k) => Ok(Some(k)),
            // A test reader.
            None => Err(Refusal::Unsupported(Cause::NoKeys)),
        },
        None => Ok(None),
    }
}

/// Open `plan` over `snapshot`, holding at most `memory_limit` bytes.
pub(crate) fn open(
    snapshot: Snapshot,
    plan: Plan,
    functions: Arc<RwLock<FunctionRegistry>>,
    memory_limit: usize,
) -> std::result::Result<Stream, Refusal> {
    let keys = keys(&snapshot)?;
    let meter = Meter::new(memory_limit);
    let ids = IdSpace::new(keys, &meter);
    Ok(stream(snapshot, ids, plan, functions, None, meter))
}

/// Open the rule query `plan` over `snapshot`: derive its relations, with
/// the magic seeds `seeds` as rows, then open its body (#548). An error from
/// the rules fails the query here.
pub(crate) fn open_rules(
    snapshot: Snapshot,
    plan: plan::RulesPlan,
    seeds: &[(EntityId, String, Value)],
    functions: Arc<RwLock<FunctionRegistry>>,
    limits: recursive::Limits,
) -> std::result::Result<Stream, Refusal> {
    let meter = Meter::new(limits.query_memory);
    let (ids, relations) = {
        let keys = keys(&snapshot)?;
        let mut ids = IdSpace::new(keys, &meter);
        let iids = plan
            .evaluated
            .attrs
            .iter()
            .map(|a| ids.iid(keys, a))
            .collect::<Result<Vec<u64>>>()?;
        let mut rels = relation::Relations::new(iids, &meter);
        for (e, a, v) in seeds {
            if let Some(&rel) = plan.evaluated.by_attr.get(a) {
                let row = [
                    cell::Cell::Eid(ids.eid(keys, e)?),
                    cell::Cell::of_value(&mut ids, keys, v)?,
                ];
                rels.add(rel, row)?;
            }
        }
        rels.end_round();
        let functions = functions
            .read()
            .map_err(|_| err_coded!(ErrorCode::Qry008))?;
        let reads = recursive::Reads {
            snapshot: &snapshot,
            keys,
            functions: &functions,
        };
        recursive::evaluate(&plan, &mut rels, &reads, &mut ids, limits, &meter)?;
        (ids, rels)
    };
    Ok(stream(
        snapshot,
        ids,
        plan.body,
        functions,
        Some(relations),
        meter,
    ))
}

/// `refused` as the executor reads it: `None` when the executor runs the
/// query, the error when it fails.
pub(crate) fn supported<T>(refused: std::result::Result<T, Refusal>) -> Result<Option<T>> {
    match refused {
        Ok(t) => Ok(Some(t)),
        Err(Refusal::Fail(e)) => Err(e),
        Err(Refusal::Unsupported(_cause)) => {
            #[cfg(test)]
            FALLBACKS.with(|f| f.borrow_mut().push(_cause));
            Ok(None)
        }
    }
}

/// The stream of `plan` over `snapshot`.
fn stream(
    snapshot: Snapshot,
    ids: IdSpace,
    plan: Plan,
    functions: Arc<RwLock<FunctionRegistry>>,
    relations: Option<relation::Relations>,
    meter: Arc<Meter>,
) -> Stream {
    let Plan {
        source,
        steps,
        find,
        vars,
    } = plan;
    let hold_ids = steps.iter().any(|s| matches!(s, plan::Step::Window { .. }));
    let root = build(source, steps, &meter);
    Stream {
        snapshot,
        ids,
        functions,
        root,
        #[cfg(test)]
        meter,
        find,
        vars,
        project: Counters::new("Project"),
        hold_ids,
        relations,
        closed: false,
    }
}

/// The operator tree of a plan's source and steps, its holders counting on
/// `meter`.
pub(crate) fn build(
    source: plan::Source,
    steps: Vec<plan::Step>,
    meter: &Arc<Meter>,
) -> Box<dyn Operator> {
    let mut root: Box<dyn Operator> = match source {
        plan::Source::Scan(spec) => Box::new(scan::Scan::new(spec)),
        plan::Source::Seed => Box::new(seed::Seed::new()),
    };
    for step in steps {
        root = match step {
            plan::Step::Join(spec) => Box::new(join::Join::new(root, spec, meter)),
            plan::Step::Anti(body) => Box::new(anti::AntiJoin::new(root, body, meter)),
            plan::Step::Union(branches) => Box::new(union::Union::new(root, branches, meter)),
            plan::Step::Distinct(slots) => Box::new(distinct::Distinct::new(root, slots, meter)),
            plan::Step::Aggregate {
                groups,
                aggs,
                zero_row,
            } => Box::new(aggregate::Aggregate::new(
                root, groups, aggs, zero_row, meter,
            )),
            plan::Step::Window { find, named, keep } => {
                Box::new(window::Window::new(root, find, named, keep, meter))
            }
            step => Box::new(filter::Apply::new(root, step)),
        };
    }
    root
}

impl Stream {
    /// A stream of `root`'s batches, projecting slot 0 as `?x`.
    #[cfg(test)]
    pub(crate) fn for_test(snapshot: Snapshot, root: Box<dyn Operator>) -> Self {
        let meter = Meter::unlimited();
        Stream {
            snapshot,
            ids: IdSpace::new(None, &meter),
            functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
            root,
            meter,
            find: vec![0],
            vars: vec!["?x".to_string()],
            project: Counters::new("Project"),
            hold_ids: false,
            relations: None,
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
        let more = matches!(rows, Ok(Some(_)));
        if !self.hold_ids || !more {
            self.ids.end_pull();
        }
        if !more {
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
            derived: self.relations.as_ref(),
        };
        let Some(batch) = self.root.next_batch(&mut ctx)? else {
            return Ok(None);
        };
        self.project.rows_in += batch.len as u64;
        let rows = project::project(&batch, &self.find, &mut ctx)?;
        self.project.out(&batch);
        Ok(Some(rows))
    }

    /// Stop every operator and release what the query holds. Idempotent;
    /// dropping the stream does the same.
    pub(crate) fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            self.root.close();
            // Drop the tree, which releases what every operator holds (a
            // hashed body keeps its table across a close, for its next
            // rewind), and keep its counters.
            let mut counters = Vec::new();
            self.root.counters(&mut counters);
            self.root = Box::new(Closed(counters));
            self.ids.release();
            self.relations = None;
        }
    }

    /// The query's meter.
    #[cfg(test)]
    pub(crate) fn meter(&self) -> &Arc<Meter> {
        &self.meter
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

/// What is left of a closed stream's operators: their counters.
struct Closed(Vec<Counters>);

impl Operator for Closed {
    fn next_batch(&mut self, _: &mut Ctx<'_>) -> Result<Option<Batch>> {
        Ok(None)
    }

    fn rewind(&mut self, _: &[cell::Cell]) {}

    fn close(&mut self) {}

    fn counters(&self, out: &mut Vec<Counters>) {
        out.extend_from_slice(&self.0);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.close();
    }
}
