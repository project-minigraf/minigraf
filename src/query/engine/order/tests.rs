use super::*;
use crate::Value;
use crate::query::engine::plan::{CollectSpec, JoinSpec, Valid};

fn spec(wildcard: bool) -> ScanSpec {
    ScanSpec {
        index: None,
        e: if wildcard { Term::Any } else { Term::Bind(0) },
        a: Term::Const(":score".into()),
        v: Term::Bind(1),
        meta: vec![(Meta::ValidAt, 2)],
        valid: Valid::Any,
        width: 3,
        derived: None,
    }
}

fn proof(kind: ValueKind) -> Ordering {
    Ordering::source(&spec(false), Domain::Single(kind))
}
fn sorted(proof: &Ordering) -> bool {
    proof.satisfies(&[1], &[(0, false)])
}
fn join(kind: JoinKind, replace: Vec<(usize, usize)>) -> Step {
    Step::Join(JoinSpec {
        inner: spec(false),
        kind,
        keys: vec![],
        new: vec![],
        replace,
        outer_width: 3,
    })
}

#[test]
fn full_order_requires_matching_prefix_direction_and_semantic_comparator() {
    let order = proof(ValueKind::Integer);
    assert!(sorted(&order));
    assert!(
        order.satisfies(&[1, 1], &[(0, false), (1, false)]),
        "same slot twice"
    );
    assert!(
        order.satisfies(&[2, 1], &[(0, true), (1, false)]),
        "a leading constant can descend"
    );
    assert!(!order.satisfies(&[1], &[(0, true)]), "no descending range");
    assert!(
        !order.satisfies(&[1, 0], &[(0, false), (1, false)]),
        "DICT order is not UUID order"
    );
    let wildcard = Ordering::source(&spec(true), Domain::Single(ValueKind::Integer));
    assert!(sorted(&wildcard));
    assert!(!wildcard.satisfies(&[1, 0], &[(0, false), (1, false)]));
    assert!(
        !order.satisfies(&[], &[(0, false)]),
        "malformed mapping is no proof"
    );
}

#[test]
fn only_probe_fallbacks_that_stream_the_outer_keep_order() {
    for kind in [
        JoinKind::Probe(0),
        JoinKind::ProbeValue {
            key: 0,
            build_inner: true,
        },
    ] {
        assert!(sorted(
            &proof(ValueKind::Integer)
                .through(&[join(kind, vec![])])
                .unwrap()
        ));
    }
    for kind in [
        JoinKind::Hash { build_inner: true },
        JoinKind::Hash { build_inner: false },
        JoinKind::ProbeValue {
            key: 0,
            build_inner: false,
        },
    ] {
        assert!(
            proof(ValueKind::Integer)
                .through(&[join(kind, vec![])])
                .is_none()
        );
    }
    let suffix = proof(ValueKind::Integer)
        .through(&[join(JoinKind::Probe(0), vec![(1, 0)])])
        .unwrap();
    assert!(
        sorted(&suffix),
        "replacing the later entity cuts only that suffix"
    );
    assert_eq!(suffix.slot, Some(1));
    let first = proof(ValueKind::Integer)
        .through(&[join(JoinKind::Probe(0), vec![(0, 1)])])
        .unwrap();
    assert!(!sorted(&first), "replacing the value removes its proof");
}

#[test]
fn expression_modes_and_pseudo_bindings_handle_constants_and_replacements() {
    let lit = Expr::Lit(Value::Integer(42));
    let new = proof(ValueKind::Integer)
        .through(&[Step::Bind(lit.clone(), vec![], 3, Out::New)])
        .unwrap();
    assert!(new.satisfies(&[3, 1], &[(0, true), (1, false)]));
    let checked = proof(ValueKind::Integer)
        .through(&[Step::Bind(lit.clone(), vec![], 1, Out::Check)])
        .unwrap();
    assert!(sorted(&checked));
    assert!(
        !checked.constants.contains(&1),
        "join-equal checks do not prove an identical constant"
    );
    let replaced = proof(ValueKind::Integer)
        .through(&[Step::Bind(lit, vec![], 1, Out::Replace)])
        .unwrap();
    assert!(replaced.constants.contains(&1));
    assert!(replaced.slot.is_none());
    let replaced = proof(ValueKind::Integer)
        .through(&[Step::Bind(
            Expr::Var("?e".into()),
            vec![("?e".into(), 0)],
            1,
            Out::Replace,
        )])
        .unwrap();
    assert!(!sorted(&replaced));
    assert!(!replaced.constants.contains(&1));
    let pseudo = proof(ValueKind::Integer)
        .through(&[Step::Pseudo {
            col: 2,
            value: Term::Bind(3),
        }])
        .unwrap();
    assert!(
        pseudo.satisfies(&[3, 1], &[(0, true), (1, false)]),
        "valid-at is constant, including nil"
    );
}

