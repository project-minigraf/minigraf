//! #776 P06: ten cases selected by the existing nightly `^query/` filter.
//! Compare pending indexes with committed covering indexes on the same file
//! backend. Fixtures and answer checks run only for selected cases, before timing.

use criterion::{BenchmarkId, Criterion, Throughput};
use minigraf::{Minigraf, OpenOptions, QueryResult, Value, ValueSet};
use std::hint::black_box;
use std::sync::OnceLock;
use std::time::Duration;

const ROWS: usize = 10_000;

struct Fixture {
    db: Minigraf,
    // Drop the database before removing its directory (also works on Windows).
    _dir: tempfile::TempDir,
}

// Half the entities have {0,1,2,3}, a quarter {4,5,6,7}, and a quarter
// have no members. Include the empty-set collect path without losing its row.
fn members(row: usize) -> std::ops::Range<usize> {
    match row % 4 {
        0 => 0..0,
        2 => 4..8,
        _ => 0..4,
    }
}

fn fixture(committed: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let db = OpenOptions::new()
        .wal_checkpoint_threshold(usize::MAX)
        .path(dir.path().join("sets.graph"))
        .open()
        .unwrap();
    for start in (0..ROWS).step_by(100) {
        let mut command = String::from("(transact [");
        for row in start..(start + 100).min(ROWS) {
            command.push_str(&format!(
                "[:p06e{row} :p06/id {row}][:p06e{row} :p06/set #{{"
            ));
            for value in members(row) {
                command.push_str(&format!("{value} "));
            }
            command.push_str("}]");
            for value in members(row) {
                command.push_str(&format!("[:p06e{row} :p06/member {value}]"));
            }
        }
        command.push_str("])");
        db.execute(&command).unwrap();
    }
    if committed {
        db.checkpoint().unwrap();
    }
    Fixture { db, _dir: dir }
}

fn results(db: &Minigraf, query: &str) -> Vec<Vec<Value>> {
    match db.execute(query).unwrap() {
        QueryResult::QueryResults { results, .. } => results,
        _ => panic!("expected query rows"),
    }
}

fn validate(db: &Minigraf, query: &str, case: &str) {
    let rows = results(db, query);
    let filtered = matches!(case, "in" | "not_in" | "all_in");
    assert_eq!(rows.len(), if filtered { ROWS / 2 } else { ROWS });
    // Check values, not just counts: a dropped filter or a broken empty probe
    // must fail the untimed check. No result ordering is assumed.
    let mut seen = vec![false; ROWS];
    for row in rows {
        let Value::Integer(id) = row[0] else {
            panic!("expected integer row id");
        };
        let id = usize::try_from(id).unwrap();
        assert!(!seen[id], "one result per entity");
        seen[id] = true;
        let hit = id % 2 == 1;
        match case {
            "in" | "all_in" => assert!(hit, "membership hit"),
            "not_in" => assert!(!hit, "membership miss"),
            "any_none" => {
                assert!(row[1] == Value::Boolean(hit), "any-in result");
                assert!(row[2] == Value::Boolean(!hit), "none-in result");
            }
            "collect" => {
                let expected =
                    ValueSet::new(members(id).map(|v| Value::Integer(i64::try_from(v).unwrap())))
                        .unwrap();
                assert!(row[1] == Value::Set(expected), "collected members");
            }
            _ => unreachable!(),
        }
    }
}

pub fn bench_set_queries(c: &mut Criterion) {
    let mut group = c.benchmark_group("query/set_queries");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.throughput(Throughput::Elements(u64::try_from(ROWS).unwrap()));
    for (mode, committed) in [("pending", false), ("committed", true)] {
        // Retain each immutable fixture across samples and related cases, but
        // never build it when this group is excluded by the Criterion filter.
        let db = OnceLock::new();
        for (case, clauses) in [
            ("in", "[?e :p06/set ?s] [(in? 1 ?s)]"),
            ("not_in", "[?e :p06/set ?s] [(not-in? 1 ?s)]"),
            ("all_in", "[?e :p06/set ?s] [(all-in? #{0 1} ?s)]"),
            (
                "any_none",
                "[?e :p06/set ?s] [(any-in? ?s #{0 2}) ?any] [(none-in? ?s #{0 2}) ?none]",
            ),
            ("collect", "[(collect-set ?e :p06/member) ?s]"),
        ] {
            let find = match case {
                "any_none" => "?i ?any ?none",
                "collect" => "?i ?s",
                _ => "?i",
            };
            let query = format!("(query [:find {find} :where [?e :p06/id ?i] {clauses}])");
            group.bench_function(BenchmarkId::new(format!("{case}/{mode}"), "10k"), |b| {
                let fixture = db.get_or_init(|| fixture(committed));
                validate(&fixture.db, &query, case);
                // Measure parse/open/full materialization, as existing query
                // benchmarks do; no writes/checkpoints/assertions in the loop.
                b.iter(|| black_box(fixture.db.execute(black_box(&query)).unwrap()));
            });
        }
    }
    group.finish();
}
