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
//! Design: docs/superpowers/specs/2026-10-07-model-based-test-design.md
#![cfg(not(target_arch = "wasm32"))]

mod reference_model;

use minigraf::MinigrafError;
use minigraf::db::{Minigraf, OpenOptions, SyncMode};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use reference_model::*;
use std::collections::HashMap;
use std::path::Path;

// ── Operations ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum OpenMode {
    Default,
    SmallCache,
    AutoCheckpoint,
}

#[derive(Clone, Debug)]
enum Op {
    Exec(Stmt),
    WriteTx {
        stmts: Vec<Stmt>,
        commit: bool,
    },
    Checkpoint,
    Reopen(OpenMode),
    /// Open a copy of the `.graph` file and its WAL taken while the handle is
    /// open, as a process death would leave them: the WAL is replayed.
    Crash(OpenMode),
}

/// Random inputs of the checks that follow a step.
#[derive(Clone, Debug)]
struct Probe {
    as_of: u16,
    at: usize,
    e: usize,
    a: usize,
    v: V,
}

#[derive(Clone, Debug)]
struct Step {
    op: Op,
    probe: Probe,
}

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
        .prop_map(|(as_of, at, e, a, v)| Probe { as_of, at, e, a, v })
}

fn arb_steps() -> impl Strategy<Value = Vec<Step>> {
    prop::collection::vec(
        (arb_op(), arb_probe()).prop_map(|(op, probe)| Step { op, probe }),
        1..50,
    )
}

/// Full rows with windows and tx counts, current or at `:as-of N`.
fn check_full_rows(
    db: &Minigraf,
    ctx: &Ctx,
    model: &Model,
    as_of: Option<u64>,
    step: usize,
) -> Result<(), TestCaseError> {
    let as_of_clause = as_of.map_or(String::new(), |n| format!(":as-of {n} "));
    let query = format!(
        "(query [:find ?e ?a ?v ?vf ?vt ?tc ?ti {as_of_clause}:any-valid-time \
         :where [?e ?a ?v] [?e :db/valid-from ?vf] [?e :db/valid-to ?vt] \
         [?e :db/tx-count ?tc] [?e :db/tx-id ?ti]])"
    );
    let mut tx_ids: HashMap<String, String> = HashMap::new();
    let mut got = Vec::new();
    for row in ctx.rows(db, &query)? {
        prop_assert_eq!(row.len(), 7, "step {}: full row width", step);
        let (vf, ti) = (&row[3], &row[6]);
        // A tx-time valid_from is the fact's own tx id.
        let vf = if vf == ti {
            "tx".to_string()
        } else {
            vf.clone()
        };
        if let Some(prev) = tx_ids.insert(row[5].clone(), ti.clone()) {
            prop_assert_eq!(&prev, ti, "step {}: one tx id per tx count", step);
        }
        got.push(format!(
            "{} {} {} [{vf}, {}) tx{}",
            row[0], row[1], row[2], row[4], row[5]
        ));
    }
    let want: Vec<String> = model
        .live(as_of)
        .iter()
        .map(|l| {
            let vf = l.window.0.map_or("tx".to_string(), |ms| format!("i{ms}"));
            format!(
                "r{} {} {} [{vf}, i{}) txi{}",
                l.e,
                attr_render(l.a),
                l.v.render(),
                l.window.1,
                l.tx
            )
        })
        .collect();
    prop_assert_eq!(
        sorted(got),
        sorted(want),
        "step {}: full rows, as-of {:?}",
        step,
        as_of
    );
    Ok(())
}

