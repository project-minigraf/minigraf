//! Bounded-exhaustive check of the parser against `tests/grammar/grammar.pest` (#643).
//!
//! For every generated input `s` (design:
//! `docs/superpowers/specs/2026-10-20-grammar-drift-verifier-design.md`):
//!
//! 1. pest rejects `s` ⇒ the parser rejects `s`;
//! 2. pest accepts `s` ⇒ the parser accepts `s`, or rejects it with a code in [`SEMANTIC`].
//!
//! Two levels generate the inputs, both deterministically:
//!
//! - lexical: every string of 1 to 3 characters over [`alphabet`], pest's
//!   `edn_value_only` against [`parse_edn`];
//! - structural: every balanced sequence of up to `k` [`TOKENS`] in the hole of each of
//!   [`TEMPLATES`], pest's `command` against [`parse_datalog_command`]. `k` is
//!   `MINIGRAF_GRAMMAR_BOUND` (default [`DEFAULT_BOUND`]).
//!
//! Drift that exists today is listed in [`KNOWN_DRIFT`]. Each failing input is put down
//! to its first cause in text order ([`lexical_cause`], [`structural_cause`]), so it
//! matches at most one entry. A failure with no known cause fails the test, and so does
//! an entry that no failure matches once `k` reaches its `min_k`: a fix removes its entry.

use super::{parse_datalog_command, parse_edn};
use crate::error::MinigrafError;
use crate::query::datalog::types::EdnValue;
use pest::Parser as _;
use std::fmt::Write as _;

#[derive(pest_derive::Parser)]
#[grammar = "tests/grammar/grammar.pest"]
struct G;

const DEFAULT_BOUND: usize = 3;

/// Fillers of up to this many tokens are also joined with no separators, including at
/// both template boundaries (`parse_edn` and pest each read `:find?e` as one keyword,
/// so the lexical level cannot see how glued tokens split).
const GLUE_BOUND: usize = 3;

const LEXICAL_MAX_LEN: usize = 3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Accept,
    Reject(&'static str),
}

