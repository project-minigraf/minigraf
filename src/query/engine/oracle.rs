//! The existing executor as an oracle for the engine (#473 spec §9).

use crate::error::{CodedError, ErrorCode};
use crate::query::datalog::executor::QueryResult;
use crate::storage::index::encode_value;
use anyhow::Result;
use std::cell::Cell;

thread_local! {
    /// Whether [`check`] is skipped on this thread ([`exempt`]).
    static EXEMPT: Cell<bool> = const { Cell::new(false) };
}

/// Skip [`check`] on this thread until the guard drops: only for the answers
/// the engine changes on purpose, whose tests check both answers (#551 spec
/// §4, plan decision 10).
#[cfg(test)]
pub(crate) fn exempt() -> Exempt {
    EXEMPT.with(|e| e.set(true));
    Exempt(())
}

#[cfg(test)]
pub(crate) struct Exempt(());

#[cfg(test)]
impl Drop for Exempt {
    fn drop(&mut self) {
        EXEMPT.with(|e| e.set(false));
    }
}

/// Panic unless `engine` and `oracle` give the same answer: both errors, or
/// the same columns and the same rows as a bag. An engine `QRY-012` agrees
/// with any answer, as the executor does not count memory (#549). Messages
/// hold counts and column names only, never values.
///
/// Built only for tests (`cfg(test)`, the `test-oracle` feature), where a
/// panic is how a disagreement fails the test.
#[allow(clippy::panic)]
pub(crate) fn check(engine: &Result<QueryResult>, oracle: &Result<QueryResult>) {
    if EXEMPT.with(Cell::get) {
        return;
    }
    match (engine, oracle) {
        (Err(_), Err(_)) => {}
        (Err(e), Ok(_))
            if e.downcast_ref::<CodedError>().map(CodedError::code) == Some(ErrorCode::Qry012) => {}
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
