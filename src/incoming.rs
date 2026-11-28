//! Bounded scalar-reference discovery for write transactions.

use crate::graph::types::{EntityId, Fact, Value};
use std::collections::HashMap;

/// Work limits for an incoming-reference lookup. Zero is a real limit.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct IncomingRefLimits {
    /// Maximum number of distinct live links in a complete result.
    pub max_links: usize,
    /// Maximum fact versions inspected across all storage layers, including
    /// retractions, superseded versions and lookahead.
    pub max_records: usize,
}

impl IncomingRefLimits {
    /// Set both limits. Neither limit reserves memory in advance.
    pub const fn new(max_links: usize, max_records: usize) -> Self {
        Self {
            max_links,
            max_records,
        }
    }
}

/// One stored scalar `Value::Ref` pointing to the requested target.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct IncomingRef {
    /// Entity containing the reference.
    pub source: EntityId,
    /// Attribute containing the reference.
    pub attribute: String,
}

impl IncomingRef {
    /// Construct one typed incoming link.
    pub fn new(source: EntityId, attribute: String) -> Self {
        Self { source, attribute }
    }
}

/// The limit that prevented a complete result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IncomingRefBudget {
    /// More than `max_links` distinct live links were established.
    Links,
    /// The complete set could not be established within `max_records`.
    Records,
}

/// A complete incoming-reference set, or exhaustion with no partial list.
/// Require `Complete` before staging a cascading delete; on exhaustion,
/// retry with larger limits or abandon the delete.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum IncomingRefs {
    /// Every live scalar reference, ignoring valid time. Order is unspecified.
    Complete {
        /// Distinct `(source, attribute)` pairs.
        links: Vec<IncomingRef>,
        /// Stored fact versions inspected across all layers.
        records_examined: usize,
    },
    /// Discovery could not finish within a limit. Contains no links.
    Exhausted {
        /// The exhausted limit.
        budget: IncomingRefBudget,
        /// Stored fact versions inspected, never above `max_records`.
        records_examined: usize,
    },
}

/// Positions only: no cloned facts, attributes or values. Empty for scalar
/// properties, keywords and sets; populated at write time, never at lookup.
#[derive(Default)]
pub(crate) struct RefPositions(HashMap<EntityId, Vec<usize>>);

impl RefPositions {
    pub(crate) fn insert(&mut self, fact: &Fact, pos: usize) {
        #[cfg(any(test, feature = "test-hooks"))]
        enumerated_many(1);
        if let Value::Ref(target) = fact.value {
            self.0
                .entry(target)
                .or_insert_with(|| Vec::with_capacity(1))
                .push(pos);
        }
    }

    pub(crate) fn visit(&self, target: EntityId, facts: &[Fact], lookup: &mut Lookup) -> bool {
        if let Some(positions) = self.0.get(&target) {
            let len = positions.partition_point(|&pos| pos < facts.len());
            let take = len.min(lookup.remaining());
            for &pos in positions.iter().take(take) {
                lookup.charge();
                enumerated();
                if let Some(f) = facts.get(pos) {
                    lookup.record(f.entity, f.attribute.clone(), f.tx_count, f.asserted);
                }
            }
            return take == len;
        }
        true
    }
}

pub(crate) struct Lookup {
    limits: IncomingRefLimits,
    examined: usize,
    layer: u8,
    winners: HashMap<(EntityId, String), (u8, u64, bool)>,
}

impl Lookup {
    pub(crate) fn new(limits: IncomingRefLimits) -> Self {
        Self {
            limits,
            examined: 0,
            layer: 0,
            winners: HashMap::new(),
        }
    }

    pub(crate) fn next_layer(&mut self) {
        self.layer += 1;
    }

    pub(crate) fn remaining(&self) -> usize {
        self.limits.max_records - self.examined
    }

    pub(crate) fn charge(&mut self) -> bool {
        if self.remaining() == 0 {
            return false;
        }
        self.examined += 1;
        true
    }

    pub(crate) fn record(&mut self, source: EntityId, attribute: String, tx: u64, asserted: bool) {
        let candidate = (self.layer, tx, asserted);
        let winner = self.winners.entry((source, attribute)).or_insert(candidate);
        if (candidate.0, candidate.1) > (winner.0, winner.1) {
            *winner = candidate;
        } else if (candidate.0, candidate.1) == (winner.0, winner.1) {
            winner.2 &= asserted;
        }
    }

    pub(crate) fn finish(self, complete: bool) -> IncomingRefs {
        let exhausted = |budget| IncomingRefs::Exhausted {
            budget,
            records_examined: self.examined,
        };
        if !complete {
            return exhausted(IncomingRefBudget::Records);
        }
        let mut links = Vec::new();
        for ((source, attribute), (_, _, asserted)) in self.winners {
            if asserted {
                if links.len() == self.limits.max_links {
                    return exhausted(IncomingRefBudget::Links);
                }
                links.push(IncomingRef { source, attribute });
            }
        }
        IncomingRefs::Complete {
            links,
            records_examined: self.examined,
        }
    }
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static ENUMERATION: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn enumerated_many(count: usize) {
    ENUMERATION.with(|n| {
        let (records, ends) = n.get();
        n.set((records + count, ends));
    });
}

// Separate from budget accounting: called at the actual fact/key access sites.
pub(crate) fn enumerated() {
    #[cfg(any(test, feature = "test-hooks"))]
    ENUMERATION.with(|n| {
        let (records, ends) = n.get();
        n.set((records + 1, ends));
    });
}

pub(crate) fn prefix_end() {
    #[cfg(any(test, feature = "test-hooks"))]
    ENUMERATION.with(|n| {
        let (records, ends) = n.get();
        n.set((records, ends + 1));
    });
}

#[cfg(any(test, feature = "test-hooks"))]
pub(crate) fn enumeration() -> (usize, usize) {
    ENUMERATION.with(|n| n.replace((0, 0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_refs_position_memory() {
        for targets in [1, 16, 10_000] {
            let mut index = RefPositions::default();
            for i in 0..10_000 {
                let fact = Fact::new(
                    EntityId::nil(),
                    ":a".into(),
                    Value::Ref(EntityId::from_u128(i % targets)),
                    0,
                );
                index.insert(&fact, usize::try_from(i).unwrap());
            }
            // Raw capacity estimate, excluding allocator metadata: std's
            // hash table uses power-of-two buckets plus control bytes.
            let buckets = index.0.capacity().next_power_of_two();
            let bytes = std::mem::size_of::<RefPositions>()
                + buckets * (std::mem::size_of::<(EntityId, Vec<usize>)>() + 1)
                + 16
                + index
                    .0
                    .values()
                    .map(|v| v.capacity() * std::mem::size_of::<usize>())
                    .sum::<usize>();
            println!(
                "ref positions: targets={targets}, facts=10000, capacity bytes={bytes}, bytes/ref={}",
                bytes as f64 / 10_000.0
            );
            if targets == 10_000 {
                assert!(index.0.values().all(|v| v.capacity() == 1));
            }
        }
        let mut index = RefPositions::default();
        index.insert(
            &Fact::new(EntityId::nil(), ":a".into(), Value::Integer(1), 0),
            0,
        );
        assert_eq!(index.0.capacity(), 0);
    }
}
