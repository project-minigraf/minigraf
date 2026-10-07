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

use minigraf::db::{Minigraf, OpenOptions, SyncMode};
use minigraf::{MinigrafError, QueryResult, Value};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

// ── Pools ─────────────────────────────────────────────────────────────────────

const ENTITIES: usize = 4;
const ATTRS: [&str; 3] = [":a0", ":a1", ":tag/丿"];

/// String pool. Indices 3 and 4 are longer than the 64-byte inline limit and
/// share their first 40 bytes; index 5 fills most of a value page.
fn pool_string(i: u8) -> String {
    match i {
        0 => "x".to_string(),
        1 => "y".to_string(),
        2 => String::new(),
        3 => format!("{}{}", "p".repeat(40), "A".repeat(40)),
        4 => format!("{}{}", "p".repeat(40), "B".repeat(40)),
        _ => "z".repeat(3000),
    }
}

/// A value from the pool. Debug output carries pool indices, never a `Uuid`.
#[derive(Clone, Debug, PartialEq)]
enum V {
    Int(i64),
    Bool(bool),
    Kw,
    Float,
    Str(u8),
    Ref(usize),
}

impl V {
    fn edn(&self) -> String {
        match self {
            V::Int(n) => n.to_string(),
            V::Bool(b) => b.to_string(),
            V::Kw => ":k0".to_string(),
            V::Float => "1.5".to_string(),
            V::Str(i) => format!("\"{}\"", pool_string(*i)),
            V::Ref(e) => format!("#uuid \"{}\"", entity_uuid(*e)),
        }
    }

    fn render(&self) -> String {
        match self {
            V::Int(n) => format!("i{n}"),
            V::Bool(b) => format!("b{b}"),
            V::Kw => "k:k0".to_string(),
            V::Float => "f1.5".to_string(),
            V::Str(i) => render_str(&pool_string(*i)),
            V::Ref(e) => format!("r{e}"),
        }
    }
}

fn render_str(s: &str) -> String {
    if s.len() <= 16 {
        format!("s\"{s}\"")
    } else {
        let head: String = s.chars().take(4).collect();
        let tail: String = s.chars().skip(s.chars().count() - 4).collect();
        format!("s\"{head}..{tail}\"#{}", s.len())
    }
}

fn entity_uuid(idx: usize) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!(":e{idx}").as_bytes())
}

/// Render an engine value like the model renders its own (`V::render`).
fn render_value(v: &Value, uuids: &HashMap<Uuid, usize>) -> String {
    match v {
        Value::Integer(n) => format!("i{n}"),
        Value::Boolean(b) => format!("b{b}"),
        Value::Keyword(k) => format!("k{k}"),
        Value::Float(f) => format!("f{f}"),
        Value::String(s) => render_str(s),
        Value::Ref(u) => match uuids.get(u) {
            Some(i) => format!("r{i}"),
            None => "r?".to_string(),
        },
        Value::Null => "null".to_string(),
    }
}

// ── Time grid ─────────────────────────────────────────────────────────────────

/// Unix ms of January 1st of `year`, UTC.
fn jan1_ms(year: i64) -> i64 {
    // Days from 1970-01-01 to year-01-01 (proleptic Gregorian).
    let y = year - 1;
    let days =
        365 * (y - 1969) + (y / 4 - 1969 / 4) - (y / 100 - 1969 / 100) + (y / 400 - 1969 / 400);
    days * 86_400_000
}

const VF_YEARS: [i64; 4] = [2000, 2002, 2004, 2006];
const VT_YEARS: [i64; 4] = [2003, 2005, 2007, 2200];
/// `:valid-at` instants: every window bound, plus points before, between and after.
const AT_YEARS: [i64; 12] = [
    1999, 2000, 2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2100, 2300,
];
const FOREVER: i64 = i64::MAX;

/// An optional `:valid-from` / `:valid-to` (grid indices).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Window {
    vf: Option<u8>,
    vt: Option<u8>,
}

impl Window {
    fn is_empty(&self) -> bool {
        self.vf.is_none() && self.vt.is_none()
    }

    fn edn(&self) -> String {
        let mut parts = Vec::new();
        if let Some(i) = self.vf {
            parts.push(format!(":valid-from \"{}-01-01\"", VF_YEARS[i as usize]));
        }
        if let Some(i) = self.vt {
            parts.push(format!(":valid-to \"{}-01-01\"", VT_YEARS[i as usize]));
        }
        format!("{{{}}}", parts.join(" "))
    }
}

/// An effective valid-time window: `valid_from` is `None` when it is the
/// transaction time (some instant between 2026 and 2100, before "now").
type Effective = (Option<i64>, i64);

fn effective(tx: Window, fact: Window) -> Effective {
    let vf = fact.vf.or(tx.vf).map(|i| jan1_ms(VF_YEARS[i as usize]));
    let vt = fact
        .vt
        .or(tx.vt)
        .map_or(FOREVER, |i| jan1_ms(VT_YEARS[i as usize]));
    (vf, vt)
}

