//! Reference model shared by the model-based test (#385) and the property
//! tests (#386): value pools, the valid-time grid, statements, and the
//! bi-temporal log that decides which facts a query sees.
#![allow(dead_code)]

use minigraf::db::{Minigraf, OpenOptions, SyncMode};
use minigraf::{MinigrafError, QueryResult, Value};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

// ── Pools ─────────────────────────────────────────────────────────────────────

pub const ENTITIES: usize = 4;
pub const ATTRS: [&str; 3] = [":a0", ":a1", ":tag/丿"];

/// String pool. Indices 3 and 4 are longer than the 64-byte inline limit and
/// share their first 40 bytes; index 5 fills most of a value page.
pub fn pool_string(i: u8) -> String {
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
pub enum V {
    Int(i64),
    Bool(bool),
    Kw,
    Float,
    Str(u8),
    Ref(usize),
}

impl V {
    pub fn edn(&self) -> String {
        match self {
            V::Int(n) => n.to_string(),
            V::Bool(b) => b.to_string(),
            V::Kw => ":k0".to_string(),
            V::Float => "1.5".to_string(),
            V::Str(i) => format!("\"{}\"", pool_string(*i)),
            V::Ref(e) => format!("#uuid \"{}\"", entity_uuid(*e)),
        }
    }

    pub fn render(&self) -> String {
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

pub fn render_str(s: &str) -> String {
    if s.len() <= 16 {
        format!("s\"{s}\"")
    } else {
        let head: String = s.chars().take(4).collect();
        let tail: String = s.chars().skip(s.chars().count() - 4).collect();
        format!("s\"{head}..{tail}\"#{}", s.len())
    }
}

pub fn entity_uuid(idx: usize) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_OID, format!(":e{idx}").as_bytes())
}

/// Render an engine value like the model renders its own (`V::render`).
pub fn render_value(v: &Value, uuids: &HashMap<Uuid, usize>) -> String {
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
pub fn jan1_ms(year: i64) -> i64 {
    // Days from 1970-01-01 to year-01-01 (proleptic Gregorian).
    let y = year - 1;
    let days =
        365 * (y - 1969) + (y / 4 - 1969 / 4) - (y / 100 - 1969 / 100) + (y / 400 - 1969 / 400);
    days * 86_400_000
}

pub const VF_YEARS: [i64; 4] = [2000, 2002, 2004, 2006];
pub const VT_YEARS: [i64; 4] = [2003, 2005, 2007, 2200];
/// `:valid-at` instants: every window bound, plus points before, between and after.
pub const AT_YEARS: [i64; 12] = [
    1999, 2000, 2001, 2002, 2003, 2004, 2005, 2006, 2007, 2008, 2100, 2300,
];
pub const FOREVER: i64 = i64::MAX;

/// An optional `:valid-from` / `:valid-to` (grid indices).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Window {
    pub vf: Option<u8>,
    pub vt: Option<u8>,
}

impl Window {
    pub fn is_empty(&self) -> bool {
        self.vf.is_none() && self.vt.is_none()
    }

    pub fn edn(&self) -> String {
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
pub type Effective = (Option<i64>, i64);

pub fn effective(tx: Window, fact: Window) -> Effective {
    let vf = fact.vf.or(tx.vf).map(|i| jan1_ms(VF_YEARS[i as usize]));
    let vt = fact
        .vt
        .or(tx.vt)
        .map_or(FOREVER, |i| jan1_ms(VT_YEARS[i as usize]));
    (vf, vt)
}

pub fn window_is_valid((vf, vt): Effective) -> bool {
    match vf {
        Some(vf) => vf < vt,
        None => vt > jan1_ms(2100),
    }
}

/// When to read: `None` = now (no `:valid-at`), `Some(ms)` = a grid instant.
pub fn valid_at(window: Effective, at: Option<i64>) -> bool {
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
pub struct FactSpec {
    pub e: usize,
    pub a: usize,
    pub v: V,
    pub window: Window,
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Transact {
        window: Window,
        facts: Vec<FactSpec>,
    },
    Retract {
        facts: Vec<(usize, usize, V)>,
    },
}

impl Stmt {
    pub fn edn(&self) -> String {
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

pub fn arb_v() -> impl Strategy<Value = V> {
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

pub fn arb_window() -> impl Strategy<Value = Window> {
    prop_oneof![
        3 => Just(Window::default()),
        2 => (prop::option::of(0u8..4), prop::option::of(0u8..4))
            .prop_map(|(vf, vt)| Window { vf, vt }),
    ]
}

/// A triple; often the same hot one, so that a triple builds a history longer
/// than the on-disk scan's seek threshold (8 entries).
pub fn arb_triple() -> impl Strategy<Value = (usize, usize, V)> {
    prop_oneof![
        2 => (0..ENTITIES, 0..ATTRS.len(), arb_v()),
        1 => Just((0, 0, V::Int(0))),
    ]
}

pub fn arb_transact() -> impl Strategy<Value = Stmt> {
    (
        arb_window(),
        prop::collection::vec((arb_triple(), arb_window()), 1..6),
        prop::option::weighted(0.15, arb_window()),
        prop::bool::weighted(0.9),
    )
        .prop_map(|(window, facts, repeat, fix_windows)| {
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
            // Usually an empty or inverted window is rewritten to start in
            // 2000 (every `:valid-to` on the grid is later); otherwise the
            // statement is an API-019 rejection (#436).
            for f in fix_windows.then_some(&mut facts).into_iter().flatten() {
                if !window_is_valid(effective(window, f.window)) {
                    f.window.vf = Some(0);
                }
            }
            Stmt::Transact { window, facts }
        })
}

pub fn arb_retract() -> impl Strategy<Value = Stmt> {
    prop::collection::vec(arb_triple(), 1..4).prop_map(|facts| Stmt::Retract { facts })
}

pub fn arb_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![3 => arb_transact(), 2 => arb_retract()]
}

// ── Reference model ───────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct Record {
    pub tx: u64,
    pub e: usize,
    pub a: usize,
    pub v: V,
    pub window: Effective,
    pub asserted: bool,
}

/// A live fact: one window per triple, from its newest assertion (#435).
#[derive(Clone, Debug)]
pub struct Live {
    pub e: usize,
    pub a: usize,
    pub v: V,
    pub window: Effective,
    pub tx: u64,
}

#[derive(Default)]
pub struct Model {
    pub log: Vec<Record>,
    pub tx_count: u64,
}

impl Model {
    /// The error code a statement is rejected with, if any: API-019 for an
    /// empty or inverted window (#436), then API-011 for one triple with two
    /// windows in the statement.
    pub fn verdict(stmt: &Stmt) -> Option<&'static str> {
        let Stmt::Transact { window, facts } = stmt else {
            return None;
        };
        let windows: Vec<Effective> = facts.iter().map(|f| effective(*window, f.window)).collect();
        if !windows.iter().all(|w| window_is_valid(*w)) {
            return Some("API-019");
        }
        for (i, x) in facts.iter().enumerate() {
            for (j, y) in facts.iter().enumerate().skip(i + 1) {
                if x.e == y.e && x.a == y.a && x.v == y.v && windows[i] != windows[j] {
                    return Some("API-011");
                }
            }
        }
        None
    }

    /// The records a committed transaction of accepted `stmts` adds: each
    /// triple's records from the last statement that wrote it (#477).
    pub fn records(stmts: &[Stmt], tx: u64) -> Vec<Record> {
        let mut out: Vec<(usize, Record)> = Vec::new();
        for (n, stmt) in stmts.iter().enumerate() {
            match stmt {
                Stmt::Transact { window, facts } => {
                    for f in facts {
                        out.push((
                            n,
                            Record {
                                tx,
                                e: f.e,
                                a: f.a,
                                v: f.v.clone(),
                                window: effective(*window, f.window),
                                asserted: true,
                            },
                        ));
                    }
                }
                Stmt::Retract { facts } => {
                    for (e, a, v) in facts {
                        out.push((
                            n,
                            Record {
                                tx,
                                e: *e,
                                a: *a,
                                v: v.clone(),
                                window: (None, FOREVER),
                                asserted: false,
                            },
                        ));
                    }
                }
            }
        }
        let last = |r: &Record| {
            out.iter()
                .filter(|(_, x)| x.e == r.e && x.a == r.a && x.v == r.v)
                .map(|(n, _)| *n)
                .max()
        };
        out.iter()
            .filter(|(n, r)| last(r) == Some(*n))
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Apply a committed transaction of accepted statements.
    pub fn commit(&mut self, stmts: &[Stmt]) {
        if stmts.is_empty() {
            return;
        }
        self.tx_count += 1;
        self.log.extend(Self::records(stmts, self.tx_count));
    }

    pub fn live(&self, as_of: Option<u64>) -> Vec<Live> {
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

pub struct Ctx {
    pub uuids: HashMap<Uuid, usize>,
}

impl Ctx {
    pub fn rows(&self, db: &Minigraf, query: &str) -> Result<Vec<Vec<String>>, TestCaseError> {
        self.result_rows(db.execute(query), query)
    }

    pub fn result_rows(
        &self,
        result: Result<QueryResult, MinigrafError>,
        query: &str,
    ) -> Result<Vec<Vec<String>>, TestCaseError> {
        let result = result
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

pub fn sorted(mut rows: Vec<String>) -> Vec<String> {
    rows.sort();
    rows
}

pub fn attr_render(a: usize) -> String {
    format!("k{}", ATTRS[a])
}

// ── Driver ────────────────────────────────────────────────────────────────────
//
// Shared by the model-based test (proptest strategies) and the `ops_sequence`
// fuzz target (`arbitrary` decoding): one oracle for both.

#[derive(Clone, Copy, Debug)]
pub enum OpenMode {
    Default,
    SmallCache,
    AutoCheckpoint,
}

#[derive(Clone, Debug)]
pub enum Op {
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
pub struct Probe {
    pub as_of: u16,
    pub at: usize,
    pub e: usize,
    pub a: usize,
    pub v: V,
    /// With `Checks::One`, picks the query shape and temporal clause.
    pub pick: u8,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub op: Op,
    pub probe: Probe,
}

/// How much `check` reads after each step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Checks {
    /// Every query shape under every temporal clause (the proptest test).
    All,
    /// One query shape and temporal clause, picked by `Probe::pick` (the fuzz
    /// target, where executions per second matter).
    One,
}

/// Full rows with windows and tx counts, current or at `:as-of N`.
pub fn check_full_rows(
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

/// The query shapes: (find vars, where clause, model filter + projection).
/// `only` restricts the check to one shape.
#[allow(clippy::too_many_arguments)]
pub fn check_shapes(
    db: &Minigraf,
    ctx: &Ctx,
    model: &Model,
    probe: &Probe,
    as_of: Option<u64>,
    at: Option<i64>,
    only: Option<usize>,
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
    for (n, (find, clause, project)) in shapes.iter().enumerate() {
        if only.is_some_and(|o| o != n) {
            continue;
        }
        let query = format!("(query [:find {find} {temporal}:where {clause}])");
        // The failure message names the probe by pool indices: a `Ref` value's
        // EDN carries a `Uuid`.
        let label = format!("shape {n} ({find}), {temporal}e{pe} a{pa} {pv:?}");
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
        prop_assert_eq!(sorted(got), sorted(want), "step {}: {}", step, label);
    }
    Ok(())
}

pub fn check(
    db: &Minigraf,
    ctx: &Ctx,
    model: &Model,
    probe: &Probe,
    checks: Checks,
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
    let temporal = [
        (None, None),
        (None, Some(at)),
        (Some(as_of), Some(at)),
        (Some(as_of), None),
    ];
    match checks {
        Checks::All => {
            for (as_of, at) in temporal {
                check_shapes(db, ctx, model, probe, as_of, at, None, step)?;
            }
        }
        Checks::One => {
            let pick = usize::from(probe.pick);
            let (as_of, at) = temporal[(pick / 6) % 4];
            check_shapes(db, ctx, model, probe, as_of, at, Some(pick % 6), step)?;
        }
    }
    Ok(())
}

pub fn open(path: &Path, mode: OpenMode) -> Result<Minigraf, TestCaseError> {
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
pub fn crash_copy(from: &Path, to: &Path, step: usize) -> Result<(), TestCaseError> {
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

pub fn verify(db: &Minigraf, step: usize) -> Result<(), TestCaseError> {
    let report = db
        .verify()
        .map_err(|e| TestCaseError::fail(format!("step {step}: verify failed ({})", e.code())))?;
    prop_assert!(report.is_ok(), "step {}: verify found problems", step);
    Ok(())
}

/// Expect `result` to match the model's verdict: Ok when `None`, else that
/// error code.
pub fn expect_outcome<T>(
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
pub const IN_TX_QUERY: &str = "(query [:find ?e ?a ?v :any-valid-time :where [?e ?a ?v]])";

pub fn run(steps: &[Step], checks: Checks) -> Result<(), TestCaseError> {
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
        check(handle, &ctx, &model, &step.probe, checks, i)?;
    }
    Ok(())
}
