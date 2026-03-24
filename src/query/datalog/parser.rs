use super::types::*;
use crate::temporal::parse_timestamp;
use uuid::Uuid;

/// Tokenizer for EDN syntax
#[derive(Debug, Clone, PartialEq)]
enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Keyword(String),
    Symbol(String),
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    TaggedLiteral(String), // e.g., "#uuid"
    Nil,
}

/// Tokenize EDN input
fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            // Whitespace
            ' ' | '\t' | '\n' | '\r' | ',' => {
                chars.next();
            }
            // Parens and brackets
            '(' => {
                tokens.push(Token::LeftParen);
                chars.next();
            }
            ')' => {
                tokens.push(Token::RightParen);
                chars.next();
            }
            '[' => {
                tokens.push(Token::LeftBracket);
                chars.next();
            }
            ']' => {
                tokens.push(Token::RightBracket);
                chars.next();
            }
            '{' => {
                tokens.push(Token::LeftBrace);
                chars.next();
            }
            '}' => {
                tokens.push(Token::RightBrace);
                chars.next();
            }
            // String literals
            '"' => {
                chars.next(); // consume opening quote
                let mut string = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch == '"' {
                        chars.next(); // consume closing quote
                        break;
                    } else if ch == '\\' {
                        chars.next();
                        // Handle escape sequences
                        if let Some(&escaped) = chars.peek() {
                            chars.next();
                            match escaped {
                                'n' => string.push('\n'),
                                't' => string.push('\t'),
                                'r' => string.push('\r'),
                                '"' => string.push('"'),
                                '\\' => string.push('\\'),
                                _ => string.push(escaped),
                            }
                        }
                    } else {
                        string.push(ch);
                        chars.next();
                    }
                }
                tokens.push(Token::String(string));
            }
            // Keywords (start with :)
            ':' => {
                chars.next();
                let mut keyword = String::from(":");
                while let Some(&ch) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '/' || ch == '-' || ch == '_' {
                        keyword.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Keyword(keyword));
            }
            // Tagged literals (start with #)
            '#' => {
                chars.next();
                let mut tag = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '-' {
                        tag.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::TaggedLiteral(tag));
            }
            // Numbers or symbols starting with -
            '-' => {
                let start_pos = chars.clone();
                chars.next();
                if let Some(&next_ch) = chars.peek() {
                    if next_ch.is_numeric() {
                        // It's a negative number
                        let mut num_str = String::from("-");
                        let (is_float, num) = parse_number(&mut chars, &mut num_str)?;
                        if is_float {
                            tokens.push(Token::Float(num.parse().unwrap()));
                        } else {
                            tokens.push(Token::Integer(num.parse().unwrap()));
                        }
                    } else {
                        // It's a symbol starting with -
                        chars = start_pos;
                        chars.next();
                        let symbol = parse_symbol(&mut chars, '-');
                        tokens.push(Token::Symbol(symbol));
                    }
                } else {
                    // Just a - symbol
                    tokens.push(Token::Symbol("-".to_string()));
                }
            }
            // Numbers
            '0'..='9' => {
                let mut num_str = String::new();
                let (is_float, num) = parse_number(&mut chars, &mut num_str)?;
                if is_float {
                    tokens.push(Token::Float(num.parse().unwrap()));
                } else {
                    tokens.push(Token::Integer(num.parse().unwrap()));
                }
            }
            // Symbols (including variables starting with ?)
            _ if ch.is_alphabetic() || ch == '?' || ch == '_' => {
                let mut symbol = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch.is_alphanumeric() || ch == '?' || ch == '_' || ch == '-' || ch == '/' {
                        symbol.push(ch);
                        chars.next();
                    } else {
                        break;
                    }
                }

                // Check for special symbols
                match symbol.as_str() {
                    "true" => tokens.push(Token::Boolean(true)),
                    "false" => tokens.push(Token::Boolean(false)),
                    "nil" => tokens.push(Token::Nil),
                    _ => tokens.push(Token::Symbol(symbol)),
                }
            }
            _ => {
                return Err(format!("Unexpected character: {}", ch));
            }
        }
    }

    Ok(tokens)
}

fn parse_number(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    num_str: &mut String,
) -> Result<(bool, String), String> {
    let mut is_float = false;

    while let Some(&ch) = chars.peek() {
        if ch.is_numeric() {
            num_str.push(ch);
            chars.next();
        } else if ch == '.' && !is_float {
            is_float = true;
            num_str.push(ch);
            chars.next();
        } else {
            break;
        }
    }

    Ok((is_float, num_str.clone()))
}

fn parse_symbol(chars: &mut std::iter::Peekable<std::str::Chars>, first: char) -> String {
    let mut symbol = String::from(first);
    while let Some(&ch) = chars.peek() {
        if ch.is_alphanumeric() || ch == '?' || ch == '_' || ch == '-' || ch == '/' {
            symbol.push(ch);
            chars.next();
        } else {
            break;
        }
    }
    symbol
}

