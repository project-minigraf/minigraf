//! Covering reads of committed facts (spec §7).
//!
//! Every index entry holds a whole fact, so a read touches only index leaves,
//! the DICT pages that translate its ids, and a value page for a long string.

use crate::graph::types::{EntityId, Fact};
use crate::storage::btree::{LeafCursor, MutexStorageBackend, prefix_scan};
use crate::storage::cache::PageCache;
use crate::storage::dict::{DictReader, SharedDictCache};
use crate::storage::keys::{self, Index, KeyFact};
use crate::storage::meta::MetaPage;
use crate::storage::{CommittedReader, Scan, StorageBackend};
use anyhow::Result;
use std::sync::{Arc, Mutex};

/// [`CommittedReader`] over one committed meta's trees.
pub struct OnDiskReader<B: StorageBackend + 'static> {
    backend: MutexStorageBackend<B>,
    cache: Arc<PageCache>,
    eavt_root: u64,
    aevt_root: u64,
    dict_root: u64,
    shared: SharedDictCache,
}

impl<B: StorageBackend + 'static> OnDiskReader<B> {
    pub fn new(backend: Arc<Mutex<B>>, cache: Arc<PageCache>, meta: &MetaPage) -> Self {
        OnDiskReader {
            backend: MutexStorageBackend(backend),
            cache,
            eavt_root: meta.eavt_root,
            aevt_root: meta.aevt_root,
            dict_root: meta.dict_root,
            shared: SharedDictCache::default(),
        }
    }

    fn dict(&self) -> DictReader<'_> {
        DictReader::new(self.dict_root, &self.backend, &self.cache).with_shared(&self.shared)
    }

    /// Facts of every `index` entry starting with `prefix`.
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
    /// Facts of `index` entries starting with `prefix` that survive net-assert
    /// at `as_of`, decided on the key bytes (#379, spec §5.3).
    ///
    /// The history of one triple is contiguous and newest first, so its first
    /// transaction at or before `as_of` decides it (#435): a transaction that
    /// holds a retraction hides the triple, otherwise its assertions are kept
    /// (one, unless written before API-011 existed). Everything older in the
    /// triple is hidden. With `as_of`, entries newer than `as_of` are passed
    /// over. Entries that are passed over are skipped one by one, and
    /// after [`SEEK_AFTER`] of them in one triple the cursor seeks past the run,
    /// so a long history costs O(log) page visits while a short one costs no
    /// seek. Only kept entries are decoded and translated through the dictionary.
    fn live_scan(
        &self,
        index: Index,
        root: u64,
        prefix: &[u8],
        as_of: Option<u64>,
        dict: &mut DictReader<'_>,
    ) -> Result<Vec<Fact>> {
        let mut out = Vec::new();
        if root == 0 {
            return Ok(out);
        }
        let mut cursor = LeafCursor::new(root, Some(prefix), &self.backend, &self.cache)?;
        let mut triple: Vec<u8> = Vec::new();
        let mut group = TxGroup::default();
        // Entries of the current triple are hidden from here on.
        let mut dead = false;
        let mut skipped = 0usize;
        loop {
            let step = match cursor.next_ref()? {
                Some((k, _)) if k.starts_with(prefix) => {
                    if dead && k.starts_with(&triple) {
                        Step::Pass(Some(0xFF))
                    } else {
                        let n = keys::triple_len(index, k)?;
                        let (key_triple, rest) = k.split_at(n);
                        let mut r = keys::Reader::new(rest);
                        let tx = r.tx_desc()?;
                        let new_triple = key_triple != triple.as_slice();
                        if new_triple {
                            group.flush(index, dict, &mut out)?;
                            triple.clear();
                            triple.extend_from_slice(key_triple);
                            dead = false;
                            skipped = 0;
                        }
                        if as_of.is_some_and(|limit| tx > limit) {
                            Step::Pass(None)
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
                    group.flush(index, dict, &mut out)?;
                    return Ok(out);
                }
            };
            match step {
                Step::Pass(past) => {
                    skipped = skipped.saturating_add(1);
                    if skipped >= SEEK_AFTER {
                        skipped = 0;
                        let mut target = triple.clone();
                        match (past, as_of) {
                            (Some(b), _) => target.push(b),
                            (None, Some(limit)) => keys::put_tx_desc(&mut target, limit),
                            (None, None) => {}
                        }
                        cursor.seek(&target)?;
                    }
                }
                Step::Entry { tx, key } => {
                    if group.tx.is_some_and(|decided| decided != tx) {
                        // The triple's newest transaction decided it; this
                        // older entry and the rest of the triple are hidden.
                        group.flush(index, dict, &mut out)?;
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
    /// Pass over it; a seek past the run appends this byte to the triple
    /// (`0xFF`: past the whole triple), or `tx↓(as_of)` when `None`.
    Pass(Option<u8>),
    /// An entry at or before `as_of`; `key` is kept for assertions only.
    Entry { tx: u64, key: Option<Vec<u8>> },
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
    /// Emit the group's assertions unless it holds a retraction, and reset it.
    fn flush(
        &mut self,
        index: Index,
        dict: &mut DictReader<'_>,
        out: &mut Vec<Fact>,
    ) -> Result<()> {
        let group = std::mem::take(self);
        if !group.retracted {
            for key in group.assertions {
                out.push(dict.fact(&KeyFact::decode(index, &key)?)?);
            }
        }
        Ok(())
    }
}

impl<B: StorageBackend + 'static> CommittedReader for OnDiskReader<B> {
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
