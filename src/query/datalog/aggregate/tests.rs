use super::*;
use crate::query::datalog::functions::{AggregateDesc, UdfOps};
use crate::query::datalog::parser::parse_datalog_command;
use crate::query::datalog::types::DatalogCommand;
use std::sync::Arc;

fn f(x: f64) -> Value {
    Value::Float(x)
}
fn i(x: i64) -> Value {
    Value::Integer(x)
}
fn kw(k: &str) -> Value {
    Value::Keyword(k.to_string())
}

/// The exact float sum of `vals`, as `Sum` gives it.
fn fsum(vals: &[Value]) -> f64 {
    let mut s = Sum::default();
    for v in vals {
        assert!(s.add(v), "a number");
    }
    match s.value() {
        Value::Float(x) => x,
        _ => f64::NAN,
    }
}

/// Every permutation of `vals`.
fn permutations(vals: &[Value]) -> Vec<Vec<Value>> {
    if vals.len() <= 1 {
        return vec![vals.to_vec()];
    }
    let mut out = Vec::new();
    for k in 0..vals.len() {
        let mut rest = vals.to_vec();
        let first = rest.remove(k);
        for mut p in permutations(&rest) {
            p.insert(0, first.clone());
            out.push(p);
        }
    }
    out
}

fn finish(func: &str, vals: &[Value]) -> Result<Option<Value>> {
    let reg = FunctionRegistry::with_builtins();
    let mut agg = Agg::new(func, &reg)?;
    for v in vals {
        agg.step(v).unwrap();
    }
    agg.finish(func)
}

fn code(r: Result<Option<Value>>) -> &'static str {
    match r {
        Err(e) => crate::MinigrafError::from(e).code(),
        Ok(_) => "",
    }
}