/// The query shapes: (where clause, find vars, model filter + projection).
fn check_shapes(
    db: &Minigraf,
    ctx: &Ctx,
    model: &Model,
    probe: &Probe,
    as_of: Option<u64>,
    at: Option<i64>,
    step: usize,
) -> Result<(), TestCaseError> {
    let (pe, pa, pv) = (probe.e, probe.a, &probe.v);
    let ea = format!(":e{pe}");
    let aa = ATTRS[pa];
    let ve = pv.edn();
    type Shape<'a> = (&'a str, String, Box<dyn Fn(&Live) -> Option<String> + 'a>);
    let shapes: Vec<Shape<'_>> = vec![
        (
            "?e ?a ?v",
            "[?e ?a ?v]".to_string(),
            Box::new(|l: &Live| Some(format!("r{} {} {}", l.e, attr_render(l.a), l.v.render()))),
        ),
        (
            "?a ?v",
            format!("[{ea} ?a ?v]"),
            Box::new(move |l: &Live| {
                (l.e == pe).then(|| format!("{} {}", attr_render(l.a), l.v.render()))
            }),
        ),
        (
            "?v",
            format!("[{ea} {aa} ?v]"),
            Box::new(move |l: &Live| (l.e == pe && l.a == pa).then(|| l.v.render())),
        ),
        (
            "?e ?v",
            format!("[?e {aa} ?v]"),
            Box::new(move |l: &Live| (l.a == pa).then(|| format!("r{} {}", l.e, l.v.render()))),
        ),
        (
            "?e",
            format!("[?e {aa} {ve}]"),
            Box::new(move |l: &Live| (l.a == pa && &l.v == pv).then(|| format!("r{}", l.e))),
        ),
        (
            "?e ?a",
            format!("[?e ?a {ve}]"),
            Box::new(move |l: &Live| {
                (&l.v == pv).then(|| format!("r{} {}", l.e, attr_render(l.a)))
            }),
        ),
    ];

    let mut temporal = String::new();
    if let Some(n) = as_of {
        temporal.push_str(&format!(":as-of {n} "));
    }
    if let Some(t) = at {
        let year = AT_YEARS.iter().find(|y| jan1_ms(**y) == t).copied();
        temporal.push_str(&format!(":valid-at \"{}-01-01\" ", year.unwrap_or(1999)));
    }

    let live = model.live(as_of);
    for (find, clause, project) in &shapes {
        let query = format!("(query [:find {find} {temporal}:where {clause}])");
        let got: Vec<String> = ctx
            .rows(db, &query)?
            .into_iter()
            .map(|r| r.join(" "))
            .collect();
        let want: Vec<String> = live
            .iter()
            .filter(|l| valid_at(l.window, at))
            .filter_map(project)
            .collect();
        prop_assert_eq!(sorted(got), sorted(want), "step {}: {}", step, query);
    }
    Ok(())
}

fn check(
    db: &Minigraf,
    ctx: &Ctx,
    model: &Model,
    probe: &Probe,
    step: usize,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(
        db.current_tx_count(),
        model.tx_count,
        "step {}: tx count",
        step
    );
    let as_of = u64::from(probe.as_of) % (model.tx_count + 2);
    let at = jan1_ms(AT_YEARS[probe.at]);

    check_full_rows(db, ctx, model, None, step)?;
    check_full_rows(db, ctx, model, Some(as_of), step)?;
    check_shapes(db, ctx, model, probe, None, None, step)?;
    check_shapes(db, ctx, model, probe, None, Some(at), step)?;
    check_shapes(db, ctx, model, probe, Some(as_of), Some(at), step)?;
    check_shapes(db, ctx, model, probe, Some(as_of), None, step)?;
    Ok(())
}

// ── Driver ────────────────────────────────────────────────────────────────────

fn open(path: &Path, mode: OpenMode) -> Result<Minigraf, TestCaseError> {
    let opts = OpenOptions::new().synchronous(SyncMode::Normal);
    let opts = match mode {
        OpenMode::Default => opts,
        OpenMode::SmallCache => opts.page_cache_size(4),
        OpenMode::AutoCheckpoint => opts.wal_checkpoint_threshold(2),
    };
    Minigraf::open_with_options(path, opts)
        .map_err(|e| TestCaseError::fail(format!("open failed ({})", e.code())))
}

/// Copy the database file and its WAL as they are on disk right now. On
/// Windows, where locks are mandatory, the caller closes the handle first,
/// which turns the step into a plain reopen.
fn crash_copy(from: &Path, to: &Path, step: usize) -> Result<(), TestCaseError> {
    let wal = |p: &Path| {
        let mut s = p.as_os_str().to_owned();
        s.push(".wal");
        std::path::PathBuf::from(s)
    };
    let copy = |a: &Path, b: &Path| {
        std::fs::copy(a, b)
            .map(|_| ())
            .map_err(|_| TestCaseError::fail(format!("step {step}: copy failed")))
    };
    copy(from, to)?;
    if wal(from).exists() {
        copy(&wal(from), &wal(to))?;
    }
    Ok(())
}

fn verify(db: &Minigraf, step: usize) -> Result<(), TestCaseError> {
    let report = db
        .verify()
        .map_err(|e| TestCaseError::fail(format!("step {step}: verify failed ({})", e.code())))?;
    prop_assert!(report.is_ok(), "step {}: verify found problems", step);
    Ok(())
}

