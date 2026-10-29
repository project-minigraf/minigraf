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

#[cfg_attr(
    all(target_arch = "wasm32", not(target_os = "wasi")),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", not(target_os = "wasi"))), test)]
fn generalized_committed_avg_preserves_numeric_and_skipped_value_semantics() {
    use crate::query::datalog::{parser::parse_datalog_command, types::DatalogCommand};
    use crate::storage::{backend::memory::MemoryBackend, persistent_facts::PersistentFactStorage};
    let populations = [
        vec![
            Value::Null,
            Value::Boolean(true),
            Value::String("x".repeat(100)),
            Value::Keyword(":skip".into()),
            Value::Ref(uuid::Uuid::from_u128(999)),
        ],
        vec![Value::Integer(i64::MAX), Value::Integer(i64::MAX)],
        vec![
            Value::Float(-0.0),
            Value::Null,
            Value::String("skip".into()),
        ],
        vec![Value::Float(f64::INFINITY), Value::Float(f64::NEG_INFINITY)],
        vec![Value::Float(f64::MAX), Value::Float(f64::MAX)],
        vec![Value::Float(1e20), Value::Integer(1), Value::Float(-1e20)],
        vec![
            Value::Float(-f64::from_bits(1)),
            Value::Float(0.0),
            Value::Float(0.0),
        ],
    ];
    let expected = [
        Value::Null,
        Value::Float(i64::MAX as f64),
        Value::Float(-0.0),
        Value::Float(f64::NAN),
        Value::Float(f64::INFINITY),
        Value::Float(1.0 / 3.0),
        Value::Float(-0.0),
    ];
    let mut facts = Vec::new();
    for (g, values) in populations.into_iter().enumerate() {
        for (i, v) in values.into_iter().enumerate() {
            let e = uuid::Uuid::from_u128((g * 100 + i + 1) as u128);
            facts.push((e, ":group".into(), Value::Integer(g as i64)));
            facts.push((e, ":arg".into(), v));
        }
    }
    let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 32).unwrap();
    pfs.storage().transact(facts, None).unwrap();
    pfs.mark_dirty();
    pfs.save().unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let DatalogCommand::Query(query) =
        parse_datalog_command("(query [:find ?g (avg ?v) :where [?e :group ?g] [?e :arg ?v]])")
            .unwrap()
    else {
        panic!("query")
    };
    for on in [false, true] {
        let _mode = set_enabled(on);
        let mut stream = crate::query::engine::open(
            pfs.storage().snapshot().unwrap(),
            crate::query::engine::plan(&query).unwrap(),
            std::sync::Arc::new(FunctionRegistry::with_builtins()),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            stream.counters().iter().any(|c| c.op == "AggregateOrdered"),
            on
        );
        let meter = stream.meter().clone();
        let mut rows = Vec::new();
        while let Some(batch) = stream.next_rows().unwrap() {
            rows.extend(batch);
        }
        assert_eq!(rows.len(), expected.len());
        for (g, (row, want)) in rows.iter().zip(&expected).enumerate() {
            assert!(row[0] == Value::Integer(g as i64));
            match (&row[1], want) {
                (Value::Float(a), Value::Float(b)) if b.is_nan() => assert!(a.is_nan()),
                (Value::Float(a), Value::Float(b)) => assert_eq!(a.to_bits(), b.to_bits()),
                (a, b) => assert!(a == b),
            }
        }
        assert_eq!(meter.used(), 0);
    }
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
        checked.constant_kind(1).is_none(),
        "join-equal checks do not prove an identical constant"
    );
    let replaced = proof(ValueKind::Integer)
        .through(&[Step::Bind(lit, vec![], 1, Out::Replace)])
        .unwrap();
    assert!(replaced.constant_kind(1).is_some());
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
    assert!(replaced.constant_kind(1).is_none());
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
            plain_group_count: 1,
            aggs: vec![("count".into(), 0)],
            zero_row: false,
            grouping: Grouping::Hash,
        },
        Step::Window {
            find: vec![],
            named: vec![],
            keep: vec![1],
            mode: WindowMode::Full,
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
/// A real committed reader on native, WASI and the browser, including canonical
/// NaNs, both zero encodings, mixed numeric groups and Boolean partitions.
#[cfg_attr(
    all(target_arch = "wasm32", not(target_os = "wasi")),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", not(target_os = "wasi"))), test)]
fn committed_partition_windows_preserve_float_edges_and_numeric_group_classes() {
    use crate::query::datalog::{parser::parse_datalog_command, types::DatalogCommand};
    use crate::storage::{backend::memory::MemoryBackend, persistent_facts::PersistentFactStorage};
    for case in 0..3 {
        let mut values = vec![
            Value::Float(-0.0),
            Value::Float(0.0),
            Value::Float(f64::NAN),
            Value::Float(-f64::NAN),
            Value::Float(f64::NEG_INFINITY),
            Value::Float(1.5),
        ];
        if case == 1 {
            values.extend([Value::Integer(10), Value::Integer(-10)]);
        }
        if case == 2 {
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
                    .map(|(i, v)| (uuid::Uuid::from_u128(i as u128 + 1), ":part".into(), v))
                    .collect(),
                None,
            )
            .unwrap();
        pfs.mark_dirty();
        pfs.save().unwrap();
        let DatalogCommand::Query(q)=parse_datalog_command("(query [:find ?e ?g (row-number :over (:partition-by ?g :order-by ?e)) (lag ?g :over (:partition-by ?g :order-by ?e)) (sum ?g :over (:partition-by ?g :order-by ?e :rows 1 preceding)) :where [?e :part ?g] :order-by [?g ?e]])").unwrap() else {panic!("query");};
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
            assert_eq!(
                stream
                    .counters()
                    .iter()
                    .any(|c| c.op == "WindowPartitioned"),
                optimized
            );
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
                _ => assert!(a == b, "same partition window result"),
            }
        }
    }
}

