//! One id space per query (#473 spec §5.3).
//!
//! Committed entities and idents have DICT ids. Pending facts hold UUIDs and
//! strings; those the DICT lacks get provisional ids, from the generation's
//! `next_eid` / `next_iid` upwards, the first time the query meets them.
//! Provisional ids are never written anywhere.
//!
//! Provisional ids are kept for the whole query, so that each pending UUID
//! or ident keeps its id. DICT lookups are only cached until the next pull
//! ([`IdSpace::end_pull`]): a DICT id never changes, and a cache that grew
//! with every row a cursor returns would hold the whole answer.
//! A stream that ends in a `Window` keeps them until it ends: the window
//! already looked up every row's ids in its first pull, and holds the whole
//! answer anyway (#547).

use crate::error::{ErrorCode, err_coded};
use crate::graph::types::EntityId;
use crate::query::datalog::matcher::edn_to_entity_id;
use crate::query::datalog::types::EdnValue;
use crate::storage::KeySource;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct IdSpace {
    /// The first provisional ids: every DICT id is below them.
    dict_eids: u64,
    dict_iids: u64,
    next_eid: u64,
    next_iid: u64,
    /// Provisional ids, for the whole query.
    provisional: Maps,
    /// DICT lookups, until the next pull.
    dict: Maps,
}

#[derive(Default)]
struct Maps {
    eid_by_uuid: HashMap<EntityId, u64>,
    uuid_by_eid: HashMap<u64, EntityId>,
    iid_by_name: HashMap<Arc<str>, u64>,
    name_by_iid: HashMap<u64, Arc<str>>,
    /// The entity each keyword names, as an entity id.
    eid_by_keyword: HashMap<u64, u64>,
}

impl IdSpace {
    pub(crate) fn new(keys: Option<&dyn KeySource>) -> Self {
        let (eids, iids) = keys.map_or((0, 0), |k| {
            let (e, i) = k.next_ids();
            (e, u64::from(i))
        });
        IdSpace {
            dict_eids: eids,
            dict_iids: iids,
            next_eid: eids,
            next_iid: iids,
            provisional: Maps::default(),
            dict: Maps::default(),
        }
    }

    /// Forget the DICT lookups of the last pull.
    pub(crate) fn end_pull(&mut self) {
        self.dict = Maps::default();
    }

    /// DICT lookups cached now.
    #[cfg(test)]
    pub(crate) fn dict_cached(&self) -> usize {
        self.dict.uuid_by_eid.len() + self.dict.name_by_iid.len()
    }

    /// The maps that hold `eid` or `iid`.
    fn maps(&mut self, provisional: bool) -> &mut Maps {
        if provisional {
            &mut self.provisional
        } else {
            &mut self.dict
        }
    }

    /// The id of `uuid`: its DICT id, or a provisional one.
    pub(crate) fn eid(&mut self, keys: Option<&dyn KeySource>, uuid: &EntityId) -> Result<u64> {
        for maps in [&self.provisional, &self.dict] {
            if let Some(&e) = maps.eid_by_uuid.get(uuid) {
                return Ok(e);
            }
        }
        let (e, provisional) = match keys.map(|k| k.eid_of(uuid)).transpose()?.flatten() {
            Some(e) => (e, false),
            None => {
                let e = self.next_eid;
                self.next_eid += 1;
                (e, true)
            }
        };
        let maps = self.maps(provisional);
        maps.eid_by_uuid.insert(*uuid, e);
        maps.uuid_by_eid.insert(e, *uuid);
        Ok(e)
    }

    /// The id of the ident `name`: its DICT id, or a provisional one.
    pub(crate) fn iid(&mut self, keys: Option<&dyn KeySource>, name: &str) -> Result<u64> {
        for maps in [&self.provisional, &self.dict] {
            if let Some(&i) = maps.iid_by_name.get(name) {
                return Ok(i);
            }
        }
        let (i, provisional) = match keys.map(|k| k.iid_of(name)).transpose()?.flatten() {
            Some(i) => (u64::from(i), false),
            None => {
                let i = self.next_iid;
                self.next_iid += 1;
                (i, true)
            }
        };
        let name: Arc<str> = Arc::from(name);
        let maps = self.maps(provisional);
        maps.iid_by_name.insert(name.clone(), i);
        maps.name_by_iid.insert(i, name);
        Ok(i)
    }

