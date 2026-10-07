//! Every fact version, streamed (#430).
//!
//! [`crate::Minigraf::fact_log`] returns a [`FactLog`] over the records of the
//! database (assertions and retractions, with their transaction and valid-time
//! bounds), filtered by a [`FactFilter`]. It reads the index trees directly,
//! without Datalog, and its memory does not grow with the database.
//!
//! The log reads the database as it was when it opened. While any log is open,
//! checkpoints are deferred: writes still succeed and stay in the WAL, and
//! [`crate::Minigraf::checkpoint`] fails with `API-013`. Close the log, or drop
//! it, to release the database.

use crate::error::{ErrorCode, MinigrafError, err_coded};
use crate::graph::storage::{FactStorage, LogPin};
use crate::graph::types::{EntityId, Fact, Value};
use crate::storage::keys::{self, Index};
use crate::storage::{CommittedReader, LogSource};
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::ops::{Bound, RangeBounds};
use std::sync::Arc;

/// Default for [`FactFilter::window`].
pub const DEFAULT_WINDOW: usize = 1 << 18;

/// One record of the fact log: an assertion or a retraction.
#[derive(Debug, Clone, PartialEq)]
pub struct FactRecord {
    /// The entity the fact is about.
    pub entity: EntityId,
    /// The attribute ident, e.g. `":person/name"`.
    pub attribute: String,
    /// The value.
    pub value: Value,
    /// The transaction counter that `:as-of N` compares against.
    pub tx_count: u64,
    /// The transaction's wall-clock time (Unix milliseconds).
    pub tx_id: u64,
    /// Start of the valid-time window (Unix milliseconds).
    pub valid_from: i64,
    /// End of the valid-time window (Unix milliseconds); `i64::MAX` is forever.
    pub valid_to: i64,
    /// `false` for a retraction record.
    pub asserted: bool,
}

impl From<Fact> for FactRecord {
    fn from(f: Fact) -> Self {
        FactRecord {
            entity: f.entity,
            attribute: f.attribute,
            value: f.value,
            tx_count: f.tx_count,
            tx_id: f.tx_id,
            valid_from: f.valid_from,
            valid_to: f.valid_to,
            asserted: f.asserted,
        }
    }
}

/// The order of a [`FactLog`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FactOrder {
    /// Ascending `tx_count`. The order within one transaction is unspecified
    /// (it is deterministic for a given file state). The storage has no index
    /// in transaction order, so each [`FactFilter::window`] records cost one
    /// scan of the selected index keys.
    #[default]
    Tx,
    /// The order the storage reads most cheaply, in one scan. Records still
    /// carry their `tx_count`.
    Storage,
}

/// Which records a [`FactLog`] returns, and in what order.
///
/// Every filter is optional; the ones that are set must all match. They are
/// checked on index keys, so records they exclude are never decoded.
///
/// ```
/// # use minigraf::{FactFilter, FactOrder};
/// let filter = FactFilter::new()
///     .attribute_prefix(":ingestion/")
///     .tx_range(10..=20)
///     .order(FactOrder::Storage);
/// ```
#[derive(Debug, Clone, Default)]
pub struct FactFilter {
    attributes: Option<HashSet<String>>,
    prefixes: Vec<String>,
    entities: Option<HashSet<EntityId>>,
    /// Inclusive `tx_count` bounds.
    tx: Option<(u64, u64)>,
    order: FactOrder,
    window: Option<usize>,
}

impl FactFilter {
    /// A filter that matches every record, in [`FactOrder::Tx`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Keep records whose attribute is one of `idents`. Combined with
    /// [`attribute_prefix`](Self::attribute_prefix), a record matching either
    /// is kept. Calling it again adds to the set.
    pub fn attributes<I, S>(mut self, idents: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.attributes
            .get_or_insert_with(HashSet::new)
            .extend(idents.into_iter().map(Into::into));
        self
    }

    /// Keep records whose attribute starts with `prefix`, such as a namespace
    /// (`":ingestion/"`). Calling it again adds another prefix.
    pub fn attribute_prefix(mut self, prefix: &str) -> Self {
        self.prefixes.push(prefix.to_string());
        self
    }

    /// Keep records of the entities `ids`. Calling it again adds to the set.
    pub fn entities<I: IntoIterator<Item = EntityId>>(mut self, ids: I) -> Self {
        self.entities.get_or_insert_with(HashSet::new).extend(ids);
        self
    }