fn outcome<T>(r: anyhow::Result<T>) -> Outcome {
    match r {
        Ok(_) => Outcome::Accept,
        Err(e) => Outcome::Reject(MinigrafError::from(e).code()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Level {
    Lexical,
    Structural,
}

// ── Inputs ────────────────────────────────────────────────────────────────

/// Printable ASCII, then `\t \n \r`, then non-ASCII stand-ins: a letter, a CJK
/// character, an Arabic-Indic digit and a no-break space.
fn alphabet() -> Vec<char> {
    let mut a: Vec<char> = (0x20u8..=0x7E).map(char::from).collect();
    a.extend(['\t', '\n', '\r', 'é', '名', '٣', '\u{00A0}']);
    a
}

const TOKENS: [&str; 32] = [
    "(",
    ")",
    "[",
    "]",
    "{",
    "}",
    "query",
    "transact",
    "rule",
    "not",
    "not-join",
    "or",
    "and",
    ":find",
    ":where",
    ":with",
    ":as-of",
    ":valid-at",
    ":any-valid-time",
    ":over",
    ":order-by",
    ":valid-from",
    ":foo",
    "?x",
    "?y",
    "foo",
    "1",
    "1.5",
    "\"2024-01-01T00:00:00Z\"",
    "<",
    "$s",
    "#uuid \"550e8400-e29b-41d4-a716-446655440000\"",
];

/// Bracket token → `(kind, opens)`.
fn bracket(t: u8) -> Option<(u8, bool)> {
    (t < 6).then_some((t / 2, t.is_multiple_of(2)))
}

/// `(prefix, suffix)`: the hole sits between them. Each puts the hole where a short
/// filler can complete a clause.
const TEMPLATES: [(&str, &str); 15] = [
    ("(query [", "])"),
    ("(query [:find", ":where [?x :a ?y]])"),
    ("(query [:find ?x :where [?x :a ?y]", "])"),
    ("(query [:find ?x :where [?x :a ?y] (not", ")])"),
    ("(query [:find ?x :where [?x :a ?y] (not-join [?x]", ")])"),
    ("(query [:find ?x :where [?x :a ?y] (or", ")])"),
    ("(query [:find ?x :where [?x :a ?y] (or-join [?x]", ")])"),
    ("(query [:find ?x :where [?x :a ?y] (or (and", "))])"),
    ("(query [:find ?x (sum ?y :over (", ")) :where [?x :a ?y]])"),
    ("(transact [", "])"),
    ("(transact [[:a :b 1", "]])"),
    ("(transact {", "} [[:a :b 1]])"),
    ("(retract [", "])"),
    ("(rule [(", ") [?x :a 1]])"),
    ("(rule [(foo ?x)", "])"),
];

/// Calls `f` on every balanced filler of exactly `len` tokens, in token-index order.
fn each_filler(len: usize, f: &mut impl FnMut(&[u8])) {
    fn go(buf: &mut Vec<u8>, stack: &mut Vec<u8>, len: usize, f: &mut impl FnMut(&[u8])) {
        let left = len - buf.len();
        if left == 0 {
            if stack.is_empty() {
                f(buf);
            }
            return;
        }
        for t in 0..TOKENS.len() as u8 {
            match bracket(t) {
                // Every open bracket needs a closer in the tokens left after this one.
                Some((kind, true)) if stack.len() < left - 1 => {
                    stack.push(kind);
                    buf.push(t);
                    go(buf, stack, len, f);
                    buf.pop();
                    stack.pop();
                }
                Some((kind, false)) if stack.last() == Some(&kind) => {
                    stack.pop();
                    buf.push(t);
                    go(buf, stack, len, f);
                    buf.pop();
                    stack.push(kind);
                }
                None if stack.len() < left => {
                    buf.push(t);
                    go(buf, stack, len, f);
                    buf.pop();
                }
                _ => {}
            }
        }
    }
    go(&mut Vec::new(), &mut Vec::new(), len, f);
}

fn bound() -> usize {
    std::env::var("MINIGRAF_GRAMMAR_BOUND")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_BOUND)
}

// ── Semantic codes ────────────────────────────────────────────────────────

/// A code that an input `grammar.pest` accepts can raise, with such an input. Every
/// site that raises a listed code must be semantic; the comment above each entry
/// gives the sites (`parser.rs`) and why.
struct Semantic {
    code: &'static str,
    witness: &'static str,
}

const SEMANTIC: &[Semantic] = &[
    // parse_query: `:with` without an aggregate in `:find` (the reference's Semantic
    // Constraints).
    Semantic {
        code: "PRS-021",
        witness: "(query [:find ?x :with ?y :where [?x :a ?y]])",
    },
    // parse_query: a `:with` variable no `:where` clause binds.
    Semantic {
        code: "PRS-022",
        witness: "(query [:find (count ?x) :with ?z :where [?x :a ?y]])",
    },
    // parse_query, two sites: a `:find` variable, or a window's variable, that no
    // `:where` clause binds (#406).
    Semantic {
        code: "PRS-080",
        witness: "(query [:find ?z :where [?x :a ?y]])",
    },
    // parse_window: a window's `:over` without `:order-by` (`over_option*` allows none).
    Semantic {
        code: "PRS-084",
        witness: "(query [:find ?x (sum ?y :over ()) :where [?x :a ?y]])",
    },
    // parse_rule: a rule body with no pattern or rule call (`rule_vector` allows
    // `where_clause*`).
    Semantic {
        code: "PRS-107",
        witness: "(rule [(foo ?x)])",
    },
];

fn is_semantic(code: &str) -> bool {
    SEMANTIC.iter().any(|s| s.code == code)
}

// ── Known drift ───────────────────────────────────────────────────────────

/// The cause a failing input is put down to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cause {
    // Lexical.
    /// `;` starts a line comment in the parser; the grammar has no comments.
    Comment,
    /// A string with no closing quote reads to the end of the input.
    UnterminatedString,
    /// A non-ASCII letter or digit in a symbol or keyword (the grammar is ASCII).
    UnicodeLetter,
    /// `:` with no keyword character after it is the keyword `:`.
    BareColon,
    /// `-` with no word character after it is the symbol `-`.
    LoneMinus,
    // Structural.
    /// `(query [])`.
    EmptyQuery,
    /// `:find` with no find spec before the next keyword or the end.
    EmptyFind,
    /// `:where` with no clause before the next keyword or the end.
    EmptyWhere,
    /// `:with` with no variable before the next keyword or the end.
    EmptyWith,
    /// A keyword the query vector does not know, with no value after it, is ignored.
    UnknownKeyword,
    /// A value after `:as-of`, `:valid-at`, `:any-valid-time` or `:max-*` and its
    /// argument continues the section before that keyword.
    ResumedSection,
    /// A clause list whose head is a variable or an operator is read as a rule call.
    CallHead,
    /// A `:find` list whose head is a variable is read as an aggregate.
    AggregateHead,
    /// A rule head whose name is not a plain symbol or whose argument is not a
    /// variable.
    RuleHead,
    /// A fact `[e a v {…} x …]`: the parser drops everything after the map.
    FactArity,
}

impl Cause {
    fn level(self) -> Level {
        match self {
            Cause::Comment
            | Cause::UnterminatedString
            | Cause::UnicodeLetter
            | Cause::BareColon
            | Cause::LoneMinus => Level::Lexical,
            _ => Level::Structural,
        }
    }
}

struct Drift {
    cause: Cause,
    issue: u32,
    /// The smallest structural bound whose inputs show it (unused at the lexical level).
    min_k: usize,
}

const KNOWN_DRIFT: &[Drift] = &[
    Drift {
        cause: Cause::Comment,
        issue: 650,
        min_k: 0,
    },
    Drift {
        cause: Cause::UnterminatedString,
        issue: 651,
        min_k: 0,
    },
    Drift {
        cause: Cause::UnicodeLetter,
        issue: 650,
        min_k: 0,
    },
    Drift {
        cause: Cause::BareColon,
        issue: 651,
        min_k: 0,
    },
    Drift {
        cause: Cause::LoneMinus,
        issue: 651,
        min_k: 0,
    },
    Drift {
        cause: Cause::EmptyQuery,
        issue: 651,
        min_k: 0,
    },
    Drift {
        cause: Cause::EmptyFind,
        issue: 651,
        min_k: 0,
    },
    Drift {
        cause: Cause::EmptyWhere,
        issue: 651,
        min_k: 1,
    },
    Drift {
        cause: Cause::EmptyWith,
        issue: 651,
        min_k: 1,
    },
    Drift {
        cause: Cause::UnknownKeyword,
        issue: 651,
        min_k: 1,
    },
    Drift {
        cause: Cause::ResumedSection,
        issue: 651,
        min_k: 3,
    },
    Drift {
        cause: Cause::CallHead,
        issue: 651,
        min_k: 3,
    },
    Drift {
        cause: Cause::AggregateHead,
        issue: 651,
        min_k: 4,
    },
    Drift {
        cause: Cause::RuleHead,
        issue: 651,
        min_k: 1,
    },
    Drift {
        cause: Cause::FactArity,
        issue: 649,
        min_k: 3,
    },
];

/// Characters the parser reads as part of a symbol after its first.
fn symbol_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '?' | '_' | '-' | '/')
}

