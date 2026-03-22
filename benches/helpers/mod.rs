// Shared benchmark fixtures. Included via `mod helpers;` in minigraf_bench.rs.
use minigraf::{Minigraf, OpenOptions};
use std::sync::Arc;

/// Placeholder — populated in Task 2.
pub fn populate_in_memory(_n: usize) -> Arc<Minigraf> {
    Arc::new(Minigraf::in_memory().unwrap())
}
