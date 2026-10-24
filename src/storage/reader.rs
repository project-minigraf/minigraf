//! Covering reads of committed facts (spec §7).
//!
//! Every index entry holds a whole fact, so a read touches only index leaves,
//! the DICT pages that translate its ids, and a value page for a long string.

use crate::error::{ErrorCode, bail_coded};
use crate::graph::types::{EntityId, Fact};
#[cfg(test)]
use crate::storage::Scan;
#[cfg(test)]
use crate::storage::btree::prefix_scan;
use crate::storage::btree::{LeafCursor, MutexStorageBackend};
use crate::storage::cache::PageCache;
use crate::storage::dict::{DictReader, SharedDictCache};
use crate::storage::keys::{self, Index, KeyFact, ValueRef};
use crate::storage::meta::MetaPage;
use crate::storage::{CommittedReader, KeySource, LogSource, StorageBackend, TxBound, TxOrder};
use anyhow::Result;
use std::sync::{Arc, Mutex, OnceLock};

/// [`CommittedReader`] over one committed meta's trees.
pub struct OnDiskReader<B: StorageBackend + 'static> {
    backend: MutexStorageBackend<B>,
    cache: Arc<PageCache>,
    eavt_root: u64,
    aevt_root: u64,
    avet_root: u64,
    vaet_root: u64,
    dict_root: u64,
    next_eid: u64,
    next_iid: u32,
    last_tx: u64,
    shared: SharedDictCache,
    /// How the generation's `tx_id`s run, read on first use (#551).
    tx_order: OnceLock<TxOrder>,
}

impl<B: StorageBackend + 'static> OnDiskReader<B> {
    pub fn new(backend: Arc<Mutex<B>>, cache: Arc<PageCache>, meta: &MetaPage) -> Self {
        OnDiskReader {
            backend: MutexStorageBackend(backend),
            cache,
            eavt_root: meta.eavt_root,
            aevt_root: meta.aevt_root,
            avet_root: meta.avet_root,
            vaet_root: meta.vaet_root,
            dict_root: meta.dict_root,
            next_eid: meta.next_eid,
            next_iid: meta.next_iid,
            last_tx: meta.last_checkpointed_tx_count,
            shared: SharedDictCache::default(),
            tx_order: OnceLock::new(),
        }
    }

    fn dict(&self) -> DictReader<'_> {
        DictReader::new(self.dict_root, &self.backend, &self.cache).with_shared(&self.shared)
    }

    /// Facts of every `index` entry starting with `prefix`.
    #[cfg(test)]
    fn scan(
        &self,
        index: Index,
        root: u64,
        prefix: &[u8],
        dict: &mut DictReader<'_>,
    ) -> Result<Vec<Fact>> {
        if root == 0 {
            return Ok(Vec::new());
        }
        prefix_scan(root, prefix, &self.backend, &self.cache)?
            .iter()
            .map(|(k, _)| dict.fact(&KeyFact::decode(index, k)?))
            .collect()
    }
}

