//! Generation pins and the page quarantine (#473 spec §4.2).
//!
//! A reader that holds a committed generation `p` reads pages that generation
//! references. A checkpoint that publishes generation `g > p` frees some of
//! them, and a later checkpoint would hand them out again. So each checkpoint
//! records the ids it freed as a quarantine entry for `g`, and the allocator
//! skips every id of an entry that is in force: one whose generation is above
//! some pinned generation. Skipped ids stay on the free list, so a crash or a
//! reopen (which has no pins) loses nothing and reuses them at once.
//!
//! An entry is dropped as soon as no pinned generation is below it: at publish
//! when nothing older is pinned, otherwise when the last such pin is released.
//!
//! Lock order: `FactData`, then this. Pins are taken and entries recorded
//! under `FactData`'s lock, so a pin is always on the active generation and an
//! entry is never dropped before a pin that needs it exists.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct State {
    /// Generation → open pins on it.
    pinned: BTreeMap<u64, usize>,
    /// Generation `g` → ids the checkpoint publishing `g` freed.
    quarantine: BTreeMap<u64, Vec<u64>>,
}

impl State {
    /// Drop the entries no pin needs: those at or below the lowest pinned
    /// generation, or all of them when nothing is pinned.
    fn prune(&mut self) {
        match self.pinned.keys().next().copied() {
            Some(lowest) => self.quarantine.retain(|&g, _| g > lowest),
            None => self.quarantine.clear(),
        }
    }
}

/// The open pins of one database and the quarantine entries they keep.
#[derive(Default)]
pub(crate) struct Pins {
    state: Mutex<State>,
}

impl Pins {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Pins::default())
    }

    /// A poisoned lock still holds consistent maps: every update below is
    /// one map operation followed by `prune`, and neither panics.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Pin `generation` until the returned guard is dropped. The caller holds
    /// `FactData`'s lock and `generation` is the active one.
    pub(crate) fn pin(self: &Arc<Self>, generation: u64) -> GenerationPin {
        let mut s = self.lock();
        let n = s.pinned.entry(generation).or_insert(0);
        *n = n.saturating_add(1);
        GenerationPin {
            pins: Arc::clone(self),
            generation,
        }
    }

    /// Record the ids freed by the checkpoint that publishes `generation`.
    /// Kept only while a lower generation is pinned. The caller holds
    /// `FactData`'s write lock.
    pub(crate) fn record(&self, generation: u64, freed: Vec<u64>) {
        let mut s = self.lock();
        if freed.is_empty() || !s.pinned.keys().any(|&p| p < generation) {
            return;
        }
        s.quarantine.entry(generation).or_default().extend(freed);
    }

    /// Every id of an entry in force: the allocator must not hand these out.
    pub(crate) fn in_force(&self) -> HashSet<u64> {
        self.lock().quarantine.values().flatten().copied().collect()
    }

    fn release(&self, generation: u64) {
        let mut s = self.lock();
        if let Some(n) = s.pinned.get_mut(&generation) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                s.pinned.remove(&generation);
            }
        }
        s.prune();
    }

    /// The number of open pins (tests).
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.lock().pinned.values().sum()
    }
}

/// Keeps the pages of one committed generation from being reused; released
/// on drop.
pub(crate) struct GenerationPin {
    pins: Arc<Pins>,
    generation: u64,
}

impl Drop for GenerationPin {
    fn drop(&mut self) {
        self.pins.release(self.generation);
    }
}

