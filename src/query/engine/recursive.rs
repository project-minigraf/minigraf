//! `Recursive` (#548 spec §5.3): the relations of a rule query, derived
//! stratum by stratum by semi-naive rounds before its body runs.
//!
//! Round 1 runs every rule with every read `Full`. A later round runs each
//! rule once per top-level invocation of its stratum, that invocation read
//! as `Delta`, those before it `Old` and those after it `Full`, so every head
//! it gives uses a fact the last round derived; a naive rule runs in full
//! every round. A head is new when its relation does not know it; new heads
//! are appended when the round ends, and a round that appends nothing ends
//! the stratum.

use super::cell::{Cell, JoinKey, Scalar};
use super::ids::IdSpace;
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, cell_bytes, growth, key_bytes};
use super::plan::{HeadArg, RuleRun, RulesPlan, ScanSpec, Source, Step, Term};
use super::relation::Relations;
use super::scan::Scan;
use super::{Ctx, Operator, build};
use crate::error::{ErrorCode, err_coded};
use crate::graph::storage::Snapshot;
use crate::query::datalog::aggregate::type_name;
use crate::query::datalog::evaluator::{derived_facts_exceeded, iterations_exceeded};
use crate::query::datalog::functions::FunctionRegistry;
use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
use crate::query::datalog::types::EdnValue;
use crate::storage::{KeySource, TxBound};
use anyhow::Result;
use std::collections::HashSet;
use std::mem::size_of;
use std::sync::Arc;

/// The limits of one rule query (spec §3.5).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_iterations: usize,
    /// Facts the rules may derive in total; stored facts and magic seeds do
    /// not count.
    pub(crate) max_derived_facts: usize,
    /// The most bytes the query may hold (#549).
    pub(crate) query_memory: usize,
}

/// What every body run reads.
pub(crate) struct Reads<'a> {
    pub(crate) snapshot: &'a Snapshot,
    pub(crate) keys: Option<&'a dyn KeySource>,
    pub(crate) functions: &'a FunctionRegistry,
    /// The versions the query's `:as-of` keeps.
    pub(crate) bound: &'a TxBound,
}

/// Derive every relation of `plan` into `rels`.
pub(crate) fn evaluate(
    plan: &RulesPlan,
    rels: &mut Relations,
    reads: &Reads<'_>,
    ids: &mut IdSpace,
    limits: Limits,
    meter: &Arc<Meter>,
) -> Result<()> {
    let mut derived = 0usize;
    for (stratum_rels, rules) in &plan.strata {
        for &rel in stratum_rels {
            know_stored(plan, rel, rels, reads, ids, meter)?;
        }
        let mut round = 0;
        loop {
            round += 1;
            if round > limits.max_iterations {
                return Err(iterations_exceeded(limits.max_iterations));
            }
            for rule in rules {
                let runs: &[RuleRun] = if round == 1 || rule.naive {
                    std::slice::from_ref(&rule.full)
                } else {
                    &rule.deltas
                };
                for run in runs {
                    let (rows, mut held) =
                        heads(run, &rule.predicate, rule.rel, rels, reads, ids, meter)?;
                    for row in rows {
                        // From the candidates to the relation.
                        held.uncharge(head_bytes(&row));
                        if rels.add(rule.rel, row)? {
                            derived += 1;
                            if derived > limits.max_derived_facts {
                                return Err(derived_facts_exceeded(limits.max_derived_facts));
                            }
                        }
                    }
                }
            }
            if !rels.end_round() {
                break;
            }
        }
    }
    Ok(())
}

/// Record the stored facts of relation `rel`, so that a head equal to one
/// is not derived (spec §5.1).
fn know_stored(
    plan: &RulesPlan,
    rel: usize,
    rels: &mut Relations,
    reads: &Reads<'_>,
    ids: &mut IdSpace,
    meter: &Arc<Meter>,
) -> Result<()> {
    let Some(attr) = plan.evaluated.attrs.get(rel) else {
        return Ok(());
    };
    let mut scan = Scan::new(ScanSpec {
        index: None,
        e: Term::Bind(0),
        a: Term::Const(attr.clone()),
        v: Term::Bind(1),
        meta: Vec::new(),
        valid: plan.valid,
        width: 2,
        derived: None,
    });
    let mut stored: Vec<(Cell, Cell)> = Vec::new();
    // The buffer of stored rows, released on return (#549 spec §3.2).
    let mut held = Held::new(meter, "Recursive");
    {
        let mut ctx = ctx(reads, ids, None);
        while let Some(batch) = scan.next_batch(&mut ctx)? {
            let (Some(es), Some(vs)) = (batch.cols.first(), batch.cols.get(1)) else {
                continue;
            };
            for (e, v) in es.iter().zip(vs) {
                let n = cell_bytes(e) + cell_bytes(v);
                held.charge(n)?;
                let refused = growth(stored.capacity(), size_of::<(Cell, Cell)>());
                held.reserve(stored.try_reserve(1), n, refused)?;
                stored.push((e.clone(), v.clone()));
            }
        }
    }
    ids.end_pull();
    for (e, v) in stored {
        rels.know(rel, &[e, v])?;
    }
    Ok(())
}

