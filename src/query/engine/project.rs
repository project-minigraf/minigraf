//! `Project` (#473 spec §6.1): the `:find` columns of a batch, as values.
//!
//! Only rows that are returned are converted, so a query reads ids until
//! the end (#530).

use super::BATCH_ROWS;
use super::Ctx;
use super::batch::Batch;
use super::cell::{Cell, Scalar};
use crate::graph::types::{Value, ValueSet};
use anyhow::Result;

/// The rows of `batch`, with the cells of the slots `find` as values.
pub(crate) fn project(batch: &Batch, find: &[usize], ctx: &mut Ctx<'_>) -> Result<Vec<Vec<Value>>> {
    let hints = Uuids::read(batch, find, ctx);
    let mut rows = Vec::with_capacity(batch.len);
    for i in 0..batch.len {
        let mut row = Vec::with_capacity(find.len());
        for &slot in find {
            let cell = batch.cols.get(slot).and_then(|c| c.get(i));
            row.push(match cell {
                Some(Cell::Eid(e)) => Value::Ref(ctx.ids.uuid_with_hint(
                    ctx.keys,
                    *e,
                    hints.as_ref().and_then(|h| h.get(*e)),
                )?),
                Some(cell) => value(cell, ctx)?,
                None => Value::Null,
            });
        }
        rows.push(row);
    }
    Ok(rows)
}

/// Bounded, unmetered stack hints: they never change query state. Populate
/// IdSpace only in projection order, so earlier errors and memory charges
/// remain scalar's. Failed speculative reads discard every partial hint.
struct Uuids {
    ids: [u64; BATCH_ROWS],
    values: [crate::graph::types::EntityId; BATCH_ROWS],
    len: usize,
}

impl Uuids {
    fn read(batch: &Batch, find: &[usize], ctx: &Ctx<'_>) -> Option<Self> {
        let keys = ctx.keys?;
        let candidates = || {
            (0..batch.len).flat_map(|row| {
                find.iter().filter_map(move |&slot| {
                    match batch.cols.get(slot).and_then(|c| c.get(row)) {
                        Some(Cell::Eid(e)) if ctx.ids.needs_uuid(*e) => Some(*e),
                        _ => None,
                    }
                })
            })
        };
        let mut probe = candidates();
        let first = probe.next()?;
        if !probe.any(|e| e != first) {
            return None;
        }
        let mut hints = Self {
            ids: [0; BATCH_ROWS],
            values: [crate::graph::types::EntityId::nil(); BATCH_ROWS],
            len: 0,
        };
        for eid in candidates() {
            if !hints.insert(eid)? {
                break;
            }
        }
        let count = hints.len;
        if count < 2 {
            return None;
        }
        let ids = hints.ids.get(..count)?;
        let mut expected = ids.iter();
        let mut output = hints.values.iter_mut();
        let mut read = 0;
        keys.uuids_of(ids, &mut |eid, uuid| {
            let wanted = expected.next();
            debug_assert_eq!(wanted, Some(&eid), "UUID batch changed requested order");
            if wanted != Some(&eid) {
                return Err(crate::error::err_coded!(
                    crate::error::ErrorCode::Int049,
                    "UUID batch returned a different id"
                ));
            }
            let dst = output.next().ok_or_else(|| {
                crate::error::err_coded!(crate::error::ErrorCode::Int049, "too many UUID hints")
            })?;
            *dst = uuid;
            read += 1;
            Ok(())
        })
        .ok()?;
        debug_assert_eq!(read, count, "incomplete UUID batch");
        (read == count).then_some(hints)
    }

    /// Keep the bounded buffer sorted and distinct without another general
    /// sort specialization in the embedded binary. Existing ids move nothing.
    fn insert(&mut self, eid: u64) -> Option<bool> {
        let at = match self.ids.get(..self.len)?.binary_search(&eid) {
            Ok(_) => return Some(true),
            Err(at) => at,
        };
        if self.len == BATCH_ROWS {
            return Some(false);
        }
        let ids = self.ids.get_mut(..=self.len)?;
        ids.copy_within(at..self.len, at + 1);
        *ids.get_mut(at)? = eid;
        self.len += 1;
        Some(true)
    }