#[test]
fn value_order_is_a_total_order() {
    let grid = vec![
        f(f64::NEG_INFINITY),
        i(i64::MIN),
        f(-9_223_372_036_854_775_808.0),
        i(-1),
        f(-0.5),
        f(-0.0),
        i(0),
        f(0.0),
        f(5e-324),
        i(1),
        f(1.0),
        f(1.5),
        i(9_007_199_254_740_993),
        f(9_007_199_254_740_992.0),
        i(i64::MAX),
        f(9_223_372_036_854_775_808.0),
        f(f64::INFINITY),
        f(f64::NAN),
        f(-f64::NAN),
        Value::String("a".into()),
        Value::String("b".into()),
        Value::Boolean(false),
        Value::Boolean(true),
        Value::Ref(uuid::Uuid::from_u128(1)),
        Value::Ref(uuid::Uuid::from_u128(2)),
        kw(":a"),
        kw(":b"),
        Value::Null,
    ];
    // Sets (#316); no zero or NaN element, whose equal sets differ in bits.
    let set = |v: Vec<Value>| Value::Set(crate::graph::types::ValueSet::new(v).unwrap());
    let grid: Vec<Value> = grid
        .into_iter()
        .chain([
            set(vec![]),
            set(vec![i(1)]),
            set(vec![i(1), Value::String("a".into())]),
            set(vec![f(1.5), kw(":a")]),
            set(vec![i(2)]),
            set(vec![Value::Null]),
        ])
        .collect();
    for a in &grid {
        for b in &grid {
            let ab = value_order(a, b);
            assert_eq!(ab, value_order(b, a).reverse(), "antisymmetric");
            let same = match (a, b) {
                (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
                _ => a == b,
            };
            assert_eq!(ab == Ordering::Equal, same, "equal only when identical");
            for c in &grid {
                if ab != Ordering::Greater && value_order(b, c) != Ordering::Greater {
                    assert_ne!(value_order(a, c), Ordering::Greater, "transitive");
                }
            }
        }
    }
}

#[test]
fn value_order_compares_integers_and_floats_exactly() {
    // 2^53 + 1 is not a float; as an integer it exceeds the float 2^53.
    assert_eq!(
        value_order(&i(9_007_199_254_740_993), &f(9_007_199_254_740_992.0)),
        Ordering::Greater
    );
    // i64::MAX is below 2^63, the float it rounds to.
    assert_eq!(
        value_order(&i(i64::MAX), &f(9_223_372_036_854_775_808.0)),
        Ordering::Less
    );
    assert_eq!(value_order(&i(1), &f(1.0)), Ordering::Less);
    assert_eq!(value_order(&f(-0.0), &f(0.0)), Ordering::Less);
    assert_eq!(
        value_order(&f(f64::NAN), &f(f64::INFINITY)),
        Ordering::Greater
    );
    assert_eq!(value_order(&f(f64::NAN), &i(i64::MAX)), Ordering::Greater);
    assert_eq!(
        value_order(&Value::String("z".into()), &f(f64::NAN)),
        Ordering::Greater
    );
}

#[test]
fn exact_sum_rounds_once() {
    assert_eq!(fsum(&vec![f(0.1); 10]), 1.0);
    assert_eq!(fsum(&[f(1e308), f(1e308), f(-1e308)]), 1e308);
    assert_eq!(fsum(&[f(1e100), f(1.0), f(-1e100)]), 1.0);
    assert_eq!(fsum(&[f(5e-324), f(5e-324)]), 1e-323);
    assert_eq!(fsum(&[f(f64::MAX), f(f64::MAX)]), f64::INFINITY);
    assert_eq!(fsum(&[f(-f64::MAX), f(-f64::MAX)]), f64::NEG_INFINITY);
    assert_eq!(fsum(&[f(f64::MAX), f(f64::MAX), f(-f64::MAX)]), f64::MAX);
    // i64::MAX + 0.5 rounds to 2^63.
    assert_eq!(fsum(&[i(i64::MAX), f(0.5)]), 9_223_372_036_854_775_808.0);
    // Ties to even: 2^53 + 1 → 2^53, 2^53 + 3 → 2^53 + 4.
    assert_eq!(
        fsum(&[f(9_007_199_254_740_992.0), i(1)]),
        9_007_199_254_740_992.0
    );
    assert_eq!(
        fsum(&[f(9_007_199_254_740_992.0), i(3)]),
        9_007_199_254_740_996.0
    );
    assert_eq!(fsum(&[f(0.5), i(-1)]), -0.5);
}

#[test]
fn exact_sum_signs_of_zero_and_specials() {
    assert_eq!(fsum(&[f(-0.0), f(-0.0)]).to_bits(), (-0.0f64).to_bits());
    assert_eq!(fsum(&[f(-0.0), f(0.0)]).to_bits(), 0.0f64.to_bits());
    assert_eq!(fsum(&[f(-0.0), i(0)]).to_bits(), 0.0f64.to_bits());
    assert_eq!(fsum(&[f(1.5), f(-1.5)]).to_bits(), 0.0f64.to_bits());
    assert!(fsum(&[f(f64::NAN), f(1.0)]).is_nan());
    assert!(fsum(&[f(f64::INFINITY), f(f64::NEG_INFINITY)]).is_nan());
    assert_eq!(fsum(&[f(f64::INFINITY), f(-f64::MAX)]), f64::INFINITY);
}

#[test]
fn exact_sum_is_the_same_in_every_order() {
    let vals = [f(1e16), f(1.0), f(-1e16), f(3.5), f(1e-300), f(-2.0)];
    for p in permutations(&vals) {
        assert_eq!(fsum(&p).to_bits(), 2.5f64.to_bits());
    }
    let mixed = [
        i(i64::MAX),
        f(0.25),
        i(-7),
        f(1e-17),
        f(-0.125),
        i(i64::MIN),
    ];
    let first = fsum(&mixed);
    for p in permutations(&mixed) {
        assert_eq!(fsum(&p).to_bits(), first.to_bits());
    }
}

#[test]
fn integer_sum_saturates_once() {
    for p in permutations(&[i(i64::MAX), i(1), i(-1)]) {
        assert_eq!(finish("sum", &p).ok().flatten(), Some(i(i64::MAX)));
    }
    assert_eq!(
        finish("sum", &[i(i64::MAX), i(i64::MAX)]).ok().flatten(),
        Some(i(i64::MAX))
    );
    assert_eq!(
        finish("sum", &[i(i64::MIN), i(-1)]).ok().flatten(),
        Some(i(i64::MIN))
    );
    assert_eq!(finish("sum", &[]).ok().flatten(), Some(i(0)));
    assert_eq!(finish("sum", &[Value::Null]).ok().flatten(), Some(i(0)));
}

#[test]
fn sum_error_is_decided_at_finish() {
    for p in permutations(&[i(1), Value::String("a".into()), i(2)]) {
        assert!(code(finish("sum", &p)) == "INT-041");
    }
}

#[test]
fn min_max_errors_do_not_depend_on_order() {
    for p in permutations(&[kw(":a"), kw(":b"), i(1)]) {
        assert!(code(finish("min", &p)) == "INT-043");
    }
    for p in permutations(&[i(1), f(2.0)]) {
        assert!(code(finish("max", &p)) == "INT-043");
    }
    assert!(code(finish("min", &[kw(":a")])) == "INT-044");
    assert!(code(finish("max", &[Value::Boolean(true)])) == "INT-044");
    assert!(matches!(finish("min", &[]), Ok(None)));
    assert!(matches!(finish("max", &[Value::Null]), Ok(None)));
}

#[test]
fn min_max_of_floats_use_the_total_order() {
    for p in permutations(&[f(0.0), f(-0.0), f(1.0)]) {
        let Ok(Some(Value::Float(m))) = finish("min", &p) else {
            unreachable!("min of floats")
        };
        assert_eq!(m.to_bits(), (-0.0f64).to_bits());
    }
    for p in permutations(&[f(0.0), f(-0.0)]) {
        let Ok(Some(Value::Float(m))) = finish("max", &p) else {
            unreachable!("max of floats")
        };
        assert_eq!(m.to_bits(), 0.0f64.to_bits());
    }
    for p in permutations(&[f(f64::NAN), f(1.0)]) {
        assert!(matches!(finish("max", &p), Ok(Some(Value::Float(m))) if m.is_nan()));
        assert_eq!(finish("min", &p).ok().flatten(), Some(f(1.0)));
    }
    assert_eq!(
        finish(
            "min",
            &[Value::String("b".into()), Value::String("a".into())]
        )
        .ok()
        .flatten(),
        Some(Value::String("a".into()))
    );
}

#[test]
fn distinct_sets_store_canonical_floats() {
    assert_eq!(
        finish(
            "count-distinct",
            &[f(0.0), f(-0.0), f(f64::NAN), f(-f64::NAN), i(0)]
        )
        .ok()
        .flatten(),
        Some(i(3))
    );
    let Ok(Some(Value::Float(s))) = finish("sum-distinct", &[f(-0.0), f(-0.0)]) else {
        unreachable!("sum-distinct of floats")
    };
    assert_eq!(s.to_bits(), 0.0f64.to_bits());
    assert_eq!(
        finish("sum-distinct", &[i(2), i(2), i(3)]).ok().flatten(),
        Some(i(5))
    );
    assert!(code(finish("sum-distinct", &[i(2), kw(":k")])) == "INT-041");
    assert_eq!(finish("count-distinct", &[]).ok().flatten(), Some(i(0)));
}

#[test]
fn count_skips_nil() {
    assert_eq!(
        finish("count", &[i(1), Value::Null, kw(":k")])
            .ok()
            .flatten(),
        Some(i(2))
    );
    assert!(
        code(finish("avg", &[i(1)])) == "INT-029",
        "avg is a window function"
    );
}

#[test]
fn unknown_aggregate_is_int_029() {
    let reg = FunctionRegistry::with_builtins();
    assert!(
        Agg::new("nope", &reg)
            .err()
            .is_some_and(|e| crate::MinigrafError::from(e).code() == "INT-029")
    );
}

fn registry_with_udf() -> FunctionRegistry {
    let mut reg = FunctionRegistry::with_builtins();
    reg.register_aggregate_desc(
        "total".to_string(),
        AggregateDesc {
            impl_: AggImpl::Udf(UdfOps {
                init: Arc::new(|| Ok(Box::new(0i64) as Box<dyn Any + Send>)),
                step: Arc::new(|acc, v| {
                    if let (Some(n), Value::Integer(x)) = (acc.downcast_mut::<i64>(), v) {
                        *n += x;
                    }
                    Ok(())
                }),
                finalise: Arc::new(|acc, n| {
                    let total = acc.downcast_ref::<i64>().copied().unwrap_or(0);
                    Ok(Value::Integer(total * 100 + i64::try_from(n).unwrap_or(0)))
                }),
            }),
        },
    )
    .expect("registers");
    reg
}

#[test]
fn registered_aggregate_steps_non_nil_values() {
    let reg = registry_with_udf();
    let mut agg = Agg::new("total", &reg).expect("registered");
    for v in [i(2), Value::Null, i(3)] {
        agg.step(&v).unwrap();
    }
    assert_eq!(agg.finish("total").ok().flatten(), Some(i(502)));
    let empty = Agg::new("total", &reg).expect("registered");
    assert_eq!(empty.finish("total").ok().flatten(), Some(Value::Null));
}

fn window(src: &str) -> WindowSpec {
    let q = format!("(query [:find {src} :where [?e :x ?v] [?e :y ?o]])");
    match parse_datalog_command(&q) {
        Ok(DatalogCommand::Query(q)) => match q.find.into_iter().next() {
            Some(FindSpec::Window(ws)) => ws,
            _ => unreachable!("a window"),
        },
        _ => unreachable!("parses"),
    }
}

#[test]
fn running_values() {
    let reg = registry_with_udf();
    let mut sum = Running::new(&window("(sum ?v :over (:order-by ?o))"), &reg).expect("sum");
    assert_eq!(sum.value().unwrap(), i(0));
    sum.step(&i(2)).unwrap();
    assert_eq!(sum.value().unwrap(), i(2));
    sum.step(&Value::String("skipped".into())).unwrap();
    sum.step(&f(0.5)).unwrap();
    assert_eq!(sum.value().unwrap(), f(2.5));

    let mut min = Running::new(&window("(min ?v :over (:order-by ?o))"), &reg).expect("min");
    assert_eq!(min.value().unwrap(), Value::Null);
    min.step(&kw(":b")).unwrap();
    min.step(&kw(":a")).unwrap();
    assert_eq!(min.value().unwrap(), kw(":a"));

    let mut count = Running::new(&window("(count ?v :over (:order-by ?o))"), &reg).expect("count");
    count.step(&Value::Null).unwrap();
    count.step(&i(1)).unwrap();
    assert_eq!(count.value().unwrap(), i(1));

    let mut avg = Running::new(&window("(avg ?v :over (:order-by ?o))"), &reg).expect("avg");
    assert_eq!(avg.value().unwrap(), Value::Null);
    avg.step(&i(1)).unwrap();
    avg.step(&i(2)).unwrap();
    assert_eq!(avg.value().unwrap(), f(1.5));

    let mut udf = Running::new(&window("(total ?v :over (:order-by ?o))"), &reg).expect("udf");
    udf.step(&Value::Null).unwrap();
    udf.step(&i(4)).unwrap();
    assert_eq!(udf.value().unwrap(), i(402));

    assert!(
        Running::new(&window("(nope ?v :over (:order-by ?o))"), &reg)
            .err()
            .is_some_and(|e| crate::MinigrafError::from(e).code() == "INT-030")
    );
}

#[test]
fn named_vars_in_find_order() {
    let parse = |src: &str| match parse_datalog_command(src) {
        Ok(DatalogCommand::Query(q)) => q.find,
        _ => unreachable!("parses"),
    };
    let find = parse(
        "(query [:find ?e (sum ?v :over (:partition-by ?p :order-by ?o)) ?v \
         :where [?e :v ?v] [?e :p ?p] [?e :o ?o]])",
    );
    assert_eq!(named_vars(&find), vec!["?e", "?v", "?p", "?o"]);
    let find = parse(
        "(query [:find ?d (count ?e) (rank :over (:order-by ?d)) \
         :where [?e :d ?d]])",
    );
    assert_eq!(named_vars(&find), vec!["?d"]);
}

#[test]
fn value_order_puts_sets_last_and_orders_them_by_elements() {
    use crate::graph::types::ValueSet;
    let set = |v: Vec<Value>| Value::Set(ValueSet::new(v).unwrap());
    let empty = set(vec![]);
    for v in [i(1), f(f64::NAN), kw(":z"), Value::Null] {
        assert_eq!(value_order(&v, &empty), Ordering::Less, "scalar first");
    }
    // Elements sorted by value_order: 1 before "a" (numbers first), unlike
    // Value's Ord (strings first).
    let a = set(vec![Value::String("a".into()), i(1)]);
    let b = set(vec![i(2)]);
    assert_eq!(value_order(&a, &b), Ordering::Less, "1 < 2 decides");
    assert_eq!(value_order(&b, &a), Ordering::Greater, "antisymmetric");
    assert_eq!(value_order(&a, &a.clone()), Ordering::Equal, "reflexive");
    let prefix = set(vec![i(1)]);
    assert_eq!(value_order(&prefix, &a), Ordering::Less, "prefix first");
}