/// The heads one run of a rule's body gives that `rels` does not know yet,
/// each once, and what they hold on the meter until the caller drops it.
#[allow(clippy::too_many_arguments)]
fn heads(
    run: &RuleRun,
    predicate: &str,
    rel: usize,
    rels: &Relations,
    reads: &Reads<'_>,
    ids: &mut IdSpace,
    meter: &Arc<Meter>,
) -> Result<(Vec<[Cell; 2]>, Held)> {
    let run = choose_run(run, rels, reads, ids)?;
    let mut out: Vec<[Cell; 2]> = Vec::new();
    let mut seen: HashSet<[JoinKey; 2]> = HashSet::new();
    let mut held = Held::new(meter, "Recursive");
    {
        let mut ctx = ctx(reads, ids, Some(rels));
        let mut root = build(run.plan.source.clone(), run.plan.steps.clone(), meter);
        // Constant head arguments' cells, read at the first row.
        let mut konst: [Option<Cell>; 2] = [None, None];
        while let Some(batch) = root.next_batch(&mut ctx)? {
            for i in 0..batch.len {
                let cell = |slot: usize| batch.cols.get(slot).and_then(|c| c.get(i));
                let row = [
                    entity(&run.entity, predicate, cell, &mut ctx, &mut konst[0])?,
                    value(run.value.as_ref(), cell, &mut ctx, &mut konst[1])?,
                ];
                if rels.known(rel, &row) {
                    continue;
                }
                let k = [JoinKey::ident(&row[0]), JoinKey::ident(&row[1])];
                if seen.contains(&k) {
                    continue;
                }
                // The candidate row and its key (#549 spec §3.2).
                let n = head_bytes(&row);
                held.charge(n)?;
                let key = key_bytes(&k[0]) + key_bytes(&k[1]) + HASH_ENTRY_OVERHEAD;
                let refused = growth(seen.capacity(), key)
                    .saturating_add(growth(out.capacity(), size_of::<[Cell; 2]>()));
                let reserved = seen.try_reserve(1).and_then(|()| out.try_reserve(1));
                held.reserve(reserved, n, refused)?;
                seen.insert(k);
                out.push(row);
            }
        }
        root.close();
    }
    ids.end_pull();
    Ok((out, held))
}

/// Reconsider each round, before opening operators. A small relation can
/// avoid the initial whole-attribute read of an in-memory value probe.
/// Committed databases keep AVET probes; a value map already built is free
/// to reuse. The cap bounds both costing work and the alternative's seed.
fn choose_run<'a>(
    run: &'a RuleRun,
    rels: &Relations,
    reads: &Reads<'_>,
    ids: &IdSpace,
) -> Result<&'a RuleRun> {
    let Some(alternative) = run.entity_first.as_deref() else {
        return Ok(run);
    };
    // In-memory databases also have an empty committed reader. With no
    // allocated entity ids it cannot contain a fact; populated generations
    // keep their AVET path. This is metadata-only, with no index read.
    if reads.keys.is_some_and(|k| k.next_ids().0 > 1) {
        return Ok(run);
    }
    let (Source::Scan(delta), Source::Scan(source)) = (&run.plan.source, &alternative.plan.source)
    else {
        return Ok(run);
    };
    let count = |s: &ScanSpec| {
        s.derived.as_ref().map_or(0, |d| {
            d.rels.iter().fold(0usize, |n, &r| {
                n.saturating_add(rels.span(r, d.range).len())
            })
        })
    };
    let n = count(source);
    if count(delta) == 0 || n > 128 {
        return Ok(run);
    }
    let (Term::Const(source_attr), Some(Step::Join(join))) =
        (&source.a, alternative.plan.steps.first())
    else {
        return Ok(run);
    };
    let Term::Const(attribute) = &join.inner.a else {
        return Ok(run);
    };
    if ids.probes().is_some_and(|p| p.has_values(attribute))
        || reads.snapshot.has_pending(source_attr, None)?
    {
        return Ok(run);
    }
    // Allow eight records per source row for the entity lookup and the
    // final delta probe. Count at most 1,024 snapshot-visible records;
    // later appends are skipped and no facts are cloned.
    let cost = n * 8;
    if reads
        .snapshot
        .pending_attribute_count(attribute, None, cost)?
        >= cost
    {
        Ok(alternative)
    } else {
        Ok(run)
    }
}

