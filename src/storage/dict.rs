//! The dictionary tree (DICT, spec §5) and the encoding of facts into keys.
//!
//! [`DictReader`] looks ids up in a committed DICT tree and translates index
//! entries back into [`Fact`]s, memoising every lookup for its own lifetime, so
//! memory is bounded by one read's results.
//!
//! [`Encoder`] turns pending facts into index keys at checkpoint time, in
//! pending (WAL) order: it assigns new eids and iids from the meta's counters,
//! records each transaction's timestamp, and deduplicates long values against
//! the committed ones and against each other. Ids are never persisted before the
//! commit that writes their DICT entries, so assigning them here is crash-safe.
//! Encoding runs in two phases so the committed reader and the page writes
//! never borrow the backend at once: [`Encoder::stage`] reads, then
//! [`Encoder::finish`] writes the new long values and builds the keys.

use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::{Fact, Value};
use crate::storage::StorageBackend;
use crate::storage::btree;
use crate::storage::cache::PageCache;
use crate::storage::keys::{self, Index, KeyFact, KeyValue, SHORT_STRING_MAX, ValueRef};
use crate::storage::node::Entry;
use crate::storage::page::PageAllocator;
use crate::storage::value_pages::{ValueWriter, read_value};
use anyhow::Result;
use std::collections::HashMap;
use uuid::Uuid;

/// Entries a [`SharedDictCache`] map holds before it is cleared.
const SHARED_CACHE_LIMIT: usize = 65_536;

/// Ident and transaction lookups shared by every read of one committed meta.
/// They never change once committed, and there are few of them compared with
/// entities, so they are cached across reads; each map is cleared when it
/// reaches [`SHARED_CACHE_LIMIT`] entries.
#[derive(Default)]
pub struct SharedDictCache {
    iids: std::sync::RwLock<HashMap<String, u32>>,
    names: std::sync::RwLock<HashMap<u32, String>>,
    tx_ids: std::sync::RwLock<HashMap<u64, u64>>,
}

fn shared_get<K: std::hash::Hash + Eq, V: Clone>(
    map: &std::sync::RwLock<HashMap<K, V>>,
    key: &K,
) -> Option<V> {
    map.read().ok()?.get(key).cloned()
}

fn shared_put<K: std::hash::Hash + Eq, V>(
    map: &std::sync::RwLock<HashMap<K, V>>,
    key: K,
    value: V,
) {
    if let Ok(mut m) = map.write() {
        if m.len() >= SHARED_CACHE_LIMIT {
            m.clear();
        }
        m.insert(key, value);
    }
}

/// Lookups in a committed DICT tree, memoised.
pub struct DictReader<'a> {
    root: u64,
    backend: &'a dyn StorageBackend,
    cache: &'a PageCache,
    shared: Option<&'a SharedDictCache>,
    uuids: HashMap<u64, Uuid>,
    names: HashMap<u32, String>,
    tx_ids: HashMap<u64, u64>,
    long_values: HashMap<ValueRef, String>,
}

impl<'a> DictReader<'a> {
    /// A reader over the DICT tree at `root` (0: the empty dictionary).
    pub fn new(root: u64, backend: &'a dyn StorageBackend, cache: &'a PageCache) -> Self {
        DictReader {
            root,
            backend,
            cache,
            shared: None,
            uuids: HashMap::new(),
            names: HashMap::new(),
            tx_ids: HashMap::new(),
            long_values: HashMap::new(),
        }
    }

    /// Consult and fill `shared` for idents and transaction timestamps.
    pub fn with_shared(mut self, shared: &'a SharedDictCache) -> Self {
        self.shared = Some(shared);
        self
    }

    /// Record a known `eid → uuid` pair (the caller looked the eid up by it).
    pub fn seed_entity(&mut self, eid: u64, uuid: Uuid) {
        self.uuids.insert(eid, uuid);
    }

    /// Record a known `iid → ident` pair.
    pub fn seed_ident(&mut self, iid: u32, ident: &str) {
        self.names.insert(iid, ident.to_string());
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        if self.root == 0 {
            return Ok(None);
        }
        btree::get(self.root, key, self.backend, self.cache)
    }