#[test]
fn canonicalization_and_buffering_operators_cannot_supply_false_certificates() {
    let int = proof(ValueKind::Integer)
        .through(&[Step::DistinctFind(vec![1])])
        .unwrap();
    assert!(sorted(&int));
    let float = proof(ValueKind::Float)
        .through(&[Step::DistinctFind(vec![1])])
        .unwrap();
    assert!(!sorted(&float));
    for step in [
        Step::Union(vec![]),
        Step::CollectSet(CollectSpec {
            entity: 0,
            attribute: ":a".into(),
            inner: spec(false),
            out: 3,
            mode: Out::New,
        }),
        Step::Aggregate {
            groups: vec![1],
            aggs: vec![("count".into(), 0)],
            zero_row: false,
            grouping: Grouping::Hash,
        },
        Step::Window {
            find: vec![],
            named: vec![],
            keep: vec![1],
        },
        Step::Sort {
            cols: vec![1],
            order: vec![(0, false)],
            keep: None,
        },
    ] {
        assert!(
            proof(ValueKind::Integer).through(&[step]).is_none(),
            "unsupported step clears proof"
        );
    }
}

/// No file or native-only API: WASI/browser tests exercise a real committed
/// reader and the optimized projection, not just the pending-store fallback.
#[cfg_attr(
    all(target_arch = "wasm32", not(target_os = "wasi")),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", not(target_os = "wasi"))), test)]
fn committed_memory_backend_orders_all_supported_scalar_edge_cases() {
    use crate::query::datalog::{
        aggregate::value_order, parser::parse_datalog_command, types::DatalogCommand,
    };
    use crate::storage::{backend::memory::MemoryBackend, persistent_facts::PersistentFactStorage};
    for values in [
        vec![
            Value::Integer(i64::MAX),
            Value::Integer(0),
            Value::Integer(i64::MIN),
            Value::Integer(-1),
        ],
        vec![
            Value::Boolean(true),
            Value::Boolean(false),
            Value::Boolean(true),
        ],
        vec![
            Value::Float(f64::NAN),
            Value::Float(-f64::NAN),
            Value::Float(f64::INFINITY),
            Value::Float(f64::NEG_INFINITY),
            Value::Float(-0.0),
            Value::Float(0.0),
            Value::Float(1.5),
        ],
    ] {
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 32).unwrap();
        let facts = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    uuid::Uuid::from_u128(i as u128 + 1),
                    ":score".into(),
                    v.clone(),
                )
            })
            .collect();
        pfs.storage().transact(facts, None).unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
        assert_eq!(
            pfs.storage().snapshot().unwrap().pending_len(),
            0,
            "committed fixture"
        );
        let DatalogCommand::Query(q) =
            parse_datalog_command("(query [:find ?v :where [?e :score ?v] :order-by [?v]])")
                .unwrap()
        else {
            panic!("query");
        };
        let mut stream = crate::query::engine::open(
            pfs.storage().snapshot().unwrap(),
            crate::query::engine::plan(&q).unwrap(),
            std::sync::Arc::new(FunctionRegistry::with_builtins()),
            usize::MAX,
        )
        .unwrap();
        assert!(
            stream.counters().iter().all(|c| c.op != "Sort"),
            "the portable path is optimized"
        );
        let mut actual = Vec::new();
        while let Some(batch) = stream.next_rows().unwrap() {
            actual.extend(batch);
        }
        let mut expected = values
            .iter()
            .map(|v| match v {
                Value::Float(f) if f.is_nan() => Value::Float(f64::NAN),
                v => v.clone(),
            })
            .collect::<Vec<_>>();
        expected.sort_by(value_order);
        assert_eq!(actual.len(), expected.len());
        for (row, expected) in actual.iter().zip(&expected) {
            match (row.first(), expected) {
                (Some(Value::Float(a)), Value::Float(b)) => assert_eq!(a.to_bits(), b.to_bits()),
                (Some(a), b) => assert!(a == b, "identical scalar value"),
                _ => panic!("one scalar column"),
            }
        }
    }
}