/// Which free ids the browser may drop the bytes of after a commit (#440):
/// the ids the commit listed free (`listed`) and those withheld by earlier
/// commits (`withheld`), split into `(release, withheld)`. An id in force
/// stays withheld, since a pinned generation may still read it; a withheld id
/// the commit wrote (`written`) was handed out again and is live, so it is in
/// neither. A listed id is never written by the commit that lists it.
#[cfg(any(test, all(target_arch = "wasm32", feature = "browser")))]
pub(crate) fn split_released(
    listed: Vec<u64>,
    withheld: Vec<u64>,
    in_force: &HashSet<u64>,
    written: impl Fn(u64) -> bool,
) -> (Vec<u64>, Vec<u64>) {
    let mut free: std::collections::BTreeSet<u64> =
        withheld.into_iter().filter(|&id| !written(id)).collect();
    free.extend(listed);
    free.into_iter().partition(|id| !in_force.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(pins: &Pins) -> Vec<u64> {
        let mut v: Vec<u64> = pins.in_force().into_iter().collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn entry_is_in_force_while_a_lower_generation_is_pinned() {
        let pins = Pins::new();
        let pin = pins.pin(3);
        pins.record(4, vec![10, 11]);
        pins.record(5, vec![12]);
        assert_eq!(ids(&pins), vec![10, 11, 12]);
        drop(pin);
        assert!(ids(&pins).is_empty(), "released with the last pin");
        assert_eq!(pins.count(), 0);
    }

    #[test]
    fn entry_with_no_older_pin_is_dropped_at_once() {
        let pins = Pins::new();
        pins.record(2, vec![7]);
        assert!(ids(&pins).is_empty(), "nothing pinned");
        let _pin = pins.pin(2);
        pins.record(2, vec![8]);
        assert!(
            ids(&pins).is_empty(),
            "a pin on the same generation needs none"
        );
    }

    #[test]
    fn two_pins_release_in_either_order() {
        for older_first in [true, false] {
            let pins = Pins::new();
            let old = pins.pin(3);
            pins.record(4, vec![20]);
            let new = pins.pin(4);
            pins.record(5, vec![21]);
            assert_eq!(ids(&pins), vec![20, 21]);
            if older_first {
                drop(old);
                assert_eq!(ids(&pins), vec![21], "entry 4 is needed by no pin");
                drop(new);
            } else {
                drop(new);
                assert_eq!(ids(&pins), vec![20, 21], "pin 3 still needs both");
                drop(old);
            }
            assert!(ids(&pins).is_empty());
        }
    }

    #[test]
    fn two_pins_on_one_generation_both_hold_it() {
        let pins = Pins::new();
        let a = pins.pin(1);
        let b = pins.pin(1);
        pins.record(2, vec![5]);
        drop(a);
        assert_eq!(ids(&pins), vec![5]);
        drop(b);
        assert!(ids(&pins).is_empty());
    }

    #[test]
    fn split_released_withholds_ids_in_force() {
        let in_force: HashSet<u64> = [3, 5].into_iter().collect();
        let (release, withheld) = split_released(vec![2, 3, 4], vec![], &in_force, |_| false);
        assert_eq!(release, vec![2, 4]);
        assert_eq!(withheld, vec![3]);
    }

    #[test]
    fn split_released_frees_withheld_ids_once_out_of_force() {
        let in_force: HashSet<u64> = [7].into_iter().collect();
        // 3 is out of force and still free (not listed again: it sits in the
        // unread tail of the list); 7 is still in force.
        let (release, withheld) = split_released(vec![9], vec![3, 7], &in_force, |_| false);
        assert_eq!(release, vec![3, 9]);
        assert_eq!(withheld, vec![7]);
    }

    #[test]
    fn split_released_drops_withheld_ids_written_again() {
        let none = HashSet::new();
        let (release, withheld) = split_released(vec![1], vec![4, 6], &none, |id| id == 4);
        assert_eq!(release, vec![1, 6], "4 holds a live page now");
        assert!(withheld.is_empty());
    }

    #[test]
    fn split_released_lists_an_id_once() {
        let in_force: HashSet<u64> = [5].into_iter().collect();
        let (release, withheld) = split_released(vec![5, 6], vec![5, 6], &in_force, |_| false);
        assert_eq!(release, vec![6]);
        assert_eq!(withheld, vec![5]);
    }
}
