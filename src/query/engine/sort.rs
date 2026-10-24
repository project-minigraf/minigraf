//! `Sort` (#473 spec §6.2, #306 spec §4.2): the rows of its child in the
//! full order of `:order-by`, as the `:find` columns.
//!
//! It holds each row's values of those columns, which the full order
//! compares, and turns them back into cells only for the rows it returns.
//! Under a `:limit` (`keep = k`) it holds at most `k` rows, in a binary heap
//! whose top is the worst row held, so its memory follows `k`, not the
//! answer. Either way it reads its whole input; skipping that when an index
//! returns the rows in order is #641.

use super::batch::Batch;
use super::cell::{Cell, Scalar};
use super::meter::{Held, Meter, growth, value_bytes};
use super::project::value;
use super::{BATCH_ROWS, Counters, Ctx, Operator};
use crate::graph::types::Value;
use crate::query::datalog::aggregate::row_order;
use anyhow::Result;
use std::cmp::Ordering;
use std::mem::size_of;
use std::sync::Arc;

pub(crate) struct Sort {
    child: Box<dyn Operator>,
    /// The slots of the `:find` columns, in `:find` order.
    cols: Vec<usize>,
    /// The full order, as `(column, descending)` over `cols`' positions.
    order: Vec<(usize, bool)>,
    /// The most rows to hold, when a `:limit` follows.
    keep: Option<usize>,
    /// The rows held: a heap, worst on top, when `keep` is given.
    rows: Vec<Vec<Value>>,
    /// The sorted rows, last first, once the input has ended.
    out: Option<Vec<Vec<Value>>>,
    held: Held,
    counters: Counters,
}

/// The bytes one held row counts.
fn bytes(row: &[Value]) -> usize {
    size_of::<Vec<Value>>() + row.iter().map(value_bytes).sum::<usize>()
}

impl Sort {
    pub(crate) fn new(
        child: Box<dyn Operator>,
        cols: Vec<usize>,
        order: Vec<(usize, bool)>,
        keep: Option<usize>,
        meter: &Arc<Meter>,
    ) -> Self {
        Sort {
            child,
            cols,
            order,
            keep,
            rows: Vec::new(),
            out: None,
            held: Held::new(meter, "Sort"),
            counters: Counters::new("Sort"),
        }
    }

    fn cmp(&self, a: &[Value], b: &[Value]) -> Ordering {
        row_order(a, b, &self.order)
    }

    /// Hold each row of `batch`, or, at `keep` rows, those better than the
    /// worst held.
    fn take(&mut self, batch: &Batch, ctx: &mut Ctx<'_>) -> Result<()> {
        let null = Cell::Val(Scalar::Null);
        for i in 0..batch.len {
            let row = self
                .cols
                .iter()
                .map(|&s| {
                    value(
                        batch.cols.get(s).and_then(|c| c.get(i)).unwrap_or(&null),
                        ctx,
                    )
                })
                .collect::<Result<Vec<Value>>>()?;
            if let Some(k) = self.keep
                && self.rows.len() >= k
            {
                // Full: replace the worst row held, if this one is better.
                let better = self
                    .rows
                    .first()
                    .is_some_and(|top| self.cmp(&row, top).is_lt());
                if better {
                    self.held.charge(bytes(&row))?;
                    if let Some(top) = self.rows.first_mut() {
                        let old = std::mem::replace(top, row);
                        self.held.uncharge(bytes(&old));
                    }
                    self.sift_down(0);
                }
                continue;
            }
            let n = bytes(&row);
            self.held.charge(n)?;
            let refused = growth(self.rows.capacity(), size_of::<Vec<Value>>());
            self.held.reserve(self.rows.try_reserve(1), n, refused)?;
            self.rows.push(row);
            if self.keep.is_some() {
                self.sift_up(self.rows.len() - 1);
            }
        }
        Ok(())
    }

    /// Whether held row `i` comes after held row `j` (both exist).
    fn after(&self, i: usize, j: usize) -> bool {
        match (self.rows.get(i), self.rows.get(j)) {
            (Some(a), Some(b)) => self.cmp(a, b).is_gt(),
            _ => false,
        }
    }

    /// Restore the heap (worst on top) above `i`.
    fn sift_up(&mut self, mut i: usize) {
        while i > 0 {
            let parent = (i - 1) / 2;
            if !self.after(i, parent) {
                break;
            }
            self.rows.swap(i, parent);
            i = parent;
        }
    }

    /// Restore the heap (worst on top) below `i`.
    fn sift_down(&mut self, mut i: usize) {
        loop {
            let mut worst = i;
            for child in [2 * i + 1, 2 * i + 2] {
                if self.after(child, worst) {
                    worst = child;
                }
            }
            if worst == i {
                break;
            }
            self.rows.swap(i, worst);
            i = worst;
        }
    }
}