/// A candidate head and its key in `heads`' set.
fn head_bytes(row: &[Cell; 2]) -> usize {
    let key = key_bytes(&JoinKey::ident(&row[0])) + key_bytes(&JoinKey::ident(&row[1]));
    cell_bytes(&row[0]) + cell_bytes(&row[1]) + key + HASH_ENTRY_OVERHEAD
}

fn ctx<'a>(reads: &Reads<'a>, ids: &'a mut IdSpace, derived: Option<&'a Relations>) -> Ctx<'a> {
    Ctx {
        snapshot: reads.snapshot,
        keys: reads.keys,
        ids,
        functions: reads.functions,
        derived,
        bound: reads.bound,
    }
}

/// The head's entity cell (spec §5.3): an entity, or the entity a keyword
/// names; anything else is `QRY-013`. A constant's is kept in `konst`.
fn entity<'c>(
    arg: &HeadArg,
    predicate: &str,
    cell: impl Fn(usize) -> Option<&'c Cell>,
    ctx: &mut Ctx<'_>,
    konst: &mut Option<Cell>,
) -> Result<Cell> {
    let kind = match arg {
        HeadArg::Unbound(v) => return Err(unbound(v)),
        HeadArg::Const(c) => {
            if let Some(k) = konst {
                return Ok(k.clone());
            }
            // As the executor reads a head (`evaluator.rs`).
            let k = match edn_to_entity_id(c) {
                Ok(u) => {
                    let e = ctx.ids.eid(ctx.keys, &u)?;
                    // Value probes find this keyword's facts from the
                    // entity (#721).
                    if let EdnValue::Keyword(name) = c {
                        let i = ctx.ids.iid(ctx.keys, name)?;
                        ctx.ids.probes_mut().note(e, i)?;
                    }
                    Cell::Eid(e)
                }
                Err(_) => {
                    let kind = edn_to_value(c).map_or("symbol", |v| type_name(&v));
                    return Err(err_coded!(ErrorCode::Qry013, predicate, kind));
                }
            };
            *konst = Some(k.clone());
            return Ok(k);
        }
        HeadArg::Slot(s) => match cell(*s) {
            Some(Cell::Eid(e)) => return Ok(Cell::Eid(*e)),
            Some(Cell::Iid(i)) => {
                let e = ctx.ids.keyword_eid(ctx.keys, *i)?;
                ctx.ids.probes_mut().note(e, *i)?;
                return Ok(Cell::Eid(e));
            }
            Some(Cell::Val(v)) => match v {
                // The executor reads a `nil` back as a symbol.
                Scalar::Null => "symbol",
                Scalar::Bool(_) => "Boolean",
                Scalar::Int(_) => "Integer",
                Scalar::Float(_) => "Float",
                Scalar::Str(_) | Scalar::Long(_) => "String",
                Scalar::Set(_) => "Set",
            },
            None => return Err(unbound("?")),
        },
    };
    Err(err_coded!(ErrorCode::Qry013, predicate, kind))
}

/// The head's value cell: any value but `nil`, which the executor cannot
/// read back (`INT-022`); `true` for a one-argument head. A constant's is
/// kept in `konst`.
fn value<'c>(
    arg: Option<&HeadArg>,
    cell: impl Fn(usize) -> Option<&'c Cell>,
    ctx: &mut Ctx<'_>,
    konst: &mut Option<Cell>,
) -> Result<Cell> {
    match arg {
        None => Ok(Cell::Val(Scalar::Bool(true))),
        Some(HeadArg::Unbound(v)) => Err(unbound(v)),
        Some(HeadArg::Const(c)) => {
            if let Some(k) = konst {
                return Ok(k.clone());
            }
            let v = edn_to_value(c).map_err(|e| {
                err_coded!(ErrorCode::Int022, format!("Failed to convert value: {e}"))
            })?;
            let k = Cell::of_value(ctx.ids, ctx.keys, &v)?;
            *konst = Some(k.clone());
            Ok(k)
        }
        Some(HeadArg::Slot(s)) => match cell(*s) {
            Some(Cell::Val(Scalar::Null)) | None => Err(err_coded!(
                ErrorCode::Int022,
                "Failed to convert value: nil"
            )),
            Some(c) => Ok(c.clone()),
        },
    }
}

