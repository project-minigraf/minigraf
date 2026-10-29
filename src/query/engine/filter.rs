//! `Filter` and `Bind` (#473 spec §6.1), and the pseudo-attribute patterns
//! on a scanned fact: the clauses after the scan, one batch at a time.
//!
//! An expression sees only the variables it reads, turned into values for
//! the row; a binding expression's result becomes a cell again.

use super::batch::Batch;
use super::cell::{Cell, join_eq};
use super::plan::{Out, Step, Term, Vars};
use super::project::value;
use super::{Counters, Ctx, Operator};
use crate::query::datalog::executor::{EvalError, LiteralKeywords, eval_expr, is_truthy};
use crate::query::datalog::types::Expr;
use anyhow::Result;
use std::collections::HashMap;

/// One clause after the scan, over the batches of its child.
pub(crate) struct Apply {
    child: Box<dyn Operator>,
    step: Step,
    counters: Counters,
    keywords: LiteralKeywords,
}

impl Apply {
    pub(crate) fn new(child: Box<dyn Operator>, step: Step) -> Self {
        let keywords = match &step {
            Step::Filter(expr, _) | Step::Bind(expr, ..) => LiteralKeywords::new(expr),
            _ => LiteralKeywords::default(),
        };
        let op = match step {
            Step::Pseudo { .. } => "Pseudo",
            Step::Filter(..) => "Filter",
            Step::Bind(..) => "Bind",
            Step::Join(_) => "Join",
            Step::CollectSet(_) => "CollectSet",
            Step::Anti(_) => "AntiJoin",
            Step::Union(_) => "Union",
            Step::Distinct(_) | Step::DistinctFind(_) => "Distinct",
            Step::Aggregate { .. } => "Aggregate",
            Step::Window { .. } => "Window",
            Step::Sort { .. } => "Sort",
            Step::Limit { .. } => "Limit",
        };
        Apply {
            child,
            step,
            counters: Counters::new(op),
            keywords,
        }
    }
}

impl Operator for Apply {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        while let Some(batch) = self.child.next_batch(ctx)? {
            self.counters.rows_in += batch.len as u64;
            let batch = apply(&self.step, &self.keywords, batch, ctx)?;
            if batch.len > 0 {
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.child.rewind(seed);
    }

    fn close(&mut self) {
        self.child.close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(self.counters);
        self.child.counters(out);
    }
}

/// Apply `step` to `batch`.
fn apply(
    step: &Step,
    keywords: &LiteralKeywords,
    mut batch: Batch,
    ctx: &mut Ctx<'_>,
) -> Result<Batch> {
    match step {
        Step::Pseudo { col, value } => {
            let meta = batch.cols.get(*col).cloned().unwrap_or_default();
            match value {
                Term::Any => {}
                Term::Bind(slot) => batch.set_col(*slot, meta),
                Term::Const(n) => {
                    let want = n.clone().map(Cell::Val);
                    let keep: Vec<bool> = meta.iter().map(|c| want.as_ref() == Some(c)).collect();
                    batch.retain(&keep);
                }
                Term::Never => batch.retain(&vec![false; batch.len]),
                Term::Eq(slot) => {
                    let other = batch.cols.get(*slot);
                    let keep: Vec<bool> = meta
                        .iter()
                        .enumerate()
                        .map(|(i, c)| other.and_then(|o| o.get(i)) == Some(c))
                        .collect();
                    batch.retain(&keep);
                }
            }
        }
        Step::Filter(expr, vars) => {
            let mut keep = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                keep.push(
                    eval(expr, keywords, vars, &batch, i, ctx)?.is_some_and(|v| is_truthy(&v)),
                );
            }
            batch.retain(&keep);
        }
        Step::Bind(expr, vars, slot, mode) => {
            let mut keep = Vec::with_capacity(batch.len);
            let mut out = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                let Some(v) = eval(expr, keywords, vars, &batch, i, ctx)? else {
                    keep.push(false);
                    continue;
                };
                let cell = Cell::of_value(ctx.ids, ctx.keys, &v)?;
                let old = match mode {
                    Out::New => None,
                    Out::Check | Out::Replace => batch.cols.get(*slot).and_then(|c| c.get(i)),
                };
                match old {
                    // The executor normalises every bound variable (#596).
                    Some(old) if !join_eq(old, &cell, true, ctx)? => keep.push(false),
                    Some(old) if *mode == Out::Check => {
                        out.push(old.clone());
                        keep.push(true);
                    }
                    _ => {
                        out.push(cell);
                        keep.push(true);
                    }
                }
            }
            batch.retain(&keep);
            batch.set_col(*slot, out);
        }
        // Joins are operators of their own (`join::Join`, `anti::AntiJoin`,
        // `union::Union`, `distinct::Distinct`, `collect::CollectSet`, `sort::Sort`,
        // `limit::Limit`).
        Step::Join(_)
        | Step::CollectSet(_)
        | Step::Anti(_)
        | Step::Union(_)
        | Step::Distinct(_)
        | Step::DistinctFind(_)
        | Step::Aggregate { .. }
        | Step::Window { .. }
        | Step::Sort { .. }
        | Step::Limit { .. } => {}
    }
    Ok(batch)
}

