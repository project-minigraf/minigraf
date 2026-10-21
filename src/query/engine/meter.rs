//! The bytes a query holds (#549 spec §3): one [`Meter`] per query, shared
//! by every holder of its state, each through a [`Held`].
//!
//! A holder charges an entry's estimated bytes before it inserts the entry,
//! then grows its container with `try_reserve`, and releases what it holds
//! when it rewinds, closes or drops. The estimate is per entry (spec §3.2),
//! so the count is the same on every target.

use super::cell::{Cell, JoinKey, Scalar};
use crate::error::{ErrorCode, err_coded};
use crate::graph::types::Value;
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
}

impl Meter {
    pub(crate) fn new(limit: usize) -> Arc<Meter> {
        Arc::new(Meter {
            limit,
            used: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
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
}

/// What one holder counts on its query's meter.
pub(crate) struct Held {
    meter: Arc<Meter>,
    op: &'static str,
    bytes: usize,
}

impl Held {
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
        Ok(())
    }

    /// Stop counting `n` of this holder's bytes: an entry it let go.
    pub(crate) fn uncharge(&mut self, n: usize) {
        let n = n.min(self.bytes);
        self.bytes -= n;
        let m = &*self.meter;
        m.used.store(
            m.used.load(Ordering::Relaxed).saturating_sub(n),
            Ordering::Relaxed,
        );
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
    /// Makes every [`Held::reserve`] of this op fail while set (spec §6.2);
    /// a query runs on the engine twice under the oracle.
    pub(crate) static FAIL_RESERVE: std::cell::Cell<Option<&'static str>> =
        const { std::cell::Cell::new(None) };
}

/// What growing a container of `capacity` entries of `entry` bytes would
/// ask for: its doubling.
pub(crate) fn growth(capacity: usize, entry: usize) -> usize {
    entry.saturating_mul(capacity.max(1))
}

/// A cell and the heap bytes of its string.
pub(crate) fn cell_bytes(c: &Cell) -> usize {
    size_of::<Cell>()
        + match c {
            Cell::Val(Scalar::Str(s)) => s.len(),
            _ => 0,
        }
}

/// A row's cells and its vector's header.
pub(crate) fn row_bytes(row: &[Cell]) -> usize {
    size_of::<Vec<Cell>>() + row.iter().map(cell_bytes).sum::<usize>()
}

/// A join key and the heap bytes of its string.
pub(crate) fn key_bytes(k: &JoinKey) -> usize {
    size_of::<JoinKey>()
        + match k {
            JoinKey::Str(s) => s.len(),
            _ => 0,
        }
}

/// A key of several cells and its vector's header.
pub(crate) fn keys_bytes(key: &[JoinKey]) -> usize {
    size_of::<Vec<JoinKey>>() + key.iter().map(key_bytes).sum::<usize>()
}

/// A value and the heap bytes of its string.
pub(crate) fn value_bytes(v: &Value) -> usize {
    size_of::<Value>()
        + match v {
            Value::String(s) | Value::Keyword(s) => s.len(),
            _ => 0,
        }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CodedError;

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
