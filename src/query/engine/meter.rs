//! The bytes a query holds (#549 spec §3): one [`Meter`] per query, shared
//! by every holder of its state, each through a [`Held`].
//!
//! A holder charges an entry's estimated bytes before it inserts the entry,
//! then grows its container with `try_reserve`, and releases what it holds
//! when it rewinds, closes or drops. The estimate is per entry (spec §3.2),
//! so the count is the same on every target.

use super::cell::{Cell, JoinKey, Scalar};
use crate::error::{ErrorCode, err_coded};
use crate::graph::types::{Value, ValueSet};
use anyhow::Result;
use std::collections::TryReserveError;
use std::mem::size_of;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What a hash table entry costs beyond its key and value: the control
/// byte, and spare capacity at hashbrown's 7/8 load factor, rounded up.
pub(crate) const HASH_ENTRY_OVERHEAD: usize = 16;

/// The limit of one query, and the bytes its holders count now.
#[derive(Debug)]
pub(crate) struct Meter {
    limit: usize,
    used: AtomicUsize,
    /// The most `used` has been, for tests and measurement.
    peak: AtomicUsize,
    #[cfg(test)]
    window: LivePeak,
    #[cfg(test)]
    aggregate: LivePeak,
    #[cfg(test)]
    ids: LivePeak,
}

/// Test-only ownership totals distinguish a window's retained rows from the
/// lookup cache. Multiple Helds with the same name contribute together.
#[cfg(test)]
#[derive(Debug, Default)]
struct LivePeak {
    used: AtomicUsize,
    peak: AtomicUsize,
}

#[cfg(test)]
impl LivePeak {
    fn add(&self, n: usize) {
        let now = self.used.load(Ordering::Relaxed).saturating_add(n);
        self.used.store(now, Ordering::Relaxed);
        self.peak.fetch_max(now, Ordering::Relaxed);
    }

    fn subtract(&self, n: usize) {
        self.used.store(
            self.used.load(Ordering::Relaxed).saturating_sub(n),
            Ordering::Relaxed,
        );
    }
}

impl Meter {
    pub(crate) fn new(limit: usize) -> Arc<Meter> {
        Arc::new(Meter {
            limit,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            #[cfg(test)]
            window: LivePeak::default(),
            #[cfg(test)]
            aggregate: LivePeak::default(),
            #[cfg(test)]
            ids: LivePeak::default(),
        })
    }