impl Operator for Sort {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        if self.out.is_none() {
            if self.keep == Some(0) {
                self.child.close();
            } else {
                while let Some(batch) = self.child.next_batch(ctx)? {
                    self.counters.rows_in += batch.len as u64;
                    self.take(&batch, ctx)?;
                    // The rows held are values, so the DICT lookups that made
                    // them are not needed again: kept, they would grow with
                    // the input, not with `keep`.
                    ctx.ids.end_pull();
                }
            }
            let mut rows = std::mem::take(&mut self.rows);
            let order = &self.order;
            // Rows equal under the full order are equal in every column, so
            // an unstable sort gives the one order; last first, to pop.
            rows.sort_unstable_by(|a, b| row_order(b, a, order));
            self.out = Some(rows);
        }
        let Some(rows) = self.out.as_mut().filter(|r| !r.is_empty()) else {
            return Ok(None);
        };
        let mut batch = Batch::new(self.cols.len());
        while batch.len < BATCH_ROWS
            && let Some(row) = rows.pop()
        {
            self.held.uncharge(bytes(&row));
            let mut cells = row
                .iter()
                .map(|v| Cell::of_value(ctx.ids, ctx.keys, v).map(Some))
                .collect::<Result<Vec<Option<Cell>>>>()?;
            batch.push(&mut cells);
        }
        self.counters.out(&batch);
        Ok(Some(batch))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.rows.clear();
        self.out = None;
        self.held.release();
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.rows = Vec::new();
        self.out = Some(Vec::new());
        self.held.release();
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(Counters {
            bytes_held: self.held.bytes() as u64,
            ..self.counters
        });
        self.child.counters(out);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::query::engine::Stream;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    /// The rows `rows`, two integer columns, in full batches; counts the
    /// rows it returns and its closes.
    struct Rows {
        rows: Vec<(i64, i64)>,
        next: usize,
        sent: Arc<AtomicUsize>,
        closes: Arc<AtomicUsize>,
    }

    impl Operator for Rows {
        fn next_batch(&mut self, _: &mut Ctx<'_>) -> Result<Option<Batch>> {
            if self.next >= self.rows.len() {
                return Ok(None);
            }
            let mut batch = Batch::new(2);
            while batch.len < BATCH_ROWS && self.next < self.rows.len() {
                let (a, b) = self.rows[self.next];
                batch.push(&mut [
                    Some(Cell::Val(Scalar::Int(a))),
                    Some(Cell::Val(Scalar::Int(b))),
                ]);
                self.next += 1;
            }
            self.sent.fetch_add(batch.len, AtomicOrdering::SeqCst);
            Ok(Some(batch))
        }

        fn rewind(&mut self, _: &[Cell]) {
            self.next = 0;
        }

        fn close(&mut self) {
            self.next = self.rows.len();
            self.closes.fetch_add(1, AtomicOrdering::SeqCst);
        }

        fn counters(&self, _: &mut Vec<Counters>) {}
    }

    /// `n` rows `(i * 7919 % n, i % 3)`: the first column a permutation of
    /// `0..n` when `n` is not a multiple of 7919.
    fn shuffled(n: usize) -> Vec<(i64, i64)> {
        (0..n)
            .map(|i| ((i * 7919 % n) as i64, (i % 3) as i64))
            .collect()
    }

    struct Run {
        rows: Vec<i64>,
        sent: usize,
        closes: usize,
        peak: usize,
    }