impl<B: StorageBackend + 'static> OnDiskReader<B> {
    /// Visit the DICT's transactions `(tx_count, tx_id)` in `tx_count` order,
    /// from `from` on (all when `None`), until `visit` returns false.
    fn dict_txs(
        &self,
        from: Option<u64>,
        visit: &mut dyn FnMut(u64, u64) -> Result<bool>,
    ) -> Result<()> {
        if self.dict_root == 0 {
            return Ok(());
        }
        let prefix = [keys::DICT_TX];
        let mut cursor =
            LeafCursor::new(self.dict_root, Some(&prefix), &self.backend, &self.cache)?;
        if let Some(m) = from {
            cursor.seek(&keys::dict_tx_key(m))?;
        }
        while let Some((k, v)) = cursor.next_ref()? {
            let Some(rest) = k.strip_prefix(&prefix) else {
                break;
            };
            let tx_count = keys::Reader::new(rest).uint()?;
            if !visit(tx_count, keys::read_uint_bytes(v)?)? {
                break;
            }
        }
        Ok(())
    }

    /// Facts of `index` entries starting with `prefix` that survive net-assert
    /// at `as_of`, decided on the key bytes (#379, spec §5.3). Only kept
    /// entries are decoded and translated through the dictionary.
    #[cfg(test)]
    fn live_scan(
        &self,
        index: Index,
        root: u64,
        prefix: &[u8],
        as_of: Option<u64>,
        dict: &mut DictReader<'_>,
    ) -> Result<Vec<Fact>> {
        let mut out = Vec::new();
        let bound = TxBound::of(as_of);
        self.live_walk(index, root, prefix, None, &bound, usize::MAX, &mut |k| {
            out.push(dict.fact(&KeyFact::decode(index, k)?)?);
            Ok(())
        })?;
        Ok(out)
    }

    fn root(&self, index: Index) -> u64 {
        match index {
            Index::Eavt => self.eavt_root,
            Index::Aevt => self.aevt_root,
            Index::Avet => self.avet_root,
            Index::Vaet => self.vaet_root,
        }
    }

    /// Visit the keys of `index` entries starting with `prefix` that survive
    /// net-assert among the versions `bound` keeps, in key order, starting
    /// after the triple `resume`.
    ///
    /// The history of one triple is contiguous and newest first, so its first
    /// transaction `bound` keeps decides it (#435): a transaction that
    /// holds a retraction hides the triple, otherwise its assertions are kept
    /// (one, unless written before API-011 existed). Everything older in the
    /// triple is hidden. Entries of transactions `bound` does not keep are
    /// passed over. Entries that are passed over are skipped one by one, and
    /// after [`SEEK_AFTER`] of them in one triple the cursor seeks past the run,
    /// so a long history costs O(log) page visits while a short one costs no
    /// seek.
    ///
    /// Stops at the first triple boundary once `max` keys were visited and
    /// returns the last whole triple, to resume after; `None` when the range
    /// is done.
    #[allow(clippy::too_many_arguments)]
    fn live_walk(
        &self,
        index: Index,
        root: u64,
        prefix: &[u8],
        resume: Option<&[u8]>,
        bound: &TxBound,
        max: usize,
        visit: &mut dyn FnMut(&[u8]) -> Result<()>,
    ) -> Result<Option<Vec<u8>>> {
        if root == 0 {
            return Ok(None);
        }
        let mut cursor = LeafCursor::new(root, Some(prefix), &self.backend, &self.cache)?;
        if let Some(after) = resume {
            // Past every entry of the triple: `tx↓` starts below 0xFF, and the
            // next triple differs from this one within its length.
            let mut target = after.to_vec();
            target.push(0xFF);
            cursor.seek(&target)?;
        }
        let mut triple: Vec<u8> = Vec::new();
        let mut group = TxGroup::default();
        let mut visited = 0usize;
        // Entries of the current triple are hidden from here on.
        let mut dead = false;
        let mut skipped = 0usize;
        loop {
            let step = match cursor.next_ref()? {
                Some((k, _)) if k.starts_with(prefix) => {
                    if dead && k.starts_with(&triple) {
                        Step::Pass(Seek::Triple)
                    } else {
                        let n = keys::triple_len(index, k)?;
                        let (key_triple, rest) = k.split_at(n);
                        let mut r = keys::Reader::new(rest);
                        let tx = r.tx_desc()?;
                        let new_triple = key_triple != triple.as_slice();
                        if new_triple {
                            visited += group.flush(visit)?;
                            if visited >= max && !triple.is_empty() {
                                return Ok(Some(triple));
                            }
                            triple.clear();
                            triple.extend_from_slice(key_triple);
                            dead = false;
                            skipped = 0;
                        }
                        if !bound.keeps_key(tx) {
                            Step::Pass(match bound.kept_below(tx) {
                                Some(older) => Seek::Tx(older),
                                None => Seek::Triple,
                            })
                        } else {
                            let retraction = k.last() == Some(&0);
                            Step::Entry {
                                tx,
                                key: (!retraction).then(|| k.clone()),
                            }
                        }
                    }
                }
                _ => {
                    group.flush(visit)?;
                    return Ok(None);
                }
            };
            match step {
                Step::Pass(past) => {
                    skipped = skipped.saturating_add(1);
                    if skipped >= SEEK_AFTER {
                        skipped = 0;
                        let mut target = triple.clone();
                        match past {
                            Seek::Triple => target.push(0xFF),
                            Seek::Tx(older) => keys::put_tx_desc(&mut target, older),
                        }
                        cursor.seek(&target)?;
                    }
                }
                Step::Entry { tx, key } => {
                    if group.tx.is_some_and(|decided| decided != tx) {
                        // The triple's newest transaction decided it; this
                        // older entry and the rest of the triple are hidden.
                        visited += group.flush(visit)?;
                        dead = true;
                        skipped = 1;
                        continue;
                    }
                    group.tx = Some(tx);
                    match key {
                        Some(key) => group.assertions.push(key),
                        None => group.retracted = true,
                    }
                }
            }
        }
    }
}

