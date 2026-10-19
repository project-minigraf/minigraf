//! `Seed` (#546): the one row a correlated body starts from, the cells it
//! imports from the outer row.

use super::batch::Batch;
use super::cell::Cell;
use super::{Counters, Ctx, Operator};
use anyhow::Result;

pub(crate) struct Seed {
    row: Vec<Cell>,
    /// Whether the row has been returned since the last rewind.
    done: bool,
    counters: Counters,
}

impl Seed {
    /// A seed with no row until the first rewind.
    pub(crate) fn new() -> Self {
        Seed {
            row: Vec::new(),
            done: true,
            counters: Counters::new("Seed"),
        }
    }
}

impl Operator for Seed {
    fn next_batch(&mut self, _: &mut Ctx<'_>) -> Result<Option<Batch>> {
        if self.done {
            return Ok(None);
        }
        self.done = true;
        let mut batch = Batch::new(self.row.len());
        let mut row: Vec<Option<Cell>> = self.row.iter().cloned().map(Some).collect();
        batch.push(&mut row);
        self.counters.out(&batch);
        Ok(Some(batch))
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.row.clear();
        self.row.extend_from_slice(seed);
        self.done = false;
    }

    fn close(&mut self) {
        self.done = true;
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
    }
}
