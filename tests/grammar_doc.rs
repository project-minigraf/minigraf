//! `docs/GRAMMAR.md` is rendered from `tests/grammar/grammar.pest` (#643).
//!
//! `grammar_doc_is_current` renders the grammar as EBNF and fails when the file differs.
//! After editing the grammar, rewrite the file with:
//!
//!     UPDATE_GRAMMAR_DOC=1 cargo test --test grammar_doc
#![cfg(not(target_arch = "wasm32"))]

use pest_meta::ast::{Expr, RuleType};
use pest_meta::parser::{self, Rule};
use std::path::Path;

const GRAMMAR: &str = include_str!("grammar/grammar.pest");

/// The verifier's entry rule: it is not part of the language.
const SKIPPED: &[&str] = &["edn_value_only"];

/// Lines longer than this are broken before a top-level `|`.
const WIDTH: usize = 88;

/// An EBNF expression, before printing.
enum Node {
    Lit(String),
    Name(String),
    Comment(String),
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Opt(Box<Node>),
    Star(Box<Node>),
    Plus(Box<Node>),
}

fn kebab(name: &str) -> String {
    name.replace('_', "-")
}

fn builtin(name: &str) -> Option<Node> {
    Some(match name {
        "ASCII_DIGIT" => Node::Name("digit".into()),
        "ASCII_ALPHA" => Node::Name("letter".into()),
        "ASCII_ALPHANUMERIC" => Node::Alt(vec![
            Node::Name("letter".into()),
            Node::Name("digit".into()),
        ]),
        "ANY" => Node::Name("any-char".into()),
        _ => return None,
    })
}

/// Pest expression → EBNF node. `None` for what EBNF leaves out (`SOI`, `EOI`).
fn convert(expr: &Expr) -> Option<Node> {
    Some(match expr {
        Expr::Str(s) => Node::Lit(s.clone()),
        Expr::Insens(s) => Node::Seq(vec![
            Node::Lit(s.clone()),
            Node::Comment("case-insensitive".into()),
        ]),
        Expr::Range(a, b) => Node::Name(format!("{}..{}", quote(a), quote(b))),
        Expr::Ident(name) if name == "SOI" || name == "EOI" => return None,
        Expr::Ident(name) => builtin(name).unwrap_or_else(|| Node::Name(kebab(name))),
        Expr::PosPred(e) => Node::Comment(format!("followed by {}", inline(e))),
        Expr::NegPred(e) => Node::Comment(format!("not followed by {}", inline(e))),
        Expr::Seq(a, b) => {
            let mut items = Vec::new();
            for e in [a, b] {
                match convert(e) {
                    Some(Node::Seq(more)) => items.extend(more),
                    Some(n) => items.push(n),
                    None => {}
                }
            }
            match items.len() {
                0 => return None,
                1 => items.pop().unwrap(),
                _ => Node::Seq(items),
            }
        }
        Expr::Choice(a, b) => {
            let mut items = Vec::new();
            for e in [a, b] {
                match convert(e).expect("SOI/EOI as a choice alternative") {
                    Node::Alt(more) => items.extend(more),
                    n => items.push(n),
                }
            }
            Node::Alt(items)
        }
        Expr::Opt(e) => Node::Opt(Box::new(convert(e)?)),
        Expr::Rep(e) => Node::Star(Box::new(convert(e)?)),
        Expr::RepOnce(e) => Node::Plus(Box::new(convert(e)?)),
        Expr::RepExact(e, n) => Node::Seq(copies(e, *n)),
        Expr::RepMin(e, n) => {
            let mut items = copies(e, *n);
            items.push(Node::Star(Box::new(convert(e)?)));
            Node::Seq(items)
        }
        Expr::RepMax(e, n) => Node::Seq(optional_copies(e, *n)),
        Expr::RepMinMax(e, m, n) => {
            let mut items = copies(e, *m);
            items.extend(optional_copies(e, n - m));
            Node::Seq(items)
        }
        #[allow(unreachable_patterns)]
        other => panic!("grammar_doc: no EBNF for the pest construct {other:?}"),
    })
}

fn copies(e: &Expr, n: u32) -> Vec<Node> {
    (0..n).filter_map(|_| convert(e)).collect()
}

fn optional_copies(e: &Expr, n: u32) -> Vec<Node> {
    (0..n)
        .filter_map(|_| convert(e).map(|n| Node::Opt(Box::new(n))))
        .collect()
}

/// A pest expression printed on one line, for a lookahead comment.
fn inline(e: &Expr) -> String {
    convert(e).map(|n| print(&n, 0)).unwrap_or_default()
}