    /// The eid of `uuid`, if it has one.
    pub fn eid_of(&self, uuid: &Uuid) -> Result<Option<u64>> {
        self.get(&keys::dict_uuid_key(uuid))?
            .map(|v| keys::read_uint_bytes(&v))
            .transpose()
    }

    /// The iid of `ident`, if it has one.
    pub fn iid_of(&self, ident: &str) -> Result<Option<u32>> {
        if let Some(i) = self
            .shared
            .and_then(|c| shared_get(&c.iids, &ident.to_string()))
        {
            return Ok(Some(i));
        }
        let found = self
            .get(&keys::dict_ident_key(ident))?
            .map(|v| {
                u32::try_from(keys::read_uint_bytes(&v)?)
                    .map_err(|_| err_coded!(ErrorCode::Int049, "iid out of range"))
            })
            .transpose()?;
        if let (Some(i), Some(c)) = (found, self.shared) {
            shared_put(&c.iids, ident.to_string(), i);
        }
        Ok(found)
    }

    /// The timestamp recorded for `tx_count`, if any.
    pub fn tx_id_lookup(&mut self, tx_count: u64) -> Result<Option<u64>> {
        if let Some(t) = self.tx_ids.get(&tx_count) {
            return Ok(Some(*t));
        }
        if let Some(t) = self.shared.and_then(|c| shared_get(&c.tx_ids, &tx_count)) {
            self.tx_ids.insert(tx_count, t);
            return Ok(Some(t));
        }
        let found = self
            .get(&keys::dict_tx_key(tx_count))?
            .map(|v| keys::read_uint_bytes(&v))
            .transpose()?;
        if let Some(t) = found {
            self.tx_ids.insert(tx_count, t);
            if let Some(c) = self.shared {
                shared_put(&c.tx_ids, tx_count, t);
            }
        }
        Ok(found)
    }

    pub fn uuid_of(&mut self, eid: u64) -> Result<Uuid> {
        if let Some(u) = self.uuids.get(&eid) {
            return Ok(*u);
        }
        let v = self
            .get(&keys::dict_eid_key(eid))?
            .ok_or_else(|| err_coded!(ErrorCode::Stg036, format!("entity id {eid}")))?;
        let u = Uuid::from_slice(&v)
            .map_err(|_| err_coded!(ErrorCode::Stg036, format!("entity id {eid}: bad UUID")))?;
        self.uuids.insert(eid, u);
        Ok(u)
    }

    pub fn name_of(&mut self, iid: u32) -> Result<String> {
        if let Some(n) = self.names.get(&iid) {
            return Ok(n.clone());
        }
        if let Some(n) = self.shared.and_then(|c| shared_get(&c.names, &iid)) {
            self.names.insert(iid, n.clone());
            return Ok(n);
        }
        let v = self
            .get(&keys::dict_iid_key(iid))?
            .ok_or_else(|| err_coded!(ErrorCode::Stg036, format!("ident id {iid}")))?;
        let n = String::from_utf8(v)
            .map_err(|_| err_coded!(ErrorCode::Stg036, format!("ident id {iid}: not UTF-8")))?;
        self.names.insert(iid, n.clone());
        if let Some(c) = self.shared {
            shared_put(&c.names, iid, n.clone());
        }
        Ok(n)
    }

    pub fn tx_id_of(&mut self, tx_count: u64) -> Result<u64> {
        self.tx_id_lookup(tx_count)?
            .ok_or_else(|| err_coded!(ErrorCode::Stg036, format!("transaction {tx_count}")))
    }

    pub fn long_value(&mut self, vref: ValueRef) -> Result<String> {
        if let Some(s) = self.long_values.get(&vref) {
            return Ok(s.clone());
        }
        let bytes = read_value(vref, self.backend, self.cache)?;
        let s = String::from_utf8(bytes).map_err(|_| {
            err_coded!(
                ErrorCode::Stg036,
                format!("long value {}:{} not UTF-8", vref.page, vref.slot)
            )
        })?;
        self.long_values.insert(vref, s.clone());
        Ok(s)
    }