/// Entries of one triple passed over before the walk seeks past them.
const SEEK_AFTER: usize = 8;

/// What the walk does with one entry.
enum Step {
    /// Pass over it; a seek past the run goes to `Seek`.
    Pass(Seek),
    /// An entry of a transaction the bound keeps; `key` is kept for
    /// assertions only.
    Entry { tx: u64, key: Option<Vec<u8>> },
}

/// Where a seek past passed-over entries of one triple goes.
enum Seek {
    /// Past the whole triple.
    Triple,
    /// To the triple's entries of this transaction and older.
    Tx(u64),
}

/// The entries of one triple with one `tx_count`.
#[derive(Default)]
struct TxGroup {
    tx: Option<u64>,
    retracted: bool,
    /// Key of each assertion (distinct windows, since keys are unique).
    assertions: Vec<Vec<u8>>,
}

impl TxGroup {
    /// Visit the group's assertions unless it holds a retraction, and reset
    /// it. Returns the number visited.
    fn flush(&mut self, visit: &mut dyn FnMut(&[u8]) -> Result<()>) -> Result<usize> {
        let group = std::mem::take(self);
        if group.retracted {
            return Ok(0);
        }
        for key in &group.assertions {
            visit(key)?;
        }
        Ok(group.assertions.len())
    }
}