/// A string literal in EBNF: double quotes, or single quotes around a `"`. Pest's
/// escapes are already decoded, so `"\\"` arrives as one backslash.
fn quote(s: &str) -> String {
    assert!(
        s.chars().all(|c| !c.is_control()),
        "grammar_doc: control character in a literal outside WHITESPACE"
    );
    if s.contains('"') {
        assert!(!s.contains('\''), "grammar_doc: literal with both quotes");
        format!("'{s}'")
    } else {
        format!("\"{s}\"")
    }
}

/// Binding strength: alternation 0, sequence 1, postfix and atoms 2. A child weaker
/// than `min` is parenthesized.
fn print(node: &Node, min: u8) -> String {
    let (text, strength) = match node {
        Node::Lit(s) => (quote(s), 2),
        Node::Name(s) => (s.clone(), 2),
        Node::Comment(s) => (format!("(* {s} *)"), 2),
        Node::Seq(items) => (
            items
                .iter()
                .map(|n| print(n, 2))
                .collect::<Vec<_>>()
                .join(" "),
            1,
        ),
        Node::Alt(items) => (
            items
                .iter()
                .map(|n| print(n, 1))
                .collect::<Vec<_>>()
                .join(" | "),
            0,
        ),
        Node::Opt(e) => (format!("{}?", print(e, 2)), 2),
        Node::Star(e) => (format!("{}*", print(e, 2)), 2),
        Node::Plus(e) => (format!("{}+", print(e, 2)), 2),
    };
    if strength < min {
        format!("({text})")
    } else {
        text
    }
}

/// One rule. A top-level alternation that does not fit in `WIDTH` is broken before
/// its `|`s, packing alternatives into lines, each `|` under the `=` of `::=`.
fn render_rule(name: &str, width: usize, node: &Node, token: bool) -> String {
    let head = format!("{name:<width$} ::= ");
    let tail = if token { "  (* token *)" } else { "" };
    let one_line = format!("{head}{}{tail}", print(node, 0));
    let Node::Alt(items) = node else {
        return one_line;
    };
    if one_line.len() <= WIDTH {
        return one_line;
    }
    let indent = format!("{:width$}| ", "", width = width + 3);
    let mut lines = vec![head.clone()];
    for (i, item) in items.iter().enumerate() {
        let alt = print(item, 1);
        let line = lines.last_mut().unwrap();
        let sep = if i == 0 { "" } else { " | " };
        if i > 0 && line.len() + sep.len() + alt.len() > WIDTH {
            lines.push(format!("{indent}{alt}"));
        } else {
            line.push_str(sep);
            line.push_str(&alt);
        }
    }
    lines.last_mut().unwrap().push_str(tail);
    lines.join("\n")
}

/// The characters `WHITESPACE` skips, as words.
fn whitespace_note(expr: &Expr) -> String {
    fn names(expr: &Expr, out: &mut Vec<&'static str>) {
        match expr {
            Expr::Choice(a, b) => {
                names(a, out);
                names(b, out);
            }
            Expr::Str(s) => out.push(match s.as_str() {
                " " => "spaces",
                "\t" => "tabs",
                "\n" => "newlines",
                "\r" => "carriage returns",
                "," => "commas",
                _ => panic!("grammar_doc: no name for a WHITESPACE literal"),
            }),
            _ => panic!("grammar_doc: WHITESPACE is not a choice of literals"),
        }
    }
    let mut words = Vec::new();
    names(expr, &mut words);
    let list = match words.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => unreachable!(),
    };
    format!(
        "Between tokens the parser skips {list}. Inside a rule marked `(* token *)` \
         nothing is skipped, and the rules it uses are read the same way."
    )
}

/// The title of a `// ── X ──` line.
fn section_title(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("//")?.trim().strip_prefix('─')?;
    let title = rest
        .trim_start_matches('─')
        .trim()
        .trim_end_matches('─')
        .trim();
    (!title.is_empty()).then_some(title)
}

/// The rule a line defines, if it starts one.
fn defined_rule(line: &str) -> Option<&str> {
    let (name, rest) = line.split_once('=')?;
    let name = name.trim();
    let starts = !line.starts_with(char::is_whitespace)
        && !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (starts && !rest.starts_with('=')).then_some(name)
}

