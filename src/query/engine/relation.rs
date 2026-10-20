//! The relations of a rule query (#548 spec §5.1): per evaluated predicate,
//! the facts its rules derive, as rows of two cells (entity, value).
//!
//! Rows are appended at the end of each round, so the relation as of a
//! round is a prefix of its rows and the round's delta a range of them:
//! `Old` is `..start`, `Delta` is `start..end`, `Full` is `..end`. A row is
//! new when no stored fact and no earlier row has the same entity and value
//! (spec §3.2), as [`JoinKey::ident`] compares cells.

use super::cell::{Cell, JoinKey};
use std::collections::{HashMap, HashSet};

/// Which rows of a relation a scan reads (spec §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Range {
    /// Stored facts and every derived row so far.
    Full,
    /// Stored facts and the rows derived before the last round.
    Old,
    /// The rows the last round derived, and no stored fact.
    Delta,
}

/// The derived rows a scan reads after the stored facts: those of the
/// relations `rels` (indexes into [`Relations`]), in `range`.
#[derive(Debug, Clone)]
pub(crate) struct Derived {
    pub(crate) rels: Vec<usize>,
    pub(crate) range: Range,
}

pub(crate) struct Relations {
    rels: Vec<Rel>,
}

struct Rel {
    /// The id of the attribute `:p` in the query's id space.
    iid: u64,
    rows: Vec<[Cell; 2]>,
    /// Stored facts and rows, and the rows of the round under way.
    ident: HashSet<[JoinKey; 2]>,
    /// Row positions by entity id.
    by_entity: HashMap<u64, Vec<usize>>,
    /// The rows of the round under way, appended at its end.
    next: Vec<[Cell; 2]>,
    start: usize,
    end: usize,
}

impl Relations {
    /// Empty relations, one per attribute id in `iids`.
    pub(crate) fn new(iids: impl IntoIterator<Item = u64>) -> Self {
        Relations {
            rels: iids
                .into_iter()
                .map(|iid| Rel {
                    iid,
                    rows: Vec::new(),
                    ident: HashSet::new(),
                    by_entity: HashMap::new(),
                    next: Vec::new(),
                    start: 0,
                    end: 0,
                })
                .collect(),
        }
    }

    /// The attribute id of relation `rel`.
    pub(crate) fn iid(&self, rel: usize) -> Option<u64> {
        self.rels.get(rel).map(|r| r.iid)
    }

    /// Record a stored fact of relation `rel`: a row equal to it is not new.
    pub(crate) fn know(&mut self, rel: usize, row: &[Cell; 2]) {
        if let Some(r) = self.rels.get_mut(rel) {
            r.ident.insert(key(row));
        }
    }

    /// Whether `row` is already a stored fact or a row of `rel`, this
    /// round's included.
    pub(crate) fn known(&self, rel: usize, row: &[Cell; 2]) -> bool {
        self.rels
            .get(rel)
            .is_none_or(|r| r.ident.contains(&key(row)))
    }

    /// Add `row` to `rel` at the end of the round, unless it is known.
    /// Returns whether it was new.
    pub(crate) fn add(&mut self, rel: usize, row: [Cell; 2]) -> bool {
        let Some(r) = self.rels.get_mut(rel) else {
            return false;
        };
        if !r.ident.insert(key(&row)) {
            return false;
        }
        r.next.push(row);
        true
    }

    /// End a round: append every relation's new rows, which become the
    /// delta. Returns whether any row was appended.
    pub(crate) fn end_round(&mut self) -> bool {
        let mut any = false;
        for r in &mut self.rels {
            r.start = r.end;
            any |= !r.next.is_empty();
            for row in r.next.drain(..) {
                if let Cell::Eid(e) = row[0] {
                    r.by_entity.entry(e).or_default().push(r.rows.len());
                }
                r.rows.push(row);
            }
            r.end = r.rows.len();
        }
        any
    }

    /// The positions of `rel`'s rows in `range`.
    pub(crate) fn span(&self, rel: usize, range: Range) -> std::ops::Range<usize> {
        let Some(r) = self.rels.get(rel) else {
            return 0..0;
        };
        match range {
            Range::Full => 0..r.end,
            Range::Old => 0..r.start,
            Range::Delta => r.start..r.end,
        }
    }

    /// The positions of `rel`'s rows whose entity is `eid`, in any range.
    pub(crate) fn of_entity(&self, rel: usize, eid: u64) -> &[usize] {
        self.rels
            .get(rel)
            .and_then(|r| r.by_entity.get(&eid))
            .map_or(&[], Vec::as_slice)
    }

    pub(crate) fn row(&self, rel: usize, at: usize) -> Option<&[Cell; 2]> {
        self.rels.get(rel).and_then(|r| r.rows.get(at))
    }

    /// Rows held, for tests.
    #[cfg(test)]
    pub(crate) fn len(&self, rel: usize) -> usize {
        self.rels.get(rel).map_or(0, |r| r.rows.len())
    }
}

fn key(row: &[Cell; 2]) -> [JoinKey; 2] {
    [JoinKey::ident(&row[0]), JoinKey::ident(&row[1])]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::engine::cell::Scalar;

    fn row(e: u64, v: Scalar) -> [Cell; 2] {
        [Cell::Eid(e), Cell::Val(v)]
    }

    /// Spec §3.2: kinds distinct, `-0.0` distinct from `0.0`, one NaN, a
    /// keyword distinct from the entity it names.
    #[test]
    fn identity_of_rows() {
        let mut r = Relations::new([7]);
        assert!(r.add(0, row(1, Scalar::Float(0.0))));
        assert!(r.add(0, row(1, Scalar::Float(-0.0))), "-0.0 is new");
        assert!(!r.add(0, row(1, Scalar::Float(0.0))), "0.0 again");
        assert!(r.add(0, row(1, Scalar::Float(f64::NAN))));
        assert!(!r.add(0, row(1, Scalar::Float(-f64::NAN))), "one NaN");
        assert!(r.add(0, row(1, Scalar::Int(0))), "integer is not float");
        assert!(r.add(0, [Cell::Eid(1), Cell::Iid(5)]));
        assert!(r.add(0, [Cell::Eid(1), Cell::Eid(5)]), "ref is not keyword");
        r.know(0, &row(2, Scalar::Bool(true)));
        assert!(!r.add(0, row(2, Scalar::Bool(true))), "stored fact");
        assert!(r.known(0, &row(1, Scalar::Int(0))));
    }

    #[test]
    fn rounds_give_old_delta_and_full() {
        let mut r = Relations::new([7]);
        r.add(0, row(1, Scalar::Int(1)));
        assert_eq!(r.span(0, Range::Full), 0..0, "not before the round ends");
        assert!(r.end_round());
        assert_eq!(r.span(0, Range::Delta), 0..1);
        r.add(0, row(1, Scalar::Int(2)));
        r.add(0, row(2, Scalar::Int(3)));
        assert!(r.end_round());
        assert_eq!(r.span(0, Range::Old), 0..1);
        assert_eq!(r.span(0, Range::Delta), 1..3);
        assert_eq!(r.span(0, Range::Full), 0..3);
        assert_eq!(r.of_entity(0, 1), &[0, 1]);
        assert!(!r.end_round());
        assert_eq!(r.span(0, Range::Delta), 3..3);
        assert_eq!(r.span(0, Range::Old), 0..3);
        assert_eq!(r.len(0), 3);
        assert_eq!(r.iid(0), Some(7));
    }
}