#[test]
fn id_adjacency_cannot_certify_aggregate_or_semantic_sort_order() {
    for kind in [ValueKind::Keyword, ValueKind::Ref] {
        let p = proof(kind);
        assert!(!p.group_ordered());
        assert!(!sorted(&p));
        assert!(p.partition_order().is_some());
    }
    let p = Ordering::entity(&spec(false)).unwrap();
    assert!(!p.group_ordered());
    assert!(!sorted(&p));
    assert_eq!(p.partition_order(), Some(PartitionOrder::Entity));
    let mut avet = spec(false);
    avet.index = Some(Index::Avet);
    assert!(
        Ordering::entity(&avet).is_none(),
        "an existing value-range hint cannot prove entity adjacency"
    );
    let mut p = proof(ValueKind::Integer);
    p.remap(&[0, 2]);
    assert!(
        p.slot.is_none(),
        "a dropped key cannot certify an output order"
    );
}

#[test]
fn generalized_tuple_certificate_rejects_payloads_and_tracks_replaced_constants() {
    for (value, allowed) in [
        (Value::Integer(2), true),
        (Value::Boolean(true), true),
        (Value::Null, true),
        (Value::Float(2.0), false),
        (Value::String("constant".into()), false),
        (Value::Keyword(":constant".into()), false),
        (Value::Ref(uuid::Uuid::from_u128(1)), false),
        (Value::Set(Default::default()), false),
    ] {
        let order = proof(ValueKind::Integer)
            .through(&[Step::Bind(Expr::Lit(value), vec![], 3, Out::New)])
            .unwrap();
        assert_eq!(order.tuple_ordered(&[3, 1]), allowed);
        assert_eq!(order.tuple_ordered(&[1, 3, 1]), allowed);
        assert!(
            !order.tuple_ordered(&[3]),
            "all-constant tuple stays materialized"
        );
        let replaced = order
            .through(&[Step::Bind(
                Expr::Var("?e".into()),
                vec![("?e".into(), 0)],
                3,
                Out::Replace,
            )])
            .unwrap();
        assert!(!replaced.tuple_ordered(&[3, 1]));
    }
    for domain in [Domain::Single(ValueKind::Float), Domain::Numbers] {
        let order = Ordering::source(&spec(false), domain);
        assert!(
            order.tuple_ordered(&[1]),
            "single numeric key remains eligible"
        );
        assert!(
            !order.tuple_ordered(&[1, 1]),
            "float tuple is conservative fallback"
        );
        assert!(
            !order.tuple_ordered(&[2, 1]),
            "even a scalar constant cannot extend a float tuple"
        );
    }
    let mut order = proof(ValueKind::Boolean);
    assert!(order.tuple_ordered(&[2, 1, 1]));
    order.remap(&[2, 1, 1]);
    assert!(order.satisfies(&[0, 2, 1], &[(0, true), (1, false), (2, true)]));
    order.invalidate(2);
    assert!(
        !order.tuple_ordered(&[0, 1]),
        "replacing an alias invalidates the ordered dimension"
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", not(target_os = "wasi")),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", not(target_os = "wasi"))), test)]
fn certified_committed_scalar_arguments_run_on_portable_backends() {
    use crate::query::datalog::{parser::parse_datalog_command, types::DatalogCommand};
    use crate::storage::{backend::memory::MemoryBackend, persistent_facts::PersistentFactStorage};
    let values = [
        Value::Null,
        Value::Boolean(true),
        Value::Integer(2),
        Value::Float(-0.0),
        Value::Float(0.0),
        Value::Float(f64::NAN),
    ];
    let mut facts = Vec::new();
    for (i, v) in values.into_iter().enumerate() {
        let e = uuid::Uuid::from_u128(i as u128 + 1);
        facts.push((e, ":group".into(), Value::Integer((i / 3) as i64)));
        facts.push((e, ":arg".into(), v));
        facts.push((
            e,
            ":ints".into(),
            if i == 0 {
                Value::Null
            } else {
                Value::Integer(i as i64)
            },
        ));
    }
    let mut pfs = PersistentFactStorage::new(MemoryBackend::new(), 32).unwrap();
    pfs.storage().transact(facts, None).unwrap();
    pfs.mark_dirty();
    pfs.save().unwrap();
    let _text_order = crate::query::datalog::optimizer::set_reorder(false);
    let DatalogCommand::Query(q) = parse_datalog_command("(query [:find ?g (sum ?v) (min ?v) (max ?v) (sum-distinct ?n) :where [?e :group ?g] [?e :arg ?v] [?e :ints ?n]])").unwrap() else { panic!("query"); };
    let mut answers = Vec::new();
    for on in [false, true] {
        let _mode = set_enabled(on);
        let mut stream = crate::query::engine::open(
            pfs.storage().snapshot().unwrap(),
            crate::query::engine::plan(&q).unwrap(),
            std::sync::Arc::new(FunctionRegistry::with_builtins()),
            usize::MAX,
        )
        .unwrap();
        assert_eq!(
            stream.counters().iter().any(|c| c.op == "AggregateOrdered"),
            on
        );
        let meter = stream.meter().clone();
        let mut rows = Vec::new();
        while let Some(batch) = stream.next_rows().unwrap() {
            rows.extend(batch);
        }
        assert_eq!(meter.used(), 0);
        answers.push(rows);
    }
    assert_eq!(answers[0].len(), 2);
    assert_eq!(answers[0].len(), answers[1].len());
    for (a, b) in answers[0].iter().flatten().zip(answers[1].iter().flatten()) {
        match (a, b) {
            (Value::Float(a), Value::Float(b)) if a.is_nan() => assert!(b.is_nan()),
            (Value::Float(a), Value::Float(b)) => assert_eq!(a.to_bits(), b.to_bits()),
            _ => assert!(a == b),
        }
    }
}