impl<B: StorageBackend + 'static> KeySource for OnDiskReader<B> {
    fn next_ids(&self) -> (u64, u32) {
        (self.next_eid, self.next_iid)
    }

    fn eid_of(&self, entity: &EntityId) -> Result<Option<u64>> {
        self.dict().eid_of(entity)
    }

    fn iid_of(&self, ident: &str) -> Result<Option<u32>> {
        self.dict().iid_of(ident)
    }

    fn uuid_of(&self, eid: u64) -> Result<EntityId> {
        self.dict().uuid_of(eid)
    }

    fn ident_of(&self, iid: u32) -> Result<String> {
        self.dict().name_of(iid)
    }

    fn long_value(&self, vref: ValueRef) -> Result<String> {
        self.dict().long_value(vref)
    }

    fn long_value_ref(&self, s: &str) -> Result<Option<ValueRef>> {
        self.dict().find_long_value(s)
    }

    fn long_bytes(&self, vref: ValueRef) -> Result<Vec<u8>> {
        self.dict().long_bytes(vref)
    }

    fn long_bytes_ref(&self, bytes: &[u8]) -> Result<Option<ValueRef>> {
        self.dict().find_long_bytes(bytes)
    }

    fn tx_id_of(&self, tx_count: u64) -> Result<u64> {
        self.dict().tx_id_of(tx_count)
    }

    fn tx_order(&self) -> Result<TxOrder> {
        if let Some(order) = self.tx_order.get() {
            return Ok(*order);
        }
        let mut order = TxOrder {
            monotonic: true,
            last: None,
        };
        self.dict_txs(None, &mut |tx_count, tx_id| {
            if order.last.is_some_and(|(_, last)| tx_id < last) {
                order.monotonic = false;
            }
            order.last = Some((tx_count, tx_id));
            Ok(true)
        })?;
        Ok(*self.tx_order.get_or_init(|| order))
    }

    fn last_tx_at(&self, t: u64) -> Result<u64> {
        // The first transaction at or after `m`, if its `tx_id` is at most
        // `t`: true up to the newest transaction with `tx_id <= t`, false
        // after it, since `tx_id`s rise with `tx_count` here.
        let at_or_before = |m: u64| -> Result<bool> {
            let mut kept = false;
            self.dict_txs(Some(m), &mut |_, tx_id| {
                kept = tx_id <= t;
                Ok(false)
            })?;
            Ok(kept)
        };
        // The least `m` in `1..=newest + 1` whose first transaction at or
        // after it is not kept; that transaction is `m - 1`.
        let newest = self.tx_order()?.last.map_or(0, |(tx_count, _)| tx_count);
        let (mut lo, mut hi) = (1u64, newest.saturating_add(1));
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if at_or_before(mid)? {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        Ok(lo - 1)
    }

    fn tx_runs(&self, t: u64, out: &mut Vec<(u64, u64)>) -> Result<()> {
        let mut open: Option<(u64, u64)> = None;
        self.dict_txs(None, &mut |tx_count, tx_id| {
            match (tx_id <= t, open.as_mut()) {
                (true, Some(run)) => run.1 = tx_count,
                (true, None) => open = Some((tx_count, tx_count)),
                (false, _) => out.extend(open.take()),
            }
            Ok(true)
        })?;
        out.extend(open);
        Ok(())
    }

    fn live_keys(
        &self,
        index: Index,
        prefix: &[u8],
        resume: Option<&[u8]>,
        bound: &TxBound,
        max: usize,
        out: &mut Vec<KeyFact>,
    ) -> Result<Option<Vec<u8>>> {
        let root = self.root(index);
        self.live_walk(index, root, prefix, resume, bound, max, &mut |k| {
            out.push(KeyFact::decode(index, k)?);
            Ok(())
        })
    }
}

impl<B: StorageBackend + 'static> LogSource for OnDiskReader<B> {
    fn last_tx(&self) -> u64 {
        self.last_tx
    }

    fn walk(
        &self,
        index: Index,
        prefix: &[u8],
        after: Option<&[u8]>,
        visit: &mut dyn FnMut(&[u8]) -> Result<bool>,
    ) -> Result<()> {
        let root = match index {
            Index::Eavt => self.eavt_root,
            Index::Aevt => self.aevt_root,
            Index::Avet | Index::Vaet => {
                bail_coded!(ErrorCode::Int049, "fact log walks EAVT or AEVT only")
            }
        };
        if root == 0 {
            return Ok(());
        }
        let mut cursor = LeafCursor::new(root, Some(prefix), &self.backend, &self.cache)?;
        if let Some(after) = after {
            // Keys are unique, so the first key after `after` is the first one
            // at or above `after ‖ 0x00`.
            let mut target = after.to_vec();
            target.push(0);
            cursor.seek(&target)?;
        }
        while let Some((k, _)) = cursor.next_ref()? {
            if !k.starts_with(prefix) || !visit(k)? {
                break;
            }
        }
        Ok(())
    }

    fn decode(&self, index: Index, keys: &[Vec<u8>]) -> Result<Vec<Fact>> {
        let mut dict = self.dict();
        keys.iter()
            .map(|k| dict.fact(&KeyFact::decode(index, k)?))
            .collect()
    }
}

