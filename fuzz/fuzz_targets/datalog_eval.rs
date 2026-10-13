#![no_main]
use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;

/// The facts every input runs against, as the tokens a pattern would use.
const FACTS: [[&str; 3]; 3] = [
    [":alice", ":name", "\"Alice\""],
    [":bob", ":name", "\"Bob\""],
    [":alice", ":age", "30"],
];

/// Inputs whose patterns could join to more rows than this are skipped.
const MAX_JOIN_ROWS: u64 = 50_000;

fuzz_target!(|data: &[u8]| {
    if let Ok(s) = std::str::from_utf8(data) {
        // The executor holds every intermediate binding, so patterns that share no
        // variables build their whole cross product: 3^k rows over these 3 facts.
        // That is a known limit, not a new bug (#575, fixed by #473/#549).
        if join_rows_bound(s) > MAX_JOIN_ROWS {
            return;
        }
        let db = match minigraf::db::Minigraf::in_memory() {
            Ok(db) => db,
            Err(_) => return,
        };
        let _ = db
            .execute(r#"(transact [[:alice :name "Alice"] [:bob :name "Bob"] [:alice :age 30]])"#);
        let _ = db.execute(s);
    }
});

/// An upper estimate of the rows a cross product of the input's patterns yields.
///
/// Every innermost vector of three or more tokens counts as a pattern. Patterns are
/// grouped by the variables they share (`_` and `?_…` are wildcards and share
/// nothing); each group's join is counted against `FACTS`, and the groups multiply.
/// A group with no matches counts as 1, so an empty pattern never hides a cross
/// product that the planner may build before reaching it.
fn join_rows_bound(s: &str) -> u64 {
    let patterns = patterns(s);
    let mut parent: Vec<usize> = (0..patterns.len()).collect();
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (i, p) in patterns.iter().enumerate() {
        for &t in p.iter().filter(|t| is_var(t)) {
            match owner.get(t) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
                None => {
                    owner.insert(t, i);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<[&str; 3]>> = HashMap::new();
    for (i, p) in patterns.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(*p);
    }
    let mut total: u64 = 1;
    for group in groups.values() {
        let mut rows = 0;
        count_join(group, &mut Vec::new(), &mut rows);
        total = total.saturating_mul(rows.max(1));
        if total > MAX_JOIN_ROWS {
            break;
        }
    }
    total
}

/// The first three tokens of every innermost `[...]` holding at least three.
fn patterns(s: &str) -> Vec<[&str; 3]> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, ch) in s.char_indices() {
        match ch {
            '[' => start = Some(i + 1),
            '(' | ')' => start = None,
            ']' => {
                if let Some(from) = start.take() {
                    let mut tokens = s[from..i].split_whitespace();
                    if let (Some(e), Some(a), Some(v)) =
                        (tokens.next(), tokens.next(), tokens.next())
                    {
                        out.push([e, a, v]);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn is_wildcard(t: &str) -> bool {
    t == "_" || t.starts_with("?_")
}

fn is_var(t: &str) -> bool {
    t.starts_with('?') && !is_wildcard(t)
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Counts the group's join over `FACTS` depth first, stopping past `MAX_JOIN_ROWS`.
fn count_join<'a>(
    group: &[[&'a str; 3]],
    bound: &mut Vec<(&'a str, &'static str)>,
    rows: &mut u64,
) {
    let Some((pattern, rest)) = group.split_first() else {
        *rows += 1;
        return;
    };
    for fact in &FACTS {
        if *rows > MAX_JOIN_ROWS {
            return;
        }
        let mark = bound.len();
        let matches = pattern.iter().zip(fact).all(|(&t, &x)| {
            if is_wildcard(t) {
                true
            } else if is_var(t) {
                match bound.iter().find(|(v, _)| *v == t) {
                    Some(&(_, y)) => y == x,
                    None => {
                        bound.push((t, x));
                        true
                    }
                }
            } else {
                t == x
            }
        });
        if matches {
            count_join(rest, bound, rows);
        }
        bound.truncate(mark);
    }
}