    /// Keep records whose `tx_count` is in `range`.
    pub fn tx_range(mut self, range: impl RangeBounds<u64>) -> Self {
        let lo = match range.start_bound() {
            Bound::Included(&n) => Some(n),
            Bound::Excluded(&n) => n.checked_add(1),
            Bound::Unbounded => Some(0),
        };
        let hi = match range.end_bound() {
            Bound::Included(&n) => Some(n),
            Bound::Excluded(&n) => n.checked_sub(1),
            Bound::Unbounded => Some(u64::MAX),
        };
        // An empty range is stored as `lo > hi`.
        self.tx = Some(match (lo, hi) {
            (Some(lo), Some(hi)) => (lo, hi),
            _ => (1, 0),
        });
        self
    }

    /// The order of the log. Defaults to [`FactOrder::Tx`].
    pub fn order(mut self, order: FactOrder) -> Self {
        self.order = order;
        self
    }

    /// In [`FactOrder::Tx`], the most records held in memory at once
    /// (default [`DEFAULT_WINDOW`]); each window costs one scan of the
    /// selected index keys. `0` is treated as 1. Not used by
    /// [`FactOrder::Storage`].
    pub fn window(mut self, records: usize) -> Self {
        self.window = Some(records);
        self
    }

    fn has_attribute_filter(&self) -> bool {
        self.attributes.is_some() || !self.prefixes.is_empty()
    }

    fn attribute_ok(&self, attribute: &str) -> bool {
        !self.has_attribute_filter()
            || self
                .attributes
                .as_ref()
                .is_some_and(|set| set.contains(attribute))
            || self
                .prefixes
                .iter()
                .any(|p| attribute.starts_with(p.as_str()))
    }

    fn tx_ok(&self, tx: u64) -> bool {
        self.tx.is_none_or(|(lo, hi)| lo <= tx && tx <= hi)
    }

    fn matches(&self, f: &Fact) -> bool {
        self.tx_ok(f.tx_count)
            && self
                .entities
                .as_ref()
                .is_none_or(|set| set.contains(&f.entity))
            && self.attribute_ok(&f.attribute)
    }
}

/// A forward-only stream of fact records, from [`crate::Minigraf::fact_log`].
///
/// Pull records with [`FactLog::next_batch`] or iterate over them. The log
/// holds the database as it was when it opened and defers checkpoints until
/// it is closed or dropped, or has returned its last record.
///
/// ```
/// # use minigraf::{FactFilter, Minigraf};
/// let db = Minigraf::in_memory().unwrap();
/// db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
/// db.execute(r#"(retract [[:alice :person/name "Alice"]])"#).unwrap();
///
/// let mut log = db.fact_log(&FactFilter::new()).unwrap();
/// while let Some(batch) = log.next_batch(1000).unwrap() {
///     for rec in batch {
///         println!("{} {} {}", rec.tx_count, rec.attribute, rec.asserted);
///     }
/// }
/// ```
pub struct FactLog {
    filter: FactFilter,
    committed: Option<Committed>,
    storage: FactStorage,
    /// Next pending fact to read, and the end of the snapshot.
    pending_pos: usize,
    pending_end: usize,
    done: bool,
    pin: Option<LogPin>,
}