fn unbound(var: &str) -> anyhow::Error {
    err_coded!(
        ErrorCode::Int022,
        format!("Unbound variable in rule head: {var}")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::storage::FactStorage;
    use crate::query::datalog::{
        magic_sets, parser::parse_datalog_command, rules::RuleRegistry, types::DatalogCommand,
    };

    /// Costing must stay bounded even when a later round has a much larger
    /// demand relation. Empty deltas must not trigger an unrelated scan.
    #[test]
    fn ancestor_costing_respects_round_and_attribute_boundaries() {
        let mut registry = RuleRegistry::new();
        for text in [
            "(rule [(boss ?x ?y) [?x :manager ?y]])",
            "(rule [(boss ?x ?z) [?x :manager ?y] (boss ?y ?z)])",
        ] {
            let DatalogCommand::Rule(rule) = parse_datalog_command(text).unwrap() else {
                panic!("expected rule");
            };
            registry.register_rule("boss".into(), rule).unwrap();
        }
        let DatalogCommand::Query(query) =
            parse_datalog_command("(query [:find ?b :where (boss :leaf ?b)])").unwrap()
        else {
            panic!("expected query");
        };
        let (registry, seeds) = magic_sets::rewrite(&query, &registry).unwrap();
        let attrs: Vec<_> = seeds.iter().map(|(_, a, _)| a.as_str()).collect();
        let plan = super::super::plan_rules(&query, &registry, &attrs).unwrap();
        let run = plan
            .strata
            .iter()
            .flat_map(|(_, rules)| rules)
            .flat_map(|rule| &rule.deltas)
            .find(|run| run.entity_first.is_some())
            .expect("ancestor plan has an entity-first alternative");
        let alternative = run.entity_first.as_deref().unwrap();
        let Source::Scan(delta) = &run.plan.source else {
            panic!("delta scan");
        };
        let Source::Scan(source) = &alternative.plan.source else {
            panic!("demand scan");
        };
        let delta_rel = delta.derived.as_ref().unwrap().rels[0];
        let source_rel = source.derived.as_ref().unwrap().rels[0];
        let Term::Const(source_attr) = &source.a else {
            panic!("demand attribute");
        };
        // (demand rows, delta rows, pending edges, stored demand, use alternative)
        for (demand, delta, edges, stored_demand, expected) in [
            (1, 0, 100, false, false),
            (1, 1, 7, false, false),
            (1, 1, 8, false, true),
            (128, 1, 1024, false, true),
            (129, 1, 1032, false, false),
            (1, 1, 100, true, false),
        ] {
            let meter = Meter::new(usize::MAX);
            let ids = IdSpace::new(None, &meter);
            let mut rels = Relations::new(0..plan.evaluated.attrs.len() as u64, &meter);
            for i in 0..demand {
                rels.add(source_rel, [Cell::Eid(i), Cell::Val(Scalar::Bool(true))])
                    .unwrap();
            }
            rels.end_round();
            for i in 0..delta {
                rels.add(delta_rel, [Cell::Eid(i), Cell::Eid(i + 1)])
                    .unwrap();
            }
            rels.end_round();
            let storage = FactStorage::new();
            let mut facts: Vec<_> = (0..edges)
                .map(|i| {
                    (
                        uuid::Uuid::from_u128(i + 1),
                        ":manager".into(),
                        crate::Value::Integer(1),
                    )
                })
                .collect();
            if stored_demand {
                facts.push((
                    uuid::Uuid::from_u128(1),
                    source_attr.clone(),
                    crate::Value::Boolean(true),
                ));
            }
            storage.transact(facts, None).unwrap();
            let snapshot = storage.snapshot().unwrap();
            let reads = Reads {
                snapshot: &snapshot,
                keys: None,
                functions: &FunctionRegistry::with_builtins(),
                bound: &TxBound::All,
            };
            let selected = choose_run(run, &rels, &reads, &ids).unwrap();
            assert_eq!(
                std::ptr::eq(selected, alternative),
                expected,
                "demand={demand}, delta={delta}, edges={edges}, stored={stored_demand}"
            );
        }
    }
}