/// The first cause, in text order, of a lexical failure.
fn lexical_cause(text: &str) -> Option<Cause> {
    let c: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < c.len() {
        let next = c.get(i + 1).copied();
        match c[i] {
            '"' => {
                i += 1;
                loop {
                    match c.get(i) {
                        None => return Some(Cause::UnterminatedString),
                        Some('\\') => i += 2,
                        Some('"') => break,
                        Some(_) => i += 1,
                    }
                }
            }
            ';' => return Some(Cause::Comment),
            ':' if !next.is_some_and(symbol_char) => return Some(Cause::BareColon),
            '-' if !next.is_some_and(symbol_char)
                && !(i > 0 && (symbol_char(c[i - 1]) || matches!(c[i - 1], ':' | '#' | '$'))) =>
            {
                return Some(Cause::LoneMinus);
            }
            ch if !ch.is_ascii() && ch.is_alphanumeric() => return Some(Cause::UnicodeLetter),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The first cause, in text order, of a structural failure, read from the parser's
/// EDN tree of the input (glued tokens split as the parser splits them).
fn structural_cause(text: &str) -> Option<Cause> {
    let EdnValue::List(cmd) = parse_edn(text).ok()? else {
        return None;
    };
    let arg = cmd.get(1);
    match cmd.first()? {
        EdnValue::Symbol(s) if s == "query" => query_cause(arg?.as_vector()?),
        EdnValue::Symbol(s) if s == "rule" => rule_cause(arg?.as_vector()?),
        EdnValue::Symbol(s) if s == "transact" => cmd.last()?.as_vector()?.iter().find_map(|f| {
            let f = f.as_vector()?;
            (f.len() > 4 && f[3].is_map()).then_some(Cause::FactArity)
        }),
        _ => None,
    }
}

/// Walks the query vector's sections as `parse_query` does.
fn query_cause(items: &[EdnValue]) -> Option<Cause> {
    if items.is_empty() {
        return Some(Cause::EmptyQuery);
    }
    let value_at = |j: usize| items.get(j).is_some_and(|v| v.as_keyword().is_none());
    // The section that values go to: `:find`, `:where`, or an unknown keyword.
    let mut section: Option<&str> = None;
    let mut i = 0;
    while i < items.len() {
        let Some(k) = items[i].as_keyword() else {
            let cause = match section {
                Some(":where") => clause_cause(&items[i]),
                Some(":find") => find_cause(&items[i]),
                _ => None,
            };
            if cause.is_some() {
                return cause;
            }
            i += 1;
            continue;
        };
        match k {
            ":as-of" | ":valid-at" | ":max-derived-facts" | ":max-results" | ":any-valid-time" => {
                i += if k == ":any-valid-time" { 1 } else { 2 };
                if section.is_some() && value_at(i) {
                    return Some(Cause::ResumedSection);
                }
            }
            ":with" => {
                if !value_at(i + 1) {
                    return Some(Cause::EmptyWith);
                }
                i += 1;
                while value_at(i) {
                    i += 1;
                }
            }
            _ if !value_at(i + 1) => {
                return Some(match k {
                    ":find" => Cause::EmptyFind,
                    ":where" => Cause::EmptyWhere,
                    _ => Cause::UnknownKeyword,
                });
            }
            _ => {
                section = Some(k);
                i += 1;
            }
        }
    }
    None
}

/// Whether pest's `plain_sym` reads all of `s`.
fn is_plain_symbol(s: &str) -> bool {
    let word = |c: char| c.is_ascii_alphanumeric() || matches!(c, '?' | '_' | '-' | '/');
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => chars.all(word),
        Some('-') => !chars.as_str().is_empty() && chars.all(word),
        _ => false,
    }
}

/// Whether pest's `op_sym` reads all of `s`.
fn is_op_symbol(s: &str) -> bool {
    matches!(s, "<=" | ">=" | "!=" | "<" | ">" | "=" | "+" | "*" | "/")
}

/// The cause in a `:find` spec: an aggregate or window list must start with a name.
fn find_cause(spec: &EdnValue) -> Option<Cause> {
    match spec.as_list()?.first()? {
        EdnValue::Symbol(f) if !is_plain_symbol(f) && !is_op_symbol(f) => {
            Some(Cause::AggregateHead)
        }
        _ => None,
    }
}

/// The first cause in a `:where` clause, a negation or disjunction body, or a rule body.
fn clause_cause(clause: &EdnValue) -> Option<Cause> {
    let list = clause.as_list()?;
    let EdnValue::Symbol(head) = list.first()? else {
        return None;
    };
    let rest = &list[1..];
    match head.as_str() {
        "not" => rest.iter().find_map(clause_cause),
        "not-join" => rest.get(1..)?.iter().find_map(clause_cause),
        "or" => rest.iter().find_map(branch_cause),
        "or-join" => rest.get(1..)?.iter().find_map(branch_cause),
        h if !is_plain_symbol(h) => Some(Cause::CallHead),
        _ => None,
    }
}

fn branch_cause(branch: &EdnValue) -> Option<Cause> {
    match branch.as_list().map(Vec::as_slice) {
        Some([EdnValue::Symbol(and), body @ ..]) if and == "and" => {
            body.iter().find_map(clause_cause)
        }
        _ => clause_cause(branch),
    }
}

fn rule_cause(rule: &[EdnValue]) -> Option<Cause> {
    let head = rule.first()?.as_list()?;
    let name_ok = matches!(head.first(), Some(EdnValue::Symbol(s)) if is_plain_symbol(s));
    let args_ok = head
        .iter()
        .skip(1)
        .all(|a| matches!(a, EdnValue::Symbol(s) if s.starts_with('?')));
    if !(name_ok && args_ok) {
        return Some(Cause::RuleHead);
    }
    rule[1..].iter().find_map(clause_cause)
}

// ── The check ─────────────────────────────────────────────────────────────

/// Which clause of the property an input breaks, if any.
fn failure(pest_ok: bool, parser: Outcome) -> Option<String> {
    match (pest_ok, parser) {
        (false, Outcome::Accept) => Some("pest rejects, parser accepts".to_string()),
        (true, Outcome::Reject(code)) if !is_semantic(code) => Some(format!(
            "pest accepts, parser rejects with syntax code {code}"
        )),
        _ => None,
    }
}

/// How many failures an entry matched, and the first.
#[derive(Default, Clone)]
struct Hits {
    count: u64,
    first: Option<String>,
}

impl Hits {
    fn add(&mut self, shown: &str) {
        self.count += 1;
        if self.first.is_none() {
            self.first = Some(shown.to_string());
        }
    }

    fn merge(&mut self, other: Hits) {
        self.count += other.count;
        if self.first.is_none() {
            self.first = other.first;
        }
    }
}

const SHOWN: usize = 20;

#[derive(Default)]
struct Report {
    inputs: u64,
    /// One per [`KNOWN_DRIFT`] entry.
    known: Vec<Hits>,
    /// Failures with no known cause, by reason.
    unknown: Vec<(String, Hits)>,
    /// The first [`SHOWN`] failures with no known cause.
    unknown_shown: Vec<String>,
}

impl Report {
    fn new() -> Self {
        Report {
            known: vec![Hits::default(); KNOWN_DRIFT.len()],
            ..Report::default()
        }
    }

    fn check(&mut self, level: Level, text: &str, pest_ok: bool, parser: Outcome) {
        self.inputs += 1;
        let Some(why) = failure(pest_ok, parser) else {
            return;
        };
        let cause = match level {
            Level::Lexical => lexical_cause(text),
            Level::Structural => structural_cause(text),
        };
        let shown = format!("{text:?} ({why})");
        match KNOWN_DRIFT.iter().position(|d| Some(d.cause) == cause) {
            Some(i) => self.known[i].add(&shown),
            None => {
                if self.unknown_shown.len() < SHOWN {
                    self.unknown_shown.push(shown.clone());
                }
                match self.unknown.iter_mut().find(|(w, _)| *w == why) {
                    Some((_, hits)) => hits.add(&shown),
                    None => {
                        let mut hits = Hits::default();
                        hits.add(&shown);
                        self.unknown.push((why, hits));
                    }
                }
            }
        }
    }

    /// Adds `other`, whose inputs come after this report's.
    fn merge(&mut self, other: Report) {
        self.inputs += other.inputs;
        for (h, o) in self.known.iter_mut().zip(other.known) {
            h.merge(o);
        }
        for (why, o) in other.unknown {
            match self.unknown.iter_mut().find(|(w, _)| *w == why) {
                Some((_, hits)) => hits.merge(o),
                None => self.unknown.push((why, o)),
            }
        }
        let room = SHOWN - self.unknown_shown.len();
        self.unknown_shown
            .extend(other.unknown_shown.into_iter().take(room));
    }

    /// Prints the entries' matches and fails on any failure with no known cause, or
    /// on an entry of `level` that matched nothing although `k` reaches it.
    fn finish(self, level: Level, k: usize) {
        let mut out = String::new();
        let mut stale = false;
        for (d, h) in KNOWN_DRIFT.iter().zip(&self.known) {
            if d.cause.level() != level {
                continue;
            }
            match &h.first {
                Some(first) => {
                    let _ = writeln!(
                        out,
                        "  known {:?} (#{}): {} inputs, first {first}",
                        d.cause, d.issue, h.count
                    );
                }
                None if level == Level::Lexical || k >= d.min_k => {
                    stale = true;
                    let _ = writeln!(
                        out,
                        "  STALE {:?} (#{}): matched no failing input; remove the entry",
                        d.cause, d.issue
                    );
                }
                None => {}
            }
        }
        let summary = format!("{level:?} level, k = {k}, {} inputs:\n{out}", self.inputs);
        if self.unknown.is_empty() && !stale {
            eprintln!("{summary}");
            return;
        }
        let mut msg = summary;
        if !self.unknown.is_empty() {
            msg.push_str("failures with no known cause, by reason:\n");
            for (why, h) in &self.unknown {
                let first = h.first.as_deref().unwrap_or("");
                let _ = writeln!(msg, "  {} x {why}; first {first}", h.count);
            }
            msg.push_str("the first of them:\n");
            for u in &self.unknown_shown {
                let _ = writeln!(msg, "  {u}");
            }
        }
        panic!("grammar drift:\n{msg}");
    }
}

/// Checks every string of `len` characters whose first character is in `first`.
fn check_lexical(chars: &[char], len: usize, first: std::ops::Range<usize>) -> Report {
    let mut report = Report::new();
    let mut digits = vec![0usize; len];
    digits[0] = first.start;
    let mut text = String::new();
    loop {
        text.clear();
        text.extend(digits.iter().map(|&d| chars[d]));
        let pest_ok = G::parse(Rule::edn_value_only, &text).is_ok();
        report.check(Level::Lexical, &text, pest_ok, outcome(parse_edn(&text)));
        // Odometer step: the last digit turns fastest.
        let mut i = len - 1;
        loop {
            digits[i] += 1;
            if digits[i] < if i == 0 { first.end } else { chars.len() } {
                break;
            }
            if i == 0 {
                return report;
            }
            digits[i] = 0;
            i -= 1;
        }
    }
}

fn check_template(template: usize, k: usize) -> Report {
    let (prefix, suffix) = TEMPLATES[template];
    let mut report = Report::new();
    let mut text = String::new();
    for len in 0..=k {
        each_filler(len, &mut |filler| {
            for sep in [" ", ""] {
                if sep.is_empty() && len > GLUE_BOUND {
                    continue;
                }
                text.clear();
                text.push_str(prefix);
                for &t in filler {
                    text.push_str(sep);
                    text.push_str(TOKENS[usize::from(t)]);
                }
                text.push_str(sep);
                text.push_str(suffix);
                let pest_ok = G::parse(Rule::command, &text).is_ok();
                let parser = outcome(parse_datalog_command(&text));
                report.check(Level::Structural, &text, pest_ok, parser);
            }
        });
    }
    report
}

/// Runs `jobs` on scoped threads and merges their reports in job order.
fn run<J: Send>(jobs: Vec<J>, f: impl Fn(J) -> Report + Sync) -> Report {
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = jobs.into_iter().map(|j| s.spawn(move || f(j))).collect();
        let mut total = Report::new();
        for h in handles {
            total.merge(h.join().expect("verifier worker panicked"));
        }
        total
    })
}

