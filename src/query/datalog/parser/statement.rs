//! `transact` and `retract` statements, read one fact at a time (#550, spec
//! `2026-10-21-streaming-transactions-design.md` §3).
//!
//! A statement's errors are found in phases (spec §3.3): 1 tokens, 2 EDN
//! structure, 3 the command, 4 each fact's shape, 5 materialisation, 6 valid
//! windows. Today's whole-input code runs each phase over the whole statement
//! before the next, so it reports the first error of the earliest phase that
//! has one. The reader keeps that order in one pass that holds one fact at a
//! time: it records the error of the earliest phase found so far, runs only the
//! phases before it from then on, and reports it at the end of the input.

use super::{Lexer, Parser, Token, parse_valid_time_map};
use crate::db::VALID_FROM_USE_TX_TIME;
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::storage::check_valid_windows;
use crate::graph::types::{Fact, TxId, VALID_TIME_FOREVER};
use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
use crate::query::datalog::types::{AttributeSpec, DatalogCommand, EdnValue, Pattern, Transaction};
use anyhow::Result;

/// Most facts a statement holds materialised at once; a statement with more
/// is read again in chunks of this many (spec §3.3).
const TX_CHUNK_FACTS: usize = 16_384;

#[cfg(test)]
thread_local! {
    static CHUNK_FACTS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Set this thread's chunk size (`None`: [`TX_CHUNK_FACTS`]), so tests reach
/// pass 2 with small statements.
#[cfg(test)]
pub(crate) fn set_tx_chunk_facts(n: Option<usize>) {
    CHUNK_FACTS.with(|c| c.set(n));
}

fn tx_chunk_facts() -> usize {
    #[cfg(test)]
    if let Some(n) = CHUNK_FACTS.with(std::cell::Cell::get) {
        return n.max(1);
    }
    TX_CHUNK_FACTS
}

/// Which statement a command is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Transact,
    Retract,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Transact => "transact",
            Kind::Retract => "retract",
        }
    }
}

/// The statement `input` is, from its first two tokens: `(` and the symbol
/// `transact` or `retract`. `None` for any other command, and when those
/// tokens do not lex (the whole-input parser reports that error).
pub(crate) fn kind_of(input: &str) -> Option<Kind> {
    let mut lexer = Lexer::new(input);
    if lexer.next()? != Token::LeftParen {
        return None;
    }
    match lexer.next()? {
        Token::Symbol(s) if s == "transact" => Some(Kind::Transact),
        Token::Symbol(s) if s == "retract" => Some(Kind::Retract),
        _ => None,
    }
}

// Phases, in the order today's code runs them (spec §3.3).
const LEX: u8 = 1;
const STRUCTURE: u8 = 2;
const COMMAND: u8 = 3;
const FACT: u8 = 4;
const MATERIALIZE: u8 = 5;
const WINDOW: u8 = 6;

/// The error of the earliest phase found so far: within a phase, the first.
struct FirstError {
    phase: u8,
    error: Option<anyhow::Error>,
}

impl FirstError {
    fn new() -> Self {
        FirstError {
            phase: u8::MAX,
            error: None,
        }
    }

    fn record(&mut self, phase: u8, error: anyhow::Error) {
        if phase < self.phase {
            self.phase = phase;
            self.error = Some(error);
        }
    }

    /// Whether `phase` still runs: no error of it or an earlier phase yet.
    fn runs(&self, phase: u8) -> bool {
        phase < self.phase
    }
}

/// A statement's transaction-level valid-time defaults.
#[derive(Clone, Copy, Default)]
struct Defaults {
    valid_from: Option<i64>,
    valid_to: Option<i64>,
}