    /// The committed long value equal to `s`, if any (§6.3 dedup).
    fn find_long_value(&mut self, s: &str) -> Result<Option<ValueRef>> {
        if self.root == 0 {
            return Ok(None);
        }
        let prefix = keys::dict_long_value_prefix(keys::hash64(s.as_bytes()));
        for (k, _) in btree::prefix_scan(self.root, &prefix, self.backend, self.cache)? {
            let vref = keys::dict_long_value_ref(&k)?;
            if self.long_value(vref)? == s {
                return Ok(Some(vref));
            }
        }
        Ok(None)
    }

    /// Translate an index entry back into a fact.
    pub fn fact(&mut self, kf: &KeyFact) -> Result<Fact> {
        let value = match &kf.v {
            KeyValue::Null => Value::Null,
            KeyValue::Bool(b) => Value::Boolean(*b),
            KeyValue::Int(n) => Value::Integer(*n),
            KeyValue::Float(f) => Value::Float(*f),
            KeyValue::Str(s) => Value::String(s.clone()),
            KeyValue::LongStr { vref, .. } => Value::String(self.long_value(*vref)?),
            KeyValue::Keyword(iid) => Value::Keyword(self.name_of(*iid)?),
            KeyValue::Ref(eid) => Value::Ref(self.uuid_of(*eid)?),
        };
        Ok(Fact {
            entity: self.uuid_of(kf.e)?,
            attribute: self.name_of(kf.a)?,
            value,
            tx_id: self.tx_id_of(kf.tx_count)?,
            tx_count: kf.tx_count,
            valid_from: kf.vf,
            valid_to: kf.vt,
            asserted: kf.asserted,
        })
    }
}

/// A value after staging: ready, or a long value still to be written.
enum StagedValue {
    Ready(KeyValue),
    NewLong(usize),
}

/// A fact with its ids assigned.
struct Staged {
    e: u64,
    a: u32,
    v: StagedValue,
    tx_count: u64,
    vf: i64,
    vt: i64,
    asserted: bool,
}

/// What one checkpoint adds: sorted, distinct entries per tree.
pub struct Encoded {
    /// EAVT, AEVT, AVET, VAET entries, in [`Index::ALL`] order.
    pub index: [Vec<Entry>; 4],
    pub dict: Vec<Entry>,
    pub next_eid: u64,
    pub next_iid: u32,
}

/// Assigns ids and builds keys for one checkpoint's facts.
pub struct Encoder {
    next_eid: u64,
    next_iid: u32,
    eids: HashMap<Uuid, u64>,
    iids: HashMap<String, u32>,
    txs: HashMap<u64, u64>,
    new_long: HashMap<String, usize>,
    new_values: Vec<String>,
    dict: Vec<Entry>,
    staged: Vec<Staged>,
}

impl Encoder {
    /// An encoder whose new ids start at the meta's counters (0 means 1).
    pub fn new(next_eid: u64, next_iid: u32) -> Self {
        Encoder {
            next_eid: next_eid.max(1),
            next_iid: next_iid.max(1),
            eids: HashMap::new(),
            iids: HashMap::new(),
            txs: HashMap::new(),
            new_long: HashMap::new(),
            new_values: Vec::new(),
            dict: Vec::new(),
            staged: Vec::new(),
        }
    }

    fn eid(&mut self, uuid: Uuid, dict: &DictReader<'_>) -> Result<u64> {
        if let Some(e) = self.eids.get(&uuid) {
            return Ok(*e);
        }
        let e = match dict.eid_of(&uuid)? {
            Some(e) => e,
            None => {
                let e = self.next_eid;
                self.next_eid = e
                    .checked_add(1)
                    .ok_or_else(|| err_coded!(ErrorCode::Int048, "entity ids exhausted"))?;
                self.dict
                    .push((keys::dict_uuid_key(&uuid), keys::uint_bytes(e)));
                self.dict
                    .push((keys::dict_eid_key(e), uuid.as_bytes().to_vec()));
                e
            }
        };
        self.eids.insert(uuid, e);
        Ok(e)
    }

