//! Tutorial Section 10 — User-Defined Functions
//!
//! Demonstrates `register_predicate` and `register_aggregate` in the context
//! of the Corestore e-commerce scenario.
//!
//! Run with:
//!   cargo run --example tutorial_udfs

use minigraf::{Minigraf, QueryResult, Value};

fn main() -> anyhow::Result<()> {
    let db = Minigraf::in_memory()?;

    // ── Seed data ────────────────────────────────────────────────────────────
    //
    // Promo codes: one valid Corestore code, one invalid short code, one from
    // a different scheme.

    db.execute(
        r#"(transact [
        [:promo-1 :promo/code "CORESTORE-SUMMER2026"]
        [:promo-2 :promo/code "SAVE10"]
        [:promo-3 :promo/code "PARTNER-EXCLUSIVE"]
    ])"#,
    )?;

    // Customers and their orders with on-time-flag attributes.
    //   Alice: order-a (on time), order-b (late)  → score = 1/2 = 0.5
    //   Ben:   order-c (on time)                  → score = 1/1 = 1.0

    db.execute(
        r#"(transact [
        [:alice :customer/name "Alice"]
        [:ben   :customer/name "Ben"]
        [:order-a :order/customer :alice]
        [:order-a :order/on-time-flag 1]
        [:order-b :order/customer :alice]
        [:order-b :order/on-time-flag 0]
        [:order-c :order/customer :ben]
        [:order-c :order/on-time-flag 1]
    ])"#,
    )?;

    // ── Predicate UDF: valid-promo? ───────────────────────────────────────────
    //
    // A valid Corestore promo code must start with "CORESTORE-" and be at
    // least 15 characters long.

    db.register_predicate("valid-promo?", |v: &Value| {
        if let Value::String(s) = v {
            s.starts_with("CORESTORE-") && s.len() >= 15
        } else {
            false
        }
    })?;

    println!("=== Predicate UDF: valid-promo? ===");
    println!();
    println!("Query: find promo codes that satisfy valid-promo?");
    println!();
    println!("  (query [:find ?code");
    println!("          :where [?p :promo/code ?code]");
    println!("                 [(valid-promo? ?code)]])");
    println!();

    let promo_result = db.execute(
        r#"(query [:find ?code
                   :where [?p :promo/code ?code]
                          [(valid-promo? ?code)]])"#,
    )?;

    let (promo_rows, promo_count) = match promo_result {
        QueryResult::QueryResults { results, .. } => {
            let n = results.len();
            (results, n)
        }
        _ => (vec![], 0),
    };

    println!("?code");
    println!("--------------------");
    for row in &promo_rows {
        for val in row {
            match val {
                Value::String(s) => print!("\"{}\"", s),
                other => print!("{:?}", other),
            }
        }
        println!();
    }
    println!();
    println!("{} result(s) found.", promo_count);
    println!();

    // ── Aggregate UDF: delivery-score ────────────────────────────────────────
    //
    // delivery-score receives a stream of integer flags (1 = on time, 0 = late)
    // and returns on_time_count / total_count as a float.
    //
    // Accumulator state: (on_time_count: i64, total_count: i64)

    db.register_aggregate(
        "delivery-score",
        || (0i64, 0i64),
        |state: &mut (i64, i64), val: &Value| {
            if let Value::Integer(flag) = val {
                state.1 += 1;
                if *flag == 1 {
                    state.0 += 1;
                }
            }
        },
        |state: &(i64, i64), _n: usize| {
            if state.1 == 0 {
                Value::Null
            } else {
                Value::Float(state.0 as f64 / state.1 as f64)
            }
        },
    )?;

    println!("=== Aggregate UDF: delivery-score ===");
    println!();
    println!("Query: on-time delivery score per customer");
    println!();
    println!("  (query [:find ?name (delivery-score ?flag)");
    println!("          :where [?customer :customer/name ?name]");
    println!("                 [?order :order/customer ?customer]");
    println!("                 [?order :order/on-time-flag ?flag]])");
    println!();

    let score_result = db.execute(
        r#"(query [:find ?name (delivery-score ?flag)
                   :where [?customer :customer/name ?name]
                          [?order :order/customer ?customer]
                          [?order :order/on-time-flag ?flag]])"#,
    )?;

    let (score_rows, score_count) = match score_result {
        QueryResult::QueryResults { results, .. } => {
            let n = results.len();
            (results, n)
        }
        _ => (vec![], 0),
    };

    println!("?name     (delivery-score ?flag)");
    println!("------------------------------------");
    for row in &score_rows {
        let name = match &row[0] {
            Value::String(s) => format!("\"{}\"", s),
            other => format!("{:?}", other),
        };
        let score = match &row[1] {
            Value::Float(f) => {
                // Always show at least one decimal place for clarity
                if f.fract() == 0.0 {
                    format!("{:.1}", f)
                } else {
                    format!("{}", f)
                }
            }
            Value::Null => "null".to_string(),
            other => format!("{:?}", other),
        };
        println!("{:<10}{}", name, score);
    }
    println!();
    println!("{} result(s) found.", score_count);

    // ── UDF aggregate in a window clause ─────────────────────────────────────────
    println!();
    println!("=== UDF aggregate in a window clause ===");
    println!();
    println!("Query: annotate each order with its customer delivery score");
    println!();

    let window_result = db.execute(
        r#"(query [:find ?order (delivery-score ?flag :over (:partition-by ?customer :order-by ?order))
               :where [?customer :customer/name ?name]
                      [?order :order/customer ?customer]
                      [?order :order/on-time-flag ?flag]])"#,
    )?;

    let (window_rows, window_count) = match window_result {
        QueryResult::QueryResults { results, .. } => {
            let n = results.len();
            (results, n)
        }
        _ => (vec![], 0),
    };

    println!("?order     (delivery-score ?flag :over ...)");
    println!("--------------------------------------------");
    for row in &window_rows {
        let order = match &row[0] {
            Value::Keyword(k) => format!(":{}", k),
            Value::Ref(u) => format!("{}", &u.to_string()[..8]),
            other => format!("{:?}", other),
        };
        let score = match &row[1] {
            Value::Float(f) => {
                if f.fract() == 0.0 {
                    format!("{:.1}", f)
                } else {
                    format!("{}", f)
                }
            }
            Value::Null => "null".to_string(),
            other => format!("{:?}", other),
        };
        println!("{:<12}{}", order, score);
    }
    println!();
    println!("{} result(s) found.", window_count);

    Ok(())
}
