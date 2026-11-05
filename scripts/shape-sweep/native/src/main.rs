use anyhow::{Result, ensure};
use minigraf::{FactRecord, LogWriter, Minigraf, OpenOptions, Value, ValueSet};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
struct Case {
    id: String,
    family: String,
    n: usize,
    shape: String,
    operation: String,
    #[serde(default)]
    k: usize,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default = "default_cache")]
    cache: usize,
    #[serde(default)]
    pending: bool,
}
fn default_cache() -> usize {
    256
}

fn entity(i: usize) -> Uuid {
    Uuid::from_u128(0x84100000000000000000000000000000 + i as u128)
}
fn integer(i: usize) -> Value {
    Value::Integer(i.try_into().unwrap())
}

fn part(c: &Case, i: usize) -> usize {
    match c.shape.as_str() {
        "one" => 0,
        "hundred" => i % 100,
        "singleton" => i,
        "skew" if i < c.n * 9 / 10 => 0,
        "skew" => 1 + (i - c.n * 9 / 10) % 99,
        _ => panic!("unknown partition layout"),
    }
}

fn members(c: &Case, i: usize) -> ValueSet {
    let base = match c.shape.as_str() {
        "repeated" => 0,
        "unique" => (i + 1) * c.k,
        "skew" if i < c.n * 9 / 10 => 0,
        "skew" => (i + 1) * c.k,
        _ => panic!("unknown set layout"),
    };
    ValueSet::new((base..base + c.k).map(integer)).unwrap()
}

fn target(c: &Case, i: usize) -> usize {
    match c.shape.as_str() {
        "dense" => i,
        "descending" => c.n - 1 - i,
        "sparse" => 16 * i,
        "permuted" => (37 * i) % c.n,
        "repeat" => i % c.k.max(1),
        "hot" if i % 10 != 0 => i % 16,
        "hot" => 16 * i,
        _ => panic!("unknown reference layout"),
    }
}

fn fixture_key(c: &Case) -> String {
    let collection = c.family == "set" && c.operation == "collect";
    format!(
        "v1-{}-{}-{}-{}-{}-{}",
        c.family, c.n, c.shape, c.k, collection, c.pending
    )
}