    fn iid(&mut self, ident: &str, dict: &DictReader<'_>) -> Result<u32> {
        if let Some(i) = self.iids.get(ident) {
            return Ok(*i);
        }
        if ident.len() > keys::MAX_IDENT_BYTES {
            bail_coded!(ErrorCode::Wal003, ident.len(), keys::MAX_IDENT_BYTES);
        }
        let i = match dict.iid_of(ident)? {
            Some(i) => i,
            None => {
                let i = self.next_iid;
                self.next_iid = i
                    .checked_add(1)
                    .ok_or_else(|| err_coded!(ErrorCode::Int048, "ident ids exhausted"))?;
                self.dict
                    .push((keys::dict_ident_key(ident), keys::uint_bytes(u64::from(i))));
                self.dict
                    .push((keys::dict_iid_key(i), ident.as_bytes().to_vec()));
                i
            }
        };
        self.iids.insert(ident.to_string(), i);
        Ok(i)
    }

    fn tx(&mut self, tx_count: u64, tx_id: u64, dict: &mut DictReader<'_>) -> Result<()> {
        let known = match self.txs.get(&tx_count) {
            Some(t) => Some(*t),
            None => dict.tx_id_lookup(tx_count)?,
        };
        match known {
            Some(t) if t != tx_id => bail_coded!(ErrorCode::Stg037, tx_count, t, tx_id),
            Some(_) => {}
            None => self
                .dict
                .push((keys::dict_tx_key(tx_count), keys::uint_bytes(tx_id))),
        }
        self.txs.insert(tx_count, tx_id);
        Ok(())
    }

    fn value(&mut self, v: &Value, dict: &mut DictReader<'_>) -> Result<StagedValue> {
        Ok(StagedValue::Ready(match v {
            Value::Null => KeyValue::Null,
            Value::Boolean(b) => KeyValue::Bool(*b),
            Value::Integer(n) => KeyValue::Int(*n),
            Value::Float(f) => KeyValue::Float(*f),
            Value::String(s) if s.len() <= SHORT_STRING_MAX => KeyValue::Str(s.clone()),
            Value::String(s) => {
                if s.len() > keys::MAX_VALUE_BYTES {
                    bail_coded!(ErrorCode::Wal003, s.len(), keys::MAX_VALUE_BYTES);
                }
                if let Some(i) = self.new_long.get(s) {
                    return Ok(StagedValue::NewLong(*i));
                }
                if let Some(vref) = dict.find_long_value(s)? {
                    keys::long_str(s, vref)
                } else {
                    let i = self.new_values.len();
                    self.new_values.push(s.clone());
                    self.new_long.insert(s.clone(), i);
                    return Ok(StagedValue::NewLong(i));
                }
            }
            Value::Keyword(k) => KeyValue::Keyword(self.iid(k, dict)?),
            Value::Ref(u) => KeyValue::Ref(self.eid(*u, dict)?),
        }))
    }

    /// Assign ids for `fact`, reading the committed dictionary.
    pub fn stage(&mut self, fact: &Fact, dict: &mut DictReader<'_>) -> Result<()> {
        let e = self.eid(fact.entity, dict)?;
        let a = self.iid(&fact.attribute, dict)?;
        let v = self.value(&fact.value, dict)?;
        self.tx(fact.tx_count, fact.tx_id, dict)?;
        self.staged.push(Staged {
            e,
            a,
            v,
            tx_count: fact.tx_count,
            vf: fact.valid_from,
            vt: fact.valid_to,
            asserted: fact.asserted,
        });
        Ok(())
    }

    /// Write the new long values to fresh value pages and build every entry.
    pub fn finish(
        mut self,
        alloc: &mut PageAllocator,
        backend: &mut dyn StorageBackend,
        cache: &PageCache,
    ) -> Result<Encoded> {
        let mut writer = ValueWriter::new();
        let mut new_refs = Vec::with_capacity(self.new_values.len());
        for s in &self.new_values {
            let vref = writer.push(s.as_bytes(), alloc, backend, cache)?;
            self.dict.push((
                keys::dict_long_value_key(keys::hash64(s.as_bytes()), vref),
                Vec::new(),
            ));
            new_refs.push(keys::long_str(s, vref));
        }
        writer.flush(alloc, backend, cache)?;

        let mut index: [Vec<Entry>; 4] = Default::default();
        for st in self.staged {
            let v = match st.v {
                StagedValue::Ready(v) => v,
                StagedValue::NewLong(i) => new_refs
                    .get(i)
                    .cloned()
                    .ok_or_else(|| err_coded!(ErrorCode::Int049, "staged long value missing"))?,
            };
            let kf = KeyFact {
                e: st.e,
                a: st.a,
                v,
                tx_count: st.tx_count,
                vf: st.vf,
                vt: st.vt,
                asserted: st.asserted,
            };
            for (tree, idx) in index.iter_mut().zip(Index::ALL) {
                if let Some(k) = kf.key(idx) {
                    tree.push((k, Vec::new()));
                }
            }
        }
        for tree in &mut index {
            tree.sort_unstable();
            tree.dedup();
        }
        self.dict.sort_unstable();
        if self
            .dict
            .windows(2)
            .any(|w| matches!(w, [a, b] if a.0 == b.0))
        {
            bail_coded!(ErrorCode::Int049, "a DICT key assigned twice");
        }
        Ok(Encoded {
            index,
            dict: self.dict,
            next_eid: self.next_eid,
            next_iid: self.next_iid,
        })
    }
}

