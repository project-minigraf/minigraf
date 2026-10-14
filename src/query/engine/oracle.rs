//! The existing executor as an oracle for the engine (#473 spec §9).

use crate::query::datalog::executor::QueryResult;
use crate::storage::index::encode_value;
use anyhow::Result;

/// Panic unless `engine` and `oracle` give the same answer: both errors, or
/// the same columns and the same rows as a bag. Messages hold counts and
/// column names only, never values.
///
/// Built only for tests (`cfg(test)`, the `test-oracle` feature), where a
/// panic is how a disagreement fails the test.
#[allow(clippy::panic)]
pub(crate) fn check(engine: &Result<QueryResult>, oracle: &Result<QueryResult>) {
    match (engine, oracle) {
        (Err(_), Err(_)) => {}
        (
            Ok(QueryResult::QueryResults {
                vars: v1,
                results: r1,
            }),
            Ok(QueryResult::QueryResults {
                vars: v2,
                results: r2,
            }),
        ) => {
            assert!(
                v1 == v2,
                "engine and oracle columns differ: {v1:?} vs {v2:?}"
            );
            assert!(
                bag(r1) == bag(r2),
                "engine and oracle rows differ for {v1:?}: {} vs {} rows",
                r1.len(),
                r2.len()
            );
        }
        (Ok(_), Err(_)) => panic!("engine answered where the oracle failed"),
        (Err(_), Ok(_)) => panic!("engine failed where the oracle answered"),
        (Ok(_), Ok(_)) => panic!("engine or oracle gave a non-query result"),
    }
}

fn bag(rows: &[Vec<crate::graph::types::Value>]) -> Vec<Vec<Vec<u8>>> {
    let mut bag: Vec<Vec<Vec<u8>>> = rows
        .iter()
        .map(|r| r.iter().map(encode_value).collect())
        .collect();
    bag.sort();
    bag
}