fn records(c: &Case, mut emit: impl FnMut(Uuid, &str, Value) -> Result<()>) -> Result<()> {
    match c.family.as_str() {
        "scalar" => {
            for j in 0..c.n {
                let i = (j * 37) % c.n;
                emit(entity(i), ":s/value", integer(i))?;
            }
        }
        "compound" => {
            for i in 0..c.n {
                emit(entity(i), ":s/value", integer(i))?;
                emit(entity(i), ":s/part", integer(i % 4))?;
                emit(entity(i), ":s/score", integer((i / 4) % 8))?;
            }
        }
        "window" => {
            for i in 0..c.n {
                emit(entity(i), ":s/value", integer(i))?;
                emit(entity(i), ":s/part", integer(part(c, i)))?;
            }
        }
        "set" => {
            for i in 0..c.n {
                emit(entity(i), ":s/id", integer(i))?;
                let set = members(c, i);
                if c.operation == "collect" {
                    for value in set.iter() {
                        emit(entity(i), ":s/member", value.clone())?;
                    }
                } else {
                    emit(entity(i), ":s/set", Value::Set(set))?;
                }
            }
        }
        "uuid" => {
            // Allocate target dictionary entries before references. The diagnostic
            // pass separately observes actual internal IDs; this is UUID order.
            for i in 0..c.n * 16 {
                emit(entity(i), ":s/target", Value::Boolean(true))?;
            }
            for i in 0..c.n {
                emit(entity(c.n * 16 + i), ":s/id", integer(i))?;
                emit(
                    entity(c.n * 16 + i),
                    ":s/ref",
                    Value::Ref(entity(target(c, i))),
                )?;
            }
        }
        "recursion" => {
            for i in 1..=c.n {
                let parent = if c.shape == "chain" {
                    i - 1
                } else {
                    (i - 1) / 10
                };
                emit(entity(i), ":s/parent", Value::Ref(entity(parent)))?;
            }
            for i in c.n + 1 - c.k..=c.n {
                emit(entity(i), ":s/demand", Value::Boolean(true))?;
            }
        }
        _ => anyhow::bail!("unknown fixture family"),
    }
    Ok(())
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

fn build(c: &Case, dir: &Path) -> Result<PathBuf> {
    ensure!(c.n > 0, "nonempty fixture required");
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.graph", fixture_key(c)));
    let meta = path.with_extension("json");
    if meta.exists() {
        ensure!(path.exists(), "manifest without fixture");
        return Ok(path);
    }
    ensure!(
        !path.exists() && !path.with_extension("graph.partial").exists(),
        "incomplete fixture must be inspected before retry"
    );
    let start = Instant::now();
    let mut count = 0usize;
    if c.pending {
        let db = OpenOptions::new()
            .wal_checkpoint_threshold(usize::MAX)
            .path(&path)
            .open()?;
        let mut command = String::from("(transact [");
        records(c, |e, a, v| {
            command.push_str(&format!("[#uuid \"{e}\" {a} {}]", edn(&v)));
            count += 1;
            if count % 1000 == 0 {
                command.push_str("])");
                db.execute(&command)?;
                command = String::from("(transact [");
            }
            Ok(())
        })?;
        if count % 1000 != 0 {
            command.push_str("])");
            db.execute(&command)?;
        }
        // MAX suppresses checkpoint-on-drop, preserving the WAL control.
    } else {
        let mut writer = LogWriter::create(&path, OpenOptions::new())?;
        records(c, |e, a, v| {
            writer.append(&FactRecord {
                entity: e,
                attribute: a.into(),
                value: v,
                tx_count: (count / 1000 + 1) as u64,
                tx_id: 1_700_000_000_000 + (count / 1000) as u64,
                valid_from: 0,
                valid_to: i64::MAX,
                asserted: true,
            })?;
            count += 1;
            Ok(())
        })?;
        writer.finish()?;
    }
    std::fs::write(
        meta,
        serde_json::to_vec_pretty(&json!({"fixture": fixture_key(c),
        "input_facts": count, "entities_parameter": c.n, "build_ns": start.elapsed().as_nanos(),
        "file_bytes": std::fs::metadata(&path)?.len(), "rss": rss(),
        "loader": if c.pending { "transact/WAL; wall-clock tx timestamps" } else { "LogWriter; fixed timestamps; 65536-fact internal checkpoint threshold" }}))?,
    )?;
    Ok(path)
}

fn query(c: &Case) -> Result<String> {
    let body = match c.family.as_str() {
        "scalar" => format!(":find ?v :where [?e :s/value ?v] :order-by [?v :{}]", c.operation),
        "compound" => ":find ?g ?s ?v :where [?e :s/part ?g] [?e :s/score ?s] [?e :s/value ?v] :order-by [?g :desc ?s]".into(),
        "window" => {
            let over = ":over (:partition-by ?p :order-by ?i)";
            let expr = match c.operation.as_str() {
                "sum" => format!("(sum ?i {over})"),
                "rank" => format!("(row-number {over})"),
                "incompatible" => format!("(row-number {over}) (row-number :over (:partition-by ?i :order-by ?p))"),
                _ => anyhow::bail!("unknown window"),
            };
            format!(":find ?i ?p {expr} :where [?e :s/part ?p] [?e :s/value ?i]")
        },
        "set" => match c.operation.as_str() {
            "project" => ":find ?i ?s :where [?e :s/id ?i] [?e :s/set ?s]".into(),
            "collect" => ":find ?i ?s :where [?e :s/id ?i] [(collect-set ?e :s/member) ?s]".into(),
            "membership" => ":find ?i ?a ?b ?c ?d ?f ?g :where [?e :s/id ?i] [?e :s/set ?s] [(in? 0 ?s) ?a] [(not-in? 0 ?s) ?b] [(all-in? #{} ?s) ?c] [(any-in? ?s #{0}) ?d] [(none-in? ?s #{0}) ?f] [(in? -1 ?s) ?g]".into(),
            _ => anyhow::bail!("unknown set operation"),
        },
        "uuid" if c.operation=="refs" => ":find ?i ?r ?r :where [?e :s/id ?i] [?e :s/ref ?r]".into(),
        "uuid" => ":find ?i ?e ?r ?r :where [?e :s/id ?i] [?e :s/ref ?r]".into(),
        "recursion" if c.k==1 => format!(":find ?a :where (anc #uuid \"{}\" ?a)",entity(c.n)),
        "recursion" => ":find ?leaf ?a :where [?leaf :s/demand true] (anc ?leaf ?a)".into(),
        _ => anyhow::bail!("unknown query family"),
    };
    let mut q = format!("(query [{body}");
    if c.offset > 0 {
        q.push_str(&format!(" :offset {}", c.offset));
    }
    if let Some(k) = c.limit {
        q.push_str(&format!(" :limit {k}"));
    }
    q.push_str("])");
    Ok(q)
}

#[derive(Default, PartialEq, Serialize)]
struct Digest {
    rows: u64,
    sum: u64,
    xor: u64,
    ordered: u64,
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
    fn add(&mut self, row: &[Value], ordered: bool) {
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
fn ordered(c: &Case) -> bool {
    matches!(c.family.as_str(), "scalar" | "compound")
}

fn oracle(c: &Case) -> Result<Digest> {
    let mut out = Digest::default();
    let end = c.n.min(c.offset.saturating_add(c.limit.unwrap_or(c.n)));
    match c.family.as_str() {
        "scalar" => {
            for j in c.offset..end {
                let i = if c.operation == "desc" {
                    c.n - 1 - j
                } else {
                    j
                };
                out.add(&[integer(i)], true);
            }
        }
        "compound" => {
            let mut position = 0;
            // Lexicographic group-desc/score-asc/value-asc, without sorting
            // a copy of the database's result or retaining N rows.
            for g in (0..4).rev() {
                for s in 0..8 {
                    for i in (g + 4 * s..c.n).step_by(32) {
                        if position >= c.offset && position < end {
                            out.add(&[integer(g), integer(s), integer(i)], true);
                        }
                        position += 1;
                    }
                }
            }
        }
        "window" => {
            ensure!(
                c.limit.is_none() && c.offset == 0,
                "window oracle requires full unordered output"
            );
            for i in 0..c.n {
                let p = part(c, i);
                let (first, stride) = match c.shape.as_str() {
                    "one" => (0, 1),
                    "hundred" => (p, 100),
                    "singleton" => (i, 1),
                    "skew" if p == 0 => (0, 1),
                    "skew" => (c.n * 9 / 10 + p - 1, 99),
                    _ => unreachable!(),
                };
                let rank = (i - first) / stride + 1;
                let val = if c.operation == "sum" {
                    (first + i) * rank / 2
                } else {
                    rank
                };
                let mut row = vec![integer(i), integer(p), integer(val)];
                if c.operation == "incompatible" {
                    row.push(integer(1));
                }
                out.add(&row, false);
            }
        }
        "set" => {
            for i in 0..c.n {
                let set = members(c, i);
                if c.operation == "membership" {
                    let hit = set.iter().any(|v| *v == integer(0));
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
                } else {
                    out.add(&[integer(i), Value::Set(set)], false);
                }
            }
        }
        "uuid" => {
            for i in 0..c.n {
                let r = Value::Ref(entity(target(c, i)));
                let mut row = vec![integer(i)];
                if c.operation != "refs" {
                    row.push(Value::Ref(entity(c.n * 16 + i)));
                }
                row.extend([r.clone(), r]);
                out.add(&row, false);
            }
        }
        "recursion" => {
            for leaf in c.n + 1 - c.k..=c.n {
                let mut i = leaf;
                while i > 0 {
                    i = if c.shape == "chain" {
                        i - 1
                    } else {
                        (i - 1) / 10
                    };
                    let mut row = Vec::new();
                    if c.k > 1 {
                        row.push(Value::Ref(entity(leaf)));
                    }
                    row.push(Value::Ref(entity(i)));
                    out.add(&row, false);
                }
            }
        }
        _ => anyhow::bail!("unknown oracle"),
    }
    Ok(out)
}

fn rss() -> Json {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |prefix: &str| {
        s.lines()
            .find(|l| l.starts_with(prefix))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.parse::<u64>().ok())
    };
    json!({"current_kib":field("VmRSS:"),"peak_kib":field("VmHWM:")})
}

fn drain(db: &Minigraf, q: &str, order: bool) -> Result<(Digest, u128, u128)> {
    let start = Instant::now();
    let mut cursor = db.query(q)?;
    let open = start.elapsed().as_nanos();
    let mut digest = Digest::default();
    while let Some(batch) = cursor.next_batch(256)? {
        for row in batch.rows() {
            digest.add(row, order);
        }
    }
    Ok((digest, open, start.elapsed().as_nanos()))
}

fn probe(c: &Case, dir: &Path) -> Result<Json> {
    Ok(probe_many(c, dir, 1)?.remove(0))
}

fn probe_many(c: &Case, dir: &Path, samples: usize) -> Result<Vec<Json>> {
    ensure!(samples > 0, "positive sample count required");
    let path = build(c, dir)?;
    let expected = oracle(c)?;
    let q = query(c)?;
    let opts = OpenOptions::new()
        .page_cache_size(c.cache)
        .wal_checkpoint_threshold(usize::MAX);
    let open = Instant::now();
    let db = Minigraf::open_with_options(&path, opts)?;
    let open_ns = open.elapsed().as_nanos();
    if c.family == "recursion" {
        db.execute("(rule [(anc ?x ?a) [?x :s/parent ?a]])")?;
        db.execute("(rule [(anc ?x ?a) [?x :s/parent ?p] (anc ?p ?a)])")?;
    }
    let (warm, _, warm_ns) = drain(&db, &q, ordered(c))?;
    ensure!(
        warm == expected,
        "warm-up result differs from independent oracle: {}",
        c.id
    );
    let mut output = Vec::new();
    for sample in 0..samples {
        let before = rss();
        let (actual, query_open_ns, elapsed_ns) = drain(&db, &q, ordered(c))?;
        let after = rss();
        ensure!(
            actual == expected,
            "timed result differs from independent oracle: {}",
            c.id
        );
        output.push(json!({"case":c.id,"query":q,"rows":actual.rows,"verified":true,
        "digest":actual,"expected":expected,"elapsed_ns":elapsed_ns,"query_open_ns":query_open_ns,
        "open_ns":open_ns,"warmup_ns":warm_ns,"rss_before":before,"rss_after":after,
        "file_bytes":std::fs::metadata(&path)?.len(),"cache_pages":c.cache,"pending":c.pending,
        "fixture":fixture_key(c),"batch_rows":256,"sample_in_process":sample,"process_samples":samples,
        "meter_peak_bytes":null,
        "meter_reason":"public API does not expose operator memory; separate diagnostic pass"}));
    }
    Ok(output)
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 4 || args.len() == 5,
        "usage: shape-sweep build|run|batch CASE.json FIXTURE_DIR [SAMPLES]"
    );
    let c: Case = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let dir = Path::new(&args[3]);
    match args[1].as_str() {
        "build" => {
            let p = build(&c, dir)?;
            println!("{}", std::fs::read_to_string(p.with_extension("json"))?);
        }
        "run" => println!("{}", probe(&c, dir)?),
        "batch" => println!(
            "{}",
            serde_json::to_string(&probe_many(
                &c,
                dir,
                args.get(4)
                    .ok_or_else(|| anyhow::anyhow!("batch needs SAMPLES"))?
                    .parse()?
            )?)?
        ),
        "query" => println!("{}", query(&c)?),
        _ => anyhow::bail!("unknown command"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(family: &str, shape: &str, operation: &str) -> Case {
        Case {
            id: format!("{family}/{shape}/{operation}"),
            family: family.into(),
            n: 100,
            shape: shape.into(),
            operation: operation.into(),
            k: 4,
            offset: 0,
            limit: None,
            cache: 256,
            pending: false,
        }
    }

    #[test]
    fn scalar_pages_include_descending_offsets_and_empty_tail() {
        let dir = tempfile::tempdir().unwrap();
        for op in ["asc", "desc"] {
            for offset in [0, 50, 99, 100] {
                let mut c = case("scalar", "permuted", op);
                c.offset = offset;
                c.limit = Some(10);
                let out = probe(&c, dir.path()).unwrap();
                assert_eq!(out["rows"], (100 - offset).min(10));
                assert_eq!(out["verified"], true);
            }
        }
    }

    #[test]
    fn windows_check_every_rank_and_running_sum_under_skew() {
        let dir = tempfile::tempdir().unwrap();
        for shape in ["one", "hundred", "singleton", "skew"] {
            for op in ["sum", "rank", "incompatible"] {
                let c = case("window", shape, op);
                let out = probe(&c, dir.path()).unwrap();
                assert_eq!(out["rows"], 100);
                assert_eq!(out["verified"], true);
            }
        }
    }

    #[test]
    fn sets_check_empty_and_repeated_and_unique_values() {
        let dir = tempfile::tempdir().unwrap();
        for shape in ["repeated", "unique", "skew"] {
            for k in [0, 1, 4, 32] {
                for op in ["project", "membership", "collect"] {
                    let mut c = case("set", shape, op);
                    c.k = k;
                    let out = probe(&c, dir.path()).unwrap();
                    assert_eq!(out["rows"], 100);
                    assert_eq!(out["verified"], true);
                }
            }
        }
    }

    #[test]
    fn uuid_projection_preserves_sparse_repeated_and_duplicate_columns() {
        let dir = tempfile::tempdir().unwrap();
        for shape in ["dense", "descending", "sparse", "permuted", "hot", "repeat"] {
            let mut c = case("uuid", shape, "project");
            c.k = 16;
            let out = probe(&c, dir.path()).unwrap();
            assert_eq!(out["rows"], 100);
            assert_eq!(out["verified"], true);
        }
    }

    #[test]
    fn recursive_ancestors_match_independent_walks_in_both_storage_states() {
        let dir = tempfile::tempdir().unwrap();
        for shape in ["chain", "tree"] {
            for pending in [false, true] {
                let mut c = case("recursion", shape, "ancestors");
                c.n = 110;
                c.k = 1;
                c.pending = pending;
                let out = probe(&c, dir.path()).unwrap();
                assert_eq!(out["verified"], true);
                assert_eq!(out["rows"], if shape == "chain" { 110 } else { 2 });
            }
        }
    }

    #[test]
    fn batched_samples_verify_each_answer_on_one_pending_handle() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = case("scalar", "permuted", "desc");
        c.pending = true;
        c.offset = 50;
        c.limit = Some(10);
        let rows = probe_many(&c, dir.path(), 5).unwrap();
        assert_eq!(rows.len(), 5);
        for row in &rows {
            assert_eq!(row["rows"], 10);
            assert_eq!(row["verified"], true);
            assert_eq!(row["digest"], rows[0]["digest"]);
        }
    }

    #[test]
    fn interrupted_committed_fixture_is_preserved_for_inspection() {
        let dir = tempfile::tempdir().unwrap();
        let c = case("scalar", "permuted", "asc");
        let path = build(&c, dir.path()).unwrap();
        let partial = dir
            .path()
            .join(format!("{}.graph.partial", fixture_key(&c)));
        let original = std::fs::read(&path).unwrap();
        std::fs::rename(&path, &partial).unwrap();
        std::fs::remove_file(path.with_extension("json")).unwrap();
        let result = build(&c, dir.path());
        assert!(result.is_err(), "interrupted fixture was silently rebuilt");
        assert!(partial.exists(), "interrupted fixture evidence disappeared");
        assert!(
            std::fs::read(partial).unwrap() == original,
            "interrupted fixture evidence changed"
        );
    }
}