/// Read the statement `input` (its kind checked by [`kind_of`]) through
/// phases 1–4. Each fact that passes them while no error is recorded goes to
/// `on_fact` with the byte offset just after it. Returns the defaults.
fn read(
    input: &str,
    kind: Kind,
    first: &mut FirstError,
    on_fact: &mut dyn FnMut(Pattern, usize, &mut FirstError),
) -> Defaults {
    let mut p = Parser::new(Lexer::new(input));
    // `(` and the command symbol.
    p.advance();
    p.advance();
    let mut shape = Shape::default();
    if let Err(e) = read_elements(&mut p, kind, &mut shape, first, on_fact) {
        first.record(STRUCTURE, e);
    }
    if let Err(e) = p.lexer.finish() {
        first.record(LEX, e);
    }
    let defaults = shape.defaults();
    if let Err(e) = shape.check(kind) {
        first.record(COMMAND, e);
    }
    defaults
}

/// What phase 3 needs to know about a statement's elements.
#[derive(Default)]
struct Shape {
    elements: usize,
    /// The transaction-level map's defaults (`transact` only), when the first
    /// element is a map.
    map: Option<Result<Defaults>>,
    /// Whether the facts element is a vector.
    facts_vector: bool,
}

impl Shape {
    /// The defaults of a well-formed transaction-level map, else none.
    fn defaults(&self) -> Defaults {
        match &self.map {
            Some(Ok(defaults)) => *defaults,
            _ => Defaults::default(),
        }
    }

    /// Phase 3, in the order `parse_transact` / `parse_retract` check it.
    fn check(self, kind: Kind) -> Result<()> {
        match kind {
            Kind::Transact => {
                if self.elements == 0 {
                    bail_coded!(ErrorCode::Prs041);
                }
                let consumed = match self.map {
                    Some(map) => {
                        map?;
                        if self.elements < 2 {
                            bail_coded!(ErrorCode::Prs048);
                        }
                        2
                    }
                    None => 1,
                };
                if self.elements > consumed {
                    bail_coded!(ErrorCode::Prs075, self.elements - consumed);
                }
                if !self.facts_vector {
                    bail_coded!(ErrorCode::Prs042);
                }
            }
            Kind::Retract => {
                if self.elements == 0 {
                    bail_coded!(ErrorCode::Prs043);
                }
                if self.elements > 1 {
                    bail_coded!(ErrorCode::Prs076, self.elements - 1);
                }
                if !self.facts_vector {
                    bail_coded!(ErrorCode::Prs044);
                }
            }
        }
        Ok(())
    }
}

/// The command list's elements, then the end of the input. Returns the first
/// structure error (phase 2); the facts go through [`read_facts`].
fn read_elements(
    p: &mut Parser<'_>,
    kind: Kind,
    shape: &mut Shape,
    first: &mut FirstError,
    on_fact: &mut dyn FnMut(Pattern, usize, &mut FirstError),
) -> Result<()> {
    loop {
        // The command list is at depth 1, as `parse_edn` parses it.
        p.depth = 1;
        let (opens_map, opens_vector) = match p.peek() {
            None => bail_coded!(ErrorCode::Prs005),
            Some(Token::RightParen) => {
                p.advance();
                break;
            }
            Some(token) => (*token == Token::LeftBrace, *token == Token::LeftBracket),
        };
        let at = shape.elements;
        shape.elements += 1;
        let facts_at = usize::from(shape.map.is_some());
        if kind == Kind::Transact && at == 0 && opens_map {
            p.keep = true;
            let map = p.parse_value();
            p.keep = false;
            let pairs = match map? {
                EdnValue::Map(pairs) => pairs,
                _ => bail_coded!(ErrorCode::Int019, "a map element parsed as another kind"),
            };
            shape.map = Some(
                parse_valid_time_map(&pairs).map(|(valid_from, valid_to)| Defaults {
                    valid_from,
                    valid_to,
                }),
            );
        } else if at == facts_at && opens_vector {
            shape.facts_vector = true;
            read_facts(p, kind, shape.defaults(), first, on_fact)?;
        } else {
            // Any other element is checked, not held (decision 2).
            p.keep = false;
            p.parse_value()?;
        }
    }
    if let Some(token) = p.peek() {
        let repr = format!("{:?}", token);
        bail_coded!(ErrorCode::Prs077, repr);
    }
    Ok(())
}