/// Parser for EDN values
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let token = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(token)
        } else {
            None
        }
    }

    fn parse_map(&mut self) -> Result<EdnValue, String> {
        self.advance(); // consume '{'
        let mut pairs = Vec::new();

        while let Some(token) = self.peek() {
            if token == &Token::RightBrace {
                self.advance(); // consume '}'
                return Ok(EdnValue::Map(pairs));
            }
            let key = self.parse_value()?;
            let val = self.parse_value()?;
            pairs.push((key, val));
        }

        Err("Unterminated map: missing '}'".to_string())
    }

    fn parse_value(&mut self) -> Result<EdnValue, String> {
        match self.peek() {
            Some(Token::LeftParen) => self.parse_list(),
            Some(Token::LeftBracket) => self.parse_vector(),
            Some(Token::LeftBrace) => self.parse_map(),
            Some(Token::Keyword(_)) => {
                if let Some(Token::Keyword(k)) = self.advance() {
                    Ok(EdnValue::Keyword(k))
                } else {
                    unreachable!()
                }
            }
            Some(Token::Symbol(_)) => {
                if let Some(Token::Symbol(s)) = self.advance() {
                    Ok(EdnValue::Symbol(s))
                } else {
                    unreachable!()
                }
            }
            Some(Token::String(_)) => {
                if let Some(Token::String(s)) = self.advance() {
                    Ok(EdnValue::String(s))
                } else {
                    unreachable!()
                }
            }
            Some(Token::Integer(_)) => {
                if let Some(Token::Integer(i)) = self.advance() {
                    Ok(EdnValue::Integer(i))
                } else {
                    unreachable!()
                }
            }
            Some(Token::Float(_)) => {
                if let Some(Token::Float(f)) = self.advance() {
                    Ok(EdnValue::Float(f))
                } else {
                    unreachable!()
                }
            }
            Some(Token::Boolean(_)) => {
                if let Some(Token::Boolean(b)) = self.advance() {
                    Ok(EdnValue::Boolean(b))
                } else {
                    unreachable!()
                }
            }
            Some(Token::TaggedLiteral(_)) => {
                if let Some(Token::TaggedLiteral(tag)) = self.advance() {
                    if tag == "uuid" {
                        // Next token should be a string containing the UUID
                        if let Some(Token::String(uuid_str)) = self.advance() {
                            match Uuid::parse_str(&uuid_str) {
                                Ok(uuid) => Ok(EdnValue::Uuid(uuid)),
                                Err(_) => Err(format!("Invalid UUID: {}", uuid_str)),
                            }
                        } else {
                            Err("Expected UUID string after #uuid tag".to_string())
                        }
                    } else {
                        Err(format!("Unknown tagged literal: #{}", tag))
                    }
                } else {
                    unreachable!()
                }
            }
            Some(Token::Nil) => {
                self.advance();
                Ok(EdnValue::Nil)
            }
            Some(token) => Err(format!("Unexpected token: {:?}", token)),
            None => Err("Unexpected end of input".to_string()),
        }
    }

    fn parse_vector(&mut self) -> Result<EdnValue, String> {
        self.advance(); // consume [
        let mut elements = Vec::new();

        while let Some(token) = self.peek() {
            if token == &Token::RightBracket {
                self.advance(); // consume ]
                return Ok(EdnValue::Vector(elements));
            }
            elements.push(self.parse_value()?);
        }

        Err("Unclosed vector".to_string())
    }

    fn parse_list(&mut self) -> Result<EdnValue, String> {
        self.advance(); // consume (
        let mut elements = Vec::new();

        while let Some(token) = self.peek() {
            if token == &Token::RightParen {
                self.advance(); // consume )
                return Ok(EdnValue::List(elements));
            }
            elements.push(self.parse_value()?);
        }

        Err("Unclosed list".to_string())
    }
}

/// Parse EDN input into EdnValue
pub fn parse_edn(input: &str) -> Result<EdnValue, String> {
    let tokens = tokenize(input)?;
    let mut parser = Parser::new(tokens);
    parser.parse_value()
}

/// Parse a Datalog command from EDN
pub fn parse_datalog_command(input: &str) -> Result<DatalogCommand, String> {
    let edn = parse_edn(input)?;

    match edn {
        EdnValue::List(elements) if !elements.is_empty() => {
            // Command is a symbol (e.g., "query", "transact")
            let command = match &elements[0] {
                EdnValue::Symbol(s) => s.as_str(),
                EdnValue::Keyword(k) => k.as_str(),
                _ => return Err("Expected command symbol".to_string()),
            };

            match command {
                "query" => parse_query(&elements[1..]),
                "transact" => parse_transact(&elements[1..]),
                "retract" => parse_retract(&elements[1..]),
                "rule" => parse_rule(&elements[1..]),
                _ => Err(format!("Unknown command: {}", command)),
            }
        }
        _ => Err("Expected a list starting with a command symbol".to_string()),
    }
}

