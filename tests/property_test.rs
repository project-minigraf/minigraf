//! Cross-feature property tests (#221, #386).
//!
//! Compares Minigraf query results against a deliberately simple reference
//! evaluator. The first three properties write one fact per `transact` and run
//! one-pattern queries. The others write random histories (batched transacts,
//! retractions, write transactions, checkpoints) and run random queries with
//! joins, `not`/`not-join`, predicates, `:as-of`, `:valid-at` and
//! `:any-valid-time`, in memory and against a file that is reopened.
//!
//! Run: cargo test --test property_test
//! More cases: PROPTEST_CASES=500 cargo test --test property_test
//!
//! Design: docs/superpowers/specs/2026-10-08-wider-property-tests-design.md
//! (v2.x backport: §9; histories leave out the known issues #371, #435, #477)
#![cfg(not(target_arch = "wasm32"))]

mod reference_model;

use minigraf::QueryResult;
use minigraf::db::{Minigraf, OpenOptions, SyncMode};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use reference_model::*;
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone)]
struct TestFact {
    entity: usize,
    attribute: String,
    value: TestValue,
}

#[derive(Debug, Clone, PartialEq)]
enum TestValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

impl TestValue {
    fn to_edn(&self) -> String {
        match self {
            TestValue::Str(s) => format!(r#""{s}""#),
            TestValue::Int(n) => n.to_string(),
            TestValue::Bool(b) => b.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct TestQuery {
    attribute: String,
    value_filter: Option<TestValue>,
    negation_attr: Option<String>,
}

// ── Reference evaluator (naive, independent of production code) ───────────────

fn ref_eval(facts: &[TestFact], query: &TestQuery) -> Vec<usize> {
    let mut matched: Vec<usize> = facts
        .iter()
        .filter(|f| {
            f.attribute == query.attribute
                && match &query.value_filter {
                    None => true,
                    Some(v) => &f.value == v,
                }
        })
        .map(|f| f.entity)
        .collect();

    if let Some(neg_attr) = &query.negation_attr {
        let neg_entities: std::collections::HashSet<usize> = facts
            .iter()
            .filter(|f| &f.attribute == neg_attr)
            .map(|f| f.entity)
            .collect();
        matched.retain(|e| !neg_entities.contains(e));
    }

    matched.sort();
    matched.dedup();
    matched
}

// ── Minigraf evaluator ────────────────────────────────────────────────────────

fn minigraf_eval(
    facts: &[TestFact],
    query: &TestQuery,
    max_entity: usize,
) -> Result<Vec<usize>, TestCaseError> {
    // Pre-compute UUID → index mapping for all possible entity indices.
    let uuid_to_idx: HashMap<Uuid, usize> = (0..max_entity).map(|i| (entity_uuid(i), i)).collect();

    let db = Minigraf::in_memory().map_err(|_| TestCaseError::fail("in_memory failed"))?;

    for fact in facts {
        let entity_kw = format!(":e{}", fact.entity);
        let val_edn = fact.value.to_edn();
        let attr = &fact.attribute;
        let edn = format!(r#"(transact [[{entity_kw} {attr} {val_edn}]])"#);
        db.execute(&edn)
            .map_err(|e| TestCaseError::fail(format!("transact failed ({})", e.code())))?;
    }

    let val_clause = match &query.value_filter {
        Some(v) => format!(" [(= ?v {})]", v.to_edn()),
        None => String::new(),
    };

    let neg_clause = match &query.negation_attr {
        Some(neg) => format!(" (not [?e {} _])", neg),
        None => String::new(),
    };

    let attr = &query.attribute;
    let datalog = format!("(query [:find ?e :where [?e {attr} ?v]{val_clause}{neg_clause}])");

    let result = db
        .execute(&datalog)
        .map_err(|e| TestCaseError::fail(format!("query failed ({}): {datalog}", e.code())))?;
    match result {
        QueryResult::QueryResults { results, .. } => {
            let mut entities: Vec<usize> = results
                .into_iter()
                .flat_map(|r| r.into_iter())
                .filter_map(|v| match v {
                    minigraf::Value::Ref(uuid) => uuid_to_idx.get(&uuid).copied(),
                    _ => None,
                })
                .collect();
            entities.sort();
            entities.dedup();
            Ok(entities)
        }
        _ => Err(TestCaseError::fail("not a query result")),
    }
}

// ── proptest generators ───────────────────────────────────────────────────────

fn arb_attribute() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(":color".to_string()),
        Just(":size".to_string()),
        Just(":active".to_string()),
        Just(":tag".to_string()),
        Just(":score".to_string()),
    ]
}

fn arb_value() -> impl Strategy<Value = TestValue> {
    prop_oneof![
        Just(TestValue::Str("red".to_string())),
        Just(TestValue::Str("blue".to_string())),
        Just(TestValue::Int(1)),
        Just(TestValue::Int(2)),
        Just(TestValue::Bool(true)),
    ]
}

fn arb_fact(max_entity: usize) -> impl Strategy<Value = TestFact> {
    (0..max_entity, arb_attribute(), arb_value()).prop_map(|(entity, attribute, value)| TestFact {
        entity,
        attribute,
        value,
    })
}

proptest! {
    /// Minigraf results must match the reference evaluator for basic queries.
    #[test]
    fn basic_query_matches_reference(facts in prop::collection::vec(arb_fact(8), 3..20)) {
        let query = TestQuery {
            attribute: ":color".to_string(),
            value_filter: None,
            negation_attr: None,
        };
        let ref_result = ref_eval(&facts, &query);
        let mg_result = minigraf_eval(&facts, &query, 8)?;
        prop_assert_eq!(ref_result, mg_result);
    }

    /// Negation: entities with negation_attr must not appear in results.
    #[test]
    fn negation_excludes_correct_entities(facts in prop::collection::vec(arb_fact(8), 3..20)) {
        let query = TestQuery {
            attribute: ":color".to_string(),
            value_filter: None,
            negation_attr: Some(":active".to_string()),
        };
        let ref_result = ref_eval(&facts, &query);
        let mg_result = minigraf_eval(&facts, &query, 8)?;

        let neg_entities: std::collections::HashSet<usize> = facts
            .iter()
            .filter(|f| f.attribute == ":active")
            .map(|f| f.entity)
            .collect();

        for &e in &mg_result {
            prop_assert!(
                !neg_entities.contains(&e),
                "entity {} has negated attribute but appears in result",
                e
            );
        }
        prop_assert_eq!(ref_result, mg_result);
    }

    /// Impossible paths return empty results.
    #[test]
    fn impossible_path_returns_empty(facts in prop::collection::vec(arb_fact(8), 3..20)) {
        let query = TestQuery {
            attribute: ":__nonexistent__".to_string(),
            value_filter: None,
            negation_attr: None,
        };
        let ref_result = ref_eval(&facts, &query);
        let mg_result = minigraf_eval(&facts, &query, 8)?;
        prop_assert!(ref_result.is_empty(), "reference: impossible path must be empty");
        prop_assert!(mg_result.is_empty(), "minigraf: impossible path must be empty");
    }
}

// ── Histories (#386) ──────────────────────────────────────────────────────────

/// One write of a history. Every generated statement is accepted.
#[derive(Clone, Debug)]
enum HistoryOp {
    Exec(Stmt),
    WriteTx(Vec<Stmt>),
    /// `checkpoint()` on the file backend; nothing in memory.
    Checkpoint,
}

/// Make a statement acceptable: an empty or inverted window starts in 2000
/// (API-019), and a triple repeated in one statement keeps its first fact
/// (API-011). Rejections are the model-based test's job.
fn accepted(stmt: Stmt) -> Stmt {
    match stmt {
        Stmt::Transact { window, facts } => {
            let mut kept: Vec<FactSpec> = Vec::new();
            for mut f in facts {
                if kept.iter().any(|k| k.e == f.e && k.a == f.a && k.v == f.v) {
                    continue;
                }
                if !window_is_valid(effective(window, f.window)) {
                    f.window.vf = Some(0);
                }
                kept.push(f);
            }
            Stmt::Transact {
                window,
                facts: kept,
            }
        }
        retract => retract,
    }
}

/// A value, weighted to small integers and entities so that facts share
/// values and joins on them match.
fn arb_dense_v() -> impl Strategy<Value = V> {
    prop_oneof![
        3 => (0i64..3).prop_map(V::Int),
        2 => (0..ENTITIES).prop_map(V::Ref),
        2 => arb_v(),
    ]
}

fn arb_dense_triple() -> impl Strategy<Value = (usize, usize, V)> {
    (0..ENTITIES, 0..ATTRS.len(), arb_dense_v())
}

/// Larger batches than the model-based test's, without its hot triple, so
/// that a history leaves many live facts to join.
fn arb_dense_transact() -> impl Strategy<Value = Stmt> {
    (
        arb_window(),
        prop::collection::vec((arb_dense_triple(), arb_window()), 1..10),
    )
        .prop_map(|(window, facts)| {
            accepted(Stmt::Transact {
                window,
                facts: facts
                    .into_iter()
                    .map(|((e, a, v), window)| FactSpec { e, a, v, window })
                    .collect(),
            })
        })
}

fn arb_dense_retract() -> impl Strategy<Value = Stmt> {
    prop::collection::vec(arb_dense_triple(), 1..4).prop_map(|facts| Stmt::Retract { facts })
}

fn arb_history_op() -> impl Strategy<Value = HistoryOp> {
    prop_oneof![
        6 => arb_dense_transact().prop_map(HistoryOp::Exec),
        2 => arb_dense_retract().prop_map(HistoryOp::Exec),
        2 => prop::collection::vec(
            prop_oneof![3 => arb_dense_transact(), 2 => arb_dense_retract()],
            1..5,
        )
        .prop_map(HistoryOp::WriteTx),
        2 => Just(HistoryOp::Checkpoint),
    ]
}

fn arb_history() -> impl Strategy<Value = Vec<HistoryOp>> {
    prop::collection::vec(arb_history_op(), 1..17).prop_map(v2_safe)
}

/// Leave out the writes that hit v2.x known issues (#421): a second value of
/// one entity and attribute in a transaction, by assertion or retraction
/// (#371, which also covers a retract and re-assert in one transaction,
/// #477), and an assertion of a triple that is already live (#435). A
/// retraction followed by an assertion in a later transaction stays.
fn v2_safe(history: Vec<HistoryOp>) -> Vec<HistoryOp> {
    let mut model = Model::default();
    let mut out = Vec::new();
    for op in history {
        let (stmts, write_tx) = match op {
            HistoryOp::Exec(stmt) => (vec![stmt], false),
            HistoryOp::WriteTx(stmts) => (stmts, true),
            HistoryOp::Checkpoint => {
                out.push(HistoryOp::Checkpoint);
                continue;
            }
        };
        let live = model.live(None);
        let mut touched: Vec<(usize, usize)> = Vec::new();
        let mut kept = Vec::new();
        for stmt in stmts {
            match stmt {
                Stmt::Transact { window, facts } => {
                    let mut ok = Vec::new();
                    for f in facts {
                        let is_live = live.iter().any(|l| l.e == f.e && l.a == f.a && l.v == f.v);
                        if !touched.contains(&(f.e, f.a)) && !is_live {
                            touched.push((f.e, f.a));
                            ok.push(f);
                        }
                    }
                    if !ok.is_empty() {
                        kept.push(Stmt::Transact { window, facts: ok });
                    }
                }
                Stmt::Retract { facts } => {
                    let mut ok = Vec::new();
                    for (e, a, v) in facts {
                        if !touched.contains(&(e, a)) {
                            touched.push((e, a));
                            ok.push((e, a, v));
                        }
                    }
                    if !ok.is_empty() {
                        kept.push(Stmt::Retract { facts: ok });
                    }
                }
            }
        }
        if kept.is_empty() {
            continue;
        }
        model.commit(&kept);
        out.push(if write_tx {
            HistoryOp::WriteTx(kept)
        } else {
            HistoryOp::Exec(kept.remove(0))
        });
    }
    out
}

// ── Queries (#386) ────────────────────────────────────────────────────────────

/// A query variable: entity-like (`?e0`–`?e2`, in E or V position), value
/// (`?v0`–`?v2`, V position only) or the attribute variable `?a`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Var {
    E(u8),
    V(u8),
    A,
}

impl Var {
    fn name(self) -> String {
        match self {
            Var::E(i) => format!("?e{i}"),
            Var::V(i) => format!("?v{i}"),
            Var::A => "?a".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Term {
    Var(Var),
    Entity(usize),
    Const(V),
    Blank,
}

#[derive(Clone, Debug)]
struct Pat {
    e: Term,
    /// `None` is the attribute variable `?a`.
    a: Option<usize>,
    v: Term,
}

impl Pat {
    fn vars(&self) -> Vec<Var> {
        let mut out = Vec::new();
        if let Term::Var(x) = self.e {
            out.push(x);
        }
        if self.a.is_none() {
            out.push(Var::A);
        }
        if let Term::Var(x) = self.v {
            out.push(x);
        }
        out
    }

    fn edn(&self) -> String {
        let term = |t: &Term| match t {
            Term::Var(x) => x.name(),
            Term::Entity(e) => format!(":e{e}"),
            Term::Const(v) => v.edn(),
            Term::Blank => "_".to_string(),
        };
        let a = self.a.map_or("?a".to_string(), |a| ATTRS[a].to_string());
        format!("[{} {a} {}]", term(&self.e), term(&self.v))
    }
}

#[derive(Clone, Debug)]
enum Neg {
    Not(Pat),
    /// `(not-join [?x] [?x a v])`, with `v` a fresh `?z` when `None`.
    NotJoin {
        var: Var,
        a: usize,
        v: Option<V>,
    },
}

#[derive(Clone, Copy, Debug)]
enum CmpOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl CmpOp {
    fn edn(self) -> &'static str {
        match self {
            CmpOp::Lt => "<",
            CmpOp::Le => "<=",
            CmpOp::Gt => ">",
            CmpOp::Ge => ">=",
            CmpOp::Eq => "=",
            CmpOp::Ne => "!=",
        }
    }
}

#[derive(Clone, Debug)]
enum Operand {
    Var(Var),
    Const(V),
}

#[derive(Clone, Debug)]
struct Pred {
    op: CmpOp,
    l: Var,
    r: Operand,
}

#[derive(Clone, Copy, Debug)]
struct Temporal {
    /// Raw `:as-of`; reduced modulo (tx count + 2) when the query runs.
    as_of: Option<u16>,
    at: At,
}

#[derive(Clone, Copy, Debug)]
enum At {
    Now,
    /// An index into `AT_YEARS`.
    Year(usize),
    Any,
}

#[derive(Clone, Debug)]
struct Query {
    pats: Vec<Pat>,
    neg: Option<Neg>,
    pred: Option<Pred>,
    temporal: Temporal,
    /// The variables in `:find`, in order.
    find: Vec<Var>,
}

/// Variables bound by the patterns, in order of first appearance.
fn bound_vars(pats: &[Pat]) -> Vec<Var> {
    let mut out: Vec<Var> = Vec::new();
    for x in pats.iter().flat_map(Pat::vars) {
        if !out.contains(&x) {
            out.push(x);
        }
    }
    out
}

impl Query {
    fn edn(&self, tx_count: u64) -> String {
        let mut opts = String::new();
        if let Some(n) = self.as_of(tx_count) {
            opts.push_str(&format!(":as-of {n} "));
        }
        match self.temporal.at {
            At::Now => {}
            At::Year(i) => opts.push_str(&format!(":valid-at \"{}-01-01\" ", AT_YEARS[i])),
            At::Any => opts.push_str(":any-valid-time "),
        }
        let mut clauses: Vec<String> = self.pats.iter().map(Pat::edn).collect();
        match &self.neg {
            None => {}
            Some(Neg::Not(p)) => clauses.push(format!("(not {})", p.edn())),
            Some(Neg::NotJoin { var, a, v }) => {
                let v = v.as_ref().map_or("?z".to_string(), V::edn);
                let x = var.name();
                clauses.push(format!("(not-join [{x}] [{x} {} {v}])", ATTRS[*a]));
            }
        }
        if let Some(p) = &self.pred {
            let r = match &p.r {
                Operand::Var(x) => x.name(),
                Operand::Const(v) => v.edn(),
            };
            clauses.push(format!("[({} {} {r})]", p.op.edn(), p.l.name()));
        }
        let find: Vec<String> = self.find.iter().map(|x| x.name()).collect();
        format!(
            "(query [:find {} {opts}:where {}])",
            find.join(" "),
            clauses.join(" ")
        )
    }

    fn as_of(&self, tx_count: u64) -> Option<u64> {
        self.temporal.as_of.map(|n| u64::from(n) % (tx_count + 2))
    }

    /// Rows are compared as multisets when each binding is a distinct row:
    /// every variable is in `:find` and no pattern has `_`.
    fn distinct_rows(&self) -> bool {
        self.find.len() == bound_vars(&self.pats).len()
            && self
                .pats
                .iter()
                .all(|p| p.e != Term::Blank && p.v != Term::Blank)
    }
}

fn arb_var() -> impl Strategy<Value = Var> {
    prop_oneof![(0u8..3).prop_map(Var::E), (0u8..3).prop_map(Var::V)]
}

fn arb_e_term() -> impl Strategy<Value = Term> {
    prop_oneof![
        4 => (0u8..3).prop_map(|i| Term::Var(Var::E(i))),
        1 => (0..ENTITIES).prop_map(Term::Entity),
    ]
}

fn arb_v_term() -> impl Strategy<Value = Term> {
    prop_oneof![
        4 => (0u8..3).prop_map(|i| Term::Var(Var::V(i))),
        2 => (0u8..3).prop_map(|i| Term::Var(Var::E(i))),
        1 => arb_dense_v().prop_map(Term::Const),
        1 => Just(Term::Blank),
    ]
}

fn arb_pat() -> impl Strategy<Value = Pat> {
    (
        arb_e_term(),
        prop::option::weighted(0.9, 0..ATTRS.len()),
        arb_v_term(),
    )
        .prop_map(|(e, a, v)| Pat { e, a, v })
}

fn arb_cmp() -> impl Strategy<Value = CmpOp> {
    prop_oneof![
        Just(CmpOp::Lt),
        Just(CmpOp::Le),
        Just(CmpOp::Gt),
        Just(CmpOp::Ge),
        Just(CmpOp::Eq),
        Just(CmpOp::Ne),
    ]
}

/// Raw query parts; `build_query` turns them into a well-formed query.
type RawQuery = (
    Vec<Pat>,
    Option<(bool, Pat, usize, Option<V>)>,
    Option<(CmpOp, bool, V)>,
    Temporal,
    Vec<bool>,
    (usize, usize),
);

fn arb_temporal() -> impl Strategy<Value = Temporal> {
    (
        prop::option::weighted(0.4, any::<u16>()),
        prop_oneof![
            2 => Just(At::Now),
            2 => (0..AT_YEARS.len()).prop_map(At::Year),
            1 => Just(At::Any),
        ],
    )
        .prop_map(|(as_of, at)| Temporal { as_of, at })
}

fn arb_query() -> impl Strategy<Value = Query> {
    (
        prop::collection::vec(arb_pat(), 1..4),
        prop::option::weighted(
            0.35,
            (
                any::<bool>(),
                arb_pat(),
                0..ATTRS.len(),
                prop::option::of(arb_dense_v()),
            ),
        ),
        prop::option::weighted(0.35, (arb_cmp(), any::<bool>(), arb_dense_v())),
        arb_temporal(),
        prop::collection::vec(any::<bool>(), 8),
        (any::<usize>(), any::<usize>()),
    )
        .prop_map(build_query)
}

/// Pick the `n`-th (mod len) of `vars` that satisfies `ok`.
fn pick(vars: &[Var], n: usize, ok: impl Fn(Var) -> bool) -> Option<Var> {
    let cands: Vec<Var> = vars.iter().copied().filter(|x| ok(*x)).collect();
    (!cands.is_empty()).then(|| cands[n % cands.len()])
}

fn build_query(raw: RawQuery) -> Query {
    let (mut pats, neg, pred, temporal, find_mask, (n1, n2)) = raw;

    // At least one variable, so `:find` is not empty.
    if bound_vars(&pats).is_empty() {
        pats[0].e = Term::Var(Var::E(0));
    }
    // Each pattern after the first shares a variable with the ones before it
    // when they bind any, so no query is a cross product.
    for i in 1..pats.len() {
        let before = bound_vars(&pats[..i]);
        if before.is_empty() || pats[i].vars().iter().any(|x| before.contains(x)) {
            continue;
        }
        let pat = &mut pats[i];
        if let Some(x) = pick(&before, n1 + i, |x| matches!(x, Var::E(_))) {
            pat.e = Term::Var(x);
        } else if let Some(x) = pick(&before, n1 + i, |x| x != Var::A) {
            pat.v = Term::Var(x);
        } else {
            pat.a = None;
        }
    }
    let bound = bound_vars(&pats);

    let neg = neg.and_then(|(plain, mut p, a, v)| {
        if plain {
            // `not` over bound variables only: an unbound one becomes `_`.
            let fix = |t: &mut Term, pos_e: bool| {
                if let Term::Var(x) = *t
                    && (!bound.contains(&x) || (pos_e && matches!(x, Var::V(_))))
                {
                    *t = if pos_e { Term::Entity(0) } else { Term::Blank };
                }
            };
            fix(&mut p.e, true);
            fix(&mut p.v, false);
            if p.a.is_none() && !bound.contains(&Var::A) {
                p.a = Some(a);
            }
            Some(Neg::Not(p))
        } else {
            pick(&bound, n2, |x| matches!(x, Var::E(_))).map(|var| Neg::NotJoin { var, a, v })
        }
    });

    let pred = pred.and_then(|(op, var_rhs, c)| {
        // Mostly a value variable: orderings over entities are type errors.
        let l = pick(&bound, n1, |x| matches!(x, Var::V(_)))
            .or_else(|| pick(&bound, n1, |x| x != Var::A))?;
        let r = match pick(&bound, n2, |x| x != Var::A && x != l) {
            Some(x) if var_rhs => Operand::Var(x),
            // Expressions take no `#uuid` literal (PRS-070).
            _ if matches!(c, V::Ref(_)) => Operand::Const(V::Int(1)),
            _ => Operand::Const(c),
        };
        Some(Pred { op, l, r })
    });

    let mut find: Vec<Var> = bound
        .iter()
        .zip(find_mask.iter().cycle())
        .filter(|(_, keep)| **keep)
        .map(|(x, _)| *x)
        .collect();
    if find.is_empty() {
        find.push(bound[n1 % bound.len()]);
    }

    Query {
        pats,
        neg,
        pred,
        temporal,
        find,
    }
}

// ── Reference query evaluator (#386) ──────────────────────────────────────────

/// A bound value: a pool value (entities are `V::Ref`) or an attribute.
#[derive(Clone, Debug, PartialEq)]
enum B {
    Val(V),
    Attr(usize),
}

impl B {
    fn render(&self) -> String {
        match self {
            B::Val(v) => v.render(),
            B::Attr(a) => attr_render(*a),
        }
    }
}

type Binding = Vec<(Var, B)>;

fn lookup(b: &Binding, x: Var) -> Option<&B> {
    b.iter().find(|(y, _)| *y == x).map(|(_, v)| v)
}

/// Unify `term` with `value` under `b`; `None` when they conflict.
fn unify(mut b: Binding, term: &Term, value: B) -> Option<Binding> {
    match term {
        Term::Blank => Some(b),
        Term::Entity(e) => (value == B::Val(V::Ref(*e))).then_some(b),
        Term::Const(c) => (value == B::Val(c.clone())).then_some(b),
        Term::Var(x) => match lookup(&b, *x) {
            Some(bound) => (*bound == value).then_some(b),
            None => {
                b.push((*x, value));
                Some(b)
            }
        },
    }
}

/// Match one pattern against one fact under `b`.
fn match_fact(b: &Binding, p: &Pat, f: &Live) -> Option<Binding> {
    let mut b = unify(b.clone(), &p.e, B::Val(V::Ref(f.e)))?;
    b = match p.a {
        Some(a) => (a == f.a).then_some(b)?,
        None => unify(b, &Term::Var(Var::A), B::Attr(f.a))?,
    };
    unify(b, &p.v, B::Val(f.v.clone()))
}

/// `eval_binop` for a comparison: `None` is a type error, which drops the row.
fn compare(op: CmpOp, l: &B, r: &B) -> Option<bool> {
    match op {
        CmpOp::Eq => return Some(l == r),
        CmpOp::Ne => return Some(l != r),
        _ => {}
    }
    let num = |b: &B| match b {
        B::Val(V::Int(n)) => Some(*n as f64),
        B::Val(V::Float) => Some(1.5),
        _ => None,
    };
    let ord = match (l, r) {
        (B::Val(V::Str(a)), B::Val(V::Str(b))) => Some(pool_string(*a).cmp(&pool_string(*b))),
        _ => num(l)?.partial_cmp(&num(r)?),
    }?;
    Some(match op {
        CmpOp::Lt => ord.is_lt(),
        CmpOp::Le => ord.is_le(),
        CmpOp::Gt => ord.is_gt(),
        CmpOp::Ge => ord.is_ge(),
        CmpOp::Eq | CmpOp::Ne => unreachable!(),
    })
}

fn ref_query(model: &Model, q: &Query) -> Vec<String> {
    let at = match q.temporal.at {
        At::Now => Some(None),
        At::Year(i) => Some(Some(jan1_ms(AT_YEARS[i]))),
        At::Any => None,
    };
    let facts: Vec<Live> = model
        .live(q.as_of(model.tx_count))
        .into_iter()
        .filter(|l| at.is_none_or(|at| valid_at(l.window, at)))
        .collect();

    let mut bindings: Vec<Binding> = vec![Vec::new()];
    for p in &q.pats {
        bindings = bindings
            .iter()
            .flat_map(|b| facts.iter().filter_map(|f| match_fact(b, p, f)))
            .collect();
    }
    match &q.neg {
        None => {}
        Some(Neg::Not(p)) => {
            bindings.retain(|b| !facts.iter().any(|f| match_fact(b, p, f).is_some()));
        }
        Some(Neg::NotJoin { var, a, v }) => {
            bindings.retain(|b| {
                let x = lookup(b, *var);
                !facts.iter().any(|f| {
                    x == Some(&B::Val(V::Ref(f.e)))
                        && f.a == *a
                        && v.as_ref().is_none_or(|v| *v == f.v)
                })
            });
        }
    }
    if let Some(p) = &q.pred {
        bindings.retain(|b| {
            let l = lookup(b, p.l);
            let r = match &p.r {
                Operand::Var(x) => lookup(b, *x).cloned(),
                Operand::Const(c) => Some(B::Val(c.clone())),
            };
            match (l, r) {
                (Some(l), Some(r)) => compare(p.op, l, &r) == Some(true),
                _ => false,
            }
        });
    }
    bindings
        .iter()
        .map(|b| {
            q.find
                .iter()
                .map(|x| lookup(b, *x).map_or("?".to_string(), B::render))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

// ── Driver (#386) ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
enum Reopen {
    Default,
    SmallCache,
}

fn open(path: &Path, mode: Reopen) -> Result<Minigraf, TestCaseError> {
    let opts = OpenOptions::new().synchronous(SyncMode::Normal);
    let opts = match mode {
        Reopen::Default => opts,
        Reopen::SmallCache => opts.page_cache_size(4),
    };
    Minigraf::open_with_options(path, opts)
        .map_err(|e| TestCaseError::fail(format!("open failed ({})", e.code())))
}

/// Write `history` to `db` and the model. `Checkpoint` runs only when
/// `checkpoints` is set (the file backend).
fn write_history(
    db: &Minigraf,
    model: &mut Model,
    history: &[HistoryOp],
    checkpoints: bool,
) -> Result<(), TestCaseError> {
    let fail = |what: &str, i: usize, code: &str| {
        TestCaseError::fail(format!("op {i}: {what} failed ({code})"))
    };
    for (i, op) in history.iter().enumerate() {
        match op {
            HistoryOp::Exec(stmt) => {
                db.execute(&stmt.edn())
                    .map_err(|e| fail("write", i, e.code()))?;
                model.commit(std::slice::from_ref(stmt));
            }
            HistoryOp::WriteTx(stmts) => {
                let mut tx = db.begin_write().map_err(|e| fail("begin", i, e.code()))?;
                for stmt in stmts {
                    tx.execute(&stmt.edn())
                        .map_err(|e| fail("statement", i, e.code()))?;
                }
                tx.commit().map_err(|e| fail("commit", i, e.code()))?;
                model.commit(stmts);
            }
            HistoryOp::Checkpoint => {
                if checkpoints {
                    db.checkpoint()
                        .map_err(|e| fail("checkpoint", i, e.code()))?;
                }
            }
        }
    }
    Ok(())
}

fn check_queries(
    db: &Minigraf,
    ctx: &Ctx,
    model: &Model,
    queries: &[Query],
    stage: &str,
) -> Result<(), TestCaseError> {
    prop_assert_eq!(db.current_tx_count(), model.tx_count, "{}: tx count", stage);
    for q in queries {
        let edn = q.edn(model.tx_count);
        let got: Vec<String> = ctx
            .rows(db, &edn)?
            .into_iter()
            .map(|r| r.join(" "))
            .collect();
        let want = ref_query(model, q);
        if q.distinct_rows() {
            prop_assert_eq!(sorted(got), sorted(want), "{}: {}", stage, edn);
        } else {
            let set = |rows: Vec<String>| {
                let mut rows = sorted(rows);
                rows.dedup();
                rows
            };
            prop_assert_eq!(set(got), set(want), "{}: {}", stage, edn);
        }
    }
    Ok(())
}

fn ctx() -> Ctx {
    Ctx {
        uuids: (0..ENTITIES).map(|i| (entity_uuid(i), i)).collect(),
    }
}

fn run_in_memory(history: &[HistoryOp], queries: &[Query]) -> Result<(), TestCaseError> {
    let db = Minigraf::in_memory().map_err(|_| TestCaseError::fail("in_memory failed"))?;
    let mut model = Model::default();
    write_history(&db, &mut model, history, false)?;
    check_queries(&db, &ctx(), &model, queries, "in memory")
}

fn run_file_backed(
    history: &[HistoryOp],
    queries: &[Query],
    reopen: Reopen,
) -> Result<(), TestCaseError> {
    let dir = tempfile::tempdir().map_err(|_| TestCaseError::fail("tempdir"))?;
    let path = dir.path().join("prop.graph");
    let ctx = ctx();
    let mut model = Model::default();
    let db = open(&path, Reopen::Default)?;
    write_history(&db, &mut model, history, true)?;
    check_queries(&db, &ctx, &model, queries, "file, open")?;
    drop(db);
    let db = open(&path, reopen)?;
    check_queries(&db, &ctx, &model, queries, "file, reopened")
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Joins, negation, predicates and temporal queries over a random history
    /// match the reference evaluator.
    #[test]
    fn query_matches_reference_in_memory(
        history in arb_history(),
        queries in prop::collection::vec(arb_query(), 1..5),
    ) {
        run_in_memory(&history, &queries)?;
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases((cases() / 10).max(1)))]

    /// The same against a file: checkpoints within the history, and queries
    /// both on the open handle and after a reopen.
    #[test]
    fn query_matches_reference_file_backed(
        history in arb_history(),
        queries in prop::collection::vec(arb_query(), 1..5),
        reopen in prop_oneof![Just(Reopen::Default), Just(Reopen::SmallCache)],
    ) {
        run_file_backed(&history, &queries, reopen)?;
    }
}