/// The facts vector, from its `[`: each fact through phase 4 while no error of
/// phases 1–4 is recorded. Returns the first structure error.
fn read_facts(
    p: &mut Parser<'_>,
    kind: Kind,
    defaults: Defaults,
    first: &mut FirstError,
    on_fact: &mut dyn FnMut(Pattern, usize, &mut FirstError),
) -> Result<()> {
    p.advance();
    loop {
        // Facts are at depth 3: the command list, then the facts vector.
        p.depth = 2;
        match p.peek() {
            None => bail_coded!(ErrorCode::Prs004),
            Some(Token::RightBracket) => {
                p.advance();
                return Ok(());
            }
            Some(_) => {}
        }
        let wanted = first.runs(FACT);
        p.keep = wanted;
        let fact = p.parse_value();
        p.keep = false;
        let fact = fact?;
        if wanted {
            // `parse_value` leaves no token peeked: the lexer is just past the fact.
            let end = p.lexer.pos;
            match fact_pattern(kind, &fact, defaults) {
                Ok(pattern) => on_fact(pattern, end, first),
                Err(e) => first.record(FACT, e),
            }
        }
    }
}

/// Phase 4 for one fact: its shape and per-fact map, as `parse_transact` /
/// `parse_retract` check each fact.
fn fact_pattern(kind: Kind, fact: &EdnValue, defaults: Defaults) -> Result<Pattern> {
    match kind {
        Kind::Transact => {
            let fact_vec = fact
                .as_vector()
                .ok_or_else(|| err_coded!(ErrorCode::Prs045))?;
            // A fact is [e a v] or [e a v {opts}]
            let (entity, attribute, value, opts) = match fact_vec.as_slice() {
                [e, a, v] => (e, a, v, None),
                [e, a, v, opts] => (e, a, v, Some(opts)),
                _ if fact_vec.len() < 3 => bail_coded!(ErrorCode::Prs046, fact_vec.len()),
                _ => bail_coded!(ErrorCode::Prs108, fact_vec.len()),
            };
            let (fact_valid_from, fact_valid_to) = match opts {
                Some(EdnValue::Map(pairs)) => parse_valid_time_map(pairs)?,
                Some(other) => {
                    let repr = format!("{:?}", other);
                    bail_coded!(ErrorCode::Prs047, repr);
                }
                None => (None, None),
            };
            // Per-fact overrides take precedence over transaction-level defaults
            Ok(Pattern::with_valid_time(
                entity.clone(),
                attribute.clone(),
                value.clone(),
                fact_valid_from.or(defaults.valid_from),
                fact_valid_to.or(defaults.valid_to),
            ))
        }
        Kind::Retract => {
            let fact_vec = fact
                .as_vector()
                .ok_or_else(|| err_coded!(ErrorCode::Prs089))?;
            if fact_vec.len() != 3 {
                bail_coded!(ErrorCode::Prs090, fact_vec.len());
            }
            Pattern::from_edn(fact_vec).map_err(anyhow::Error::msg)
        }
    }
}

/// Parse a statement for [`super::parse_datalog_command`]: phases 1–4, every
/// fact kept as a [`Pattern`].
pub(super) fn parse(input: &str, kind: Kind) -> Result<DatalogCommand> {
    let mut first = FirstError::new();
    let mut patterns = Vec::new();
    let defaults = read(input, kind, &mut first, &mut |pattern, _, _| {
        patterns.push(pattern);
    });
    if let Some(e) = first.error {
        return Err(e);
    }
    let mut tx = Transaction::new(patterns);
    Ok(match kind {
        Kind::Transact => {
            tx.valid_from = defaults.valid_from;
            tx.valid_to = defaults.valid_to;
            DatalogCommand::Transact(tx)
        }
        Kind::Retract => DatalogCommand::Retract(tx),
    })
}

