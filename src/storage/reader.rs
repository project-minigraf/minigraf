//! Covering reads of committed facts (spec §7).
//!
//! Every index entry holds a whole fact, so a read touches only index leaves,
//! the DICT pages that translate its ids, and a value page for a long string.

use crate::graph::types::{EntityId, Fact};
use crate::storage::btree::{LeafCursor, MutexStorageBackend, prefix_scan};
use crate::storage::cache::PageCache;
use crate::storage::dict::DictReader;
use crate::storage::keys::{self, Index, KeyFact};
use crate::storage::meta::MetaPage;
use crate::storage::{CommittedReader, StorageBackend};
use anyhow::Result;
use std::sync::{Arc, Mutex};

/// [`CommittedReader`] over one committed meta's trees.
pub struct OnDiskReader<B: StorageBackend + 'static> {
    backend: MutexStorageBackend<B>,
    cache: Arc<PageCache>,
    eavt_root: u64,
    aevt_root: u64,
    dict_root: u64,
}

impl<B: StorageBackend + 'static> OnDiskReader<B> {
    pub fn new(backend: Arc<Mutex<B>>, cache: Arc<PageCache>, meta: &MetaPage) -> Self {
        OnDiskReader {
            backend: MutexStorageBackend(backend),
            cache,
            eavt_root: meta.eavt_root,
            aevt_root: meta.aevt_root,
            dict_root: meta.dict_root,
        }
    }

    fn dict(&self) -> DictReader<'_> {
        DictReader::new(self.dict_root, &self.backend, &self.cache)
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

impl<B: StorageBackend + 'static> CommittedReader for OnDiskReader<B> {
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
        let prefix = keys::entity_attribute_prefix(e, a);
        self.scan(Index::Eavt, self.eavt_root, &prefix, &mut dict)
    }

    fn facts_for_attribute(&self, attribute: &str) -> Result<Vec<Fact>> {
        let mut dict = self.dict();
        let Some(a) = dict.iid_of(attribute)? else {
            return Ok(Vec::new());
        };
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
