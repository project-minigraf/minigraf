//! Differential fuzzing of storage operations against the reference model.
//!
//! The input decodes (`arbitrary::Unstructured`) into at most 32 steps:
//! transact, retract, a write transaction (commit or rollback), checkpoint,
//! reopen and crash (open a copy of the file and WAL), each followed by a
//! probe. The driver and the oracle are the model-based test's
//! (`tests/reference_model`): after every step, the full rows now and at a
//! fuzzer-chosen `:as-of`, and one fuzzer-chosen query shape, must match the
//! model, and a statement the model rejects must fail with its error code. A
//! mismatch panics with the step number; messages carry pool indices, never a
//! `Uuid`.
//!
//! The values, attributes and valid-time grid are the model's small pools, so
//! statements hit each other often.
#![no_main]
use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

#[path = "../../tests/reference_model/mod.rs"]
mod reference_model;
use reference_model::*;

const MAX_STEPS: usize = 32;

/// A choice weighted like the proptest strategies' `prop_oneof!`.
fn weighted(u: &mut Unstructured, weights: &[u32]) -> Result<usize> {
    let total: u32 = weights.iter().sum();
    let mut n = u.int_in_range(0..=total - 1)?;
    for (i, w) in weights.iter().enumerate() {
        if n < *w {
            return Ok(i);
        }
        n -= w;
    }
    Ok(weights.len() - 1)
}

fn v(u: &mut Unstructured) -> Result<V> {
    Ok(match weighted(u, &[3, 1, 1, 1, 1, 3, 2])? {
        0 => V::Int(u.int_in_range(0..=2)?),
        1 => V::Int(i64::MIN),
        2 => V::Bool(u.arbitrary()?),
        3 => V::Kw,
        4 => V::Float,
        5 => V::Str(u.int_in_range(0..=5)?),
        _ => V::Ref(u.int_in_range(0..=ENTITIES - 1)?),
    })
}

fn grid(u: &mut Unstructured) -> Result<Option<u8>> {
    Ok(if u.arbitrary()? {
        Some(u.int_in_range(0..=3)?)
    } else {
        None
    })
}

fn window(u: &mut Unstructured) -> Result<Window> {
    Ok(if weighted(u, &[3, 2])? == 0 {
        Window::default()
    } else {
        Window {
            vf: grid(u)?,
            vt: grid(u)?,
        }
    })
}

/// Often the same hot triple, so one triple builds a long history.
fn triple(u: &mut Unstructured) -> Result<(usize, usize, V)> {
    Ok(if weighted(u, &[2, 1])? == 0 {
        (
            u.int_in_range(0..=ENTITIES - 1)?,
            u.int_in_range(0..=ATTRS.len() - 1)?,
            v(u)?,
        )
    } else {
        (0, 0, V::Int(0))
    })
}

/// Mirrors `arb_transact`: sometimes a triple twice (an identical repeat or an
/// API-011 rejection), and usually an empty or inverted window rewritten to
/// start in 2000 (otherwise an API-019 rejection).
fn transact(u: &mut Unstructured) -> Result<Stmt> {
    let tx_window = window(u)?;
    let mut facts = Vec::new();
    for _ in 0..u.int_in_range(1..=5)? {
        let (e, a, v) = triple(u)?;
        facts.push(FactSpec {
            e,
            a,
            v,
            window: window(u)?,
        });
    }
    if u.ratio(15, 100)? {
        let mut again = facts[0].clone();
        again.window = window(u)?;
        facts.push(again);
    }
    if u.ratio(9, 10)? {
        for f in &mut facts {
            if !window_is_valid(effective(tx_window, f.window)) {
                f.window.vf = Some(0);
            }
        }
    }
    Ok(Stmt::Transact {
        window: tx_window,
        facts,
    })
}

fn retract(u: &mut Unstructured) -> Result<Stmt> {
    let mut facts = Vec::new();
    for _ in 0..u.int_in_range(1..=3)? {
        facts.push(triple(u)?);
    }
    Ok(Stmt::Retract { facts })
}

fn stmt(u: &mut Unstructured) -> Result<Stmt> {
    if weighted(u, &[3, 2])? == 0 {
        transact(u)
    } else {
        retract(u)
    }
}

fn open_mode(u: &mut Unstructured) -> Result<OpenMode> {
    Ok(match u.int_in_range(0..=2)? {
        0 => OpenMode::Default,
        1 => OpenMode::SmallCache,
        _ => OpenMode::AutoCheckpoint,
    })
}

fn op(u: &mut Unstructured) -> Result<Op> {
    Ok(match weighted(u, &[6, 3, 2, 2, 2, 2])? {
        0 => Op::Exec(transact(u)?),
        1 => Op::Exec(retract(u)?),
        2 => {
            let mut stmts = Vec::new();
            for _ in 0..u.int_in_range(0..=3)? {
                stmts.push(stmt(u)?);
            }
            // Retract a triple this transaction asserts, before or after the
            // assertion: the later statement wins (#477).
            let own = stmts.iter().find_map(|s| match s {
                Stmt::Transact { facts, .. } => Some(facts[0].clone()),
                Stmt::Retract { .. } => None,
            });
            if let (true, Some(f)) = (u.arbitrary::<bool>()?, own) {
                let retract = Stmt::Retract {
                    facts: vec![(f.e, f.a, f.v)],
                };
                if u.arbitrary()? {
                    stmts.push(retract);
                } else {
                    stmts.insert(0, retract);
                }
            }
            Op::WriteTx {
                stmts,
                commit: u.ratio(4, 5)?,
            }
        }
        3 => Op::Checkpoint,
        4 => Op::Reopen(open_mode(u)?),
        _ => Op::Crash(open_mode(u)?),
    })
}

fn probe(u: &mut Unstructured) -> Result<Probe> {
    Ok(Probe {
        as_of: u.arbitrary()?,
        at: u.int_in_range(0..=AT_YEARS.len() - 1)?,
        e: u.int_in_range(0..=ENTITIES - 1)?,
        a: u.int_in_range(0..=ATTRS.len() - 1)?,
        v: v(u)?,
        pick: u.arbitrary()?,
    })
}

fn steps(data: &[u8]) -> Vec<Step> {
    let mut u = Unstructured::new(data);
    let mut out = Vec::new();
    while out.len() < MAX_STEPS && !u.is_empty() {
        let (Ok(op), Ok(probe)) = (op(&mut u), probe(&mut u)) else {
            break;
        };
        out.push(Step { op, probe });
    }
    out
}

fuzz_target!(|data: &[u8]| {
    let steps = steps(data);
    if steps.is_empty() {
        return;
    }
    if let Err(e) = run(&steps, Checks::One) {
        panic!("ops_sequence: {e}");
    }
});