#[test]
fn lexical_level() {
    let chars = alphabet();
    assert_eq!(chars.len(), 102, "alphabet size");
    let n = chars.len();
    let parts = std::thread::available_parallelism().map_or(4, |p| p.get().min(n));
    // One job per (length, range of first characters), in input order.
    let jobs: Vec<_> = (1..=LEXICAL_MAX_LEN)
        .flat_map(|len| (0..parts).map(move |p| (len, p * n / parts..(p + 1) * n / parts)))
        .collect();
    run(jobs, |(len, first)| check_lexical(&chars, len, first)).finish(Level::Lexical, 0);
}

#[test]
fn structural_level() {
    let k = bound();
    run((0..TEMPLATES.len()).collect(), |t| check_template(t, k)).finish(Level::Structural, k);
}

#[test]
fn filler_counts_match_the_spec() {
    let count = |k: usize| {
        let mut n = 0u64;
        for len in 0..=k {
            each_filler(len, &mut |_| n += 1);
        }
        n
    };
    assert_eq!(count(3), 18_516);
    assert_eq!(count(4), 487_678);
}

#[test]
fn semantic_witnesses_raise_their_code() {
    for s in SEMANTIC {
        assert!(
            G::parse(Rule::command, s.witness).is_ok(),
            "pest rejects the witness of {}",
            s.code
        );
        assert_eq!(
            outcome(parse_datalog_command(s.witness)),
            Outcome::Reject(s.code),
            "witness of {}",
            s.code
        );
    }
}
