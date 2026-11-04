use std::collections::HashMap;

/// The facts every input runs against, as the tokens a pattern would use.
const FACTS: [[&str; 3]; 3] = [
    [":alice", ":name", "\"Alice\""],
    [":bob", ":name", "\"Bob\""],
    [":alice", ":age", "30"],
];

/// Inputs whose patterns could build more bindings than this are skipped.
pub(crate) const MAX_JOIN_ROWS: u64 = 50_000;

/// An upper estimate of the bindings the input's patterns build, partial ones
/// included.
///
/// Every innermost vector of three or more tokens counts as a pattern. Patterns are
/// grouped by the variables they share within a parenthesized scope (`_` and
/// `?_…` are wildcards and share nothing). Negation/branch bodies can execute
/// independently or once per outer binding: outer constraints must not shrink
/// their estimate (#802). Each group's join is counted against `FACTS`, and the
/// groups multiply, including groups in different scopes. Join-variable declaration
/// vectors are excluded because they are not fact patterns. `:db/valid-at`
/// scans are counted as `[entity _ _]`, one row per visible fact (#835).
/// A group with no matches counts as 1, so an empty pattern never hides a cross
/// product that the planner may build before reaching it.
pub(crate) fn join_rows_bound(s: &str) -> u64 {
    let patterns = patterns(s);
    let mut parent: Vec<usize> = (0..patterns.len()).collect();
    let mut owner: HashMap<(usize, &str), usize> = HashMap::new();
    for (i, (scope, p)) in patterns.iter().enumerate() {
        for &t in p.iter().filter(|t| is_var(t)) {
            match owner.get(&(*scope, t)) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
                None => {
                    owner.insert((*scope, t), i);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<[&str; 3]>> = HashMap::new();
    for (i, (_, p)) in patterns.iter().enumerate() {
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

/// The scope and first three tokens of every innermost pattern-like vector.
/// Delimiters inside string literals are text. This is a conservative lexical
/// guard, not a Datalog parser; malformed input still reaches the real parser
/// when its estimate is within budget.
fn patterns(s: &str) -> Vec<(usize, [&str; 3])> {
    let mut out = Vec::new();
    let mut start = None;
    let mut scopes = vec![(0, 0)];
    let mut next_scope = 0;
    let (mut in_string, mut escaped) = (false, false);
    for (i, ch) in s.char_indices() {
        if in_string {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => {
                let &(scope, from) = scopes.last().unwrap_or(&(0, 0));
                // (not-join [vars...] ...) and (or-join [vars...] ...).
                start = if matches!(s[from..i].trim(), "not-join" | "or-join") {
                    None
                } else {
                    Some((i + 1, scope))
                };
            }
            '(' => {
                next_scope += 1;
                scopes.push((next_scope, i + 1));
                start = None;
            }
            ')' => {
                if scopes.len() > 1 {
                    scopes.pop();
                }
                start = None;
            }
            ']' => {
                if let Some((from, scope)) = start.take() {
                    let mut tokens = pattern_tokens(&s[from..i]);
                    if let (Some(e), Some(a), Some(v)) =
                        (tokens.next(), tokens.next(), tokens.next())
                    {
                        // Valid-at scans every visible fact of the entity; its
                        // value is query metadata, not the stored fact's value.
                        let p = if a == ":db/valid-at" {
                            [e, "_", "_"]
                        } else {
                            [e, a, v]
                        };
                        out.push((scope, p));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Lex the pattern's atoms without requiring whitespace between a symbol and
/// keyword/string. EDN commas separate tokens, including before a variable
/// (#835). Keep strings whole so their contents cannot become join variables.
fn pattern_tokens(mut s: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        if s.is_empty() {
            return None;
        }
        let mut end = s.len();
        if s.starts_with('"') {
            let mut escaped = false;
            for (i, c) in s.char_indices().skip(1) {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    end = i + 1;
                    break;
                }
            }
        } else {
            for (i, c) in s.char_indices().skip(1) {
                if c.is_whitespace() || matches!(c, ',' | ':' | '"') {
                    end = i;
                    break;
                }
            }
        }
        let (token, rest) = s.split_at(end);
        s = rest;
        Some(token)
    })
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

/// Counts the bindings the group's join over `FACTS` builds in text order, after
/// each pattern and not only after the last one, depth first, stopping past
/// `MAX_JOIN_ROWS`.
fn count_join<'a>(
    group: &[[&'a str; 3]],
    bound: &mut Vec<(&'a str, &'static str)>,
    rows: &mut u64,
) {
    let Some((pattern, rest)) = group.split_first() else {
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
            *rows += 1;
            count_join(rest, bound, rows);
        }
        bound.truncate(mark);
    }
}
