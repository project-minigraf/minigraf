//! Bounded schedule exploration of the real storage methods (#760).
//! Compiled only in the native, instrumented unit-test build. See
//! docs/testing/deterministic-schedules.md for commands and limitations.

use super::*;
use crate::storage::{CommittedReader, Scan};
use shuttle::{Config, MaxSteps, Runner, scheduler::RandomScheduler, thread};

fn check(workload: fn()) {
    if let Ok(schedule) = std::env::var("MINIGRAF_DST_REPLAY") {
        shuttle::replay(workload, &schedule);
        return;
    }
    let iterations = std::env::var("MINIGRAF_DST_ITERATIONS")
        .map(|s| s.parse::<usize>().expect("positive iteration count"))
        .unwrap_or(1_000);
    assert!(iterations > 0, "must explore at least one schedule");
    let seed = std::env::var("MINIGRAF_DST_SEED")
        .map(|s| s.parse::<u64>().expect("integer seed"))
        .unwrap_or(760);
    let mut config = Config::default();
    config.max_steps = MaxSteps::FailAfter(10_000);
    let explored =
        Runner::new(RandomScheduler::new_from_seed(seed, iterations), config).run(workload);
    assert_eq!(explored, iterations, "all requested schedules ran");
    eprintln!("DST seed={seed} schedules={explored}");
}

// Fixed timestamps and ids: the wall clock captured by snapshot() is not
// consulted by these assertions. No UUID generation, file I/O or OS threads.
fn pair(n: u64) -> Vec<Fact> {
    [":pair/x", ":pair/y"]
        .into_iter()
        .map(|a| {
            Fact::with_valid_time(
                uuid::Uuid::from_u128(u128::from(n)),
                a.into(),
                Value::Integer(1),
                n,
                n,
                0,
                VALID_TIME_FOREVER,
            )
        })
        .collect()
}

fn ids(facts: Vec<Fact>) -> Vec<(u128, String)> {
    let mut ids: Vec<_> = facts
        .into_iter()
        .map(|f| (f.entity.as_u128(), f.attribute))
        .collect();
    ids.sort();
    ids
}

fn whole_pairs(facts: Vec<Fact>, counts: &[usize]) {
    assert!(
        counts.contains(&facts.len()),
        "partial or duplicated transaction"
    );
    let xs: Vec<_> = facts
        .iter()
        .filter(|f| f.attribute == ":pair/x")
        .map(|f| f.entity)
        .collect();
    let ys: Vec<_> = facts
        .iter()
        .filter(|f| f.attribute == ":pair/y")
        .map(|f| f.entity)
        .collect();
    assert!(xs == ys, "attributes came from different transactions");
}

#[test]
fn batch_append_is_atomic() {
    check(|| {
        let storage = FactStorage::new();
        let writer = storage.clone();
        let writer = thread::spawn(move || {
            // Yield between facts while load_facts holds its batch lock.
            // Replacing it with per-fact locking must make this test fail.
            writer
                .load_facts(pair(1).into_iter().inspect(|_| thread::yield_now()))
                .unwrap();
        });
        let snap = storage.snapshot().unwrap();
        whole_pairs(snap.all_facts().unwrap(), &[0, 2]);
        writer.join().unwrap();
        whole_pairs(storage.snapshot().unwrap().all_facts().unwrap(), &[2]);
    });
}

#[test]
fn one_snapshot_keeps_its_cutoff_across_scans() {
    check(|| {
        let storage = FactStorage::new();
        storage.load_facts(pair(1)).unwrap();
        let writer = storage.clone();
        let writer = thread::spawn(move || {
            writer.load_facts(pair(2)).unwrap();
        });
        let snap = storage.snapshot().unwrap();
        let xs = snap
            .live_facts_of_scans(&[Scan::Attribute(":pair/x")], None)
            .unwrap();
        thread::yield_now();
        let ys = snap
            .live_facts_of_scans(&[Scan::Attribute(":pair/y")], None)
            .unwrap();
        let mut x_ids: Vec<_> = xs.iter().map(|f| f.entity).collect();
        let mut y_ids: Vec<_> = ys.iter().map(|f| f.entity).collect();
        x_ids.sort();
        y_ids.sort();
        assert!(x_ids == y_ids, "one snapshot saw two database states");
        writer.join().unwrap();
        assert_eq!(
            snap.all_facts().unwrap().len(),
            xs.len() * 2,
            "snapshot grew after capture"
        );
    });
}

// Immutable committed facts isolate publication from page I/O. This is not
// a model of checkpoint durability, B+tree traversal or the page allocator.
struct Reader(Vec<Fact>);

impl CommittedReader for Reader {
    fn all_facts(&self) -> Result<Vec<Fact>> {
        Ok(self.0.clone())
    }
    fn facts_for_entity(&self, e: &EntityId) -> Result<Vec<Fact>> {
        Ok(self.0.iter().filter(|f| f.entity == *e).cloned().collect())
    }
    fn facts_for_entity_attribute(&self, e: &EntityId, a: &str) -> Result<Vec<Fact>> {
        Ok(self
            .0
            .iter()
            .filter(|f| f.entity == *e && f.attribute == a)
            .cloned()
            .collect())
    }
    fn facts_for_attribute(&self, a: &str) -> Result<Vec<Fact>> {
        Ok(self
            .0
            .iter()
            .filter(|f| f.attribute == a)
            .cloned()
            .collect())
    }
}

#[test]
fn checkpoint_publication_preserves_readers_and_late_appends() {
    check(|| {
        let storage = FactStorage::new();
        storage.set_committed(Arc::new(Reader(pair(1))), 1);
        storage.load_facts(pair(2)).unwrap();
        let held = storage.snapshot().unwrap();
        let before = ids(held.all_facts().unwrap());
        let checkpoint = storage.clone();
        let checkpoint = thread::spawn(move || {
            let committed = [pair(1), pair(2)].concat();
            checkpoint.publish(Arc::new(Reader(committed)), 2, 2, vec![7]);
        });
        let writer = storage.clone();
        let writer = thread::spawn(move || {
            writer.load_facts(pair(3)).unwrap();
        });
        let snap = storage.snapshot().unwrap();
        let observed = ids(snap.all_facts().unwrap());
        assert!(
            observed == before || observed == ids([pair(1), pair(2), pair(3)].concat()),
            "publication mixed committed and pending generations"
        );
        checkpoint.join().unwrap();
        writer.join().unwrap();
        assert!(
            ids(held.all_facts().unwrap()) == before,
            "old reader changed"
        );
        assert!(
            ids(snap.all_facts().unwrap()) == observed,
            "concurrent reader changed"
        );
        assert!(
            storage.pages_in_force().contains(&7),
            "old reader lost its page pin"
        );
        assert!(
            ids(storage.snapshot().unwrap().all_facts().unwrap())
                == ids([pair(1), pair(2), pair(3)].concat()),
            "publication lost or duplicated a late append"
        );
        drop(held);
        drop(snap);
        assert!(
            storage.pages_in_force().is_empty(),
            "last reader did not release its pin"
        );
    });
}
