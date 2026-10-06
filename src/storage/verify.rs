//! Integrity check of a committed v8 file (#373).
//!
//! Every page read already checks its CRC, id and generation (spec §4.2), so
//! physical damage is an error on the read that hits it. This module finds the
//! logical damage those checks cannot see: an index missing entries, indexes
//! that disagree, keys out of order, ids without dictionary entries, pages that
//! are leaked or referenced twice. It also picks the source index for
//! `rebuild_indexes`.
//!
//! The walk keeps no per-fact state. Indexes are compared by an order-independent
//! digest of their entries in canonical (EAVT) key form, so memory is bounded by
//! the page count and the dictionary sizes.

use crate::error::{ErrorCode, err_coded};
use crate::storage::cache::PageCache;
use crate::storage::keys::{self, Index, KeyFact, KeyValue, Reader, SHORT_STRING_MAX, ValueRef};
use crate::storage::meta::MetaPage;
use crate::storage::node::{Internal, decode_leaf};
use crate::storage::page::{PAGE_TYPE_INTERNAL, PAGE_TYPE_LEAF};
use crate::storage::value_pages::read_value;
use crate::storage::{StorageBackend, freelist};
use anyhow::Result;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Deepest tree the walk descends (as `btree::MAX_TREE_DEPTH`).
const MAX_DEPTH: usize = 32;

/// Order-independent digest of a set of index entries: count and two wrapping
/// sums of independent 64-bit hashes of each entry's canonical key. Equal sets
/// give equal digests; different ones collide with probability about 2^-64.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Digest {
    pub count: u64,
    sum1: u64,
    sum2: u64,
}

impl Digest {
    fn add(&mut self, canonical: &[u8]) {
        self.count = self.count.wrapping_add(1);
        self.sum1 = self.sum1.wrapping_add(keys::hash64(canonical));
        self.sum2 = self.sum2.wrapping_add(mix64(canonical));
    }
}

/// A second hash, independent of FNV-1a: a multiply-rotate over each byte,
/// finished with the splitmix64 mixer.
fn mix64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    for &b in bytes {
        h = (h ^ u64::from(b))
            .wrapping_mul(0xBF58_476D_1CE4_E5B9)
            .rotate_left(27);
    }
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^ (h >> 29)
}

/// What a walk of one index tree found.
pub(crate) struct IndexScan {
    pub index: Index,
    /// The walk finished: every page verified and decoded, keys in order.
    pub clean: bool,
    pub digest: Digest,
    /// Digest of the ref facts only (used to check VAET against EAVT).
    pub ref_digest: Digest,
    /// Node pages reached.
    pub nodes: Vec<u64>,
}

/// What a walk of the DICT tree found.
pub(crate) struct DictScan {
    pub clean: bool,
    pub nodes: Vec<u64>,
    /// Pages holding long values (through 0x06 entries).
    pub value_pages: BTreeSet<u64>,
    eids: HashSet<u64>,
    iids: HashSet<u32>,
    txs: HashSet<u64>,
    long_values: HashMap<ValueRef, (u64, Vec<u8>)>,
}

/// Ids, transactions and long values used by index keys.
#[derive(Default)]
struct IdUse {
    eids: HashSet<u64>,
    iids: HashSet<u32>,
    txs: HashSet<u64>,
    long: HashMap<ValueRef, (u64, Vec<u8>)>,
}

/// The outcome of [`verify`].
pub(crate) struct Findings {
    pub facts: u64,
    pub pages: u64,
    pub problems: Vec<anyhow::Error>,
}

fn tree_name(index: Index) -> &'static str {
    match index {
        Index::Eavt => "EAVT",
        Index::Aevt => "AEVT",
        Index::Avet => "AVET",
        Index::Vaet => "VAET",
    }
}

fn root_of(meta: &MetaPage, index: Index) -> u64 {
    match index {
        Index::Eavt => meta.eavt_root,
        Index::Aevt => meta.aevt_root,
        Index::Avet => meta.avet_root,
        Index::Vaet => meta.vaet_root,
    }
}

/// A node to visit: page id, depth, and the key range its parent gives it
/// (lower bound inclusive, upper bound exclusive).
type Visit = (u64, usize, Option<Vec<u8>>, Option<Vec<u8>>);

/// A walk over the committed file at one meta.
pub(crate) struct Walker<'a> {
    backend: &'a dyn StorageBackend,
    cache: &'a PageCache,
    meta: MetaPage,
    /// Every tree node reached so far, across all trees.
    seen: HashSet<u64>,
    pub problems: Vec<anyhow::Error>,
}

