//! The #841 shape-sweep fixtures, queries and oracles for the #863 nightly
//! candidates: a trimmed port of `scripts/shape-sweep/native/src/main.rs`
//! (`records`, `query`, `Digest`, `oracle`), which stays authoritative for the
//! sweep. Entity UUIDs, records and query text are the harness's, so answers
//! are the #841 answers.
//!
//! Shared by `benches/minigraf_bench/nightly_candidates.rs` and
//! `tests/nightly_candidates_acceptance_test.rs` (`#[path]`).
#![allow(dead_code)]

use minigraf::{FactRecord, LogWriter, Minigraf, OpenOptions, Value, ValueSet};
use std::path::Path;
use uuid::Uuid;

/// One #841 case, by family.
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    /// `n` rows (`2n` facts), 90% in partition 0, the rest over 99 more;
    /// running `sum` partitioned by part, ordered by value.
    WindowSkewSum { n: usize },
    /// `n` entities with `k`-element sets, 90% sharing `0..k`; six
    /// membership predicates per entity.
    SetMembership { n: usize, k: usize },
    /// `n` integers inserted in a permuted order; `:order-by` with offset
    /// and limit.
    Sort {
        n: usize,
        desc: bool,
        offset: usize,
        limit: Option<usize>,
    },
    /// A width-10 tree of `edges` parent edges; the ancestors of one leaf
    /// (`k == 1`, a literal call) or of the last `k` leaves (demanded).
    Ancestors { edges: usize, k: usize },
}

pub fn entity(i: usize) -> Uuid {
    Uuid::from_u128(0x84100000000000000000000000000000 + i as u128)
}

fn integer(i: usize) -> Value {
    Value::Integer(i.try_into().unwrap())
}

fn skew_part(n: usize, i: usize) -> usize {
    if i < n * 9 / 10 {
        0
    } else {
        1 + (i - n * 9 / 10) % 99
    }
}

fn skew_members(n: usize, k: usize, i: usize) -> ValueSet {
    let base = if i < n * 9 / 10 { 0 } else { (i + 1) * k };
    ValueSet::new((base..base + k).map(integer)).unwrap()
}

impl Shape {
    /// Every fact of the fixture, in the harness's order.
    pub fn records(&self, mut emit: impl FnMut(Uuid, &str, Value)) {
        match *self {
            Shape::WindowSkewSum { n } => {
                for i in 0..n {
                    emit(entity(i), ":s/value", integer(i));
                    emit(entity(i), ":s/part", integer(skew_part(n, i)));
                }
            }
            Shape::SetMembership { n, k } => {
                for i in 0..n {
                    emit(entity(i), ":s/id", integer(i));
                    emit(entity(i), ":s/set", Value::Set(skew_members(n, k, i)));
                }
            }
            Shape::Sort { n, .. } => {
                for j in 0..n {
                    let i = (j * 37) % n;
                    emit(entity(i), ":s/value", integer(i));
                }
            }
            Shape::Ancestors { edges, k } => {
                for i in 1..=edges {
                    emit(entity(i), ":s/parent", Value::Ref(entity((i - 1) / 10)));
                }
                for i in edges + 1 - k..=edges {
                    emit(entity(i), ":s/demand", Value::Boolean(true));
                }
            }
        }
    }

    pub fn query(&self) -> String {
        let body = match *self {
            Shape::WindowSkewSum { .. } => {
                ":find ?i ?p (sum ?i :over (:partition-by ?p :order-by ?i)) \
                 :where [?e :s/part ?p] [?e :s/value ?i]"
                    .to_string()
            }
            Shape::SetMembership { .. } => {
                ":find ?i ?a ?b ?c ?d ?f ?g :where [?e :s/id ?i] [?e :s/set ?s] \
                 [(in? 0 ?s) ?a] [(not-in? 0 ?s) ?b] [(all-in? #{} ?s) ?c] \
                 [(any-in? ?s #{0}) ?d] [(none-in? ?s #{0}) ?f] [(in? -1 ?s) ?g]"
                    .to_string()
            }
            Shape::Sort {
                desc,
                offset,
                limit,
                ..
            } => {
                let mut q = format!(
                    ":find ?v :where [?e :s/value ?v] :order-by [?v :{}]",
                    if desc { "desc" } else { "asc" }
                );
                if offset > 0 {
                    q.push_str(&format!(" :offset {offset}"));
                }
                if let Some(l) = limit {
                    q.push_str(&format!(" :limit {l}"));
                }
                q
            }
            Shape::Ancestors { edges, k: 1 } => {
                format!(":find ?a :where (anc #uuid \"{}\" ?a)", entity(edges))
            }
            Shape::Ancestors { .. } => {
                ":find ?leaf ?a :where [?leaf :s/demand true] (anc ?leaf ?a)".to_string()
            }
        };
        format!("(query [{body}])")
    }

    /// Whether the answer's order is part of it.
    pub fn ordered(&self) -> bool {
        matches!(self, Shape::Sort { .. })
    }

    /// Rows the answer has.
    pub fn rows(&self) -> usize {
        usize::try_from(self.oracle().rows).expect("fixture value fits in usize")
    }