/// `expr` over row `i`, or `None` where it has no value; a failing user
/// predicate fails the query (`QRY-014`, #180).
fn eval(
    expr: &Expr,
    keywords: &LiteralKeywords,
    vars: &Vars,
    batch: &Batch,
    i: usize,
    ctx: &mut Ctx<'_>,
) -> Result<Option<crate::graph::types::Value>> {
    let mut binding = HashMap::with_capacity(vars.len());
    for (name, slot) in vars {
        if let Some(cell) = batch.cols.get(*slot).and_then(|c| c.get(i)) {
            binding.insert(name.clone(), value(cell, ctx)?);
        }
    }
    match eval_expr(expr, &binding, Some(ctx.functions), keywords) {
        Ok(v) => Ok(Some(v)),
        Err(EvalError::NoValue) => Ok(None),
        Err(EvalError::Udf(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use crate::Minigraf;
    use crate::query::datalog::matcher::KEYWORD_RESOLUTIONS;

    /// Literal resolution work must not grow with the number of input rows,
    /// including nested expressions and operators inside rule/negation bodies.
    #[test]
    fn literal_keyword_resolution_is_independent_of_row_count() {
        let cases = [
            "[:find ?f :where [?f :name ?n] [(= ?f :p5)]]",
            "[:find ?f :where [?f :name ?n] [(= :p5 ?f)]]",
            "[:find ?f :where [?f :name ?n] [(!= ?f :p5)]]",
            "[:find ?f :where [?f :name ?n] [(!= :p5 ?f)]]",
            "[:find ?f :where [?f :name ?n] [(= (= ?f :p5) true)]]",
            "[:find ?b :where [?f :name ?n] [(= :p5 ?f) ?b]]",
            "[:find ?f :where [?f :name ?n] (not [(= ?f :p5)])]",
            "[:find ?f :where [?f :name ?n] (or [(= ?f :p5)] [(= ?f :p6)])]",
            "[:find ?f :where (chosen ?f)]",
        ];
        let counts = |rows| {
            let db = Minigraf::in_memory().unwrap();
            let facts: String = (0..rows).map(|i| format!("[:p{i} :name {i}]")).collect();
            db.execute(&format!("(transact [{facts}])")).unwrap();
            db.execute("(rule [(chosen ?f) [?f :name ?n] [(= ?f :p5)]])")
                .unwrap();
            cases.map(|q| {
                KEYWORD_RESOLUTIONS.with(|n| n.set(0));
                db.execute(&format!("(query {q})")).unwrap();
                KEYWORD_RESOLUTIONS.with(|n| n.get())
            })
        };
        let (small, large) = (counts(8), counts(2_048));
        for ((a, b), q) in small.into_iter().zip(large).zip(cases) {
            assert_eq!(a, b, "literal resolution count grows with input: {q}");
        }
    }
}