/// True if `key` is a DICT long-value entry (its value ref keeps a value page alive).
#[cfg(test)]
pub fn is_long_value_entry(key: &[u8]) -> bool {
    key.first() == Some(&keys::DICT_LONG_VALUE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::VALID_TIME_FOREVER;
    use crate::storage::backend::MemoryBackend;

    fn fact(e: u128, a: &str, v: Value, tx: u64) -> Fact {
        Fact {
            entity: Uuid::from_u128(e),
            attribute: a.to_string(),
            value: v,
            tx_id: 1_000 + tx,
            tx_count: tx,
            valid_from: 5,
            valid_to: VALID_TIME_FOREVER,
            asserted: true,
        }
    }

    /// Encode `facts` on top of the dictionary at `root`, write the new DICT
    /// tree, and return `(new_root, encoded)`.
    fn checkpoint(
        backend: &mut MemoryBackend,
        cache: &PageCache,
        root: u64,
        counters: (u64, u32),
        facts: &[Fact],
    ) -> (u64, Encoded) {
        let mut enc = Encoder::new(counters.0, counters.1);
        {
            let mut dict = DictReader::new(root, &*backend, cache);
            for f in facts {
                enc.stage(f, &mut dict).unwrap();
            }
        }
        let start = backend.page_count().unwrap().max(2);
        let mut alloc = PageAllocator::new(Vec::new(), start, 1);
        let encoded = enc.finish(&mut alloc, backend, cache).unwrap();
        let mut freed = Vec::new();
        let new_root = btree::cow_insert(
            root,
            encoded.dict.clone(),
            backend,
            cache,
            &mut alloc,
            &mut freed,
        )
        .unwrap();
        (new_root, encoded)
    }

    fn decode_all(
        encoded: &Encoded,
        root: u64,
        backend: &MemoryBackend,
        cache: &PageCache,
    ) -> Vec<Fact> {
        let mut dict = DictReader::new(root, backend, cache);
        encoded.index[0]
            .iter()
            .map(|(k, _)| {
                dict.fact(&KeyFact::decode(Index::Eavt, k).unwrap())
                    .unwrap()
            })
            .collect()
    }

    fn same_facts(mut a: Vec<Fact>, mut b: Vec<Fact>) -> bool {
        let key = |f: &Fact| crate::storage::index::encode_value(&f.value);
        a.sort_by(|x, y| {
            (x.entity, &x.attribute, key(x), x.tx_count).cmp(&(
                y.entity,
                &y.attribute,
                key(y),
                y.tx_count,
            ))
        });
        b.sort_by(|x, y| {
            (x.entity, &x.attribute, key(x), x.tx_count).cmp(&(
                y.entity,
                &y.attribute,
                key(y),
                y.tx_count,
            ))
        });
        a == b
    }

    #[test]
    fn facts_round_trip_through_keys_and_dictionary() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let long = "L".repeat(200);
        let facts = vec![
            fact(1, ":name", Value::String("Alice".into()), 1),
            fact(1, ":status", Value::Keyword(":active".into()), 1),
            fact(1, ":friend", Value::Ref(Uuid::from_u128(2)), 2),
            fact(2, ":bio", Value::String(long.clone()), 2),
            fact(2, ":score", Value::Float(-0.5), 3),
            fact(2, ":n", Value::Integer(-7), 3),
            fact(3, ":flag", Value::Boolean(true), 3),
            fact(3, ":none", Value::Null, 3),
        ];
        let (root, enc) = checkpoint(&mut backend, &cache, 0, (1, 1), &facts);
        assert_eq!(enc.index[3].len(), 1, "one ref fact in VAET");
        assert!(same_facts(decode_all(&enc, root, &backend, &cache), facts));
        assert_eq!(enc.next_eid, 4, "entities 1, 2, 3 (2 via a ref)");
    }

    #[test]
    fn ids_are_assigned_once_and_shared_between_attribute_and_keyword() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let first = vec![
            fact(1, ":color", Value::Keyword(":red".into()), 1),
            fact(2, ":red", Value::Integer(1), 1),
        ];
        let (root, enc) = checkpoint(&mut backend, &cache, 0, (1, 1), &first);
        // Idents :color and :red: two iids, :red shared by attribute and keyword.
        assert_eq!(enc.next_iid, 3);
        let dict = DictReader::new(root, &backend, &cache);
        let red = dict.iid_of(":red").unwrap().unwrap();
        let e1 = dict.eid_of(&Uuid::from_u128(1)).unwrap().unwrap();

        // A later checkpoint reuses committed ids and continues the counters.
        let second = vec![
            fact(1, ":red", Value::Keyword(":blue".into()), 2),
            fact(9, ":color", Value::Keyword(":red".into()), 2),
        ];
        let (root2, enc2) = checkpoint(
            &mut backend,
            &cache,
            root,
            (enc.next_eid, enc.next_iid),
            &second,
        );
        let dict2 = DictReader::new(root2, &backend, &cache);
        assert_eq!(
            dict2.iid_of(":red").unwrap(),
            Some(red),
            "iid never changes"
        );
        assert_eq!(dict2.eid_of(&Uuid::from_u128(1)).unwrap(), Some(e1));
        assert_eq!(enc2.next_iid, 4, "only :blue is new");
        assert_eq!(enc2.next_eid, enc.next_eid + 1, "only entity 9 is new");
        assert!(same_facts(
            decode_all(&enc2, root2, &backend, &cache),
            second
        ));
    }

    #[test]
    fn assignment_is_deterministic() {
        let facts: Vec<Fact> = (0..300u128)
            .map(|i| {
                fact(
                    i * 7919 % 1000,
                    &format!(":a{}", i % 13),
                    Value::Integer(1),
                    u64::try_from(i / 10).unwrap(),
                )
            })
            .collect();
        let run = || {
            let mut backend = MemoryBackend::new();
            let cache = PageCache::new(256);
            let (_, enc) = checkpoint(&mut backend, &cache, 0, (1, 1), &facts);
            (enc.index, enc.dict)
        };
        assert!(
            run() == run(),
            "same facts in the same order give the same keys"
        );
    }

    #[test]
    fn long_values_are_deduplicated() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let long = "x".repeat(500);
        let mut retract = fact(1, ":bio", Value::String(long.clone()), 2);
        retract.asserted = false;
        let facts = vec![
            fact(1, ":bio", Value::String(long.clone()), 1),
            retract,
            fact(2, ":bio", Value::String(long.clone()), 2),
        ];
        let (root, enc) = checkpoint(&mut backend, &cache, 0, (1, 1), &facts);
        let pages_after_first = backend.page_count().unwrap();
        let long_entries = enc
            .dict
            .iter()
            .filter(|(k, _)| is_long_value_entry(k))
            .count();
        assert_eq!(long_entries, 1, "one stored copy within a checkpoint");

        // Re-asserting in a later checkpoint writes no value page.
        let again = vec![fact(3, ":bio", Value::String(long), 3)];
        let mut enc2 = Encoder::new(enc.next_eid, enc.next_iid);
        {
            let mut dict = DictReader::new(root, &backend, &cache);
            enc2.stage(&again[0], &mut dict).unwrap();
        }
        let mut alloc = PageAllocator::new(Vec::new(), pages_after_first, 2);
        let enc2 = enc2.finish(&mut alloc, &mut backend, &cache).unwrap();
        assert_eq!(alloc.next_append(), pages_after_first, "no new value page");
        assert!(!enc2.dict.iter().any(|(k, _)| is_long_value_entry(k)));
    }

    #[test]
    fn hash_collisions_still_find_the_right_value() {
        // Two DICT 0x06 entries under one hash: dedup compares full values.
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let a = "a".repeat(100);
        let b = "b".repeat(100);
        let mut alloc = PageAllocator::new(Vec::new(), 2, 1);
        let mut w = ValueWriter::new();
        let ra = w
            .push(a.as_bytes(), &mut alloc, &mut backend, &cache)
            .unwrap();
        let rb = w
            .push(b.as_bytes(), &mut alloc, &mut backend, &cache)
            .unwrap();
        w.flush(&mut alloc, &mut backend, &cache).unwrap();
        let h = keys::hash64(b.as_bytes());
        // Forge a collision: file `a` under `b`'s hash, ahead of `b`'s own entry.
        let mut entries = vec![
            (keys::dict_long_value_key(h, ra), Vec::new()),
            (keys::dict_long_value_key(h, rb), Vec::new()),
        ];
        entries.sort();
        let root = btree::build_btree(entries, &mut backend, &cache, &mut alloc).unwrap();
        let mut dict = DictReader::new(root, &backend, &cache);
        assert_eq!(dict.find_long_value(&b).unwrap(), Some(rb));
        assert_eq!(dict.find_long_value(&"c".repeat(100)).unwrap(), None);
    }

    #[test]
    fn one_tx_count_with_two_timestamps_is_stg_037() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let mut other = fact(2, ":a", Value::Integer(1), 1);
        other.tx_id += 1;
        let mut enc = Encoder::new(1, 1);
        let mut dict = DictReader::new(0, &backend, &cache);
        enc.stage(&fact(1, ":a", Value::Integer(1), 1), &mut dict)
            .unwrap();
        let err = enc.stage(&other, &mut dict).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-037");

        // Also against a committed timestamp.
        let (root, enc) = checkpoint(
            &mut backend,
            &cache,
            0,
            (1, 1),
            &[fact(1, ":a", Value::Integer(1), 1)],
        );
        let mut enc2 = Encoder::new(enc.next_eid, enc.next_iid);
        let mut dict = DictReader::new(root, &backend, &cache);
        let err = enc2.stage(&other, &mut dict).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-037");
    }

    #[test]
    fn missing_dictionary_entry_is_stg_036() {
        let backend = MemoryBackend::new();
        let cache = PageCache::new(16);
        let mut dict = DictReader::new(0, &backend, &cache);
        let kf = KeyFact {
            e: 5,
            a: 1,
            v: KeyValue::Null,
            tx_count: 1,
            vf: 0,
            vt: VALID_TIME_FOREVER,
            asserted: true,
        };
        let err = dict.fact(&kf).unwrap_err();
        assert_eq!(crate::error::MinigrafError::from(err).code(), "STG-036");
    }

    /// A shared cache answers later readers without changing any result, and
    /// clears itself at its size limit instead of growing.
    #[test]
    fn shared_cache_is_transparent_and_bounded() {
        let mut backend = MemoryBackend::new();
        let cache = PageCache::new(256);
        let facts = vec![
            fact(1, ":name", Value::Keyword(":k/a".into()), 1),
            fact(2, ":name", Value::Keyword(":k/b".into()), 2),
        ];
        let (root, enc) = checkpoint(&mut backend, &cache, 0, (1, 1), &facts);
        let shared = SharedDictCache::default();
        for _ in 0..2 {
            let mut dict = DictReader::new(root, &backend, &cache).with_shared(&shared);
            let got: Vec<Fact> = enc.index[0]
                .iter()
                .map(|(k, _)| {
                    dict.fact(&KeyFact::decode(Index::Eavt, k).unwrap())
                        .unwrap()
                })
                .collect();
            assert!(
                same_facts(got, facts.clone()),
                "same facts with a warm cache"
            );
        }
        assert!(shared.names.read().unwrap().len() >= 3, "idents cached");
        assert_eq!(shared.tx_ids.read().unwrap().len(), 2, "timestamps cached");

        let map = std::sync::RwLock::new(HashMap::new());
        for i in 0..SHARED_CACHE_LIMIT as u64 + 10 {
            shared_put(&map, i, i);
        }
        assert!(map.read().unwrap().len() <= SHARED_CACHE_LIMIT, "bounded");
    }
}