/// Phase 5 for one assertion: a [`Fact`] with `tx_id` and `tx_count` 0 and an
/// unset `valid_from` as [`VALID_FROM_USE_TX_TIME`], stamped at commit.
pub(crate) fn materialize_assertion(
    pattern: &Pattern,
    defaults_from: Option<i64>,
    defaults_to: Option<i64>,
) -> Result<Fact> {
    let entity = edn_to_entity_id(&pattern.entity).map_err(|e| err_coded!(ErrorCode::Int002, e))?;
    let attr = real_attribute(pattern)?;
    let value = edn_to_value(&pattern.value).map_err(|e| err_coded!(ErrorCode::Int003, e))?;
    let valid_from = pattern
        .valid_from
        .or(defaults_from)
        .unwrap_or(VALID_FROM_USE_TX_TIME);
    let valid_to = pattern
        .valid_to
        .or(defaults_to)
        .unwrap_or(VALID_TIME_FOREVER);
    Ok(Fact::with_valid_time(
        entity, attr, value, 0, 0, valid_from, valid_to,
    ))
}

/// Phase 5 for one retraction, with `tx_id` and `tx_count` 0.
pub(crate) fn materialize_retraction_of(pattern: &Pattern) -> Result<Fact> {
    let entity = edn_to_entity_id(&pattern.entity).map_err(|e| err_coded!(ErrorCode::Int002, e))?;
    let attr = real_attribute(pattern)?;
    let value = edn_to_value(&pattern.value).map_err(|e| err_coded!(ErrorCode::Int003, e))?;
    let mut f = Fact::retract(entity, attr, value, 0);
    f.tx_count = 0;
    Ok(f)
}

fn real_attribute(pattern: &Pattern) -> Result<String> {
    match &pattern.attribute {
        AttributeSpec::Real(EdnValue::Keyword(k)) => Ok(k.clone()),
        AttributeSpec::Real(_) => bail_coded!(ErrorCode::Api003),
        AttributeSpec::Pseudo(_) => bail_coded!(ErrorCode::Api004),
    }
}

fn materialize(kind: Kind, pattern: &Pattern) -> Result<Fact> {
    match kind {
        // The pattern already carries the transaction-level defaults.
        Kind::Transact => materialize_assertion(pattern, None, None),
        Kind::Retract => materialize_retraction_of(pattern),
    }
}

/// A `transact` or `retract` checked by pass 1 (spec §3.3).
pub(crate) struct Statement<'a> {
    input: &'a str,
    kind: Kind,
    defaults: Defaults,
    chunk: usize,
    /// The first facts, at most `chunk` of them.
    kept: Vec<Fact>,
    /// When more facts follow the kept ones (a large statement): the byte
    /// offset just after the last kept fact, where pass 2 resumes.
    resume: Option<usize>,
    /// The first error of phase 5 or 6, reported when the facts are taken
    /// (decision 6 of the plan).
    deferred: Option<anyhow::Error>,
}

/// Pass 1 over the statement `input` of kind `kind`: phases 1–6, with
/// `tx_id` as the time an unset `valid_from` takes. A phase 1–4 error is
/// returned here; a phase 5–6 one by [`Statement::facts`].
pub(crate) fn read_statement(input: &str, kind: Kind, tx_id: TxId) -> Result<Statement<'_>> {
    let chunk = tx_chunk_facts();
    let mut kept: Vec<Fact> = Vec::new();
    let mut last_end = 0usize;
    let mut resume = None;
    let mut first = FirstError::new();
    let defaults = read(input, kind, &mut first, &mut |pattern, end, first| {
        if !first.runs(MATERIALIZE) {
            return;
        }
        let fact = match materialize(kind, &pattern) {
            Ok(fact) => fact,
            Err(e) => return first.record(MATERIALIZE, e),
        };
        if first.runs(WINDOW)
            && let Err(e) = check_valid_windows(std::slice::from_ref(&fact), tx_id)
        {
            first.record(WINDOW, e);
        }
        if kept.len() < chunk {
            kept.push(fact);
            last_end = end;
        } else if resume.is_none() {
            resume = Some(last_end);
        }
    });
    if first.phase <= FACT
        && let Some(e) = first.error
    {
        return Err(e);
    }
    Ok(Statement {
        input,
        kind,
        defaults,
        chunk,
        kept,
        resume,
        deferred: first.error,
    })
}

