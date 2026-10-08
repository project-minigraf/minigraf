//! Reference model shared by the model-based test (#385) and the property
//! tests (#386): value pools, the valid-time grid, statements, and the
//! bi-temporal log that decides which facts a query sees.
#![allow(dead_code)]

use minigraf::db::Minigraf;
use minigraf::{MinigrafError, QueryResult, Value};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use std::collections::HashMap;
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