impl std::fmt::Debug for FactLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FactLog")
            .field("order", &self.filter.order)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl FactLog {
    /// Open a log over the current state of `storage`. The caller holds the
    /// write lock, so no transaction is half applied and no checkpoint runs.
    pub(crate) fn open(storage: &FactStorage, filter: &FactFilter) -> anyhow::Result<Self> {
        let pin = storage.pin_log();
        let (reader, pending_end) = storage.log_snapshot()?;
        let committed = match reader {
            Some(reader) => Committed::new(reader, filter)?,
            None => None,
        };
        Ok(FactLog {
            filter: filter.clone(),
            committed,
            storage: storage.clone(),
            pending_pos: 0,
            pending_end,
            done: false,
            pin: Some(pin),
        })
    }

    /// The next batch of at most `max` records, or `None` once every record
    /// has been returned. A batch is never empty; `max == 0` is treated as 1.
    ///
    /// # Errors
    ///
    /// A page read error or damaged index entry. The log is finished after an
    /// error.
    pub fn next_batch(&mut self, max: usize) -> Result<Option<Vec<FactRecord>>, MinigrafError> {
        if self.done {
            return Ok(None);
        }
        match self.fill(max.max(1)) {
            Ok(batch) if !batch.is_empty() => Ok(Some(batch)),
            Ok(_) => {
                self.finish();
                Ok(None)
            }
            Err(e) => {
                self.finish();
                Err(e.into())
            }
        }
    }

    /// Stop reading and release the database. Equivalent to dropping the log.
    pub fn close(self) {}

    fn finish(&mut self) {
        self.done = true;
        self.committed = None;
        self.pin = None;
    }

    fn fill(&mut self, n: usize) -> anyhow::Result<Vec<FactRecord>> {
        let mut out = Vec::new();
        if let Some(c) = &mut self.committed {
            c.fill(n, &self.filter, &mut out)?;
            if c.finished() {
                self.committed = None;
            }
        }
        while out.len() < n && self.pending_pos < self.pending_end {
            let end = self
                .pending_pos
                .saturating_add(n - out.len())
                .min(self.pending_end);
            let facts = self.storage.pending_range(self.pending_pos, end)?;
            if facts.is_empty() {
                // The snapshot ends here; nothing past it can appear.
                self.pending_pos = self.pending_end;
                break;
            }
            self.pending_pos = self.pending_pos.saturating_add(facts.len());
            out.extend(
                facts
                    .into_iter()
                    .filter(|f| self.filter.matches(f))
                    .map(FactRecord::from),
            );
        }
        Ok(out)
    }
}

