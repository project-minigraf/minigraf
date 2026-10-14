//! One id space per query (#473 spec §5.3).
//!
//! Committed entities and idents have DICT ids. Pending facts hold UUIDs and
//! strings; those the DICT lacks get provisional ids, from the generation's
//! `next_eid` / `next_iid` upwards, the first time the query meets them.
//! Provisional ids are never written anywhere.

use crate::error::{ErrorCode, err_coded};
use crate::graph::types::EntityId;
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
    eid_by_uuid: HashMap<EntityId, u64>,
    uuid_by_eid: HashMap<u64, EntityId>,
    iid_by_name: HashMap<Arc<str>, u64>,
    name_by_iid: HashMap<u64, Arc<str>>,
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
            eid_by_uuid: HashMap::new(),
            uuid_by_eid: HashMap::new(),
            iid_by_name: HashMap::new(),
            name_by_iid: HashMap::new(),
        }
    }

    /// The id of `uuid`: its DICT id, or a provisional one.
    pub(crate) fn eid(&mut self, keys: Option<&dyn KeySource>, uuid: &EntityId) -> Result<u64> {
        if let Some(&e) = self.eid_by_uuid.get(uuid) {
            return Ok(e);
        }
        let e = match keys.map(|k| k.eid_of(uuid)).transpose()?.flatten() {
            Some(e) => e,
            None => {
                let e = self.next_eid;
                self.next_eid += 1;
                e
            }
        };
        self.eid_by_uuid.insert(*uuid, e);
        self.uuid_by_eid.insert(e, *uuid);
        Ok(e)
    }

    /// The id of the ident `name`: its DICT id, or a provisional one.
    pub(crate) fn iid(&mut self, keys: Option<&dyn KeySource>, name: &str) -> Result<u64> {
        if let Some(&i) = self.iid_by_name.get(name) {
            return Ok(i);
        }
        let i = match keys.map(|k| k.iid_of(name)).transpose()?.flatten() {
            Some(i) => u64::from(i),
            None => {
                let i = self.next_iid;
                self.next_iid += 1;
                i
            }
        };
        let name: Arc<str> = Arc::from(name);
        self.iid_by_name.insert(name.clone(), i);
        self.name_by_iid.insert(i, name);
        Ok(i)
    }

    /// The entity with id `eid`.
    pub(crate) fn uuid(&mut self, keys: Option<&dyn KeySource>, eid: u64) -> Result<EntityId> {
        if let Some(u) = self.uuid_by_eid.get(&eid) {
            return Ok(*u);
        }
        let Some(k) = keys.filter(|_| eid < self.dict_eids) else {
            return Err(err_coded!(
                ErrorCode::Int049,
                format!("query engine: unknown entity id {eid}")
            ));
        };
        let u = k.uuid_of(eid)?;
        self.uuid_by_eid.insert(eid, u);
        self.eid_by_uuid.insert(u, eid);
        Ok(u)
    }

    /// The ident with id `iid`.
    pub(crate) fn name(&mut self, keys: Option<&dyn KeySource>, iid: u64) -> Result<Arc<str>> {
        if let Some(n) = self.name_by_iid.get(&iid) {
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
        self.name_by_iid.insert(iid, n.clone());
        self.iid_by_name.insert(n.clone(), iid);
        Ok(n)
    }
}