impl<B: StorageBackend + 'static> CommittedReader for OnDiskReader<B> {
    fn log_source(&self) -> Option<&dyn LogSource> {
        Some(self)
    }

    fn key_source(&self) -> Option<&dyn KeySource> {
        Some(self)
    }

    #[cfg(test)]
    fn live_facts(&self, scan: Scan<'_>, as_of: Option<u64>) -> Result<Vec<Fact>> {
        let mut dict = self.dict();
        let (index, root, prefix) = match scan {
            Scan::All => (Index::Eavt, self.eavt_root, Vec::new()),
            Scan::Entity(entity) => {
                let Some(e) = dict.eid_of(entity)? else {
                    return Ok(Vec::new());
                };
                dict.seed_entity(e, *entity);
                (Index::Eavt, self.eavt_root, keys::entity_prefix(e))
            }
            Scan::EntityAttribute(entity, attribute) => {
                let (Some(e), Some(a)) = (dict.eid_of(entity)?, dict.iid_of(attribute)?) else {
                    return Ok(Vec::new());
                };
                dict.seed_entity(e, *entity);
                dict.seed_ident(a, attribute);
                (
                    Index::Eavt,
                    self.eavt_root,
                    keys::entity_attribute_prefix(e, a),
                )
            }
            Scan::Attribute(attribute) => {
                let Some(a) = dict.iid_of(attribute)? else {
                    return Ok(Vec::new());
                };
                dict.seed_ident(a, attribute);
                (Index::Aevt, self.aevt_root, keys::attribute_prefix(a))
            }
        };
        self.live_scan(index, root, &prefix, as_of, &mut dict)
    }

    #[cfg(test)]
    fn all_facts(&self) -> Result<Vec<Fact>> {
        if self.eavt_root == 0 {
            return Ok(Vec::new());
        }
        let mut dict = self.dict();
        let mut cursor = LeafCursor::new(self.eavt_root, None, &self.backend, &self.cache)?;
        let mut facts = Vec::new();
        while let Some((k, _)) = cursor.next_ref()? {
            facts.push(dict.fact(&KeyFact::decode(Index::Eavt, k)?)?);
        }
        Ok(facts)
    }

    #[cfg(test)]
    fn facts_for_entity(&self, entity: &EntityId) -> Result<Vec<Fact>> {
        let mut dict = self.dict();
        let Some(e) = dict.eid_of(entity)? else {
            return Ok(Vec::new());
        };
        dict.seed_entity(e, *entity);
        self.scan(
            Index::Eavt,
            self.eavt_root,
            &keys::entity_prefix(e),
            &mut dict,
        )
    }

    #[cfg(test)]
    fn facts_for_entity_attribute(&self, entity: &EntityId, attribute: &str) -> Result<Vec<Fact>> {
        let mut dict = self.dict();
        let (Some(e), Some(a)) = (dict.eid_of(entity)?, dict.iid_of(attribute)?) else {
            return Ok(Vec::new());
        };
        dict.seed_entity(e, *entity);
        dict.seed_ident(a, attribute);
        let prefix = keys::entity_attribute_prefix(e, a);
        self.scan(Index::Eavt, self.eavt_root, &prefix, &mut dict)
    }

    #[cfg(test)]
    fn facts_for_attribute(&self, attribute: &str) -> Result<Vec<Fact>> {
        let mut dict = self.dict();
        let Some(a) = dict.iid_of(attribute)? else {
            return Ok(Vec::new());
        };
        dict.seed_ident(a, attribute);
        self.scan(
            Index::Aevt,
            self.aevt_root,
            &keys::attribute_prefix(a),
            &mut dict,
        )
    }
}

/// Every fact in `root`'s tree of kind `index`, in key order (tests, verify).
#[cfg(test)]
pub fn all_entries_as_facts(
    index: Index,
    root: u64,
    dict_root: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> Result<Vec<Fact>> {
    let mut dict = DictReader::new(dict_root, backend, cache);
    if root == 0 {
        return Ok(Vec::new());
    }
    prefix_scan(root, &[], backend, cache)?
        .iter()
        .map(|(k, _)| dict.fact(&KeyFact::decode(index, k)?))
        .collect()
}