/// Expect `result` to match the model's verdict: Ok when `None`, else that
/// error code.
fn expect_outcome<T>(
    result: Result<T, MinigrafError>,
    verdict: Option<&str>,
    step: usize,
) -> Result<(), TestCaseError> {
    match (result, verdict) {
        (Ok(_), None) => Ok(()),
        (Err(e), Some(code)) if e.code() == code => Ok(()),
        (Ok(_), Some(code)) => Err(TestCaseError::fail(format!(
            "step {step}: accepted, model expects {code}"
        ))),
        (Err(e), _) => Err(TestCaseError::fail(format!(
            "step {step}: unexpected error {}",
            e.code()
        ))),
    }
}

/// The current `(e, a, v)` of every fact, any valid time.
const IN_TX_QUERY: &str = "(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])";

fn run(steps: &[Step]) -> Result<(), TestCaseError> {
    let dir = tempfile::tempdir().map_err(|_| TestCaseError::fail("tempdir"))?;
    let mut path = dir.path().join("model.graph");
    let ctx = Ctx {
        uuids: (0..ENTITIES).map(|i| (entity_uuid(i), i)).collect(),
    };
    let mut model = Model::default();
    let mut db = Some(open(&path, OpenMode::Default)?);

    for (i, step) in steps.iter().enumerate() {
        match &step.op {
            Op::Exec(stmt) => {
                let handle = db
                    .as_ref()
                    .ok_or_else(|| TestCaseError::fail("no handle"))?;
                let result = handle.execute(&stmt.edn());
                let verdict = Model::verdict(stmt);
                if verdict.is_none() {
                    model.commit(std::slice::from_ref(stmt));
                }
                expect_outcome(result, verdict, i)?;
            }
            Op::WriteTx { stmts, commit } => {
                let handle = db
                    .as_ref()
                    .ok_or_else(|| TestCaseError::fail("no handle"))?;
                let mut tx = handle
                    .begin_write()
                    .map_err(|e| TestCaseError::fail(format!("begin_write ({})", e.code())))?;
                // A rejected statement stages nothing; the rest still commit.
                let mut accepted = Vec::new();
                for stmt in stmts {
                    let verdict = Model::verdict(stmt);
                    expect_outcome(tx.execute(&stmt.edn()), verdict, i)?;
                    if verdict.is_none() {
                        accepted.push(stmt.clone());
                    }
                }
                // Reads inside the transaction see what the commit will hold.
                let mut after = Model {
                    log: model.log.clone(),
                    tx_count: model.tx_count,
                };
                after.commit(&accepted);
                let got: Vec<String> = ctx
                    .result_rows(tx.execute(IN_TX_QUERY), IN_TX_QUERY)?
                    .into_iter()
                    .map(|r| r.join(" "))
                    .collect();
                let want: Vec<String> = after
                    .live(None)
                    .iter()
                    .map(|l| format!("r{} {} {}", l.e, attr_render(l.a), l.v.render()))
                    .collect();
                prop_assert_eq!(
                    sorted(got),
                    sorted(want),
                    "step {}: reads inside the transaction",
                    i
                );
                if *commit {
                    model = after;
                    expect_outcome(tx.commit(), None, i)?;
                } else {
                    tx.rollback();
                }
            }
            Op::Checkpoint => {
                let handle = db
                    .as_ref()
                    .ok_or_else(|| TestCaseError::fail("no handle"))?;
                handle
                    .checkpoint()
                    .map_err(|e| TestCaseError::fail(format!("checkpoint ({})", e.code())))?;
                verify(handle, i)?;
            }
            Op::Reopen(mode) => {
                drop(db.take());
                let handle = open(&path, *mode)?;
                verify(&handle, i)?;
                db = Some(handle);
            }
            Op::Crash(mode) => {
                let copy = dir.path().join(format!("crash{i}.graph"));
                if cfg!(windows) {
                    drop(db.take());
                }
                crash_copy(&path, &copy, i)?;
                drop(db.take());
                path = copy;
                let handle = open(&path, *mode)?;
                verify(&handle, i)?;
                db = Some(handle);
            }
        }
        let handle = db
            .as_ref()
            .ok_or_else(|| TestCaseError::fail("no handle"))?;
        check(handle, &ctx, &model, &step.probe, i)?;
    }
    Ok(())
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
        run(&steps)?;
    }
}