    /// The entity with id `eid`.
    pub(crate) fn uuid(&mut self, keys: Option<&dyn KeySource>, eid: u64) -> Result<EntityId> {
        let maps = self.maps(eid >= self.dict_eids);
        if let Some(u) = maps.uuid_by_eid.get(&eid) {
            return Ok(*u);
        }
        let Some(k) = keys.filter(|_| eid < self.dict_eids) else {
            return Err(err_coded!(
                ErrorCode::Int049,
                format!("query engine: unknown entity id {eid}")
            ));
        };
        let u = k.uuid_of(eid)?;
        self.dict.uuid_by_eid.insert(eid, u);
        self.dict.eid_by_uuid.insert(u, eid);
        Ok(u)
    }

    /// The id of the entity the keyword with id `iid` names
    /// (`edn_to_entity_id`): a keyword and a ref to that entity are equal in
    /// a join (#545).
    pub(crate) fn keyword_eid(&mut self, keys: Option<&dyn KeySource>, iid: u64) -> Result<u64> {
        for maps in [&self.provisional, &self.dict] {
            if let Some(&e) = maps.eid_by_keyword.get(&iid) {
                return Ok(e);
            }
        }
        let name = self.name(keys, iid)?;
        let uuid = edn_to_entity_id(&EdnValue::Keyword(name.to_string()))
            .map_err(|e| err_coded!(ErrorCode::Int049, format!("query engine: {e}")))?;
        let e = self.eid(keys, &uuid)?;
        // Kept as long as the shorter-lived of the two ids.
        let provisional = iid >= self.dict_iids && e >= self.dict_eids;
        self.maps(provisional).eid_by_keyword.insert(iid, e);
        Ok(e)
    }

    /// `eid` as a DICT id, or `None` for a provisional one.
    pub(crate) fn dict_eid(&self, eid: u64) -> Option<u64> {
        (eid < self.dict_eids).then_some(eid)
    }

    /// The ident with id `iid`.
    pub(crate) fn name(&mut self, keys: Option<&dyn KeySource>, iid: u64) -> Result<Arc<str>> {
        let maps = self.maps(iid >= self.dict_iids);
        if let Some(n) = maps.name_by_iid.get(&iid) {
            return Ok(n.clone());
        }
        let k = keys.filter(|_| iid < self.dict_iids);
        let (Some(k), Ok(i)) = (k, u32::try_from(iid)) else {
            return Err(err_coded!(
                ErrorCode::Int049,
                format!("query engine: unknown ident id {iid}")
            ));
        };
        let n: Arc<str> = Arc::from(k.ident_of(i)?);
        self.dict.name_by_iid.insert(iid, n.clone());
        self.dict.iid_by_name.insert(n.clone(), iid);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisional_ids_outlive_a_pull() {
        let mut ids = IdSpace::new(None);
        let (a, b) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
        let ea = ids.eid(None, &a).unwrap();
        let ia = ids.iid(None, ":a").unwrap();
        ids.end_pull();
        assert_eq!(ids.eid(None, &a).unwrap(), ea);
        assert_ne!(ids.eid(None, &b).unwrap(), ea);
        assert_eq!(ids.iid(None, ":a").unwrap(), ia);
        assert!(ids.uuid(None, ea).unwrap() == a, "round trip");
        assert_eq!(&*ids.name(None, ia).unwrap(), ":a");
        assert!(ids.uuid(None, 99).is_err(), "unknown id");
    }

    /// A keyword's entity id is the id of the UUID it names, whether that
    /// UUID was met first as an entity or not.
    #[test]
    fn keyword_eid_is_the_named_entitys_id() {
        let mut ids = IdSpace::new(None);
        let named = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b":alice");
        let e = ids.eid(None, &named).unwrap();
        let k = ids.iid(None, ":alice").unwrap();
        assert_eq!(ids.keyword_eid(None, k).unwrap(), e);
        ids.end_pull();
        assert_eq!(ids.keyword_eid(None, k).unwrap(), e);
        let u = uuid::Uuid::from_u128(7);
        let k = ids.iid(None, &format!(":{u}")).unwrap();
        let e = ids.keyword_eid(None, k).unwrap();
        assert!(ids.uuid(None, e).unwrap() == u, "UUID keyword");
    }
}