impl<'a> Statement<'a> {
    /// Every fact of the statement, or its phase 5–6 error. A large statement
    /// is read again (pass 2) from where pass 1 stopped keeping facts.
    pub(crate) fn facts(self) -> Result<Vec<Fact>> {
        if let Some(e) = self.deferred {
            return Err(e);
        }
        let mut facts = self.kept;
        if let Some(pos) = self.resume {
            let mut rest = Pass2::new(self.input, pos, self.kind, self.defaults);
            while let Some(chunk) = rest.next_chunk(self.chunk)? {
                facts.extend(chunk);
            }
        }
        Ok(facts)
    }
}

/// Pass 2: the facts after the kept ones, `chunk` at a time.
struct Pass2<'a> {
    parser: Option<Parser<'a>>,
    kind: Kind,
    defaults: Defaults,
}

impl<'a> Pass2<'a> {
    /// Resume at byte `pos` of `input`, inside the facts vector.
    fn new(input: &'a str, pos: usize, kind: Kind, defaults: Defaults) -> Self {
        Pass2 {
            parser: Some(Parser::new(Lexer::at(input, pos))),
            kind,
            defaults,
        }
    }

    /// The next at most `chunk` facts, `None` at the end of the facts vector.
    /// Pass 1 has checked every phase, so an error here is one it did not see.
    fn next_chunk(&mut self, chunk: usize) -> Result<Option<Vec<Fact>>> {
        let Some(p) = self.parser.as_mut() else {
            return Ok(None);
        };
        let mut facts = Vec::new();
        while facts.len() < chunk {
            p.depth = 2;
            match p.peek() {
                Some(Token::RightBracket) => {
                    self.parser = None;
                    break;
                }
                None => {
                    let e = p.lexer.error.take();
                    self.parser = None;
                    return Err(e.unwrap_or_else(|| err_coded!(ErrorCode::Prs004)));
                }
                Some(_) => {}
            }
            let fact = p.parse_value()?;
            let pattern = fact_pattern(self.kind, &fact, self.defaults)?;
            facts.push(materialize(self.kind, &pattern)?);
        }
        Ok((!facts.is_empty()).then_some(facts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Minigraf;
    use crate::error::MinigrafError;
    use crate::graph::storage::check_one_window_per_triple;
    use crate::query::datalog::parser::{parse_datalog_command, parse_whole_command};

    /// An outcome to compare: the value, or the error's code and text.
    fn outcome<T: PartialEq>(r: Result<T>) -> std::result::Result<T, (String, String)> {
        r.map_err(|e| {
            let text = format!("{e:#}");
            (MinigrafError::from(e).code().to_string(), text)
        })
    }

    /// Today's write path: the whole-input parser, then phases 5–7.
    fn old_write(input: &str, tx_id: TxId) -> Result<Vec<Fact>> {
        let facts = match parse_whole_command(input)? {
            DatalogCommand::Transact(tx) => {
                let facts = Minigraf::materialize_transaction(&tx)?;
                check_valid_windows(&facts, tx_id)?;
                facts
            }
            DatalogCommand::Retract(tx) => Minigraf::materialize_retraction(&tx)?,
            _ => bail_coded!(ErrorCode::Int019, "not a statement"),
        };
        check_one_window_per_triple(&facts)?;
        Ok(facts)
    }

    /// The write path through the statement reader.
    fn new_write(input: &str, kind: Kind, tx_id: TxId) -> Result<Vec<Fact>> {
        let facts = read_statement(input, kind, tx_id)?.facts()?;
        check_valid_windows(&facts, tx_id)?;
        check_one_window_per_triple(&facts)?;
        Ok(facts)
    }

    /// `input` gives the same command or error through both parsers, and the
    /// same facts or error through both write paths at every chunk size.
    /// Returns the write's error code, or the number of facts written.
    fn check_same(input: &str) -> std::result::Result<usize, String> {
        let Some(kind) = kind_of(input) else {
            assert!(
                !input.trim_start().starts_with("(transact")
                    && !input.trim_start().starts_with("(retract"),
                "a statement not dispatched: {input}"
            );
            return Err(String::new());
        };
        assert!(
            outcome(parse_datalog_command(input)) == outcome(parse_whole_command(input)),
            "parse differs: {input}"
        );
        let tx_id = crate::graph::types::tx_id_now();
        let old = outcome(old_write(input, tx_id));
        for chunk in [None, Some(1), Some(2), Some(3)] {
            set_tx_chunk_facts(chunk);
            let new = outcome(new_write(input, kind, tx_id));
            set_tx_chunk_facts(None);
            assert!(old == new, "write differs at chunk {chunk:?}: {input}");
        }
        old.map(|facts| facts.len()).map_err(|(code, _)| code)
    }

    const TEMPLATES: [(&str, &str); 10] = [
        ("(transact [", "])"),
        ("(transact [[:a :b 1", "]])"),
        ("(transact {", "} [[:a :b 1]])"),
        ("(transact {:valid-from \"2020-01-01T00:00:00Z\"} [", "])"),
        ("(transact [[:a :b 1] ", " [:c :d 2]])"),
        ("(transact ", ")"),
        ("(transact [", ""),
        ("(retract [", "])"),
        ("(retract [[:a :b 1] ", "])"),
        ("(retract ", ")"),
    ];

    const TOKENS: [&str; 34] = [
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        "[:a :b 1]",
        "[:a :b 1]",
        "[:e :f \"x\" {:valid-from \"2021-01-01T00:00:00Z\"}]",
        "[:a :b 1 {:valid-to \"2021-01-01T00:00:00Z\"}]",
        "[:a :b 1 {:valid-from \"2030-01-01T00:00:00Z\" :valid-to \"2025-01-01T00:00:00Z\"}]",
        "[:a :b 1 {:valid-from \"2019-01-01T00:00:00Z\"}]",
        "[:a :b]",
        "[:a :b 1 2 3]",
        "[:a :b 1 2]",
        "[1.5 :b 1]",
        "[:a b 1]",
        "[:a :b ?x]",
        "[#uuid \"550e8400-e29b-41d4-a716-446655440000\" :b 1]",
        ":valid-from",
        ":valid-to",
        "\"2024-01-01T00:00:00Z\"",
        "\"bad-ts\"",
        "{:foo 1}",
        "1",
        "nil",
        "?x",
        "!",
        "\"open",
        "#foo",
        "#uuid 5",
        "$s",
        "transact",
        "; c\n",
    ];

    fn each_sequence(len: usize, f: &mut impl FnMut(&[usize])) {
        fn go(buf: &mut Vec<usize>, len: usize, f: &mut impl FnMut(&[usize])) {
            if buf.len() == len {
                return f(buf);
            }
            for t in 0..TOKENS.len() {
                buf.push(t);
                go(buf, len, f);
                buf.pop();
            }
        }
        go(&mut Vec::new(), len, f);
    }

    #[test]
    fn statements_match_the_whole_input_parser() {
        // One run per template: its codes and most facts.
        let run = |&(prefix, suffix): &(&str, &str)| {
            let mut text = String::new();
            let mut codes = std::collections::BTreeSet::new();
            let mut most_facts = 0;
            for len in 0..=3 {
                each_sequence(len, &mut |seq| {
                    text.clear();
                    text.push_str(prefix);
                    for &t in seq {
                        text.push(' ');
                        text.push_str(TOKENS[t]);
                    }
                    text.push(' ');
                    text.push_str(suffix);
                    match check_same(&text) {
                        Ok(n) => most_facts = most_facts.max(n),
                        Err(code) => {
                            codes.insert(code);
                        }
                    }
                });
            }
            (codes, most_facts)
        };
        // A thread per template where there are threads (not on wasm32).
        #[cfg(not(target_arch = "wasm32"))]
        let results: Vec<_> = std::thread::scope(|scope| {
            let workers: Vec<_> = TEMPLATES
                .iter()
                .map(|t| scope.spawn(move || run(t)))
                .collect();
            workers
                .into_iter()
                .map(|w| w.join().expect("differential worker panicked"))
                .collect()
        });
        #[cfg(target_arch = "wasm32")]
        let results: Vec<_> = TEMPLATES.iter().map(run).collect();
        let most_facts = results.iter().map(|(_, n)| *n).max().unwrap_or(0);
        let codes: std::collections::BTreeSet<String> =
            results.into_iter().flat_map(|(c, _)| c).collect();
        // The sequences reach pass 2 and every phase.
        assert!(most_facts > 3, "a statement over three chunks of one");
        for code in [
            "PRS-002", "PRS-003", "PRS-004", "PRS-005", "PRS-042", "PRS-044", "PRS-045", "PRS-046",
            "PRS-047", "PRS-071", "PRS-073", "PRS-075", "PRS-076", "PRS-077", "PRS-088", "PRS-089",
            "PRS-090", "PRS-108", "PRS-109", "INT-002", "INT-003", "API-003", "API-011", "API-019",
        ] {
            assert!(codes.contains(code), "no statement raised {code}");
        }
    }

    /// Statements with errors in several phases, each reporting today's.
    #[test]
    fn multi_error_statements_report_todays_error() {
        let long = "x".repeat(super::super::MAX_SYMBOL_LENGTH + 1);
        let corpus = [
            // A later lexer error beats an earlier structure error.
            "(transact [[:a :b 1)] !)".to_string(),
            format!("(transact [[:a :b] [{long} :b 1]])"),
            // A later structure error beats an earlier fact error.
            "(transact [[:a :b] [:c :d 1 }])".to_string(),
            "(transact [[:a :b 1 2 3]] [)".to_string(),
            "(transact [[1 :b 1]]) extra".to_string(),
            // Phase 3: extra elements before a facts element that is not a vector.
            "(transact 5 6)".to_string(),
            "(transact {:foo 1} 5 6)".to_string(),
            "(transact {:valid-from 1})".to_string(),
            "(transact [[:a :b]] [[:c :d 1]])".to_string(),
            "(retract [[:a :b]] 5)".to_string(),
            "(retract 5)".to_string(),
            "(retract)".to_string(),
            "(transact)".to_string(),
            // A later fact error beats an earlier materialisation error.
            "(transact [[1 :b 1] [:a :b]])".to_string(),
            "(retract [[:a b 1] [:a :b 1 2]])".to_string(),
            // A later materialisation error beats an earlier window error.
            "(transact [[:a :b 1 {:valid-to \"2001-01-01T00:00:00Z\"}] [:a b 1]])".to_string(),
            // Window and one-window-per-triple errors.
            "(transact [[:a :b 1 {:valid-to \"2001-01-01T00:00:00Z\"}] [:c :d 2 {:valid-to \"2002-01-01T00:00:00Z\"}]])".to_string(),
            "(transact [[:a :b 1 {:valid-from \"2001-01-01T00:00:00Z\"}] [:a :b 1 {:valid-from \"2002-01-01T00:00:00Z\"}]])".to_string(),
            // Too deep, inside a fact and inside an extra element.
            format!("(transact [[:a :b {}1{}]])", "[".repeat(120), "]".repeat(120)),
            format!("(transact [] {}{})", "(".repeat(120), ")".repeat(120)),
            // Comments and whitespace between facts.
            "(transact ; c\n [[:a :b 1] ; d\n [:c :d 2]] ; e\n)".to_string(),
            "(transact [[:a :b 1]".to_string(),
            "(transact [[:a :b 1]]".to_string(),
            "(transact".to_string(),
            "(retract".to_string(),
        ];
        for input in &corpus {
            let _ = check_same(input);
        }
    }

    #[test]
    fn large_statements_resume_after_the_last_kept_fact() {
        let input = "(transact [[:a :n 1] [:a :n 2] [:a :n 3] [:a :n 4] [:a :n 5]])";
        set_tx_chunk_facts(Some(2));
        let statement = read_statement(input, Kind::Transact, 1).unwrap();
        set_tx_chunk_facts(None);
        assert_eq!(statement.kept.len(), 2, "kept facts");
        let resume = statement.resume.expect("large");
        assert_eq!(
            input.get(resume..),
            Some(" [:a :n 3] [:a :n 4] [:a :n 5]])")
        );
        let mut rest = Pass2::new(input, resume, Kind::Transact, Defaults::default());
        let sizes: Vec<usize> = std::iter::from_fn(|| rest.next_chunk(2).unwrap())
            .map(|c| c.len())
            .collect();
        assert_eq!(sizes, vec![2, 1], "chunk sizes");

        set_tx_chunk_facts(Some(5));
        let exact = read_statement(input, Kind::Transact, 1).unwrap();
        set_tx_chunk_facts(None);
        assert!(exact.resume.is_none(), "five facts fit a chunk of five");
        assert_eq!(exact.facts().unwrap().len(), 5, "facts");
    }

    #[test]
    fn materialisation_and_window_errors_come_with_the_facts() {
        let code = |r: Result<Vec<Fact>>| MinigrafError::from(r.unwrap_err()).code();
        let statement =
            read_statement("(transact [[1 :b 1]])", Kind::Transact, 1).expect("phases 1-4 pass");
        assert_eq!(code(statement.facts()), "INT-002");
        let statement = read_statement(
            "(transact [[:a :b 1 {:valid-to \"2001-01-01T00:00:00Z\"}]])",
            Kind::Transact,
            crate::graph::types::tx_id_now(),
        )
        .expect("phases 1-4 pass");
        assert_eq!(code(statement.facts()), "API-019");
        let shape = read_statement("(transact [[:a :b]])", Kind::Transact, 1);
        assert_eq!(
            MinigrafError::from(shape.err().expect("phase 4")).code(),
            "PRS-046"
        );
    }

    #[test]
    fn parser_without_keep_checks_structure_and_holds_nothing() {
        let mut p = Parser::new(Lexer::new(
            "[1 (2 {:a [3]}) #uuid \"550e8400-e29b-41d4-a716-446655440000\"]",
        ));
        p.keep = false;
        assert!(matches!(p.parse_value().unwrap(), EdnValue::Vector(v) if v.is_empty()));
        let mut p = Parser::new(Lexer::new("[1 {:a} 2]"));
        p.keep = false;
        let e = p.parse_value().unwrap_err();
        assert_eq!(MinigrafError::from(e).code(), "PRS-003");
    }

    #[test]
    fn lexer_offsets_are_byte_offsets() {
        let mut lexer = Lexer::new("[\"é名\" :k]");
        assert_eq!(lexer.next(), Some(Token::LeftBracket));
        assert_eq!(lexer.next(), Some(Token::String("é名".to_string())));
        assert_eq!(lexer.pos, 8, "after the string");
        let mut resumed = Lexer::at("[\"é名\" :k]", 8);
        assert_eq!(resumed.next(), Some(Token::Keyword(":k".to_string())));
        assert_eq!(resumed.next(), Some(Token::RightBracket));
        assert_eq!(resumed.next(), None);
        assert!(resumed.finish().is_ok());
        let mut bad = Lexer::new("[ ! ]");
        assert_eq!(bad.next(), Some(Token::LeftBracket));
        assert_eq!(bad.next(), None, "stops at the error");
        assert_eq!(bad.next(), None, "and stays stopped");
        assert!(bad.finish().is_err());
    }
}