    /// A meter with no limit.
    #[cfg(test)]
    pub(crate) fn unlimited() -> Arc<Meter> {
        Meter::new(usize::MAX)
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        // A query runs on one thread at a time (spec §3.1).
        self.used.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn peak(&self) -> usize {
        self.peak.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn tracked(&self, op: &str) -> Option<&LivePeak> {
        match op {
            "Window" => Some(&self.window),
            "Aggregate" => Some(&self.aggregate),
            "IdSpace" => Some(&self.ids),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn peak_for(&self, op: &str) -> usize {
        self.tracked(op)
            .map_or(0, |p| p.peak.load(Ordering::Relaxed))
    }
}

/// What one holder counts on its query's meter.
pub(crate) struct Held {
    meter: Arc<Meter>,
    op: &'static str,
    bytes: usize,
}

impl Held {
    /// The meter this holder counts on.
    pub(crate) fn meter(&self) -> &Arc<Meter> {
        &self.meter
    }

    pub(crate) fn new(meter: &Arc<Meter>, op: &'static str) -> Self {
        Held {
            meter: meter.clone(),
            op,
            bytes: 0,
        }
    }

    /// The bytes this holder counts now.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Count `n` more bytes, or `QRY-012` and count nothing when the query
    /// would hold more than its limit.
    pub(crate) fn charge(&mut self, n: usize) -> Result<()> {
        let m = &*self.meter;
        let used = m.used.load(Ordering::Relaxed);
        if used.saturating_add(n) > m.limit {
            return Err(err_coded!(
                ErrorCode::Qry012,
                format!(
                    "{} needs {n} more bytes and the query already holds {used} of its {}-byte limit",
                    self.op, m.limit
                )
            ));
        }
        let now = used.saturating_add(n);
        m.used.store(now, Ordering::Relaxed);
        m.peak.fetch_max(now, Ordering::Relaxed);
        self.bytes = self.bytes.saturating_add(n);
        #[cfg(test)]
        if let Some(peak) = m.tracked(self.op) {
            peak.add(n);
        }
        Ok(())
    }

    /// Stop counting `n` of this holder's bytes: an entry it let go.
    pub(crate) fn uncharge(&mut self, n: usize) {
        let n = n.min(self.bytes);
        self.bytes -= n;
        let m = &*self.meter;
        #[cfg(test)]
        if let Some(peak) = m.tracked(self.op) {
            peak.subtract(n);
        }
        m.used.store(
            m.used.load(Ordering::Relaxed).saturating_sub(n),
            Ordering::Relaxed,
        );
    }

    /// Take over what `other` counts, which goes on being counted until this
    /// holder lets it go.
    pub(crate) fn absorb(&mut self, mut other: Held) {
        #[cfg(test)]
        if self.op != other.op {
            if let Some(peak) = self.meter.tracked(other.op) {
                peak.subtract(other.bytes);
            }
            if let Some(peak) = self.meter.tracked(self.op) {
                peak.add(other.bytes);
            }
        }
        self.bytes = self.bytes.saturating_add(other.bytes);
        other.bytes = 0;
    }

    /// Stop counting every byte of this holder.
    pub(crate) fn release(&mut self) {
        self.uncharge(self.bytes);
    }

    /// The result of growing a container with `try_reserve(1)`. A refusal
    /// is `QRY-012` (allocator form) and takes back the `charged` bytes;
    /// `refused` estimates what the container asked for, as `TryReserveError`
    /// does not say on stable Rust.
    pub(crate) fn reserve(
        &mut self,
        r: std::result::Result<(), TryReserveError>,
        charged: usize,
        refused: usize,
    ) -> Result<()> {
        #[cfg(test)]
        let r = if FAIL_RESERVE.with(|f| f.get() == Some(self.op)) {
            Vec::<u64>::new().try_reserve(usize::MAX)
        } else {
            r
        };
        if r.is_err() {
            self.uncharge(charged);
            return Err(err_coded!(
                ErrorCode::Qry012,
                format!("the allocator refused {refused} bytes for {}", self.op)
            ));
        }
        Ok(())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
thread_local! {
    /// Makes every [`Held::reserve`] of this op fail while set (spec §6.2).
    pub(crate) static FAIL_RESERVE: std::cell::Cell<Option<&'static str>> =
        const { std::cell::Cell::new(None) };
}

/// What growing a container of `capacity` entries of `entry` bytes would
/// ask for: its doubling.
pub(crate) fn growth(capacity: usize, entry: usize) -> usize {
    entry.saturating_mul(capacity.max(1))
}

/// A cell and the heap bytes of its string or set.
pub(crate) fn cell_bytes(c: &Cell) -> usize {
    size_of::<Cell>()
        + match c {
            Cell::Val(Scalar::Str(s)) => s.len(),
            Cell::Val(Scalar::Set(s)) => set_bytes(s),
            _ => 0,
        }
}

/// A row's cells and its vector's header.
pub(crate) fn row_bytes(row: &[Cell]) -> usize {
    size_of::<Vec<Cell>>() + row.iter().map(cell_bytes).sum::<usize>()
}

/// A join key and the heap bytes of its string or set.
pub(crate) fn key_bytes(k: &JoinKey) -> usize {
    size_of::<JoinKey>()
        + match k {
            JoinKey::Str(s) => s.len(),
            JoinKey::Set(s) => set_bytes(s),
            _ => 0,
        }
}

/// A key of several cells and its vector's header.
pub(crate) fn keys_bytes(key: &[JoinKey]) -> usize {
    size_of::<Vec<JoinKey>>() + key.iter().map(key_bytes).sum::<usize>()
}

/// A value and the heap bytes of its string or set.
pub(crate) fn value_bytes(v: &Value) -> usize {
    size_of::<Value>()
        + match v {
            Value::String(s) | Value::Keyword(s) => s.len(),
            Value::Set(s) => set_bytes(s),
            _ => 0,
        }
}

/// Conservative per-entry set payload: the set header, Arc counters (also
/// counted for owned values), and each element's value/string bytes. Sixteen
/// extra bytes per element estimate BTree node headers and spare capacity;
/// `value_bytes` already includes the element's own `Value` size. As with
/// string keys, shared payloads are counted for each entry retaining them.
fn set_bytes(set: &ValueSet) -> usize {
    const TREE_SLACK_PER_ELEMENT: usize = 16;
    size_of::<ValueSet>()
        + 2 * size_of::<usize>()
        + set
            .iter()
            .map(|v| value_bytes(v) + TREE_SLACK_PER_ELEMENT)
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CodedError;

    /// Retained set estimates must grow with their payload, in every holder.
    #[test]
    fn set_estimators_include_elements_and_string_payloads() {
        let set = |n| {
            Arc::new(
                crate::ValueSet::new([Value::String("x".repeat(n)), Value::Keyword(":tag".into())])
                    .unwrap(),
            )
        };
        let estimates = |s: &Arc<crate::ValueSet>| {
            [
                cell_bytes(&Cell::Val(Scalar::Set(s.clone()))),
                key_bytes(&JoinKey::Set(s.clone())),
                value_bytes(&Value::Set(s.as_ref().clone())),
            ]
        };
        for (small, large) in estimates(&set(8)).into_iter().zip(estimates(&set(1024))) {
            assert_eq!(large - small, 1016, "string payload growth is charged");
            assert!(
                large >= 1024 + 4 + 2 * size_of::<Value>(),
                "all elements are counted"
            );
        }
    }

    fn code(e: &anyhow::Error) -> Option<ErrorCode> {
        e.downcast_ref::<CodedError>().map(CodedError::code)
    }

    #[test]
    fn charge_over_the_limit_counts_nothing() {
        let meter = Meter::new(100);
        let mut a = Held::new(&meter, "HashJoin");
        a.charge(60).unwrap();
        let err = a.charge(41).unwrap_err();
        assert!(code(&err) == Some(ErrorCode::Qry012), "QRY-012");
        let text = err.to_string();
        assert!(text.contains("HashJoin needs 41 more bytes"), "{text}");
        assert!(text.contains("holds 60 of its 100-byte limit"), "{text}");
        assert_eq!(meter.used(), 60);
        assert_eq!(a.bytes(), 60);
        a.charge(40).unwrap();
        assert_eq!(meter.used(), 100);
    }

    #[test]
    fn release_and_drop_give_the_bytes_back() {
        let meter = Meter::new(1000);
        let mut a = Held::new(&meter, "Distinct");
        let mut b = Held::new(&meter, "Aggregate");
        a.charge(300).unwrap();
        b.charge(200).unwrap();
        a.uncharge(100);
        assert_eq!(meter.used(), 400);
        a.uncharge(10_000);
        assert_eq!((a.bytes(), meter.used()), (0, 200), "never below 0");
        b.charge(50).unwrap();
        drop(b);
        assert_eq!(meter.used(), 0);
        assert_eq!(meter.peak(), 500);
        a.charge(10).unwrap();
        a.release();
        assert_eq!(meter.used(), 0);
    }

    #[test]
    fn a_refused_reserve_is_the_allocator_form() {
        let meter = Meter::unlimited();
        let mut a = Held::new(&meter, "Window");
        a.charge(64).unwrap();
        a.reserve(Ok(()), 64, 128).unwrap();
        assert_eq!(meter.used(), 64);
        FAIL_RESERVE.with(|f| f.set(Some("Window")));
        let err = a.reserve(Ok(()), 64, 128).unwrap_err();
        assert!(code(&err) == Some(ErrorCode::Qry012), "QRY-012");
        assert!(
            err.to_string()
                .contains("the allocator refused 128 bytes for Window"),
            "allocator form"
        );
        assert_eq!(meter.used(), 0, "the charge is taken back");
        FAIL_RESERVE.with(|f| f.set(None));
        a.reserve(Ok(()), 0, 0).unwrap();
    }

    #[test]
    fn no_limit_never_fails() {
        let meter = Meter::unlimited();
        let mut a = Held::new(&meter, "Union");
        a.charge(usize::MAX).unwrap();
        a.charge(1).unwrap();
        assert_eq!(meter.used(), usize::MAX);
    }
}

#[cfg(test)]
#[test]
fn op_peaks_count_overlapping_holders_and_transferred_ownership() {
    let meter = Meter::unlimited();
    let mut rows = Held::new(&meter, "Window");
    let mut columns = Held::new(&meter, "Window");
    let mut ids = Held::new(&meter, "IdSpace");
    let mut active = Held::new(&meter, "Aggregate");
    let mut finished = Held::new(&meter, "Aggregate");
    active.charge(300).unwrap();
    finished.charge(400).unwrap();
    assert_eq!(meter.peak_for("Aggregate"), 700);
    active.release();
    finished.release();
    rows.charge(100).unwrap();
    columns.charge(30).unwrap();
    ids.charge(200).unwrap();
    assert_eq!(meter.peak_for("Window"), 130);
    assert_eq!(meter.peak_for("IdSpace"), 200);
    rows.uncharge(80);
    drop(columns);
    let mut transferred = Held::new(&meter, "other");
    transferred.charge(500).unwrap();
    ids.absorb(transferred);
    assert_eq!(meter.peak_for("IdSpace"), 700);
    assert_eq!(meter.peak_for("Window"), 130);
    drop(rows);
    drop(ids);
    assert_eq!(meter.used(), 0);
}
