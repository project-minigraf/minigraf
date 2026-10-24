//! `Limit` (#473 spec §6.1, #306 spec §4.1): skips the first `offset` rows
//! of its child, passes on the next `limit`, then closes the child, so the
//! operators below it read nothing more.

use super::batch::Batch;
use super::cell::Cell;
use super::{Counters, Ctx, Operator};
use anyhow::Result;

pub(crate) struct Limit {
    child: Box<dyn Operator>,
    offset: usize,
    limit: Option<usize>,
    /// Rows still to skip.
    skip: usize,
    /// Rows still to pass on; `None` passes on every row.
    left: Option<usize>,
    counters: Counters,
}

impl Limit {
    pub(crate) fn new(child: Box<dyn Operator>, offset: usize, limit: Option<usize>) -> Self {
        Limit {
            child,
            offset,
            limit,
            skip: offset,
            left: limit,
            counters: Counters::new("Limit"),
        }
    }
}

impl Operator for Limit {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        loop {
            if self.left == Some(0) {
                self.child.close();
                return Ok(None);
            }
            let Some(mut batch) = self.child.next_batch(ctx)? else {
                return Ok(None);
            };
            self.counters.rows_in += batch.len as u64;
            let skip = self.skip.min(batch.len);
            self.skip -= skip;
            let take = self
                .left
                .map_or(batch.len - skip, |l| l.min(batch.len - skip));
            if let Some(l) = &mut self.left {
                *l -= take;
            }
            batch.slice(skip, take);
            if batch.len > 0 {
                if self.left == Some(0) {
                    self.child.close();
                }
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.skip = self.offset;
        self.left = self.limit;
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.child.counters(out);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::graph::types::Value;
    use crate::query::engine::cell::Scalar;
    use crate::query::engine::{BATCH_ROWS, Stream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Rows `0..rows` in full batches; counts the rows it returns and its
    /// closes.
    struct Numbers {
        rows: usize,
        next: usize,
        sent: Arc<AtomicUsize>,
        closes: Arc<AtomicUsize>,
    }

    impl Operator for Numbers {
        fn next_batch(&mut self, _: &mut Ctx<'_>) -> Result<Option<Batch>> {
            if self.next >= self.rows {
                return Ok(None);
            }
            let mut batch = Batch::new(1);
            while batch.len < BATCH_ROWS && self.next < self.rows {
                batch.push(&mut [Some(Cell::Val(Scalar::Int(self.next as i64)))]);
                self.next += 1;
            }
            self.sent.fetch_add(batch.len, Ordering::SeqCst);
            Ok(Some(batch))
        }

        fn rewind(&mut self, _: &[Cell]) {
            self.next = 0;
        }

        fn close(&mut self) {
            self.next = self.rows;
            self.closes.fetch_add(1, Ordering::SeqCst);
        }

        fn counters(&self, _: &mut Vec<Counters>) {}
    }

    struct Run {
        rows: Vec<i64>,
        sent: usize,
        closes: usize,
    }

    fn run(rows: usize, offset: usize, limit: Option<usize>) -> Run {
        let (sent, closes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let child = Numbers {
            rows,
            next: 0,
            sent: sent.clone(),
            closes: closes.clone(),
        };
        let snapshot = crate::graph::storage::FactStorage::new()
            .snapshot()
            .unwrap();
        let mut stream = Stream::for_test(
            snapshot,
            Box::new(Limit::new(Box::new(child), offset, limit)),
        );
        let mut out = Vec::new();
        while let Some(batch) = stream.next_rows().unwrap() {
            assert!(!batch.is_empty(), "never an empty batch");
            for row in batch {
                match row.as_slice() {
                    [Value::Integer(n)] => out.push(*n),
                    _ => panic!("one integer per row"),
                }
            }
        }
        Run {
            rows: out,
            sent: sent.load(Ordering::SeqCst),
            closes: closes.load(Ordering::SeqCst),
        }
    }

    #[test]
    fn passes_on_at_most_limit_rows() {
        for n in [0, 1, 1023, 1024, 1025, 5000, 6000] {
            let r = run(5000, 0, Some(n));
            assert_eq!(
                r.rows,
                (0..n.min(5000) as i64).collect::<Vec<_>>(),
                "limit {n}"
            );
        }
        let r = run(5000, 0, None);
        assert_eq!(r.rows.len(), 5000, "no limit");
    }

    #[test]
    fn stops_reading_once_it_has_its_rows() {
        let r = run(5000, 0, Some(0));
        assert_eq!(r.sent, 0, "limit 0 pulls nothing");
        assert!(r.closes >= 1, "and closes the child");

        let r = run(5000, 0, Some(10));
        assert_eq!(r.sent, BATCH_ROWS, "one batch for 10 rows");
        assert!(r.closes >= 1);

        let r = run(5000, 0, Some(BATCH_ROWS + 1));
        assert_eq!(r.sent, 2 * BATCH_ROWS, "two batches for one more row");
    }

    #[test]
    fn skips_offset_rows_first() {
        assert_eq!(run(5000, 3, Some(2)).rows, [3, 4]);
        assert_eq!(run(5000, 1023, Some(3)).rows, [1023, 1024, 1025]);
        assert_eq!(run(5000, 4999, Some(5)).rows, [4999]);
        assert!(run(5000, 5000, Some(5)).rows.is_empty());
        assert!(run(5000, 9000, None).rows.is_empty());
        assert_eq!(run(5000, 4990, None).rows.len(), 10);
    }

    #[test]
    fn rewind_starts_again() {
        let (sent, closes) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let child = Numbers {
            rows: 100,
            next: 0,
            sent,
            closes,
        };
        let mut limit = Limit::new(Box::new(child), 2, Some(3));
        limit.skip = 0;
        limit.left = Some(0);
        limit.rewind(&[]);
        assert_eq!(
            (limit.skip, limit.left),
            (2, Some(3)),
            "offset and limit restored"
        );
        let snapshot = crate::graph::storage::FactStorage::new()
            .snapshot()
            .unwrap();
        let mut stream = Stream::for_test(snapshot, Box::new(limit));
        let mut n = 0;
        while let Some(b) = stream.next_rows().unwrap() {
            n += b.len();
        }
        assert_eq!(n, 3, "and the child read again from its start");
    }
}