fn window_is_valid((vf, vt): Effective) -> bool {
    match vf {
        Some(vf) => vf < vt,
        None => vt > jan1_ms(2100),
    }
}

/// When to read: `None` = now (no `:valid-at`), `Some(ms)` = a grid instant.
fn valid_at(window: Effective, at: Option<i64>) -> bool {
    let (vf, vt) = window;
    match at {
        None => vt == FOREVER || vt > jan1_ms(2100),
        Some(t) => {
            let starts = match vf {
                Some(vf) => vf <= t,
                None => t >= jan1_ms(2100),
            };
            starts && t < vt
        }
    }
}

// ── Operations ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct FactSpec {
    e: usize,
    a: usize,
    v: V,
    window: Window,
}

#[derive(Clone, Debug)]
enum Stmt {
    Transact {
        window: Window,
        facts: Vec<FactSpec>,
    },
    Retract {
        facts: Vec<(usize, usize, V)>,
    },
}

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

impl Stmt {
    fn edn(&self) -> String {
        match self {
            Stmt::Transact { window, facts } => {
                let facts: Vec<String> = facts
                    .iter()
                    .map(|f| {
                        let w = if f.window.is_empty() {
                            String::new()
                        } else {
                            format!(" {}", f.window.edn())
                        };
                        format!("[:e{} {} {}{w}]", f.e, ATTRS[f.a], f.v.edn())
                    })
                    .collect();
                let w = if window.is_empty() {
                    String::new()
                } else {
                    format!("{} ", window.edn())
                };
                format!("(transact {w}[{}])", facts.join(" "))
            }
            Stmt::Retract { facts } => {
                let facts: Vec<String> = facts
                    .iter()
                    .map(|(e, a, v)| format!("[:e{e} {} {}]", ATTRS[*a], v.edn()))
                    .collect();
                format!("(retract [{}])", facts.join(" "))
            }
        }
    }
}

// ── Strategies ────────────────────────────────────────────────────────────────

fn arb_v() -> impl Strategy<Value = V> {
    prop_oneof![
        3 => (0i64..3).prop_map(V::Int),
        1 => Just(V::Int(i64::MIN)),
        1 => any::<bool>().prop_map(V::Bool),
        1 => Just(V::Kw),
        1 => Just(V::Float),
        3 => (0u8..6).prop_map(V::Str),
        2 => (0..ENTITIES).prop_map(V::Ref),
    ]
}

fn arb_window() -> impl Strategy<Value = Window> {
    prop_oneof![
        3 => Just(Window::default()),
        2 => (prop::option::of(0u8..4), prop::option::of(0u8..4))
            .prop_map(|(vf, vt)| Window { vf, vt }),
    ]
}

/// A triple; often the same hot one, so that a triple builds a history longer
/// than the on-disk scan's seek threshold (8 entries).
fn arb_triple() -> impl Strategy<Value = (usize, usize, V)> {
    prop_oneof![
        2 => (0..ENTITIES, 0..ATTRS.len(), arb_v()),
        1 => Just((0, 0, V::Int(0))),
    ]
}

fn arb_transact() -> impl Strategy<Value = Stmt> {
    (
        arb_window(),
        prop::collection::vec((arb_triple(), arb_window()), 1..6),
        prop::option::weighted(0.15, arb_window()),
    )
        .prop_map(|(window, facts, repeat)| {
            let mut facts: Vec<FactSpec> = facts
                .into_iter()
                .map(|((e, a, v), w)| FactSpec { e, a, v, window: w })
                .collect();
            // Sometimes the same triple twice: an identical repeat or, when the
            // windows differ, an API-011 rejection.
            if let Some(w) = repeat {
                let mut again = facts[0].clone();
                again.window = w;
                facts.push(again);
            }
            // An empty or inverted window (#436) is rewritten to start in 2000;
            // every `:valid-to` on the grid is later.
            for f in &mut facts {
                if !window_is_valid(effective(window, f.window)) {
                    f.window.vf = Some(0);
                }
            }
            Stmt::Transact { window, facts }
        })
}

fn arb_retract() -> impl Strategy<Value = Stmt> {
    prop::collection::vec(arb_triple(), 1..4).prop_map(|facts| Stmt::Retract { facts })
}

fn arb_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![3 => arb_transact(), 2 => arb_retract()]
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
                // the assertion: the retraction wins either way.
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

// ── Reference model ───────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Record {
    tx: u64,
    e: usize,
    a: usize,
    v: V,
    window: Effective,
    asserted: bool,
}

/// A live fact: one window per triple, from its newest assertion (#435).
#[derive(Clone, Debug)]
struct Live {
    e: usize,
    a: usize,
    v: V,
    window: Effective,
    tx: u64,
}

#[derive(Default)]
struct Model {
    log: Vec<Record>,
    tx_count: u64,
}