impl<'a> Walker<'a> {
    pub fn new(backend: &'a dyn StorageBackend, cache: &'a PageCache, meta: MetaPage) -> Self {
        Walker {
            backend,
            cache,
            meta,
            seen: HashSet::new(),
            problems: Vec::new(),
        }
    }

    /// Walk the tree at `root` in key order, calling `on_entry` for each entry.
    ///
    /// Checks every node once: it verifies (through the cache), decodes, is not
    /// reached twice (across all trees walked by this walker), and its keys lie
    /// within the bounds its parent's separators give and increase strictly.
    /// Returns the nodes reached and whether the walk finished; the first
    /// problem stops it and is recorded.
    fn walk_tree(
        &mut self,
        root: u64,
        name: &str,
        mut on_entry: impl FnMut(&[u8], &[u8]) -> Result<()>,
    ) -> (Vec<u64>, bool) {
        let mut nodes = Vec::new();
        if root == 0 {
            return (nodes, true);
        }
        let malformed = |msg: String| err_coded!(ErrorCode::Stg039, name.to_string(), msg);
        let mut prev: Option<Vec<u8>> = None;
        let mut stack: Vec<Visit> = vec![(root, 0, None, None)];
        let result: Result<()> = (|| {
            while let Some((id, depth, lo, hi)) = stack.pop() {
                if depth > MAX_DEPTH {
                    return Err(malformed(format!("deeper than {MAX_DEPTH} at page {id}")));
                }
                if !self.seen.insert(id) {
                    return Err(malformed(format!("page {id} is reached twice")));
                }
                nodes.push(id);
                let page = self.cache.get_or_load(id, self.backend)?;
                match page.first().copied() {
                    Some(PAGE_TYPE_LEAF) => {
                        let entries = decode_leaf(&page[..])
                            .map_err(|e| malformed(format!("leaf {id} does not decode: {e}")))?;
                        for (k, v) in entries {
                            if lo.as_deref().is_some_and(|l| k.as_slice() < l)
                                || hi.as_deref().is_some_and(|h| k.as_slice() >= h)
                            {
                                return Err(malformed(format!(
                                    "a key in leaf {id} is outside its parent's range"
                                )));
                            }
                            if prev.as_deref().is_some_and(|p| k.as_slice() <= p) {
                                return Err(malformed(format!("keys out of order in leaf {id}")));
                            }
                            on_entry(&k, &v)?;
                            prev = Some(k);
                        }
                    }
                    Some(PAGE_TYPE_INTERNAL) => {
                        let bad_node =
                            |e: anyhow::Error| malformed(format!("node {id} does not decode: {e}"));
                        let node = Internal::new(&page[..]).map_err(bad_node)?;
                        let n = node.count();
                        let mut children = Vec::with_capacity(n.saturating_add(1));
                        for i in 0..=n {
                            let c_lo = match i.checked_sub(1) {
                                Some(j) => Some(node.sep(j).map_err(bad_node)?.to_vec()),
                                None => lo.clone(),
                            };
                            let c_hi = if i < n {
                                Some(node.sep(i).map_err(bad_node)?.to_vec())
                            } else {
                                hi.clone()
                            };
                            if let (Some(l), Some(h)) = (&c_lo, &c_hi)
                                && l >= h
                            {
                                return Err(malformed(format!(
                                    "separators out of order in node {id}"
                                )));
                            }
                            let child = node.child(i).map_err(bad_node)?;
                            children.push((child, depth.saturating_add(1), c_lo, c_hi));
                        }
                        stack.extend(children.into_iter().rev());
                    }
                    _ => return Err(err_coded!(ErrorCode::Stg013, id)),
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => (nodes, true),
            Err(e) => {
                self.problems.push(e);
                (nodes, false)
            }
        }
    }

    /// Walk one index tree: decode every key, digest it, and record the ids it
    /// uses in `ids` (when given).
    fn scan_index_with(&mut self, index: Index, mut ids: Option<&mut IdUse>) -> IndexScan {
        let name = tree_name(index);
        let last_tx = self.meta.last_checkpointed_tx_count;
        let mut digest = Digest::default();
        let mut ref_digest = Digest::default();
        let root = root_of(&self.meta, index);
        let (nodes, clean) = self.walk_tree(root, name, |k, v| {
            if !v.is_empty() {
                return Err(err_coded!(
                    ErrorCode::Stg039,
                    name.to_string(),
                    "an index entry carries a value"
                ));
            }
            let kf = KeyFact::decode(index, k).map_err(|e| {
                err_coded!(ErrorCode::Stg039, name.to_string(), format!("bad key: {e}"))
            })?;
            if kf.tx_count > last_tx {
                return Err(err_coded!(
                    ErrorCode::Stg038,
                    name.to_string(),
                    format!(
                        "transaction {} is newer than the last checkpoint ({last_tx})",
                        kf.tx_count
                    )
                ));
            }
            let canonical = kf
                .key(Index::Eavt)
                .ok_or_else(|| err_coded!(ErrorCode::Int049, "EAVT key missing"))?;
            digest.add(&canonical);
            if matches!(kf.v, KeyValue::Ref(_)) {
                ref_digest.add(&canonical);
            }
            if let Some(ids) = ids.as_deref_mut() {
                ids.eids.insert(kf.e);
                ids.iids.insert(kf.a);
                ids.txs.insert(kf.tx_count);
                match kf.v {
                    KeyValue::Ref(t) => {
                        ids.eids.insert(t);
                    }
                    KeyValue::Keyword(i) => {
                        ids.iids.insert(i);
                    }
                    KeyValue::LongStr { prefix, hash, vref } => {
                        ids.long.insert(vref, (hash, prefix));
                    }
                    _ => {}
                }
            }
            Ok(())
        });
        IndexScan {
            index,
            clean,
            digest,
            ref_digest,
            nodes,
        }
    }

    pub fn scan_index(&mut self, index: Index) -> IndexScan {
        self.scan_index_with(index, None)
    }

    /// Walk the DICT tree, checking each entry's shape, the two inverse maps,
    /// the id counters and every long value.
    pub fn scan_dict(&mut self) -> DictScan {
        let next_eid = self.meta.next_eid;
        let next_iid = self.meta.next_iid;
        let backend = self.backend;
        let cache = self.cache;
        let mut uuid_to_eid: HashMap<Vec<u8>, u64> = HashMap::new();
        let mut eid_to_uuid: HashMap<u64, Vec<u8>> = HashMap::new();
        let mut ident_to_iid: HashMap<Vec<u8>, u32> = HashMap::new();
        let mut iid_to_ident: HashMap<u32, Vec<u8>> = HashMap::new();
        let mut txs = HashSet::new();
        let mut long_values = HashMap::new();
        let mut value_pages = BTreeSet::new();
        let bad = |msg: String| err_coded!(ErrorCode::Stg040, msg);
        let iid_from =
            |n: u64| u32::try_from(n).map_err(|_| bad(format!("ident id {n} too large")));
        let root = self.meta.dict_root;
        let (nodes, clean) = self.walk_tree(root, "DICT", |k, v| {
            let (&tag, rest) = k
                .split_first()
                .ok_or_else(|| bad("empty key".to_string()))?;
            let uint_key = |rest: &[u8]| -> Result<u64> {
                let mut r = Reader::new(rest);
                let n = r.uint()?;
                if !r.is_empty() {
                    return Err(bad("trailing bytes after an id".to_string()));
                }
                Ok(n)
            };
            match tag {
                keys::DICT_UUID_TO_EID => {
                    if rest.len() != 16 {
                        return Err(bad("UUID key is not 16 bytes".to_string()));
                    }
                    uuid_to_eid.insert(rest.to_vec(), keys::read_uint_bytes(v)?);
                }
                keys::DICT_EID_TO_UUID => {
                    let e = uint_key(rest)?;
                    if v.len() != 16 {
                        return Err(bad(format!("entity id {e} maps to {} bytes", v.len())));
                    }
                    if e == 0 || e >= next_eid.max(1) {
                        return Err(bad(format!("entity id {e} not below next_eid {next_eid}")));
                    }
                    eid_to_uuid.insert(e, v.to_vec());
                }
                keys::DICT_IDENT_TO_IID => {
                    ident_to_iid.insert(rest.to_vec(), iid_from(keys::read_uint_bytes(v)?)?);
                }
                keys::DICT_IID_TO_IDENT => {
                    let i = iid_from(uint_key(rest)?)?;
                    if i == 0 || i >= next_iid.max(1) {
                        return Err(bad(format!("ident id {i} not below next_iid {next_iid}")));
                    }
                    if std::str::from_utf8(v).is_err() {
                        return Err(bad(format!("ident id {i} is not UTF-8")));
                    }
                    iid_to_ident.insert(i, v.to_vec());
                }
                keys::DICT_TX => {
                    txs.insert(uint_key(rest)?);
                    keys::read_uint_bytes(v)?;
                }
                keys::DICT_LONG_VALUE => {
                    let vref = keys::dict_long_value_ref(k)?;
                    let hash = rest
                        .get(..8)
                        .and_then(|b| <[u8; 8]>::try_from(b).ok())
                        .map(u64::from_be_bytes)
                        .ok_or_else(|| bad("long-value key too short".to_string()))?;
                    let bytes = read_value(vref, backend, cache)?;
                    if bytes.len() <= SHORT_STRING_MAX
                        || keys::hash64(&bytes) != hash
                        || std::str::from_utf8(&bytes).is_err()
                    {
                        return Err(bad(format!(
                            "long value {}:{} does not match its entry",
                            vref.page, vref.slot
                        )));
                    }
                    let prefix =
                        keys::long_str(std::str::from_utf8(&bytes).unwrap_or_default(), vref);
                    let KeyValue::LongStr { prefix, .. } = prefix else {
                        return Err(err_coded!(ErrorCode::Int049, "long_str shape"));
                    };
                    value_pages.insert(vref.page);
                    long_values.insert(vref, (hash, prefix));
                }
                other => return Err(bad(format!("unknown tag {other:#04x}"))),
            }
            Ok(())
        });
        let mut ok = clean;
        if clean {
            let pairs_match = uuid_to_eid.len() == eid_to_uuid.len()
                && uuid_to_eid
                    .iter()
                    .all(|(u, e)| eid_to_uuid.get(e) == Some(u));
            if !pairs_match {
                self.problems
                    .push(bad("UUID and entity-id maps are not inverse".to_string()));
                ok = false;
            }
            let idents_match = ident_to_iid.len() == iid_to_ident.len()
                && ident_to_iid
                    .iter()
                    .all(|(n, i)| iid_to_ident.get(i) == Some(n));
            if !idents_match {
                self.problems
                    .push(bad("ident and ident-id maps are not inverse".to_string()));
                ok = false;
            }
        }
        DictScan {
            clean: ok,
            nodes,
            value_pages,
            eids: eid_to_uuid.into_keys().collect(),
            iids: iid_to_ident.into_keys().collect(),
            txs,
            long_values,
        }
    }

    /// Read the free list: `(free ids, chain pages)`, recording a problem if it
    /// cannot be read or its count disagrees with the meta.
    pub fn read_free_list(&mut self) -> Option<(Vec<u64>, Vec<u64>)> {
        if self.meta.freelist_head == 0 {
            if self.meta.freelist_count != 0 {
                self.problems.push(err_coded!(
                    ErrorCode::Stg035,
                    format!(
                        "empty chain but freelist_count {}",
                        self.meta.freelist_count
                    )
                ));
                return None;
            }
            return Some((Vec::new(), Vec::new()));
        }
        match freelist::read_chain(
            self.meta.freelist_head,
            self.backend,
            self.cache,
            self.meta.page_count,
        ) {
            Ok((ids, chain)) => {
                let n = u64::try_from(ids.len()).unwrap_or(u64::MAX);
                if n != self.meta.freelist_count {
                    self.problems.push(err_coded!(
                        ErrorCode::Stg035,
                        format!(
                            "chain holds {n} ids, meta says {}",
                            self.meta.freelist_count
                        )
                    ));
                    return None;
                }
                Some((ids, chain))
            }
            Err(e) => {
                self.problems.push(e);
                None
            }
        }
    }
}

/// Check the committed file at `meta` (spec 2026-10-06 §3).
pub(crate) fn verify(backend: &dyn StorageBackend, cache: &PageCache, meta: MetaPage) -> Findings {
    let mut w = Walker::new(backend, cache, meta);
    let mut ids = IdUse::default();
    let scans: Vec<IndexScan> = Index::ALL
        .iter()
        .map(|&i| w.scan_index_with(i, Some(&mut ids)))
        .collect();
    let dict = w.scan_dict();
    let facts = scans.first().map_or(0, |s| s.digest.count);

    // C4, C5: cardinality and content against EAVT.
    if let Some(eavt) = scans.iter().find(|s| s.index == Index::Eavt && s.clean) {
        if eavt.digest.count != meta.fact_count {
            w.problems.push(err_coded!(
                ErrorCode::Stg038,
                "EAVT",
                format!(
                    "{} entries, meta records {}",
                    eavt.digest.count, meta.fact_count
                )
            ));
        }
        for s in scans.iter().filter(|s| s.index != Index::Eavt && s.clean) {
            let expected = if s.index == Index::Vaet {
                eavt.ref_digest
            } else {
                eavt.digest
            };
            if s.digest != expected {
                w.problems.push(err_coded!(
                    ErrorCode::Stg038,
                    tree_name(s.index),
                    format!(
                        "{} entries differ from EAVT's {}",
                        s.digest.count, expected.count
                    )
                ));
            }
        }
    }

    // C7, C9: every id and long value an index uses has its DICT entry.
    if dict.clean {
        let missing = |what: String| err_coded!(ErrorCode::Stg036, what);
        if let Some(e) = ids.eids.iter().find(|e| !dict.eids.contains(e)) {
            w.problems.push(missing(format!("entity id {e}")));
        }
        if let Some(i) = ids.iids.iter().find(|i| !dict.iids.contains(i)) {
            w.problems.push(missing(format!("ident id {i}")));
        }
        if let Some(t) = ids.txs.iter().find(|t| !dict.txs.contains(t)) {
            w.problems.push(missing(format!("transaction {t}")));
        }
        for (vref, (hash, prefix)) in &ids.long {
            match dict.long_values.get(vref) {
                None => {
                    w.problems
                        .push(missing(format!("long value {}:{}", vref.page, vref.slot)));
                    break;
                }
                Some((h, p)) if h != hash || p != prefix => {
                    w.problems.push(err_coded!(
                        ErrorCode::Stg038,
                        "index",
                        format!(
                            "a key's long value {}:{} does not match the stored value",
                            vref.page, vref.slot
                        )
                    ));
                    break;
                }
                Some(_) => {}
            }
        }
    }

    // C10: every page from 2 up is reached once or free, never both.
    let all_clean = scans.iter().all(|s| s.clean) && dict.clean;
    let free = w.read_free_list();
    let mut pages = u64::try_from(w.seen.len()).unwrap_or(u64::MAX);
    if let Some((free_ids, chain)) = free {
        let mut reached: BTreeSet<u64> = w.seen.iter().copied().collect();
        for &p in dict.value_pages.iter().chain(chain.iter()) {
            if !reached.insert(p) {
                w.problems.push(err_coded!(
                    ErrorCode::Stg035,
                    format!("page {p} is used twice")
                ));
            }
        }
        pages = u64::try_from(reached.len()).unwrap_or(u64::MAX);
        let mut free_set = BTreeSet::new();
        for &id in &free_ids {
            if !free_set.insert(id) {
                w.problems.push(err_coded!(
                    ErrorCode::Stg035,
                    format!("page {id} is listed twice")
                ));
            } else if reached.contains(&id) {
                w.problems.push(err_coded!(
                    ErrorCode::Stg035,
                    format!("free page {id} is still in use")
                ));
            }
        }
        if all_clean {
            let leaked = (2..meta.page_count)
                .filter(|p| !reached.contains(p) && !free_set.contains(p))
                .count();
            if leaked > 0 {
                w.problems.push(err_coded!(
                    ErrorCode::Stg035,
                    format!("{leaked} page(s) neither free nor in use")
                ));
            }
            if let Some(p) = reached.iter().find(|&&p| p < 2 || p >= meta.page_count) {
                w.problems.push(err_coded!(
                    ErrorCode::Stg035,
                    format!("page {p} in use outside 2..{}", meta.page_count)
                ));
            }
        }
    }

    Findings {
        facts,
        pages,
        problems: w.problems,
    }
}

/// The index to rebuild from (spec 2026-10-06 §4): the first of two clean
/// indexes that agree, else the only clean one. `None` if neither applies.
pub(crate) fn choose_source(scans: &[IndexScan]) -> Option<Index> {
    let clean: Vec<&IndexScan> = scans.iter().filter(|s| s.clean).collect();
    for (i, a) in clean.iter().enumerate() {
        if clean
            .iter()
            .skip(i.saturating_add(1))
            .any(|b| b.digest == a.digest)
        {
            return Some(a.index);
        }
    }
    match clean.as_slice() {
        [only] => Some(only.index),
        _ => None,
    }
}