/// The whole of `docs/GRAMMAR.md` for a pest grammar.
fn render(src: &str) -> String {
    let pairs = parser::parse(Rule::grammar_rules, src).expect("grammar.pest parses");
    let rules = parser::consume_rules(pairs).expect("grammar.pest is valid");

    // Section titles in text order, each attached to the next rule defined after it.
    let mut pending: Option<String> = None;
    let mut title_of = std::collections::HashMap::new();
    for line in src.lines() {
        if let Some(title) = section_title(line) {
            pending = Some(title.to_string());
        } else if let Some(name) = defined_rule(line)
            && name != "WHITESPACE"
            && !SKIPPED.contains(&name)
            && let Some(title) = pending.take()
        {
            title_of.insert(name.to_string(), title);
        }
    }

    // Rules grouped into sections; names are aligned within a section.
    let mut note = None;
    let mut sections: Vec<(Option<String>, Vec<(String, Node, bool)>)> = Vec::new();
    for rule in &rules {
        if rule.name == "WHITESPACE" {
            note = Some(whitespace_note(&rule.expr));
            continue;
        }
        if SKIPPED.contains(&rule.name.as_str()) {
            continue;
        }
        let title = title_of.remove(&rule.name);
        if title.is_some() || sections.is_empty() {
            sections.push((title, Vec::new()));
        }
        let node = convert(&rule.expr).expect("a rule matches something");
        let token = matches!(rule.ty, RuleType::Atomic | RuleType::CompoundAtomic);
        sections
            .last_mut()
            .unwrap()
            .1
            .push((kebab(&rule.name), node, token));
    }
    assert!(
        title_of.is_empty(),
        "grammar_doc: a section title lost its rule"
    );

    let mut ebnf = Vec::new();
    for (title, rules) in &sections {
        if let Some(title) = title {
            ebnf.push(format!("(* ── {title} ── *)"));
        }
        let width = rules.iter().map(|(n, ..)| n.len()).max().unwrap_or(0);
        for (name, node, token) in rules {
            ebnf.push(render_rule(name, width, node, *token));
        }
        ebnf.push(String::new());
    }
    ebnf.push(
        "(* letter = \"a\"..\"z\" | \"A\"..\"Z\"; digit = \"0\"..\"9\"; \
         any-char = any one character *)"
            .into(),
    );

    format!(
        "# Datalog Grammar\n\
         \n\
         <!-- Generated from tests/grammar/grammar.pest by tests/grammar_doc.rs. \
         Do not edit: change the grammar, then run \
         UPDATE_GRAMMAR_DOC=1 cargo test --test grammar_doc -->\n\
         \n\
         This EBNF is the structural syntax of Minigraf's Datalog commands. It is \
         generated from `tests/grammar/grammar.pest`, the grammar the parser is tested \
         against: for every input up to a size bound, a test checks that the parser rejects \
         what the grammar rejects, and accepts what it accepts unless a semantic rule \
         rejects it. To change this page, \
         edit the grammar and run `UPDATE_GRAMMAR_DOC=1 cargo test --test grammar_doc`.\n\
         \n\
         {note}\n\
         \n\
         EBNF has no lookahead, so a pest lookahead appears as a comment: \
         `(* not followed by x *)` means the next characters must not match `x`, \
         and nothing is read. Literals have no escapes: `\"\\\"` is one backslash and \
         `'\"'` one double quote.\n\
         \n\
         ```ebnf\n{}\n```\n\
         \n\
         Input that matches this grammar can still be rejected: the parser also enforces \
         the rules in the Semantic Constraints section of the Datalog reference \
         (variable safety, binding, aggregate and window compatibility, UUID and \
         timestamp formats).\n",
        ebnf.join("\n").trim_end(),
        note = note.expect("grammar.pest defines WHITESPACE"),
    )
}

#[test]
fn grammar_doc_is_current() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/GRAMMAR.md");
    let rendered = render(GRAMMAR);
    if std::env::var_os("UPDATE_GRAMMAR_DOC").is_some_and(|v| v == "1") {
        std::fs::write(&path, &rendered).expect("write docs/GRAMMAR.md");
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if current != rendered {
        let (n, want, have) = rendered
            .lines()
            .map(Some)
            .chain(std::iter::repeat(None))
            .zip(current.lines().map(Some).chain(std::iter::repeat(None)))
            .take(rendered.lines().count().max(current.lines().count()))
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| (i + 1, a.unwrap_or("<end>"), b.unwrap_or("<end>")))
            .unwrap_or((0, "<trailing newline>", "<trailing newline>"));
        panic!(
            "docs/GRAMMAR.md is stale at line {n}:\n  expected: {want}\n  found:    {have}\n\
             Run: UPDATE_GRAMMAR_DOC=1 cargo test --test grammar_doc"
        );
    }
}

fn ebnf_of(src: &str) -> String {
    let doc = render(src);
    let start = doc.find("```ebnf\n").unwrap() + 8;
    let end = doc[start..].find("\n```").unwrap() + start;
    doc[start..end].to_string()
}

