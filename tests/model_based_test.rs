//! Model-based (stateful) test of the file-backed database (#385).
//!
//! Random sequences of transact, retract, write transactions, checkpoints and
//! reopens run against a `.graph` file in a temp dir. After every step, the
//! results of entity-, attribute-, value-bound and full-scan queries, under
//! `:as-of N` and `:valid-at T`, are compared with a plain reference model.
//!
//! Run: cargo test --test model_based_test
//! More cases: PROPTEST_CASES=500 cargo test --test model_based_test
//!
//! The driver and the checks live in `reference_model`, shared with the
//! `ops_sequence` fuzz target.
//!
//! Design: docs/superpowers/specs/2026-10-07-model-based-test-design.md
#![cfg(not(target_arch = "wasm32"))]

mod reference_model;

use proptest::prelude::*;
use reference_model::*;

// ── Strategies ────────────────────────────────────────────────────────────────

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => arb_transact().prop_map(Op::Exec),
        3 => arb_retract().prop_map(Op::Exec),
        2 => (
            prop::collection::vec(arb_stmt(), 0..4),
            any::<bool>(),
            prop::bool::weighted(0.8),
        )
            .prop_map(|(mut stmts, retract_own, commit)| {
                // Retract a triple this transaction asserts, before or after
                // the assertion: the later statement wins (#477).
                let own = stmts.iter().find_map(|s| match s {
                    Stmt::Transact { facts, .. } => Some(facts[0].clone()),
                    Stmt::Retract { .. } => None,
                });
                if let (true, Some(f)) = (retract_own, own) {
                    let retract = Stmt::Retract { facts: vec![(f.e, f.a, f.v)] };
                    if stmts.len() % 2 == 0 {
                        stmts.push(retract);
                    } else {
                        stmts.insert(0, retract);
                    }
                }
                Op::WriteTx { stmts, commit }
            }),
        2 => Just(Op::Checkpoint),
        2 => prop_oneof![
            Just(OpenMode::Default),
            Just(OpenMode::SmallCache),
            Just(OpenMode::AutoCheckpoint),
        ]
        .prop_map(Op::Reopen),
        2 => prop_oneof![
            Just(OpenMode::Default),
            Just(OpenMode::SmallCache),
            Just(OpenMode::AutoCheckpoint),
        ]
        .prop_map(Op::Crash),
    ]
}

fn arb_probe() -> impl Strategy<Value = Probe> {
    (
        any::<u16>(),
        0..AT_YEARS.len(),
        0..ENTITIES,
        0..ATTRS.len(),
        arb_v(),
    )
        .prop_map(|(as_of, at, e, a, v)| Probe {
            as_of,
            at,
            e,
            a,
            v,
            pick: 0,
        })
}

fn arb_steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(
        (arb_op(), arb_probe()).prop_map(|(op, probe)| Step { op, probe }),
        1..50,
    )
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Every read after every step matches the reference model.
    #[test]
    fn file_backed_db_matches_model(steps in arb_steps()) {
        run(&steps, Checks::All)?;
    }
}
