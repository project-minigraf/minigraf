//! Columnar batches of rows (#473 spec §5.1).

use super::cell::Cell;

/// At most [`BATCH_ROWS`](super::BATCH_ROWS) rows, one column per variable
/// slot of the plan.
#[derive(Debug)]
pub(crate) struct Batch {
    pub(crate) cols: Vec<Vec<Cell>>,
    pub(crate) len: usize,
}

impl Batch {
    pub(crate) fn new(width: usize) -> Self {
        Batch {
            cols: vec![Vec::new(); width],
            len: 0,
        }
    }

    /// Append one row. Every slot must be filled.
    pub(crate) fn push(&mut self, row: &mut [Option<Cell>]) {
        for (col, cell) in self.cols.iter_mut().zip(row.iter_mut()) {
            if let Some(cell) = cell.take() {
                col.push(cell);
            }
        }
        self.len += 1;
    }
}