impl Model {
    /// The records a committed transaction of `stmts` would add, or `None`
    /// when the transaction is rejected with API-011 (one triple, two windows).
    fn records(stmts: &[Stmt], tx: u64) -> Option<Vec<Record>> {
        let mut out = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::Transact { window, facts } => {
                    for f in facts {
                        out.push(Record {
                            tx,
                            e: f.e,
                            a: f.a,
                            v: f.v.clone(),
                            window: effective(*window, f.window),
                            asserted: true,
                        });
                    }
                }
                Stmt::Retract { facts } => {
                    for (e, a, v) in facts {
                        out.push(Record {
                            tx,
                            e: *e,
                            a: *a,
                            v: v.clone(),
                            window: (None, FOREVER),
                            asserted: false,
                        });
                    }
                }
            }
        }
        let asserted: Vec<&Record> = out.iter().filter(|r| r.asserted).collect();
        for (i, x) in asserted.iter().enumerate() {
            for y in &asserted[i + 1..] {
                if x.e == y.e && x.a == y.a && x.v == y.v && x.window != y.window {
                    return None;
                }
            }
        }
        Some(out)
    }

    /// Apply a committed transaction. Returns false when it is rejected.
    fn commit(&mut self, stmts: &[Stmt]) -> bool {
        if stmts.is_empty() {
            return true;
        }
        match Self::records(stmts, self.tx_count + 1) {
            Some(records) => {
                self.tx_count += 1;
                self.log.extend(records);
                true
            }
            None => false,
        }
    }

    fn live(&self, as_of: Option<u64>) -> Vec<Live> {
        let limit = as_of.unwrap_or(u64::MAX);
        let mut out: Vec<Live> = Vec::new();
        let mut seen: Vec<(usize, usize, &V)> = Vec::new();
        for r in &self.log {
            if seen
                .iter()
                .any(|(e, a, v)| *e == r.e && *a == r.a && *v == &r.v)
            {
                continue;
            }
            seen.push((r.e, r.a, &r.v));
            let same = |x: &&Record| x.tx <= limit && x.e == r.e && x.a == r.a && x.v == r.v;
            let newest_assert = self
                .log
                .iter()
                .filter(same)
                .filter(|x| x.asserted)
                .max_by_key(|x| x.tx);
            let newest_retract = self
                .log
                .iter()
                .filter(same)
                .filter(|x| !x.asserted)
                .map(|x| x.tx)
                .max()
                .unwrap_or(0);
            if let Some(a) = newest_assert.filter(|a| a.tx > newest_retract) {
                out.push(Live {
                    e: a.e,
                    a: a.a,
                    v: a.v.clone(),
                    window: a.window,
                    tx: a.tx,
                });
            }
        }
        out
    }
}

// ── Checks ────────────────────────────────────────────────────────────────────

struct Ctx {
    uuids: HashMap<Uuid, usize>,
}

impl Ctx {
    fn rows(&self, db: &Minigraf, query: &str) -> Result<Vec<Vec<String>>, TestCaseError> {
        let result = db
            .execute(query)
            .map_err(|e| TestCaseError::fail(format!("query failed ({}): {query}", e.code())))?;
        match result {
            QueryResult::QueryResults { results, .. } => Ok(results
                .iter()
                .map(|row| row.iter().map(|v| render_value(v, &self.uuids)).collect())
                .collect()),
            _ => Err(TestCaseError::fail(format!("not a query result: {query}"))),
        }
    }
}

fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}

fn attr_render(a: usize) -> String {
    format!("k{}", ATTRS[a])
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

/// Expect `result` to match the model's verdict: Ok when accepted, API-011
/// when rejected.
fn expect_outcome<T>(
    result: Result<T, MinigrafError>,
    accepted: bool,
    step: usize,
) -> Result<(), TestCaseError> {
    match (result, accepted) {
        (Ok(_), true) => Ok(()),
        (Err(e), false) if e.code() == "API-011" => Ok(()),
        (Ok(_), false) => Err(TestCaseError::fail(format!(
            "step {step}: accepted, model expects API-011"
        ))),
        (Err(e), _) => Err(TestCaseError::fail(format!(
            "step {step}: unexpected error {}",
            e.code()
        ))),
    }
}

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
                let accepted = model.commit(std::slice::from_ref(stmt));
                expect_outcome(result, accepted, i)?;
            }
            Op::WriteTx { stmts, commit } => {
                let handle = db
                    .as_ref()
                    .ok_or_else(|| TestCaseError::fail("no handle"))?;
                let mut tx = handle
                    .begin_write()
                    .map_err(|e| TestCaseError::fail(format!("begin_write ({})", e.code())))?;
                for stmt in stmts {
                    tx.execute(&stmt.edn()).map_err(|e| {
                        TestCaseError::fail(format!("step {i}: staged write ({})", e.code()))
                    })?;
                }
                if *commit {
                    let accepted = model.commit(stmts);
                    expect_outcome(tx.commit(), accepted, i)?;
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