fn parse_query(elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // Parse (query [:find ?x ?y :as-of N :valid-at "ts" :where [patterns...]])
    if elements.is_empty() {
        return Err("Query requires a map argument".to_string());
    }

    let query_vector = elements[0]
        .as_vector()
        .ok_or("Query argument must be a vector")?;

    let mut find_vars = Vec::new();
    let mut where_clauses = Vec::new();
    let mut current_clause: Option<&str> = None;
    let mut query_as_of: Option<AsOf> = None;
    let mut query_valid_at: Option<ValidAt> = None;

    let mut i = 0;
    while i < query_vector.len() {
        if let Some(keyword) = query_vector[i].as_keyword() {
            match keyword {
                ":as-of" => {
                    // Next element is the value
                    i += 1;
                    if i >= query_vector.len() {
                        return Err(":as-of requires a value".to_string());
                    }
                    let as_of = match &query_vector[i] {
                        EdnValue::Integer(n) if *n >= 0 => AsOf::Counter(*n as u64),
                        EdnValue::Integer(n) => {
                            return Err(format!(":as-of counter must be non-negative, got {}", n));
                        }
                        EdnValue::String(s) => {
                            let ts = parse_timestamp(s).map_err(|e| e.to_string())?;
                            AsOf::Timestamp(ts)
                        }
                        other => {
                            return Err(format!(
                                ":as-of must be an integer (counter) or ISO 8601 string, got {:?}",
                                other
                            ));
                        }
                    };
                    query_as_of = Some(as_of);
                    i += 1;
                    continue;
                }
                ":valid-at" => {
                    i += 1;
                    if i >= query_vector.len() {
                        return Err(":valid-at requires a value".to_string());
                    }
                    let valid_at = match &query_vector[i] {
                        EdnValue::String(s) => {
                            let ts = parse_timestamp(s).map_err(|e| e.to_string())?;
                            ValidAt::Timestamp(ts)
                        }
                        EdnValue::Keyword(k) if k == ":any-valid-time" => ValidAt::AnyValidTime,
                        other => {
                            return Err(format!(
                                ":valid-at must be an ISO 8601 string or :any-valid-time, got {:?}",
                                other
                            ));
                        }
                    };
                    query_valid_at = Some(valid_at);
                    i += 1;
                    continue;
                }
                _ => {
                    current_clause = Some(keyword);
                    i += 1;
                    continue;
                }
            }
        }

        match current_clause {
            Some(":find") => {
                // Collect find variables
                if let Some(var) = query_vector[i].as_variable() {
                    find_vars.push(var.to_string());
                } else {
                    return Err(format!(
                        "Expected variable in :find clause, got {:?}",
                        query_vector[i]
                    ));
                }
            }
            Some(":where") => {
                // Parse both patterns (vectors) and rule invocations (lists)
                if let Some(pattern_vec) = query_vector[i].as_vector() {
                    // This is a pattern: [?e :attr ?v]
                    let pattern = Pattern::from_edn(pattern_vec)?;
                    where_clauses.push(WhereClause::Pattern(pattern));
                } else if let Some(rule_list) = query_vector[i].as_list() {
                    let clause = parse_list_as_where_clause(rule_list, true)?;
                    where_clauses.push(clause);
                } else {
                    return Err(format!(
                        "Expected pattern vector or rule invocation in :where clause, got {:?}",
                        query_vector[i]
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "Unexpected element in query: {:?}",
                    query_vector[i]
                ));
            }
        }

        i += 1;
    }

    // Safety check: all variables in (not ...) must be bound by outer clauses
    let outer_bound: std::collections::HashSet<String> = where_clauses
        .iter()
        .flat_map(outer_vars_from_clause)
        .collect();
    check_not_safety(&where_clauses, &outer_bound)?;

    let mut query = DatalogQuery::new(find_vars, where_clauses);
    query.as_of = query_as_of;
    query.valid_at = query_valid_at;
    Ok(DatalogCommand::Query(query))
}

fn parse_transact(elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // Parse (transact [facts]) or (transact {opts} [facts])
    if elements.is_empty() {
        return Err("Transact requires a vector of facts".to_string());
    }

    // Check if the first element is a map (transaction-level options)
    let (tx_valid_from, tx_valid_to, facts_element) = if elements[0].is_map() {
        let map = elements[0].as_map().unwrap();
        let (from, to) = parse_valid_time_map(map)?;
        if elements.len() < 2 {
            return Err("Transact with options requires a facts vector after the map".to_string());
        }
        (from, to, &elements[1])
    } else {
        (None, None, &elements[0])
    };

    let facts_vector = facts_element
        .as_vector()
        .ok_or("Transact argument must be a vector of facts")?;

    let mut patterns = Vec::new();
    for fact in facts_vector {
        let fact_vec = fact
            .as_vector()
            .ok_or("Each fact must be a vector [e a v] or [e a v {opts}]")?;

        // A fact is [e a v] or [e a v {opts}]
        if fact_vec.len() < 3 {
            return Err(format!(
                "Fact must have at least 3 elements (E A V), got {}",
                fact_vec.len()
            ));
        }

        let entity = fact_vec[0].clone();
        let attribute = fact_vec[1].clone();
        let value = fact_vec[2].clone();

        // Check for optional per-fact map at position 3
        let (fact_valid_from, fact_valid_to) = if fact_vec.len() >= 4 {
            match &fact_vec[3] {
                EdnValue::Map(pairs) => parse_valid_time_map(pairs)?,
                other => {
                    return Err(format!(
                        "Optional 4th element of a fact must be a map {{:valid-from ... :valid-to ...}}, got {:?}",
                        other
                    ));
                }
            }
        } else {
            (None, None)
        };

        // Per-fact overrides take precedence over transaction-level defaults
        let effective_from = fact_valid_from.or(tx_valid_from);
        let effective_to = fact_valid_to.or(tx_valid_to);

        patterns.push(Pattern::with_valid_time(
            entity,
            attribute,
            value,
            effective_from,
            effective_to,
        ));
    }

    let mut tx = Transaction::new(patterns);
    tx.valid_from = tx_valid_from;
    tx.valid_to = tx_valid_to;
    Ok(DatalogCommand::Transact(tx))
}

/// Parse a valid-time map `{:valid-from "ts" :valid-to "ts"}` into millisecond timestamps.
/// Both keys are optional.
fn parse_valid_time_map(
    pairs: &[(EdnValue, EdnValue)],
) -> Result<(Option<i64>, Option<i64>), String> {
    let mut valid_from = None;
    let mut valid_to = None;

    for (key, val) in pairs {
        match key.as_keyword() {
            Some(":valid-from") => {
                let s = match val {
                    EdnValue::String(s) => s,
                    other => {
                        return Err(format!(
                            ":valid-from must be an ISO 8601 string, got {:?}",
                            other
                        ));
                    }
                };
                valid_from = Some(parse_timestamp(s).map_err(|e| e.to_string())?);
            }
            Some(":valid-to") => {
                let s = match val {
                    EdnValue::String(s) => s,
                    other => {
                        return Err(format!(
                            ":valid-to must be an ISO 8601 string, got {:?}",
                            other
                        ));
                    }
                };
                valid_to = Some(parse_timestamp(s).map_err(|e| e.to_string())?);
            }
            _ => {
                return Err(format!(
                    "Unknown key in valid-time map: {:?}; expected :valid-from or :valid-to",
                    key
                ));
            }
        }
    }

    Ok((valid_from, valid_to))
}

fn parse_retract(elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // Same structure as transact
    if elements.is_empty() {
        return Err("Retract requires a vector of facts".to_string());
    }

    let facts_vector = elements[0]
        .as_vector()
        .ok_or("Retract argument must be a vector")?;

    let mut patterns = Vec::new();
    for fact in facts_vector {
        let fact_vec = fact
            .as_vector()
            .ok_or("Each fact must be a vector [e a v]")?;
        patterns.push(Pattern::from_edn(fact_vec)?);
    }

    Ok(DatalogCommand::Retract(Transaction::new(patterns)))
}

/// Parse a list item (EDN List) appearing in a :where clause or rule body.
/// Returns Err if the list is empty, has an unknown form, or contains nested `not`.
fn parse_list_as_where_clause(
    list: &[EdnValue],
    allow_not: bool,
) -> Result<WhereClause, String> {
    if list.is_empty() {
        return Err("Empty list in :where clause".to_string());
    }
    match &list[0] {
        EdnValue::Symbol(s) if s == "not" => {
            if !allow_not {
                return Err(
                    "(not ...) cannot appear inside another (not ...)".to_string(),
                );
            }
            if list.len() < 2 {
                return Err("(not) requires at least one clause".to_string());
            }
            let mut inner = Vec::new();
            for item in &list[1..] {
                if let Some(vec) = item.as_vector() {
                    let pattern = Pattern::from_edn(vec)?;
                    inner.push(WhereClause::Pattern(pattern));
                } else if let Some(inner_list) = item.as_list() {
                    // Recurse with allow_not=false to reject nested not
                    let clause = parse_list_as_where_clause(inner_list, false)?;
                    inner.push(clause);
                } else {
                    return Err(format!(
                        "expected pattern or rule invocation inside (not), got {:?}",
                        item
                    ));
                }
            }
            Ok(WhereClause::Not(inner))
        }
        EdnValue::Symbol(predicate) => {
            let args = list[1..].to_vec();
            Ok(WhereClause::RuleInvocation {
                predicate: predicate.clone(),
                args,
            })
        }
        _ => Err(format!(
            "Rule invocation must start with predicate name (symbol), got {:?}",
            list[0]
        )),
    }
}

/// Collect all variable names that appear in a where clause (non-recursively into Not).
fn outer_vars_from_clause(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Pattern(p) => {
            let mut vars = Vec::new();
            for v in [&p.entity, &p.attribute, &p.value] {
                if let Some(name) = v.as_variable()
                    && !name.starts_with("?_")
                {
                    vars.push(name.to_string());
                }
            }
            vars
        }
        WhereClause::RuleInvocation { args, .. } => args
            .iter()
            .filter_map(|a| {
                a.as_variable().and_then(|s| {
                    if !s.starts_with("?_") {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
            })
            .collect(),
        WhereClause::Not(_) => vec![], // not counted as "outer"
    }
}

/// Collect all variable names that appear inside a Not clause.
fn vars_in_not(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Not(inner) => inner
            .iter()
            .flat_map(outer_vars_from_clause)
            .collect(),
        _ => vec![],
    }
}

/// Validate safety: every variable in a (not ...) body must be bound by an outer clause.
fn check_not_safety(
    clauses: &[WhereClause],
    outer_bound: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for clause in clauses {
        if let WhereClause::Not(_) = clause {
            for var in vars_in_not(clause) {
                if !outer_bound.contains(&var) {
                    return Err(format!(
                        "variable {} in (not ...) is not bound by any outer clause",
                        var
                    ));
                }
            }
        }
    }
    Ok(())
}

fn parse_rule(elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // Rule syntax: (rule [(predicate ?args) [pattern1] [pattern2] ...])
    // elements[0] = Vector with head (list) + body (patterns/rule calls)

    if elements.is_empty() {
        return Err("Rule must have a body".to_string());
    }

    // Parse the rule body (single vector containing head + body patterns)
    let body_vec = elements[0]
        .as_vector()
        .ok_or("Rule body must be a vector")?;

    if body_vec.is_empty() {
        return Err("Rule body cannot be empty".to_string());
    }

    // First element is the head (must be a list)
    let head_list = body_vec[0]
        .as_list()
        .ok_or("Rule head must be a list: (predicate ?args)")?;

    if head_list.is_empty() {
        return Err("Rule head cannot be empty".to_string());
    }

    // Verify head starts with a symbol (predicate name)
    match &head_list[0] {
        EdnValue::Symbol(_) => {}
        _ => return Err("Rule head must start with a symbol (predicate name)".to_string()),
    }

    // Rest of body_vec are patterns, rule invocations, or (not ...) clauses
    let mut body_clauses: Vec<WhereClause> = Vec::new();
    for item in &body_vec[1..] {
        if let Some(vec) = item.as_vector() {
            let pattern = Pattern::from_edn(vec)?;
            body_clauses.push(WhereClause::Pattern(pattern));
        } else if let Some(list) = item.as_list() {
            let clause = parse_list_as_where_clause(list, true)?;
            body_clauses.push(clause);
        } else {
            return Err(format!(
                "Rule body clause must be a vector (pattern) or list (rule invocation / not), got {:?}",
                item
            ));
        }
    }

    if body_clauses.is_empty() {
        return Err("Rule must have at least one pattern or rule invocation in body".to_string());
    }

    // Safety check: variables in (not ...) must be bound by the rule head or outer body clauses
    let mut outer_bound: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Head args count as binding sites
    for v in &head_list[1..] {
        if let Some(name) = v.as_variable() {
            outer_bound.insert(name.to_string());
        }
    }
    // Non-not body clauses
    for clause in &body_clauses {
        for var in outer_vars_from_clause(clause) {
            outer_bound.insert(var);
        }
    }
    check_not_safety(&body_clauses, &outer_bound)?;

    Ok(DatalogCommand::Rule(Rule {
        head: head_list.clone(),
        body: body_clauses,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_basic() {
        let input = "(query [:find ?x])";
        let tokens = tokenize(input).unwrap();
        assert_eq!(tokens[0], Token::LeftParen);
        assert_eq!(tokens[1], Token::Symbol("query".to_string()));
        assert_eq!(tokens[2], Token::LeftBracket);
        assert_eq!(tokens[3], Token::Keyword(":find".to_string()));
        assert_eq!(tokens[4], Token::Symbol("?x".to_string()));
        assert_eq!(tokens[5], Token::RightBracket);
        assert_eq!(tokens[6], Token::RightParen);
    }

    #[test]
    fn test_tokenize_numbers() {
        let tokens = tokenize("42 4.5 -5 -2.5").unwrap();
        assert_eq!(tokens[0], Token::Integer(42));
        assert_eq!(tokens[1], Token::Float(4.5));
        assert_eq!(tokens[2], Token::Integer(-5));
        assert_eq!(tokens[3], Token::Float(-2.5));
    }

    #[test]
    fn test_tokenize_strings() {
        let tokens = tokenize(r#""hello" "world\"test""#).unwrap();
        assert_eq!(tokens[0], Token::String("hello".to_string()));
        assert_eq!(tokens[1], Token::String("world\"test".to_string()));
    }

    #[test]
    fn test_tokenize_booleans() {
        let tokens = tokenize("true false nil").unwrap();
        assert_eq!(tokens[0], Token::Boolean(true));
        assert_eq!(tokens[1], Token::Boolean(false));
        assert_eq!(tokens[2], Token::Nil);
    }

    #[test]
    fn test_parse_edn_vector() {
        let result = parse_edn("[1 2 3]").unwrap();
        match result {
            EdnValue::Vector(v) => {
                assert_eq!(v.len(), 3);
                assert_eq!(v[0], EdnValue::Integer(1));
            }
            _ => panic!("Expected vector"),
        }
    }

    #[test]
    fn test_parse_edn_list() {
        let result = parse_edn("(query :find ?x)").unwrap();
        match result {
            EdnValue::List(l) => {
                assert_eq!(l.len(), 3);
                assert_eq!(l[0], EdnValue::Symbol("query".to_string()));
                assert_eq!(l[1], EdnValue::Keyword(":find".to_string()));
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_parse_simple_query() {
        let input = r#"(query [:find ?name :where [?e :person/name ?name]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find, vec!["?name"]);
                let patterns = q.get_patterns();
                assert_eq!(patterns.len(), 1);
                assert_eq!(
                    patterns[0].attribute,
                    EdnValue::Keyword(":person/name".to_string())
                );
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn test_parse_transact() {
        let input = r#"(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Transact(tx) => {
                assert_eq!(tx.facts.len(), 2);
                assert_eq!(tx.facts[0].entity, EdnValue::Keyword(":alice".to_string()));
                assert_eq!(tx.facts[0].value, EdnValue::String("Alice".to_string()));
                assert_eq!(tx.facts[1].value, EdnValue::Integer(30));
            }
            _ => panic!("Expected Transact command"),
        }
    }

    #[test]
    fn test_parse_uuid() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let input = format!(r#"#uuid "{}""#, uuid_str);
        let result = parse_edn(&input).unwrap();

        match result {
            EdnValue::Uuid(uuid) => {
                assert_eq!(uuid.to_string(), uuid_str);
            }
            _ => panic!("Expected UUID"),
        }
    }

    #[test]
    fn test_parse_complex_query() {
        let input =
            r#"(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find, vec!["?name", "?age"]);
                assert_eq!(q.get_patterns().len(), 2);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn test_parse_retract() {
        let input = r#"(retract [[:alice :person/age 30]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Retract(tx) => {
                assert_eq!(tx.facts.len(), 1);
            }
            _ => panic!("Expected Retract command"),
        }
    }

    #[test]
    fn test_parse_simple_rule() {
        let input = r#"(rule [(reachable ?x ?y) [?x :connected ?y]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Rule(rule) => {
                // Verify head: (reachable ?x ?y)
                assert_eq!(rule.head.len(), 3);
                assert_eq!(rule.head[0], EdnValue::Symbol("reachable".to_string()));
                assert_eq!(rule.head[1], EdnValue::Symbol("?x".to_string()));
                assert_eq!(rule.head[2], EdnValue::Symbol("?y".to_string()));

                // Verify body has one pattern
                assert_eq!(rule.body.len(), 1);
            }
            _ => panic!("Expected Rule command"),
        }
    }

    #[test]
    fn test_parse_recursive_rule() {
        let input = r#"(rule [(reachable ?x ?y) [?x :connected ?z] (reachable ?z ?y)])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Rule(rule) => {
                // Verify head
                assert_eq!(rule.head.len(), 3);
                assert_eq!(rule.head[0], EdnValue::Symbol("reachable".to_string()));

                // Verify body has two clauses: pattern + rule invocation
                assert_eq!(rule.body.len(), 2);

                // First clause should be a Pattern
                assert!(matches!(rule.body[0], WhereClause::Pattern(_)));

                // Second clause should be a RuleInvocation
                assert!(matches!(rule.body[1], WhereClause::RuleInvocation { .. }));
            }
            _ => panic!("Expected Rule command"),
        }
    }

    #[test]
    fn test_parse_rule_with_multiple_patterns() {
        let input = r#"(rule [(ancestor ?a ?d) [?a :parent ?p] [?p :parent ?d]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Rule(rule) => {
                assert_eq!(rule.head[0], EdnValue::Symbol("ancestor".to_string()));
                // Two patterns in body
                assert_eq!(rule.body.len(), 2);
                assert!(matches!(rule.body[0], WhereClause::Pattern(_)));
                assert!(matches!(rule.body[1], WhereClause::Pattern(_)));
            }
            _ => panic!("Expected Rule command"),
        }
    }

    #[test]
    fn test_parse_rule_empty_body_fails() {
        let input = r#"(rule [(reachable ?x ?y)])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rule_invalid_head_fails() {
        // Head must be a list, not a vector
        let input = r#"(rule [[reachable ?x ?y] [?x :connected ?y]])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_query_with_rule_invocation() {
        let input = r#"(query [:find ?to :where (reachable :alice ?to)])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find, vec!["?to"]);
                assert_eq!(q.where_clauses.len(), 1);

                // Check it's a rule invocation
                assert!(q.uses_rules());

                let rule_invocations = q.get_rule_invocations();
                assert_eq!(rule_invocations.len(), 1);
                assert_eq!(rule_invocations[0].0, "reachable");
                assert_eq!(rule_invocations[0].1.len(), 2);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn test_parse_query_mixed_pattern_and_rule() {
        let input = r#"(query [:find ?name :where (reachable :alice ?person) [?person :person/name ?name]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find, vec!["?name"]);
                assert_eq!(q.where_clauses.len(), 2);

                // Should have both rule and pattern
                assert!(q.uses_rules());
                assert_eq!(q.get_rule_invocations().len(), 1);
                assert_eq!(q.get_patterns().len(), 1);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn test_parse_query_multiple_rule_invocations() {
        let input = r#"(query [:find ?z :where (reachable :alice ?x) (reachable ?x ?z)])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find, vec!["?z"]);
                assert_eq!(q.where_clauses.len(), 2);
                assert_eq!(q.get_rule_invocations().len(), 2);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn test_parse_query_pattern_only_no_rules() {
        let input = r#"(query [:find ?name :where [?e :person/name ?name]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert!(!q.uses_rules());
                assert_eq!(q.get_rule_invocations().len(), 0);
                assert_eq!(q.get_patterns().len(), 1);
            }
            _ => panic!("Expected Query command"),
        }
    }

    #[test]
    fn test_parse_rule_invocation_empty_fails() {
        let input = r#"(query [:find ?x :where ()])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_edn_map() {
        let result = parse_edn(r#"{:valid-from "2023-01-01" :valid-to "2023-06-30"}"#);
        let map = match result.unwrap() {
            EdnValue::Map(pairs) => pairs,
            _ => panic!("expected map"),
        };
        assert_eq!(map.len(), 2);
        assert_eq!(map[0].0, EdnValue::Keyword(":valid-from".to_string()));
        assert_eq!(map[0].1, EdnValue::String("2023-01-01".to_string()));
    }

    #[test]
    fn test_parse_empty_map() {
        let result = parse_edn("{}");
        assert!(matches!(result.unwrap(), EdnValue::Map(pairs) if pairs.is_empty()));
    }

    // --- Task 8: Temporal parsing tests ---

    #[test]
    fn test_parse_as_of_counter() {
        let cmd =
            parse_datalog_command("(query [:find ?name :as-of 50 :where [?e :person/name ?name]])")
                .unwrap();
        let query = match cmd {
            DatalogCommand::Query(q) => q,
            _ => panic!("expected Query"),
        };
        assert_eq!(query.as_of, Some(AsOf::Counter(50)));
    }

    #[test]
    fn test_parse_as_of_timestamp() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?name :as-of "2024-01-15T10:00:00Z" :where [?e :person/name ?name]])"#,
        )
        .unwrap();
        let query = match cmd {
            DatalogCommand::Query(q) => q,
            _ => panic!("expected Query"),
        };
        assert!(matches!(query.as_of, Some(AsOf::Timestamp(_))));
    }

    #[test]
    fn test_parse_as_of_negative_counter_is_error() {
        let result =
            parse_datalog_command("(query [:find ?n :as-of -1 :where [?e :person/name ?n]])");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("non-negative"));
    }

    #[test]
    fn test_parse_valid_at_timestamp() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?s :valid-at "2023-06-01" :where [:alice :employment/status ?s]])"#,
        )
        .unwrap();
        let query = match cmd {
            DatalogCommand::Query(q) => q,
            _ => panic!("expected Query"),
        };
        assert!(matches!(query.valid_at, Some(ValidAt::Timestamp(_))));
    }

    #[test]
    fn test_parse_valid_at_any() {
        let cmd = parse_datalog_command(
            "(query [:find ?name :valid-at :any-valid-time :where [?e :person/name ?name]])",
        )
        .unwrap();
        let query = match cmd {
            DatalogCommand::Query(q) => q,
            _ => panic!("expected Query"),
        };
        assert_eq!(query.valid_at, Some(ValidAt::AnyValidTime));
    }

    #[test]
    fn test_parse_transact_with_tx_level_valid_time() {
        let cmd = parse_datalog_command(
            r#"(transact {:valid-from "2023-01-01" :valid-to "2023-06-30"} [[:alice :employment/status :active]])"#,
        )
        .unwrap();
        let tx = match cmd {
            DatalogCommand::Transact(t) => t,
            _ => panic!("expected Transact"),
        };
        assert!(tx.valid_from.is_some());
        assert!(tx.valid_to.is_some());
    }

    #[test]
    fn test_parse_transact_with_per_fact_valid_time() {
        let cmd = parse_datalog_command(
            r#"(transact {:valid-from "2023-01-01"} [[:alice :employment/status :active {:valid-to "2023-06-30"}] [:alice :person/name "Alice"]])"#,
        )
        .unwrap();
        let tx = match cmd {
            DatalogCommand::Transact(t) => t,
            _ => panic!("expected Transact"),
        };
        assert_eq!(tx.facts.len(), 2);
        // First fact has per-fact valid_to + tx-level valid_from
        assert!(tx.facts[0].valid_from.is_some());
        assert!(tx.facts[0].valid_to.is_some());
        // Second fact inherits tx-level valid_from only
        assert!(tx.facts[1].valid_from.is_some());
        assert!(tx.facts[1].valid_to.is_none());
    }

    #[test]
    fn test_parse_reject_timezone_offset_in_as_of() {
        let result = parse_datalog_command(
            r#"(query [:find ?n :as-of "2024-01-15T10:00:00+05:30" :where [?e :person/name ?n]])"#,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("timezone offsets are not supported"),
            "error was: {}",
            msg
        );
    }

    #[test]
    fn test_parse_transact_no_map_backward_compatible() {
        // Old syntax without map should still work
        let cmd = parse_datalog_command(
            r#"(transact [[:alice :person/name "Alice"] [:alice :person/age 30]])"#,
        )
        .unwrap();
        let tx = match cmd {
            DatalogCommand::Transact(t) => t,
            _ => panic!("expected Transact"),
        };
        assert_eq!(tx.facts.len(), 2);
        assert!(tx.valid_from.is_none());
        assert!(tx.valid_to.is_none());
    }

    #[test]
    fn test_parse_as_of_and_valid_at_together() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?status :as-of 100 :valid-at "2023-06-01" :where [:alice :employment/status ?status]])"#,
        )
        .unwrap();
        let query = match cmd {
            DatalogCommand::Query(q) => q,
            _ => panic!("expected Query"),
        };
        assert!(matches!(query.as_of, Some(AsOf::Counter(100))));
        assert!(matches!(query.valid_at, Some(ValidAt::Timestamp(_))));
    }

    #[test]
    fn test_parse_not_with_pattern_in_query() {
        let input = r#"(query [:find ?person :where [?person :name ?n] (not [?person :banned true])])"#;
        let cmd = parse_datalog_command(input).unwrap();
        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.where_clauses.len(), 2);
                assert!(matches!(q.where_clauses[0], WhereClause::Pattern(_)));
                match &q.where_clauses[1] {
                    WhereClause::Not(inner) => {
                        assert_eq!(inner.len(), 1);
                        assert!(matches!(inner[0], WhereClause::Pattern(_)));
                    }
                    other => panic!("Expected Not, got {:?}", other),
                }
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_parse_not_with_rule_invocation_in_query() {
        let input = r#"(query [:find ?person :where [?person :name ?n] (not (blocked ?person))])"#;
        let cmd = parse_datalog_command(input).unwrap();
        match cmd {
            DatalogCommand::Query(q) => {
                match &q.where_clauses[1] {
                    WhereClause::Not(inner) => {
                        assert!(matches!(inner[0], WhereClause::RuleInvocation { .. }));
                    }
                    other => panic!("Expected Not, got {:?}", other),
                }
            }
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_parse_not_in_rule_body() {
        let input = r#"(rule [(eligible ?x) [?x :applied true] (not (rejected ?x))])"#;
        let cmd = parse_datalog_command(input).unwrap();
        match cmd {
            DatalogCommand::Rule(rule) => {
                assert_eq!(rule.body.len(), 2);
                assert!(matches!(rule.body[0], WhereClause::Pattern(_)));
                assert!(matches!(rule.body[1], WhereClause::Not(_)));
            }
            _ => panic!("Expected Rule"),
        }
    }

    #[test]
    fn test_parse_not_empty_body_is_error() {
        let input = r#"(query [:find ?x :where [?x :a ?v] (not)])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("requires at least one clause"), "got: {msg}");
    }

    #[test]
    fn test_parse_nested_not_is_error() {
        let input = r#"(query [:find ?x :where [?x :a ?v] (not (not [?x :banned true]))])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("cannot appear inside another"), "got: {msg}");
    }

    #[test]
    fn test_parse_not_unbound_variable_is_error() {
        // ?y is only in the not body, not in any outer clause
        let input = r#"(query [:find ?x :where [?x :a ?v] (not [?y :banned true])])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("not bound"), "got: {msg}");
    }

    #[test]
    fn test_parse_not_unbound_variable_in_rule_body_is_error() {
        // ?y only in not, not in head or non-not body
        let input = r#"(rule [(eligible ?x) [?x :applied true] (not [?y :banned true])])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("not bound"), "got: {msg}");
    }

    #[test]
    fn test_parse_not_with_multiple_clauses() {
        // (not [?person :role :admin] [?person :active false])
        let input = r#"(query [:find ?person :where [?person :name ?n] (not [?person :role :admin] [?person :active false])])"#;
        let cmd = parse_datalog_command(input).unwrap();
        match cmd {
            DatalogCommand::Query(q) => {
                match &q.where_clauses[1] {
                    WhereClause::Not(inner) => {
                        assert_eq!(inner.len(), 2);
                        assert!(matches!(inner[0], WhereClause::Pattern(_)));
                        assert!(matches!(inner[1], WhereClause::Pattern(_)));
                    }
                    other => panic!("Expected Not with 2 clauses, got {:?}", other),
                }
            }
            _ => panic!("Expected Query"),
        }
    }
}