    fn run(
        input: Vec<(i64, i64)>,
        order: Vec<(usize, bool)>,
        keep: Option<usize>,
        meter: Arc<Meter>,
    ) -> Result<Run> {
        let (sent, closes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let child = Rows {
            rows: input,
            next: 0,
            sent: sent.clone(),
            closes: closes.clone(),
        };
        let snapshot = crate::graph::storage::FactStorage::new()
            .snapshot()
            .unwrap();
        let sort = Sort::new(Box::new(child), vec![0, 1], order, keep, &meter);
        let mut stream = Stream::for_test(snapshot, Box::new(sort));
        let mut rows = Vec::new();
        while let Some(b) = stream.next_rows()? {
            for row in b {
                match row.as_slice() {
                    // `for_test` projects the first column.
                    [Value::Integer(a)] => rows.push(*a),
                    _ => panic!("one integer column"),
                }
            }
        }
        Ok(Run {
            rows,
            sent: sent.load(AtomicOrdering::SeqCst),
            closes: closes.load(AtomicOrdering::SeqCst),
            peak: meter.peak(),
        })
    }

    /// The expected first column: `input` sorted by `order`, first `keep`.
    fn expected(input: &[(i64, i64)], order: &[(usize, bool)], keep: Option<usize>) -> Vec<i64> {
        let mut rows: Vec<Vec<Value>> = input
            .iter()
            .map(|&(a, b)| vec![Value::Integer(a), Value::Integer(b)])
            .collect();
        rows.sort_by(|a, b| row_order(a, b, order));
        rows.truncate(keep.unwrap_or(usize::MAX));
        rows.iter()
            .map(|r| match r.first() {
                Some(Value::Integer(a)) => *a,
                _ => unreachable!(),
            })
            .collect()
    }

    fn firsts(run: &Run) -> Vec<i64> {
        run.rows.clone()
    }

    #[test]
    fn sorts_every_row() {
        let input = shuffled(5000);
        for order in [vec![(0, false), (1, false)], vec![(0, true), (1, false)]] {
            let r = run(input.clone(), order.clone(), None, Meter::unlimited()).unwrap();
            assert_eq!(firsts(&r), expected(&input, &order, None));
            assert_eq!(r.rows.len(), 5000);
        }
    }

    #[test]
    fn top_k_keeps_the_first_k_of_the_full_sort() {
        let input = shuffled(5000);
        for order in [vec![(0, false), (1, false)], vec![(0, true), (1, false)]] {
            for k in [1, 10, 1024, 4999, 5000, 6000] {
                let r = run(input.clone(), order.clone(), Some(k), Meter::unlimited()).unwrap();
                assert_eq!(firsts(&r), expected(&input, &order, Some(k)), "k {k}");
            }
        }
    }

    #[test]
    fn ties_fall_to_the_next_column() {
        // Order by the second column descending, then the first ascending.
        let input = shuffled(300);
        let order = vec![(1, true), (0, false)];
        for keep in [None, Some(7), Some(150)] {
            let r = run(input.clone(), order.clone(), keep, Meter::unlimited()).unwrap();
            assert_eq!(firsts(&r), expected(&input, &order, keep), "{keep:?}");
        }
    }

    #[test]
    fn keep_zero_pulls_nothing() {
        let r = run(
            shuffled(5000),
            vec![(0, false)],
            Some(0),
            Meter::unlimited(),
        )
        .unwrap();
        assert!(r.rows.is_empty());
        assert_eq!(r.sent, 0, "nothing pulled");
        assert!(r.closes >= 1, "and the child closed");
    }

    #[test]
    fn top_k_holds_k_rows() {
        let order = vec![(0, true), (1, false)];
        let small = run(shuffled(1000), order.clone(), Some(10), Meter::unlimited()).unwrap();
        let large = run(
            shuffled(100_001),
            order.clone(),
            Some(10),
            Meter::unlimited(),
        )
        .unwrap();
        assert_eq!(small.peak, large.peak, "memory follows k, not the input");
        // `k` rows, and the one replacing the worst while both are counted.
        assert!(large.peak <= 11 * bytes(&[Value::Integer(0), Value::Integer(0)]));
        let all = run(shuffled(100_001), order, None, Meter::unlimited()).unwrap();
        assert!(all.peak > 1000 * large.peak, "a full sort holds every row");
    }

    #[test]
    fn over_the_limit_is_qry_012() {
        let order = vec![(0, false)];
        let row = bytes(&[Value::Integer(0), Value::Integer(0)]);
        let e = run(shuffled(5000), order.clone(), None, Meter::new(100 * row))
            .err()
            .expect("a full sort over the limit");
        assert_eq!(crate::MinigrafError::from(e).code(), "QRY-012");
        let e = run(
            shuffled(5000),
            order.clone(),
            Some(200),
            Meter::new(100 * row),
        )
        .err()
        .expect("a top-k over the limit");
        assert_eq!(crate::MinigrafError::from(e).code(), "QRY-012");
        run(shuffled(5000), order, Some(50), Meter::new(100 * row)).unwrap();
    }

    #[test]
    fn rewind_sorts_again() {
        let (sent, closes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let child = Rows {
            rows: shuffled(100),
            next: 50,
            sent,
            closes,
        };
        let meter = Meter::unlimited();
        let mut sort = Sort::new(
            Box::new(child),
            vec![0, 1],
            vec![(0, true)],
            Some(3),
            &meter,
        );
        sort.out = Some(Vec::new());
        sort.rows.push(vec![Value::Integer(-1)]);
        sort.held.charge(10).unwrap();
        sort.rewind(&[]);
        assert!(sort.out.is_none() && sort.rows.is_empty(), "rewound");
        assert_eq!(meter.used(), 0, "and released");
        let snapshot = crate::graph::storage::FactStorage::new()
            .snapshot()
            .unwrap();
        let mut stream = Stream::for_test(snapshot, Box::new(sort));
        let mut got = Vec::new();
        while let Some(b) = stream.next_rows().unwrap() {
            for row in b {
                if let Some(Value::Integer(n)) = row.first() {
                    got.push(*n);
                }
            }
        }
        assert_eq!(got, [99, 98, 97], "the child read from its start");
        assert_eq!(meter.used(), 0, "every row released as returned");
    }
}