    /// The answer, computed without the database.
    pub fn oracle(&self) -> Digest {
        let mut out = Digest::default();
        match *self {
            Shape::WindowSkewSum { n } => {
                for i in 0..n {
                    let p = skew_part(n, i);
                    let (first, stride) = if p == 0 {
                        (0, 1)
                    } else {
                        (n * 9 / 10 + p - 1, 99)
                    };
                    let rank = (i - first) / stride + 1;
                    let sum = (first + i) * rank / 2;
                    out.add(&[integer(i), integer(p), integer(sum)], false);
                }
            }
            Shape::SetMembership { n, k } => {
                for i in 0..n {
                    let hit = skew_members(n, k, i).iter().any(|v| *v == integer(0));
                    out.add(
                        &[
                            integer(i),
                            Value::Boolean(hit),
                            Value::Boolean(!hit),
                            Value::Boolean(true),
                            Value::Boolean(hit),
                            Value::Boolean(!hit),
                            Value::Boolean(false),
                        ],
                        false,
                    );
                }
            }
            Shape::Sort {
                n,
                desc,
                offset,
                limit,
            } => {
                let end = n.min(offset.saturating_add(limit.unwrap_or(n)));
                for j in offset..end {
                    let i = if desc { n - 1 - j } else { j };
                    out.add(&[integer(i)], true);
                }
            }
            Shape::Ancestors { edges, k } => {
                for leaf in edges + 1 - k..=edges {
                    let mut i = leaf;
                    while i > 0 {
                        i = (i - 1) / 10;
                        let mut row = Vec::new();
                        if k > 1 {
                            row.push(Value::Ref(entity(leaf)));
                        }
                        row.push(Value::Ref(entity(i)));
                        out.add(&row, false);
                    }
                }
            }
        }
        out
    }

    /// Build the fixture at `path`: committed (`LogWriter`, fixed
    /// timestamps, 1,000 facts per transaction) or pending (1,000-fact
    /// `transact`s left in the WAL). Then open it, with the `anc` rules for
    /// an ancestor shape.
    pub fn build(&self, path: &Path, pending: bool) -> Minigraf {
        if pending {
            let db = self.open(path);
            let mut command = String::from("(transact [");
            let mut count = 0usize;
            self.records(|e, a, v| {
                command.push_str(&format!("[#uuid \"{e}\" {a} {}]", edn(&v)));
                count += 1;
                if count.is_multiple_of(1000) {
                    command.push_str("])");
                    db.execute(&command).unwrap();
                    command = String::from("(transact [");
                }
            });
            if !count.is_multiple_of(1000) {
                command.push_str("])");
                db.execute(&command).unwrap();
            }
            db
        } else {
            let mut writer = LogWriter::create(path, OpenOptions::new()).unwrap();
            let mut count = 0usize;
            self.records(|e, a, v| {
                writer
                    .append(&FactRecord {
                        entity: e,
                        attribute: a.into(),
                        value: v,
                        tx_count: (count / 1000 + 1) as u64,
                        tx_id: 1_700_000_000_000 + (count / 1000) as u64,
                        valid_from: 0,
                        valid_to: i64::MAX,
                        asserted: true,
                    })
                    .unwrap();
                count += 1;
            });
            writer.finish().unwrap();
            self.open(path)
        }
    }

    /// Open `path` without checkpoints (a pending fixture stays pending,
    /// also on drop), with the `anc` rules for an ancestor shape.
    pub fn open(&self, path: &Path) -> Minigraf {
        let db = OpenOptions::new()
            .wal_checkpoint_threshold(usize::MAX)
            .path(path)
            .open()
            .unwrap();
        if matches!(self, Shape::Ancestors { .. }) {
            db.execute("(rule [(anc ?x ?a) [?x :s/parent ?a]])")
                .unwrap();
            db.execute("(rule [(anc ?x ?a) [?x :s/parent ?p] (anc ?p ?a)])")
                .unwrap();
        }
        db
    }
}

fn edn(v: &Value) -> String {
    match v {
        Value::Integer(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Ref(u) => format!("#uuid \"{u}\""),
        Value::Set(s) => format!("#{{{}}}", s.iter().map(edn).collect::<Vec<_>>().join(" ")),
        _ => panic!("unexpected fixture value"),
    }
}

/// An order-insensitive digest of an answer (row count, wrapping sum and
/// rotated xor of row hashes), plus an order-sensitive fold for ordered
/// answers: the harness's `Digest`.
#[derive(Default, PartialEq, Eq, Debug, Clone, Copy)]
pub struct Digest {
    pub rows: u64,
    pub sum: u64,
    pub xor: u64,
    pub ordered: u64,
}

fn mix(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h = (*h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
}

fn hash_value(h: &mut u64, value: &Value) {
    match value {
        Value::Integer(n) => {
            mix(h, &[1]);
            mix(h, &n.to_le_bytes());
        }
        Value::Boolean(b) => mix(h, &[2, u8::from(*b)]),
        Value::Ref(u) => {
            mix(h, &[3]);
            mix(h, u.as_bytes());
        }
        Value::Set(s) => {
            mix(h, &[4]);
            mix(h, &(s.len() as u64).to_le_bytes());
            for v in s.iter() {
                hash_value(h, v);
            }
        }
        _ => panic!("unexpected result type in oracle"),
    }
}

impl Digest {
    pub fn add(&mut self, row: &[Value], ordered: bool) {
        let mut h = 0xcbf29ce484222325;
        mix(&mut h, &(row.len() as u64).to_le_bytes());
        for v in row {
            hash_value(&mut h, v);
        }
        self.rows += 1;
        self.sum = self.sum.wrapping_add(h);
        self.xor ^= h.rotate_left((h % 64) as u32);
        if ordered {
            self.ordered = self.ordered.wrapping_mul(0x100000001b3) ^ h;
        }
    }
}

/// Drain `query` and digest its rows.
pub fn drain(db: &Minigraf, query: &str, ordered: bool) -> Digest {
    let mut cursor = db.query(query).unwrap();
    let mut digest = Digest::default();
    while let Some(batch) = cursor.next_batch(1000).unwrap() {
        for row in batch.rows() {
            digest.add(row, ordered);
        }
    }
    digest
}