    fn get(&self, eid: u64) -> Option<crate::graph::types::EntityId> {
        let i = self.ids.get(..self.len)?.binary_search(&eid).ok()?;
        self.values.get(i).copied()
    }
}

/// The value of `cell`.
pub(crate) fn value(cell: &Cell, ctx: &mut Ctx<'_>) -> Result<Value> {
    Ok(match cell {
        Cell::Eid(e) => Value::Ref(ctx.ids.uuid(ctx.keys, *e)?),
        Cell::Iid(i) => Value::Keyword(ctx.ids.name(ctx.keys, *i)?.to_string()),
        Cell::Val(Scalar::Null) => Value::Null,
        Cell::Val(Scalar::Bool(b)) => Value::Boolean(*b),
        Cell::Val(Scalar::Int(n)) => Value::Integer(*n),
        Cell::Val(Scalar::Float(f)) => Value::Float(*f),
        Cell::Val(Scalar::Str(s)) => Value::String(s.to_string()),
        Cell::Val(Scalar::Set(s)) => Value::Set(ValueSet::clone(s)),
        Cell::Val(Scalar::Long(vref)) => match ctx.keys {
            Some(k) => Value::String(k.long_value(*vref)?),
            None => {
                return Err(crate::error::err_coded!(
                    crate::error::ErrorCode::Int049,
                    "query engine: long value without a committed reader"
                ));
            }
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorCode, err_coded};
    use crate::graph::storage::FactStorage;
    use crate::query::datalog::functions::FunctionRegistry;
    use crate::query::engine::ids::IdSpace;
    use crate::query::engine::meter::Meter;
    use crate::storage::keys::{Index, KeyFact, ValueRef};
    use crate::storage::{KeySource, TxBound, TxOrder};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Keys {
        bulk: AtomicUsize,
        scalar: AtomicUsize,
        fail_bulk: bool,
        limit: u64,
    }

    impl Keys {
        fn new(fail_bulk: bool) -> Self {
            Self {
                bulk: AtomicUsize::new(0),
                scalar: AtomicUsize::new(0),
                fail_bulk,
                limit: 3,
            }
        }
    }

    // The unused storage operations cannot affect a projection-only test.
    impl KeySource for Keys {
        fn any_key(&self, _: Index, _: &[u8]) -> Result<bool> {
            panic!("unused")
        }
        fn next_ids(&self) -> (u64, u32) {
            (self.limit, 1)
        }
        fn eid_of(&self, _: &uuid::Uuid) -> Result<Option<u64>> {
            Ok(None)
        }
        fn uuid_of(&self, eid: u64) -> Result<uuid::Uuid> {
            self.scalar.fetch_add(1, Ordering::Relaxed);
            assert!(eid < self.limit, "provisional ids must not read DICT");
            Ok(uuid::Uuid::from_u128(u128::from(eid)))
        }
        fn uuids_of(
            &self,
            eids: &[u64],
            visit: &mut dyn FnMut(u64, uuid::Uuid) -> Result<()>,
        ) -> Result<()> {
            self.bulk.fetch_add(1, Ordering::Relaxed);
            for &e in eids {
                assert!(e < self.limit, "only committed misses are batched");
                visit(e, uuid::Uuid::from_u128(u128::from(e)))?;
                if self.fail_bulk {
                    anyhow::bail!("speculative failure after one hint");
                }
            }
            Ok(())
        }
        fn iid_of(&self, _: &str) -> Result<Option<u32>> {
            panic!("unused")
        }
        fn ident_of(&self, _: u32) -> Result<String> {
            panic!("unused")
        }
        fn long_value(&self, _: ValueRef) -> Result<String> {
            Err(err_coded!(ErrorCode::Stg036, "first scalar error"))
        }
        fn long_value_ref(&self, _: &str) -> Result<Option<ValueRef>> {
            panic!("unused")
        }
        fn long_bytes(&self, _: ValueRef) -> Result<Vec<u8>> {
            panic!("unused")
        }
        fn long_bytes_ref(&self, _: &[u8]) -> Result<Option<ValueRef>> {
            panic!("unused")
        }
        fn tx_id_of(&self, _: u64) -> Result<u64> {
            panic!("unused")
        }
        fn tx_order(&self) -> Result<TxOrder> {
            panic!("unused")
        }
        fn last_tx_at(&self, _: u64) -> Result<u64> {
            panic!("unused")
        }
        fn tx_runs(&self, _: u64, _: &mut Vec<(u64, u64)>) -> Result<()> {
            panic!("unused")
        }
        fn live_keys(
            &self,
            _: Index,
            _: &[u8],
            _: Option<&[u8]>,
            _: &TxBound,
            _: usize,
            _: &mut Vec<KeyFact>,
        ) -> Result<Option<Vec<u8>>> {
            panic!("unused")
        }
    }

    fn project_with(
        keys: &Keys,
        batch: &Batch,
        find: &[usize],
        limit: usize,
    ) -> Result<Vec<Vec<Value>>> {
        let snapshot = FactStorage::new().snapshot().unwrap();
        let meter = Meter::new(limit);
        let mut ids = IdSpace::new(Some(keys), &meter);
        let functions = FunctionRegistry::with_builtins();
        let mut ctx = Ctx {
            snapshot: &snapshot,
            keys: Some(keys),
            ids: &mut ids,
            functions: &functions,
            derived: None,
            bound: &TxBound::All,
        };
        project(batch, find, &mut ctx)
    }

    #[test]
    fn projection_batches_committed_ids_among_provisional_and_repeated_ids() {
        let keys = Keys::new(false);
        let snapshot = FactStorage::new().snapshot().unwrap();
        let meter = Meter::unlimited();
        let mut ids = IdSpace::new(Some(&keys), &meter);
        let pending = uuid::Uuid::from_u128(100);
        let provisional = ids.eid(None, &pending).unwrap();
        let mut batch = Batch::new(1);
        for e in [2, provisional, 1, 2] {
            batch.push(&mut [Some(Cell::Eid(e))]);
        }
        let functions = FunctionRegistry::with_builtins();
        let mut ctx = Ctx {
            snapshot: &snapshot,
            keys: Some(&keys),
            ids: &mut ids,
            functions: &functions,
            derived: None,
            bound: &TxBound::All,
        };
        let rows = project(&batch, &[0, 0], &mut ctx).unwrap();
        let expected: Vec<_> = [
            uuid::Uuid::from_u128(2),
            pending,
            uuid::Uuid::from_u128(1),
            uuid::Uuid::from_u128(2),
        ]
        .into_iter()
        .map(|u| vec![Value::Ref(u); 2])
        .collect();
        assert!(
            rows == expected,
            "row order, duplicate columns and pending UUIDs preserved"
        );
        assert_eq!(keys.bulk.load(Ordering::Relaxed), 1);
        assert_eq!(keys.scalar.load(Ordering::Relaxed), 0);
        assert_eq!(ids.dict_cached(), 2);
        ids.end_pull();
        assert_eq!(ids.dict_cached(), 0);
    }

    #[test]
    fn projection_reclaims_duplicate_hint_slots() {
        for find in [vec![0, 0], vec![0, 1, 0, 1]] {
            let mut keys = Keys::new(false);
            keys.limit = u64::try_from(BATCH_ROWS + 1).unwrap();
            let mut batch = Batch::new(2);
            for i in 0..BATCH_ROWS {
                let e = u64::try_from(i + 1).unwrap();
                let f = u64::try_from((i + BATCH_ROWS / 2) % BATCH_ROWS + 1).unwrap();
                batch.push(&mut [Some(Cell::Eid(e)), Some(Cell::Eid(f))]);
            }
            let rows = project_with(&keys, &batch, &find, usize::MAX).unwrap();
            assert_eq!(rows.len(), BATCH_ROWS);
            assert_eq!(keys.bulk.load(Ordering::Relaxed), 1);
            assert_eq!(
                keys.scalar.load(Ordering::Relaxed),
                0,
                "duplicates must not exhaust the hint buffer"
            );
        }
    }

    #[test]
    fn repeated_ids_reuse_a_nearly_full_hint_buffer() {
        let mut keys = Keys::new(false);
        keys.limit = u64::try_from(BATCH_ROWS).unwrap();
        let mut batch = Batch::new(2);
        for i in 0..BATCH_ROWS {
            let e = u64::try_from((2 * i + 1).min(BATCH_ROWS - 1)).unwrap();
            let f = u64::try_from((2 * i + 2).min(BATCH_ROWS - 1)).unwrap();
            batch.push(&mut [Some(Cell::Eid(e)), Some(Cell::Eid(f))]);
        }
        let rows = project_with(&keys, &batch, &[0, 1], usize::MAX).unwrap();
        assert_eq!(rows.len(), BATCH_ROWS);
        assert_eq!(keys.scalar.load(Ordering::Relaxed), 0);
        assert_eq!(keys.bulk.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn sorted_hint_buffer_matches_a_set_at_every_capacity_boundary() {
        use std::collections::BTreeSet;
        let n = u64::try_from(BATCH_ROWS).unwrap();
        for input in [
            (1..=n + 1).collect::<Vec<_>>(),
            (1..=n + 1).rev().collect(),
            (0..3 * n).map(|i| 1 + (i * 7919) % (n + 1)).collect(),
        ] {
            let mut hints = Uuids {
                ids: [0; BATCH_ROWS],
                values: [uuid::Uuid::nil(); BATCH_ROWS],
                len: 0,
            };
            let mut expected = BTreeSet::new();
            for eid in input {
                let fits = expected.len() < BATCH_ROWS || expected.contains(&eid);
                assert_eq!(hints.insert(eid), Some(fits));
                if fits {
                    expected.insert(eid);
                }
                assert_eq!(
                    hints.ids.get(..hints.len).unwrap(),
                    expected.iter().copied().collect::<Vec<_>>()
                );
            }
        }
    }

    #[test]
    fn unprojected_entity_cells_make_no_scalar_or_speculative_reads() {
        let keys = Keys::new(false);
        let mut batch = Batch::new(2);
        for e in [1, 2] {
            batch.push(&mut [Some(Cell::Eid(e)), Some(Cell::Val(Scalar::Int(7)))]);
        }
        let rows = project_with(&keys, &batch, &[1], usize::MAX).unwrap();
        assert!(rows == vec![vec![Value::Integer(7)]; 2]);
        assert_eq!(keys.bulk.load(Ordering::Relaxed), 0);
        assert_eq!(keys.scalar.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn speculative_failure_falls_back_without_spending_the_query_budget() {
        let keys = Keys::new(true);
        let mut batch = Batch::new(1);
        for e in [2, 1] {
            batch.push(&mut [Some(Cell::Eid(e))]);
        }
        let rows = project_with(&keys, &batch, &[0], usize::MAX).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(keys.bulk.load(Ordering::Relaxed), 1);
        assert_eq!(
            keys.scalar.load(Ordering::Relaxed),
            2,
            "discard every partial hint"
        );
        let error = project_with(&Keys::new(false), &batch, &[0], 0).unwrap_err();
        assert_eq!(crate::MinigrafError::from(error).code(), "QRY-012");
    }

    #[test]
    fn hints_preserve_scalar_error_order_before_id_charges() {
        for fail_bulk in [false, true] {
            let keys = Keys::new(fail_bulk);
            let mut batch = Batch::new(2);
            for e in [2, 1] {
                batch.push(&mut [
                    Some(Cell::Val(Scalar::Long(ValueRef { page: 42, slot: 0 }))),
                    Some(Cell::Eid(e)),
                ]);
            }
            let error = project_with(&keys, &batch, &[0, 1], 0).unwrap_err();
            let error = crate::MinigrafError::from(error);
            assert_eq!(
                error.code(),
                "STG-036",
                "earlier long-value error beats later memory refusal"
            );
            assert!(error.to_string().contains("first scalar error"));
            assert_eq!(keys.scalar.load(Ordering::Relaxed), 0);
        }
    }
}