const WS: &str = "WHITESPACE = _{ \" \" }\n";

#[test]
fn renders_sequence_choice_and_grouping() {
    let ebnf = ebnf_of(&format!(
        "{WS}a = {{ \"x\" ~ (b | \"y\" ~ b)* ~ b? ~ b+ }}\nb = {{ \"z\" | \"w\" }}\n"
    ));
    assert!(ebnf.contains("a ::= \"x\" (b | \"y\" b)* b? b+\n"));
    assert!(ebnf.contains("b ::= \"z\" | \"w\"\n"));
}

#[test]
fn renders_names_builtins_and_drops_soi_eoi() {
    let ebnf = ebnf_of(&format!(
        "{WS}top_rule = {{ SOI ~ ASCII_ALPHANUMERIC ~ ASCII_DIGIT ~ ASCII_ALPHA ~ ANY ~ EOI }}\n"
    ));
    assert!(ebnf.contains("top-rule ::= (letter | digit) digit letter any-char\n"));
}

#[test]
fn renders_lookaheads_as_comments() {
    let ebnf = ebnf_of(&format!(
        "{WS}k = @{{ \":k\" ~ !(\"a\" | \"b\") ~ &\"c\" }}\n"
    ));
    assert!(ebnf.contains(
        "k ::= \":k\" (* not followed by \"a\" | \"b\" *) (* followed by \"c\" *)  (* token *)"
    ));
}

#[test]
fn renders_exact_and_bounded_repetition() {
    let ebnf = ebnf_of(&format!(
        "{WS}a = {{ \"x\"{{3}} }}\nb = {{ \"y\"{{1,}} }}\nc = {{ \"z\"{{1,2}} }}\nd = {{ \"w\"{{,1}} }}\n"
    ));
    assert!(ebnf.contains("a ::= \"x\" \"x\" \"x\"\n"));
    assert!(ebnf.contains("b ::= \"y\" \"y\"*\n"));
    assert!(ebnf.contains("c ::= \"z\" \"z\"?\n"));
    assert!(ebnf.contains("d ::= \"w\"?\n"));
}

#[test]
fn quotes_literals_and_marks_only_atomic_rules() {
    let ebnf = ebnf_of(&format!(
        "{WS}q = @{{ \"\\\"\" ~ \"\\\\\" }}\ns = _{{ \"s\" }}\nc = ${{ \"c\" }}\n"
    ));
    assert!(ebnf.contains("q ::= '\"' \"\\\"  (* token *)\n"));
    assert!(ebnf.contains("s ::= \"s\"\n"));
    assert!(ebnf.contains("c ::= \"c\"  (* token *)"));
}

#[test]
fn places_section_titles_and_skips_the_verifier_rule() {
    let ebnf = ebnf_of(&format!(
        "// ── SKIPPED ──\n{WS}// ── ONE ──\n// plain comment\nlong_name = {{ \"a\" }}\nb = {{ \"b\" }}\n\
         // ── TWO ──\nedn_value_only = {{ SOI ~ b ~ EOI }}\nc = {{ b }}\n"
    ));
    assert!(ebnf.starts_with(
        "(* ── ONE ── *)\nlong-name ::= \"a\"\nb         ::= \"b\"\n\n(* ── TWO ── *)\nc ::= b\n"
    ));
    assert!(!ebnf.contains("SKIPPED"));
    assert!(!ebnf.contains("edn-value-only"));
}

#[test]
fn breaks_long_alternations_under_the_definition() {
    let alts: Vec<String> = (0..12).map(|i| format!("\"alternative{i}\"")).collect();
    let ebnf = ebnf_of(&format!("{WS}r = {{ {} }}\n", alts.join(" | ")));
    let lines: Vec<&str> = ebnf.lines().take_while(|l| !l.is_empty()).collect();
    assert!(lines.len() > 1, "expected a broken rule");
    assert!(lines.iter().all(|l| l.len() <= WIDTH));
    assert!(lines[0].starts_with("r ::= \"alternative0\" | "));
    assert!(lines[1..].iter().all(|l| l.starts_with("    | ")));
}

#[test]
fn whitespace_becomes_the_note() {
    let src = "WHITESPACE = _{ \" \" | \"\\t\" | \"\\n\" | \"\\r\" | \",\" }\na = { \"a\" }\n";
    assert!(render(src).contains(
        "Between tokens the parser skips spaces, tabs, newlines, carriage returns and commas."
    ));
    assert_eq!(ebnf_of(src).lines().next(), Some("a ::= \"a\""));
}