/// Group equality uses Value::Ord: integer and float ranges stay separate,
/// signed zeros share a group and the shared finisher chooses its representative.
#[cfg_attr(
    all(target_arch = "wasm32", not(target_os = "wasi")),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", not(target_os = "wasi"))), test)]
fn committed_group_counts_preserve_float_representatives_and_mixed_numeric_order() {
    use crate::query::datalog::{parser::parse_datalog_command, types::DatalogCommand};
    use crate::storage::{backend::memory::MemoryBackend, persistent_facts::PersistentFactStorage};
    for case in 0..3 {
        let mixed = case == 1;
        let mut values = vec![
            Value::Float(-0.0),
            Value::Float(0.0),
            Value::Float(f64::NAN),
            Value::Float(-f64::NAN),
            Value::Float(-100.0),
            Value::Float(f64::INFINITY),
        ];
        if mixed {
            values.extend([Value::Integer(10), Value::Integer(-10)]);
        } else if case == 2 {
            values = vec![
                Value::Boolean(true),
                Value::Boolean(false),
                Value::Boolean(true),
            ];
        }
        let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 32).unwrap();
        pfs.storage()
            .transact(
                values
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| (uuid::Uuid::from_u128(i as u128 + 1), ":group".into(), v))
                    .collect(),
                None,
            )
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
        for sorted in [false, true] {
            let query = format!(
                "(query [:find (count-distinct ?e) ?g (count ?e) :where [?e :group ?g] {}])",
                if sorted { ":order-by [?g]" } else { "" }
            );
            let DatalogCommand::Query(q) = parse_datalog_command(&query).unwrap() else {
                panic!("query");
            };
            let mut results = Vec::new();
            for optimized in [false, true] {
                let _mode = set_enabled(optimized);
                let mut stream = crate::query::engine::open(
                    pfs.storage().snapshot().unwrap(),
                    crate::query::engine::plan(&q).unwrap(),
                    std::sync::Arc::new(FunctionRegistry::with_builtins()),
                    usize::MAX,
                )
                .unwrap();
                let counters = stream.counters();
                if optimized {
                    assert!(counters.iter().any(|c| c.op == "AggregateOrdered"));
                    assert_eq!(
                        counters.iter().any(|c| c.op == "Sort"),
                        mixed && sorted,
                        "mixed numbers group in Value::Ord but sort numerically"
                    );
                }
                let mut rows = Vec::new();
                while let Some(batch) = stream.next_rows().unwrap() {
                    rows.extend(batch);
                }
                results.push(rows);
            }
            assert_eq!(results[0].len(), results[1].len());
            for (a, b) in results[0].iter().flatten().zip(results[1].iter().flatten()) {
                match (a, b) {
                    (Value::Float(a), Value::Float(b)) => assert_eq!(a.to_bits(), b.to_bits()),
                    _ => assert!(a == b, "same group result"),
                }
            }
        }
    }
}

#[test]
fn a_unique_group_key_proves_tie_columns_until_a_join_can_multiply_it() {
    let mut order = proof(ValueKind::Integer);
    order.unique = true;
    assert!(order.satisfies(&[1, 0], &[(0, false), (1, true)]));
    let joined = order.through(&[join(JoinKind::Probe(0), vec![])]).unwrap();
    assert!(!joined.satisfies(&[1, 0], &[(0, false), (1, true)]));
    assert!(sorted(&joined));
}