impl Iterator for FactLog {
    type Item = Result<FactRecord, MinigrafError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_batch(1) {
            Ok(Some(batch)) => batch.into_iter().next().map(Ok),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

/// The committed part of a log: key ranges of one index in the generation the
/// log opened on.
struct Committed {
    reader: Arc<dyn CommittedReader>,
    index: Index,
    /// Key prefixes to walk, in ascending order.
    ranges: Vec<Vec<u8>>,
    /// Whether attribute ids still need checking against the filter.
    check_attribute: bool,
    /// The filter's answer per attribute id.
    attribute_ok: HashMap<u64, bool>,
    state: State,
}

enum State {
    Storage {
        range: usize,
        after: Option<Vec<u8>>,
    },
    Tx {
        window: usize,
        /// The greatest `(tx_count, key)` already taken into a window.
        after: Option<(u64, Vec<u8>)>,
        /// Keys of the current window, ascending, not yet returned.
        buffer: VecDeque<Vec<u8>>,
        /// The last window held every remaining key.
        exhausted: bool,
    },
}

impl Committed {
    /// `None` when the filter leaves nothing to read in the committed part.
    fn new(reader: Arc<dyn CommittedReader>, filter: &FactFilter) -> anyhow::Result<Option<Self>> {
        let src = log_source(&reader)?;
        if let Some((lo, hi)) = filter.tx
            && (lo > hi || lo > src.last_tx())
        {
            return Ok(None);
        }
        let (index, ranges, check_attribute) = if let Some(entities) = &filter.entities {
            let mut eids = Vec::new();
            for e in entities {
                if let Some(eid) = src.eid_of(e)? {
                    eids.push(eid);
                }
            }
            eids.sort_unstable();
            let ranges = eids.into_iter().map(keys::entity_prefix).collect();
            (Index::Eavt, ranges, filter.has_attribute_filter())
        } else if let (Some(attributes), true) = (&filter.attributes, filter.prefixes.is_empty()) {
            let mut iids = Vec::new();
            for a in attributes {
                if let Some(iid) = src.iid_of(a)? {
                    iids.push(iid);
                }
            }
            iids.sort_unstable();
            let ranges = iids.into_iter().map(keys::attribute_prefix).collect();
            (Index::Aevt, ranges, false)
        } else {
            (Index::Eavt, vec![Vec::new()], filter.has_attribute_filter())
        };
        if ranges.is_empty() {
            return Ok(None);
        }
        let state = match filter.order {
            FactOrder::Storage => State::Storage {
                range: 0,
                after: None,
            },
            FactOrder::Tx => State::Tx {
                window: filter.window.unwrap_or(DEFAULT_WINDOW).max(1),
                after: None,
                buffer: VecDeque::new(),
                exhausted: false,
            },
        };
        Ok(Some(Committed {
            reader,
            index,
            ranges,
            check_attribute,
            attribute_ok: HashMap::new(),
            state,
        }))
    }

    fn finished(&self) -> bool {
        match &self.state {
            State::Storage { range, .. } => *range >= self.ranges.len(),
            State::Tx {
                buffer, exhausted, ..
            } => buffer.is_empty() && *exhausted,
        }
    }

    /// Append up to `n - out.len()` records to `out`.
    fn fill(
        &mut self,
        n: usize,
        filter: &FactFilter,
        out: &mut Vec<FactRecord>,
    ) -> anyhow::Result<()> {
        let src = log_source(&self.reader)?;
        let index = self.index;
        let check_attribute = self.check_attribute;
        let memo = &mut self.attribute_ok;
        // The key's tx_count if the filter keeps it.
        let mut keep = |key: &[u8]| -> anyhow::Result<Option<u64>> {
            let (_, a, tx) = keys::entity_attribute_tx(index, key)?;
            if !filter.tx_ok(tx) {
                return Ok(None);
            }
            if check_attribute {
                let ok = match memo.get(&a) {
                    Some(ok) => *ok,
                    None => {
                        let iid = u32::try_from(a)
                            .map_err(|_| err_coded!(ErrorCode::Int049, "iid out of range"))?;
                        let ok = filter.attribute_ok(&src.ident_of(iid)?);
                        memo.insert(a, ok);
                        ok
                    }
                };
                if !ok {
                    return Ok(None);
                }
            }
            Ok(Some(tx))
        };

        match &mut self.state {
            State::Storage { range, after } => {
                while out.len() < n && *range < self.ranges.len() {
                    let want = n - out.len();
                    let prefix = self.ranges.get(*range).map_or(&[][..], Vec::as_slice);
                    let mut taken: Vec<Vec<u8>> = Vec::new();
                    src.walk(index, prefix, after.as_deref(), &mut |k| {
                        if keep(k)?.is_some() {
                            taken.push(k.to_vec());
                        }
                        Ok(taken.len() < want)
                    })?;
                    if taken.len() < want {
                        *range += 1;
                        *after = None;
                    } else {
                        *after = taken.last().cloned();
                    }
                    out.extend(src.decode(index, &taken)?.into_iter().map(FactRecord::from));
                }
            }
            State::Tx {
                window,
                after,
                buffer,
                exhausted,
            } => {
                while out.len() < n {
                    if buffer.is_empty() {
                        if *exhausted {
                            break;
                        }
                        // One pass: the `window` smallest (tx, key) after `after`.
                        let mut heap: BinaryHeap<(u64, Vec<u8>)> = BinaryHeap::new();
                        for prefix in &self.ranges {
                            src.walk(index, prefix, None, &mut |k| {
                                let Some(tx) = keep(k)? else {
                                    return Ok(true);
                                };
                                if let Some((atx, akey)) = &*after
                                    && (tx, k) <= (*atx, akey.as_slice())
                                {
                                    return Ok(true);
                                }
                                if heap.len() >= *window {
                                    match heap.peek() {
                                        Some((mtx, mkey)) if (tx, k) < (*mtx, mkey.as_slice()) => {
                                            heap.pop();
                                        }
                                        _ => return Ok(true),
                                    }
                                }
                                heap.push((tx, k.to_vec()));
                                Ok(true)
                            })?;
                        }
                        *exhausted = heap.len() < *window;
                        let sorted = heap.into_sorted_vec();
                        if let Some(last) = sorted.last() {
                            *after = Some(last.clone());
                        }
                        buffer.extend(sorted.into_iter().map(|(_, k)| k));
                        if buffer.is_empty() {
                            *exhausted = true;
                            break;
                        }
                    }
                    let take = (n - out.len()).min(buffer.len());
                    let keys: Vec<Vec<u8>> = buffer.drain(..take).collect();
                    out.extend(src.decode(index, &keys)?.into_iter().map(FactRecord::from));
                }
            }
        }
        Ok(())
    }
}

fn log_source(reader: &Arc<dyn CommittedReader>) -> anyhow::Result<&dyn LogSource> {
    reader
        .log_source()
        .ok_or_else(|| err_coded!(ErrorCode::Int049, "committed reader has no index keys"))
}
