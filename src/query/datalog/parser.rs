use super::types::*;
use crate::graph::types::Value;
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
            // Operator symbols: <, <=, >, >=, =, !=, +, *, /
            '<' | '>' | '=' | '+' | '*' | '/' => {
                chars.next();
                let mut sym = String::from(ch);
                // Consume a trailing '=' to form <=, >=
                if (ch == '<' || ch == '>') && chars.peek() == Some(&'=') {
                    sym.push('=');
                    chars.next();
                }
                tokens.push(Token::Symbol(sym));
            }
            '!' => {
                chars.next();
                if chars.peek() == Some(&'=') {
                    chars.next();
                    tokens.push(Token::Symbol("!=".to_string()));
                } else {
                    return Err("Unexpected character: !".to_string());
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
                                Err(_) => Err("Invalid UUID".to_string()),
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

/// Parse an aggregate expression list: (func-name ?var)
/// e.g., [Symbol("count"), Symbol("?e")] → FindSpec::Aggregate { Count, "?e" }
fn parse_aggregate(elems: &[EdnValue]) -> Result<FindSpec, String> {
    if elems.len() != 2 {
        return Err(format!(
            "Aggregate expression must have exactly 2 elements (func ?var), got {}",
            elems.len()
        ));
    }
    let func = match &elems[0] {
        EdnValue::Symbol(s) => match s.as_str() {
            "count" => AggFunc::Count,
            "count-distinct" => AggFunc::CountDistinct,
            "sum" => AggFunc::Sum,
            "sum-distinct" => AggFunc::SumDistinct,
            "min" => AggFunc::Min,
            "max" => AggFunc::Max,
            other => return Err(format!("Unknown aggregate function: '{}'", other)),
        },
        other => {
            return Err(format!(
                "Aggregate function name must be a symbol, got {:?}",
                other
            ));
        }
    };
    let var = match &elems[1] {
        EdnValue::Symbol(s) if s.starts_with('?') => s.clone(),
        _ => return Err("Aggregate argument must be a variable (starting with ?)".to_string()),
    };
    Ok(FindSpec::Aggregate { func, var })
}

fn parse_query(elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // Parse (query [:find ?x ?y :as-of N :valid-at "ts" :where [patterns...]])
    if elements.is_empty() {
        return Err("Query requires a map argument".to_string());
    }

    let query_vector = elements[0]
        .as_vector()
        .ok_or("Query argument must be a vector")?;

    let mut find_specs: Vec<FindSpec> = Vec::new();
    let mut with_vars: Vec<String> = Vec::new();
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
                ":any-valid-time" => {
                    // Shorthand for `:valid-at :any-valid-time`; disables automatic
                    // valid-time filtering so pseudo-attribute patterns are accessible.
                    query_valid_at = Some(ValidAt::AnyValidTime);
                    i += 1;
                    continue;
                }
                ":with" => {
                    // Collect ?-prefixed symbols until the next keyword or end of vector
                    i += 1;
                    while i < query_vector.len() {
                        match &query_vector[i] {
                            EdnValue::Symbol(s) if s.starts_with('?') => {
                                with_vars.push(s.clone());
                                i += 1;
                            }
                            EdnValue::Keyword(_) => break,
                            other => {
                                return Err(format!(
                                    "':with' clause accepts only variables, got {:?}",
                                    other
                                ));
                            }
                        }
                    }
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
            Some(":find") => match &query_vector[i] {
                EdnValue::Symbol(s) if s.starts_with('?') => {
                    find_specs.push(FindSpec::Variable(s.clone()));
                }
                EdnValue::List(elems) => {
                    find_specs.push(parse_aggregate(elems)?);
                }
                other => {
                    return Err(format!(
                        "Expected variable or aggregate expression in :find clause, got {:?}",
                        other
                    ));
                }
            },
            Some(":where") => {
                // Parse both patterns (vectors) and rule invocations (lists)
                if let Some(pattern_vec) = query_vector[i].as_vector() {
                    // Expr clause: [(list-expr) ?out?] — element 0 is a List
                    if matches!(pattern_vec.first(), Some(EdnValue::List(_))) {
                        let clause = parse_expr_clause(pattern_vec)?;
                        where_clauses.push(clause);
                    } else {
                        let pattern = parse_query_pattern(pattern_vec)?;
                        where_clauses.push(WhereClause::Pattern(pattern));
                    }
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

    // Safety check: all variables in (not ...) and (not-join ...) must be bound by outer clauses
    let outer_bound: std::collections::HashSet<String> = where_clauses
        .iter()
        .flat_map(outer_vars_from_clause)
        .collect();
    check_not_safety(&where_clauses, &outer_bound)?;
    check_not_join_safety(&where_clauses, &outer_bound)?;
    check_expr_safety(&where_clauses)?;

    // Validate aggregate and :with vars are bound in :where
    for spec in &find_specs {
        if let FindSpec::Aggregate { var, .. } = spec
            && !outer_bound.contains(var)
        {
            return Err(format!("Aggregate variable {} not bound in :where", var));
        }
    }
    for var in &with_vars {
        if !outer_bound.contains(var) {
            return Err(format!("':with' variable {} not bound in :where", var));
        }
    }
    // :with without any aggregate is an error
    if !with_vars.is_empty()
        && !find_specs
            .iter()
            .any(|s| matches!(s, FindSpec::Aggregate { .. }))
    {
        return Err("':with' clause requires at least one aggregate in :find".to_string());
    }

    let mut query = DatalogQuery::new(find_specs, where_clauses);
    query.as_of = query_as_of;
    query.valid_at = query_valid_at;
    query.with_vars = with_vars;
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

/// Convert a single EDN token to an Expr leaf, or recurse for a nested list.
fn parse_expr_arg(edn: &EdnValue) -> Result<Expr, String> {
    match edn {
        EdnValue::Symbol(s) if s.starts_with('?') => Ok(Expr::Var(s.clone())),
        EdnValue::Integer(n) => Ok(Expr::Lit(Value::Integer(*n))),
        EdnValue::Float(f) => Ok(Expr::Lit(Value::Float(*f))),
        EdnValue::String(s) => Ok(Expr::Lit(Value::String(s.clone()))),
        EdnValue::Boolean(b) => Ok(Expr::Lit(Value::Boolean(*b))),
        EdnValue::Nil => Ok(Expr::Lit(Value::Null)),
        EdnValue::Keyword(k) => Ok(Expr::Lit(Value::Keyword(k.clone()))),
        EdnValue::List(inner) => parse_expr(inner),
        other => Err(format!("unsupported expression argument: {:?}", other)),
    }
}

/// Parse an EDN list `(op arg arg?)` into an Expr tree.
///
/// For `matches?`, the second argument must be a string literal and is
/// validated as a valid regex pattern at parse time.
fn parse_expr(list: &[EdnValue]) -> Result<Expr, String> {
    if list.is_empty() {
        return Err("expression list cannot be empty".to_string());
    }
    let head = match &list[0] {
        EdnValue::Symbol(s) => s.as_str(),
        other => return Err(format!("expression head must be a symbol, got {:?}", other)),
    };

    match head {
        // Unary type predicates
        "string?" | "integer?" | "float?" | "boolean?" | "nil?" => {
            if list.len() != 2 {
                return Err(format!("{} takes exactly 1 argument", head));
            }
            let op = match head {
                "string?" => UnaryOp::StringQ,
                "integer?" => UnaryOp::IntegerQ,
                "float?" => UnaryOp::FloatQ,
                "boolean?" => UnaryOp::BooleanQ,
                "nil?" => UnaryOp::NilQ,
                _ => unreachable!(),
            };
            let arg = parse_expr_arg(&list[1])?;
            Ok(Expr::UnaryOp(op, Box::new(arg)))
        }

        // Binary operators
        "<" | ">" | "<=" | ">=" | "=" | "!=" | "+" | "-" | "*" | "/" | "starts-with?"
        | "ends-with?" | "contains?" | "matches?" => {
            if list.len() != 3 {
                return Err(format!("{} takes exactly 2 arguments", head));
            }
            let op = match head {
                "<" => BinOp::Lt,
                ">" => BinOp::Gt,
                "<=" => BinOp::Lte,
                ">=" => BinOp::Gte,
                "=" => BinOp::Eq,
                "!=" => BinOp::Neq,
                "+" => BinOp::Add,
                "-" => BinOp::Sub,
                "*" => BinOp::Mul,
                "/" => BinOp::Div,
                "starts-with?" => BinOp::StartsWith,
                "ends-with?" => BinOp::EndsWith,
                "contains?" => BinOp::Contains,
                "matches?" => BinOp::Matches,
                _ => unreachable!(),
            };
            let lhs = parse_expr_arg(&list[1])?;
            let rhs = parse_expr_arg(&list[2])?;

            // matches? second arg must be a string literal; validate regex now.
            if op == BinOp::Matches {
                match &rhs {
                    Expr::Lit(Value::String(pattern)) => {
                        regex_lite::Regex::new(pattern)
                            .map_err(|e| format!("invalid regex pattern {:?}: {}", pattern, e))?;
                    }
                    _ => {
                        return Err("matches? second argument must be a string literal".to_string());
                    }
                }
            }
            Ok(Expr::BinOp(op, Box::new(lhs), Box::new(rhs)))
        }

        other => Err(format!("unknown expression operator: {}", other)),
    }
}

/// Parse a vector clause whose first element is a list: `[(expr)]` or `[(expr) ?out]`.
///
/// Called from `:where` dispatch when `vec[0]` is an `EdnValue::List`.
fn parse_expr_clause(vec: &[EdnValue]) -> Result<WhereClause, String> {
    let inner_list = match &vec[0] {
        EdnValue::List(l) => l.as_slice(),
        _ => return Err("parse_expr_clause called with non-list element 0".to_string()),
    };
    let expr = parse_expr(inner_list)?;
    let binding = match vec.len() {
        1 => None,
        2 => match &vec[1] {
            EdnValue::Symbol(s) if s.starts_with('?') => Some(s.clone()),
            other => {
                return Err(format!(
                    "expression output must be a ?variable, got {:?}",
                    other
                ));
            }
        },
        n => {
            return Err(format!(
                "expression clause must be [(expr)] or [(expr) ?out], got {} elements",
                n
            ));
        }
    };
    Ok(WhereClause::Expr { expr, binding })
}

/// Parse a list item (EDN List) appearing in a :where clause or rule body.
/// Returns Err if the list is empty, has an unknown form, or contains nested `not`.
fn parse_list_as_where_clause(list: &[EdnValue], allow_not: bool) -> Result<WhereClause, String> {
    if list.is_empty() {
        return Err("Empty list in :where clause".to_string());
    }
    match &list[0] {
        EdnValue::Symbol(s) if s == "not" => {
            if !allow_not {
                return Err("(not ...) cannot appear inside another (not ...)".to_string());
            }
            if list.len() < 2 {
                return Err("(not) requires at least one clause".to_string());
            }
            let mut inner = Vec::new();
            for item in &list[1..] {
                if let Some(vec) = item.as_vector() {
                    if matches!(vec.first(), Some(EdnValue::List(_))) {
                        let clause = parse_expr_clause(vec)?;
                        inner.push(clause);
                    } else {
                        let pattern = parse_query_pattern(vec)?;
                        inner.push(WhereClause::Pattern(pattern));
                    }
                } else if let Some(inner_list) = item.as_list() {
                    // Reject (or ...)/(or-join ...) inside not bodies
                    if matches!(inner_list.first(), Some(EdnValue::Symbol(s)) if s == "or" || s == "or-join")
                    {
                        return Err(
                            "(or)/(or-join) cannot appear inside (not)/(not-join)".to_string()
                        );
                    }
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
        EdnValue::Symbol(s) if s == "not-join" => {
            if !allow_not {
                return Err(
                    "(not-join ...) cannot appear inside another (not ...) or (not-join ...)"
                        .to_string(),
                );
            }
            // Syntax: (not-join [?v1 ?v2 ...] clause1 clause2 ...)
            if list.len() < 3 {
                return Err(
                    "(not-join) requires a join-vars vector and at least one clause".to_string(),
                );
            }
            let join_var_vec = match &list[1] {
                EdnValue::Vector(v) => v,
                _ => {
                    return Err(
                        "(not-join) first argument must be a vector of join variables".to_string(),
                    );
                }
            };
            let join_vars: Vec<String> = join_var_vec
                .iter()
                .map(|v| match v {
                    EdnValue::Symbol(s) if s.starts_with('?') => Ok(s.clone()),
                    _ => Err(format!(
                        "(not-join) join variables must be logic variables, got {:?}",
                        v
                    )),
                })
                .collect::<Result<_, _>>()?;
            let mut inner = Vec::new();
            for item in &list[2..] {
                if let Some(vec) = item.as_vector() {
                    if matches!(vec.first(), Some(EdnValue::List(_))) {
                        let clause = parse_expr_clause(vec)?;
                        inner.push(clause);
                    } else {
                        let pattern = parse_query_pattern(vec)?;
                        inner.push(WhereClause::Pattern(pattern));
                    }
                } else if let Some(inner_list) = item.as_list() {
                    // Reject (or ...)/(or-join ...) inside not-join bodies
                    if matches!(inner_list.first(), Some(EdnValue::Symbol(s)) if s == "or" || s == "or-join")
                    {
                        return Err(
                            "(or)/(or-join) cannot appear inside (not)/(not-join)".to_string()
                        );
                    }
                    // allow_not=false to reject nested (not ...) or (not-join ...)
                    let clause = parse_list_as_where_clause(inner_list, false)?;
                    inner.push(clause);
                } else {
                    return Err(format!(
                        "expected pattern or rule invocation inside (not-join), got {:?}",
                        item
                    ));
                }
            }
            Ok(WhereClause::NotJoin {
                join_vars,
                clauses: inner,
            })
        }
        EdnValue::Symbol(s) if s == "or" => {
            if list.len() < 2 {
                return Err("(or) requires at least one branch".to_string());
            }
            let mut branches: Vec<Vec<WhereClause>> = Vec::new();
            for item in &list[1..] {
                let branch = parse_or_branch(item)?;
                branches.push(branch);
            }
            Ok(WhereClause::Or(branches))
        }
        EdnValue::Symbol(s) if s == "or-join" => {
            if list.len() < 3 {
                return Err(
                    "(or-join) requires a join-vars vector and at least one branch".to_string(),
                );
            }
            let join_var_vec = match &list[1] {
                EdnValue::Vector(v) => v,
                _ => {
                    return Err(
                        "(or-join) first argument must be a vector of join variables".to_string(),
                    );
                }
            };
            let join_vars: Vec<String> = join_var_vec
                .iter()
                .map(|v| match v {
                    EdnValue::Symbol(s) if s.starts_with('?') => Ok(s.clone()),
                    _ => Err(format!(
                        "(or-join) join variables must be logic variables, got {:?}",
                        v
                    )),
                })
                .collect::<Result<_, _>>()?;
            let mut branches: Vec<Vec<WhereClause>> = Vec::new();
            for item in &list[2..] {
                let branch = parse_or_branch(item)?;
                branches.push(branch);
            }
            Ok(WhereClause::OrJoin {
                join_vars,
                branches,
            })
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

/// Parse a where-clause pattern vector with pseudo-attribute detection.
///
/// Detects `:db/*` keywords in the attribute position and wraps them in
/// `AttributeSpec::Pseudo`. Rejects `:db/*` keywords in entity or value positions.
/// Falls through to `Pattern::from_edn` for regular patterns.
fn parse_query_pattern(vec: &[EdnValue]) -> Result<Pattern, String> {
    if vec.len() != 3 {
        return Err(format!(
            "Pattern must have exactly 3 elements (E A V), got {}",
            vec.len()
        ));
    }

    // Reject :db/* in entity position
    if let EdnValue::Keyword(k) = &vec[0]
        && PseudoAttr::from_keyword(k).is_some()
    {
        return Err(format!(
            "pseudo-attribute {} is not valid in entity position",
            k
        ));
    }

    // Reject :db/* in value position
    if let EdnValue::Keyword(k) = &vec[2]
        && PseudoAttr::from_keyword(k).is_some()
    {
        return Err(format!(
            "pseudo-attribute {} is not valid in value position",
            k
        ));
    }

    // Detect pseudo-attribute in attribute position
    if let EdnValue::Keyword(k) = &vec[1]
        && let Some(pseudo) = PseudoAttr::from_keyword(k)
    {
        return Ok(Pattern::pseudo(vec[0].clone(), pseudo, vec[2].clone()));
    }

    Pattern::from_edn(vec)
}

/// Parse a single branch of an (or ...) or (or-join ...) clause.
///
/// A branch is either:
/// - A single clause: `[pattern]` or `(rule-invocation)` or `[(expr)]`
/// - A grouped list of clauses: `(and clause1 clause2 ...)`
fn parse_or_branch(item: &EdnValue) -> Result<Vec<WhereClause>, String> {
    match item {
        EdnValue::List(inner) if matches!(inner.first(), Some(EdnValue::Symbol(s)) if s == "and") =>
        {
            // (and clause1 clause2 ...) — multi-clause branch
            if inner.len() < 2 {
                return Err("(and) inside or/or-join requires at least one clause".to_string());
            }
            let mut clauses = Vec::new();
            for clause_item in &inner[1..] {
                clauses.push(parse_or_branch_item(clause_item)?);
            }
            Ok(clauses)
        }
        other => {
            // Single-clause branch
            Ok(vec![parse_or_branch_item(other)?])
        }
    }
}

/// Parse a single clause item within an or branch.
fn parse_or_branch_item(item: &EdnValue) -> Result<WhereClause, String> {
    match item {
        EdnValue::Vector(vec) => {
            if matches!(vec.first(), Some(EdnValue::List(_))) {
                parse_expr_clause(vec)
            } else {
                Ok(WhereClause::Pattern(parse_query_pattern(vec)?))
            }
        }
        EdnValue::List(inner_list) => {
            // allow_not=true: or branches can contain not/not-join/or/or-join
            parse_list_as_where_clause(inner_list, true)
        }
        _ => Err(format!("expected clause inside or branch, got {:?}", item)),
    }
}

/// Collect all variable names that appear in a where clause (non-recursively into Not).
fn outer_vars_from_clause(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Pattern(p) => {
            let mut vars = Vec::new();
            if let Some(name) = p.entity.as_variable()
                && !name.starts_with("?_")
            {
                vars.push(name.to_string());
            }
            if let AttributeSpec::Real(attr_edn) = &p.attribute
                && let Some(name) = attr_edn.as_variable()
                && !name.starts_with("?_")
            {
                vars.push(name.to_string());
            }
            if let Some(name) = p.value.as_variable()
                && !name.starts_with("?_")
            {
                vars.push(name.to_string());
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
        WhereClause::NotJoin { .. } => vec![], // not counted as "outer"
        WhereClause::Or(branches) => {
            if branches.is_empty() {
                return vec![];
            }
            // Variables available after `or` = intersection across all branches
            let branch_var_sets: Vec<std::collections::HashSet<String>> = branches
                .iter()
                .map(|branch| {
                    branch
                        .iter()
                        .flat_map(outer_vars_from_clause)
                        .collect::<std::collections::HashSet<_>>()
                })
                .collect();
            branch_var_sets[0]
                .iter()
                .filter(|v| branch_var_sets[1..].iter().all(|s| s.contains(*v)))
                .cloned()
                .collect()
        }
        WhereClause::OrJoin { join_vars, .. } => join_vars.clone(),
        WhereClause::Expr { binding, .. } => match binding {
            Some(var) => vec![var.clone()],
            None => vec![],
        },
    }
}

/// Collect all variable names that appear inside a Not clause.
fn vars_in_not(clause: &WhereClause) -> Vec<String> {
    match clause {
        WhereClause::Not(inner) => inner.iter().flat_map(outer_vars_from_clause).collect(),
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

/// Validate not-join safety: every variable listed in join_vars must be bound
/// by an outer clause. Variables that appear only in the not-join body but are
/// NOT in join_vars are existentially quantified — no error.
fn check_not_join_safety(
    clauses: &[WhereClause],
    outer_bound: &std::collections::HashSet<String>,
) -> Result<(), String> {
    for clause in clauses {
        if let WhereClause::NotJoin { join_vars, .. } = clause {
            for var in join_vars {
                if !var.starts_with("?_") && !outer_bound.contains(var) {
                    return Err(format!(
                        "join variable {} in (not-join ...) is not bound by any outer clause",
                        var
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Collect all Var names referenced in an Expr tree.
fn expr_vars(expr: &Expr) -> Vec<String> {
    match expr {
        Expr::Var(v) => vec![v.clone()],
        Expr::Lit(_) => vec![],
        Expr::BinOp(_, lhs, rhs) => {
            let mut v = expr_vars(lhs);
            v.extend(expr_vars(rhs));
            v
        }
        Expr::UnaryOp(_, arg) => expr_vars(arg),
    }
}

/// Forward-pass safety check: every Var in an Expr clause must be bound by
/// an earlier clause. Binding-form Expr clauses add their output var to scope.
///
/// Called for both query `:where` clauses and rule body clauses.
fn check_expr_safety(clauses: &[WhereClause]) -> Result<(), String> {
    check_expr_safety_with_bound(clauses, &mut std::collections::HashSet::new())
}

fn check_expr_safety_with_bound(
    clauses: &[WhereClause],
    bound: &mut std::collections::HashSet<String>,
) -> Result<(), String> {
    for clause in clauses {
        match clause {
            WhereClause::Expr { expr, binding } => {
                for var in expr_vars(expr) {
                    if !bound.contains(&var) {
                        return Err(format!(
                            "variable {} in expression clause is not bound by any earlier clause",
                            var
                        ));
                    }
                }
                if let Some(out) = binding {
                    bound.insert(out.clone());
                }
            }
            WhereClause::Not(inner) => {
                // Recurse into not body with a clone of the current bound set.
                // Not bodies can reference outer-bound variables, but cannot
                // extend the outer bound (negation doesn't introduce new bindings).
                let mut inner_bound = bound.clone();
                check_expr_safety_with_bound(inner, &mut inner_bound)?;
            }
            WhereClause::NotJoin { clauses: inner, .. } => {
                let mut inner_bound = bound.clone();
                check_expr_safety_with_bound(inner, &mut inner_bound)?;
            }
            WhereClause::Or(branches) => {
                if !branches.is_empty() {
                    let mut branch_new_var_sets: Vec<std::collections::HashSet<String>> =
                        Vec::new();
                    for branch in branches {
                        let mut branch_bound = bound.clone();
                        check_expr_safety_with_bound(branch, &mut branch_bound)?;
                        let new_vars: std::collections::HashSet<String> =
                            branch_bound.difference(bound).cloned().collect();
                        branch_new_var_sets.push(new_vars);
                    }
                    if branch_new_var_sets.windows(2).any(|w| w[0] != w[1]) {
                        return Err(
                            "all branches of (or ...) must introduce the same set of new variables"
                                .to_string(),
                        );
                    }
                    if let Some(new_vars) = branch_new_var_sets.first() {
                        for var in new_vars {
                            bound.insert(var.clone());
                        }
                    }
                }
            }
            WhereClause::OrJoin {
                join_vars,
                branches,
            } => {
                for var in join_vars {
                    if !var.starts_with("?_") && !bound.contains(var) {
                        return Err(format!(
                            "join variable {} in (or-join ...) is not bound by any earlier clause",
                            var
                        ));
                    }
                }
                for branch in branches {
                    let mut branch_bound = bound.clone();
                    check_expr_safety_with_bound(branch, &mut branch_bound)?;
                }
                // or-join does NOT add new variables to the outer bound
            }
            other => {
                for var in outer_vars_from_clause(other) {
                    bound.insert(var);
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
            if matches!(vec.first(), Some(EdnValue::List(_))) {
                let clause = parse_expr_clause(vec)?;
                body_clauses.push(clause);
            } else {
                let pattern = parse_query_pattern(vec)?;
                body_clauses.push(WhereClause::Pattern(pattern));
            }
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
    check_not_join_safety(&body_clauses, &outer_bound)?;
    check_expr_safety(&body_clauses)?;

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
                assert_eq!(q.find, vec![FindSpec::Variable("?name".to_string())]);
                let patterns = q.get_patterns();
                assert_eq!(patterns.len(), 1);
                assert_eq!(
                    patterns[0].attribute,
                    AttributeSpec::Real(EdnValue::Keyword(":person/name".to_string()))
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
                assert_eq!(
                    q.find,
                    vec![
                        FindSpec::Variable("?name".to_string()),
                        FindSpec::Variable("?age".to_string()),
                    ]
                );
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
                assert_eq!(q.find, vec![FindSpec::Variable("?to".to_string())]);
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
                assert_eq!(q.find, vec![FindSpec::Variable("?name".to_string())]);
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
                assert_eq!(q.find, vec![FindSpec::Variable("?z".to_string())]);
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
        let input =
            r#"(query [:find ?person :where [?person :name ?n] (not [?person :banned true])])"#;
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
                    _ => panic!("Expected Not clause"),
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
            DatalogCommand::Query(q) => match &q.where_clauses[1] {
                WhereClause::Not(inner) => {
                    assert!(matches!(inner[0], WhereClause::RuleInvocation { .. }));
                }
                _ => panic!("Expected Not clause"),
            },
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
        assert!(msg.contains("requires at least one clause"));
    }

    #[test]
    fn test_parse_nested_not_is_error() {
        let input = r#"(query [:find ?x :where [?x :a ?v] (not (not [?x :banned true]))])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("cannot appear inside another"));
    }

    #[test]
    fn test_parse_not_unbound_variable_is_error() {
        // ?y is only in the not body, not in any outer clause
        let input = r#"(query [:find ?x :where [?x :a ?v] (not [?y :banned true])])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("not bound"));
    }

    #[test]
    fn test_parse_not_unbound_variable_in_rule_body_is_error() {
        // ?y only in not, not in head or non-not body
        let input = r#"(rule [(eligible ?x) [?x :applied true] (not [?y :banned true])])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("not bound"));
    }

    #[test]
    fn test_parse_not_with_multiple_clauses() {
        // (not [?person :role :admin] [?person :active false])
        let input = r#"(query [:find ?person :where [?person :name ?n] (not [?person :role :admin] [?person :active false])])"#;
        let cmd = parse_datalog_command(input).unwrap();
        match cmd {
            DatalogCommand::Query(q) => match &q.where_clauses[1] {
                WhereClause::Not(inner) => {
                    assert_eq!(inner.len(), 2);
                    assert!(matches!(inner[0], WhereClause::Pattern(_)));
                    assert!(matches!(inner[1], WhereClause::Pattern(_)));
                }
                _ => panic!("Expected Not clause with 2 items"),
            },
            _ => panic!("Expected Query"),
        }
    }

    #[test]
    fn test_parse_not_join_basic() {
        // (query [:find ?e :where [?e :name ?n] (not-join [?e] [?e :banned true])])
        let result = parse_datalog_command(
            "(query [:find ?e :where [?e :name ?n] (not-join [?e] [?e :banned true])])",
        );
        assert!(result.is_ok(), "basic not-join must parse OK");
        if let Ok(DatalogCommand::Query(q)) = result {
            assert_eq!(q.where_clauses.len(), 2);
            assert!(matches!(
                &q.where_clauses[1],
                WhereClause::NotJoin { join_vars, clauses }
                if join_vars == &["?e".to_string()] && clauses.len() == 1
            ));
        } else {
            panic!("expected Query");
        }
    }

    #[test]
    fn test_parse_not_join_multiple_join_vars() {
        let result = parse_datalog_command(
            "(query [:find ?e :where [?e :name ?n] [?e :role ?r] \
             (not-join [?e ?r] [?e :has-role ?r] [?r :is-admin true])])",
        );
        assert!(result.is_ok(), "multi-join-var not-join must parse");
        if let Ok(DatalogCommand::Query(q)) = result {
            if let WhereClause::NotJoin { join_vars, clauses } = &q.where_clauses[2] {
                assert_eq!(join_vars.len(), 2);
                assert_eq!(clauses.len(), 2);
            } else {
                panic!("expected NotJoin");
            }
        }
    }

    #[test]
    fn test_parse_not_join_inner_var_need_not_be_outer_bound() {
        // ?tag appears only in the not-join body — this is legal
        let result = parse_datalog_command(
            "(query [:find ?e :where [?e :name ?n] \
             (not-join [?e] [?e :has-tag ?tag] [?tag :is-bad true])])",
        );
        assert!(
            result.is_ok(),
            "inner-only var ?tag must be allowed in not-join"
        );
    }

    #[test]
    fn test_parse_not_join_unbound_join_var_rejected() {
        // ?role is in join_vars but not bound by any outer clause
        let result = parse_datalog_command(
            "(query [:find ?e :where [?e :name ?n] \
             (not-join [?role] [?e :has-role ?role])])",
        );
        assert!(result.is_err(), "unbound join var must be rejected");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("?role") && msg.contains("not bound"),
            "error must name the offending variable"
        );
    }

    #[test]
    fn test_parse_not_join_missing_join_vars_vector_rejected() {
        // First arg is not a vector
        let result = parse_datalog_command(
            "(query [:find ?e :where [?e :name ?n] (not-join ?e [?e :banned true])])",
        );
        assert!(result.is_err(), "non-vector first arg must fail");
    }

    #[test]
    fn test_parse_not_join_too_few_args_rejected() {
        // Only join-vars vector, no body clauses
        let result =
            parse_datalog_command("(query [:find ?e :where [?e :name ?n] (not-join [?e])])");
        assert!(result.is_err(), "not-join with no clauses must fail");
    }

    #[test]
    fn test_parse_not_join_nested_inside_not_rejected() {
        let result = parse_datalog_command(
            "(query [:find ?e :where [?e :name ?n] \
             (not (not-join [?e] [?e :banned true]))])",
        );
        assert!(result.is_err(), "not-join nested inside not must fail");
    }

    #[test]
    fn test_parse_not_join_in_rule_body() {
        let result = parse_datalog_command(
            "(rule [(eligible ?x) \
             [?x :applied true] \
             (not-join [?x] [?x :dep ?d] [?d :status :rejected])])",
        );
        assert!(result.is_ok(), "not-join in rule body must parse");
        if let Ok(DatalogCommand::Rule(rule)) = result {
            assert_eq!(rule.body.len(), 2);
            assert!(
                matches!(&rule.body[1], WhereClause::NotJoin { join_vars, .. }
                if join_vars == &["?x".to_string()])
            );
        }
    }

    #[test]
    fn test_parse_not_join_rule_body_unbound_join_var_rejected() {
        // ?dep is in join_vars but never bound by outer body
        let result = parse_datalog_command(
            "(rule [(eligible ?x) \
             [?x :applied true] \
             (not-join [?dep] [?x :dep ?dep])])",
        );
        assert!(
            result.is_err(),
            "unbound join var in rule body not-join must fail"
        );
    }

    #[test]
    fn test_parse_count_in_find() {
        let result =
            parse_datalog_command("(query [:find (count ?e) :where [?e :person/name ?n]])");
        let cmd = result.expect("parse failed");
        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find.len(), 1);
                assert_eq!(
                    q.find[0],
                    FindSpec::Aggregate {
                        func: AggFunc::Count,
                        var: "?e".to_string()
                    }
                );
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_mixed_find_var_and_aggregate() {
        let result = parse_datalog_command(
            r#"(query [:find ?dept (count-distinct ?e) :where [?e :dept ?dept]])"#,
        );
        let cmd = result.expect("parse failed");
        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find.len(), 2);
                assert_eq!(q.find[0], FindSpec::Variable("?dept".to_string()));
                assert_eq!(
                    q.find[1],
                    FindSpec::Aggregate {
                        func: AggFunc::CountDistinct,
                        var: "?e".to_string()
                    }
                );
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_all_aggregate_functions() {
        let cases = [
            ("count", AggFunc::Count),
            ("count-distinct", AggFunc::CountDistinct),
            ("sum", AggFunc::Sum),
            ("sum-distinct", AggFunc::SumDistinct),
            ("min", AggFunc::Min),
            ("max", AggFunc::Max),
        ];
        for (name, expected_func) in cases {
            let input = format!("(query [:find ({} ?v) :where [?e :a ?v]])", name);
            let cmd = parse_datalog_command(&input).expect("parse failed");
            match cmd {
                DatalogCommand::Query(q) => {
                    assert_eq!(
                        q.find[0],
                        FindSpec::Aggregate {
                            func: expected_func,
                            var: "?v".to_string()
                        }
                    );
                }
                _ => panic!("expected Query"),
            }
        }
    }

    #[test]
    fn test_parse_with_clause_single_var() {
        let result = parse_datalog_command(
            r#"(query [:find ?dept (sum ?salary) :with ?e :where [?e :dept ?dept] [?e :salary ?salary]])"#,
        );
        let cmd = result.expect("parse failed");
        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.with_vars, vec!["?e".to_string()]);
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_with_clause_multiple_vars() {
        let result = parse_datalog_command(
            r#"(query [:find (count ?e) :with ?dept ?role :where [?e :dept ?dept] [?e :role ?role]])"#,
        );
        let cmd = result.expect("parse failed");
        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.with_vars, vec!["?dept".to_string(), "?role".to_string()]);
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_error_unknown_aggregate() {
        let result = parse_datalog_command("(query [:find (average ?e) :where [?e :a ?v]])");
        assert!(result.is_err(), "unknown aggregate should fail");
        assert!(
            result.unwrap_err().contains("Unknown aggregate function"),
            "wrong error message"
        );
    }

    #[test]
    fn test_parse_error_aggregate_arg_not_variable() {
        let result = parse_datalog_command("(query [:find (count :not-a-var) :where [?e :a ?v]])");
        assert!(result.is_err(), "non-variable aggregate arg should fail");
    }

    #[test]
    fn test_parse_error_with_without_aggregate() {
        let result = parse_datalog_command(r#"(query [:find ?e :with ?x :where [?e :a ?x]])"#);
        assert!(result.is_err(), ":with without aggregate should fail");
        assert!(
            result
                .unwrap_err()
                .contains("requires at least one aggregate"),
            "wrong error message"
        );
    }

    #[test]
    fn test_parse_error_aggregate_var_unbound() {
        let result = parse_datalog_command(r#"(query [:find (count ?unbound) :where [?e :a ?v]])"#);
        assert!(result.is_err(), "unbound aggregate var should fail");
        assert!(
            result.unwrap_err().contains("not bound in :where"),
            "wrong error message"
        );
    }

    // Helper used by parse_expr tests (Task 2 / Phase 7.2b)
    fn parse(s: &str) -> Result<DatalogCommand, String> {
        parse_datalog_command(s)
    }

    #[test]
    fn test_parse_expr_lt_filter() {
        // [(< ?v 100)] — filter clause
        let input = "(query [:find ?e :where [?e :item/price ?v] [(< ?v 100)]])";
        let result = parse(input);
        assert!(result.is_ok(), "parse failed");
        match result.unwrap() {
            DatalogCommand::Query(q) => {
                assert_eq!(q.where_clauses.len(), 2);
                assert!(matches!(
                    q.where_clauses[1],
                    WhereClause::Expr { binding: None, .. }
                ));
            }
            _ => panic!("expected query"),
        }
    }

    #[test]
    fn test_parse_expr_add_binding() {
        // [(+ ?a ?b) ?sum] — binding clause
        let input = "(query [:find ?sum :where [?e :x ?a] [?e :y ?b] [(+ ?a ?b) ?sum]])";
        let result = parse(input);
        assert!(result.is_ok(), "parse failed");
        match result.unwrap() {
            DatalogCommand::Query(q) => {
                assert_eq!(q.where_clauses.len(), 3);
                assert!(matches!(
                    q.where_clauses[2],
                    WhereClause::Expr {
                        binding: Some(_),
                        ..
                    }
                ));
            }
            _ => panic!("expected query"),
        }
    }

    #[test]
    fn test_parse_expr_nested_arithmetic() {
        // [(+ (* ?a 2) ?b) ?result]
        let input =
            "(query [:find ?result :where [?e :x ?a] [?e :y ?b] [(+ (* ?a 2) ?b) ?result]])";
        let result = parse(input);
        assert!(result.is_ok(), "parse nested arithmetic");
    }

    #[test]
    fn test_parse_expr_string_predicate() {
        let input = "(query [:find ?e :where [?e :item/tag ?tag] [(starts-with? ?tag \"work\")]])";
        let result = parse(input);
        assert!(result.is_ok(), "parse starts-with?");
    }

    #[test]
    fn test_parse_expr_matches_valid_regex() {
        let input = "(query [:find ?e :where [?e :person/email ?addr] [(matches? ?addr \"^[^@]+@[^@]+$\")]])";
        let result = parse(input);
        assert!(result.is_ok(), "parse matches? with valid regex");
    }

    #[test]
    fn test_parse_expr_matches_invalid_regex_is_error() {
        let input = "(query [:find ?e :where [?e :a ?v] [(matches? ?v \"[unclosed\")]])";
        let result = parse(input);
        assert!(result.is_err(), "invalid regex must be a parse error");
    }

    #[test]
    fn test_parse_expr_unbound_variable_is_error() {
        // ?v is not bound by any pattern before the expr clause
        let input = "(query [:find ?e :where [?e :x ?a] [(< ?v 100)]])";
        let result = parse(input);
        // check_expr_safety rejects this: ?v is not bound by any earlier clause
        assert!(
            result.is_err(),
            "unbound variable in expr must be parse error"
        );
    }

    #[test]
    fn test_parse_expr_three_element_vector_stays_pattern() {
        // [?e :a ?v] must still parse as a Pattern, not an Expr clause
        let input = "(query [:find ?v :where [?e :attr ?v]])";
        let result = parse(input);
        assert!(result.is_ok(), "three-element vector is a pattern");
        match result.unwrap() {
            DatalogCommand::Query(q) => {
                assert!(matches!(q.where_clauses[0], WhereClause::Pattern(_)));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn test_parse_pseudo_attr_in_where_clause() {
        let cmd = parse_datalog_command(
            "(query [:find ?vf :any-valid-time :where [?e :person/name _] [?e :db/valid-from ?vf]])"
        ).unwrap();
        match cmd {
            DatalogCommand::Query(q) => {
                let patterns = q.get_patterns();
                assert!(
                    patterns
                        .iter()
                        .any(|p| matches!(p.attribute, AttributeSpec::Pseudo(_))),
                    "expected a Pseudo attribute pattern"
                );
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_error_pseudo_attr_entity_position() {
        let result = parse_datalog_command(
            "(query [:find ?v :any-valid-time :where [:db/valid-from :person/name ?v]])",
        );
        assert!(
            result.is_err(),
            "pseudo-attr in entity position should error"
        );
    }

    #[test]
    fn test_parse_error_pseudo_attr_value_position() {
        let result = parse_datalog_command(
            "(query [:find ?e :any-valid-time :where [?e :person/name :db/valid-from]])",
        );
        assert!(
            result.is_err(),
            "pseudo-attr in value position should error"
        );
    }
}

#[cfg(test)]
mod or_parse_tests {
    use super::*;

    #[test]
    fn test_parse_or_two_branches() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :a ?v]
                              (or [?e :b ?v] [?e :c ?v])])"#,
        );
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert_eq!(q.where_clauses.len(), 2);
            assert!(matches!(q.where_clauses[1], WhereClause::Or(_)));
        }
    }

    #[test]
    fn test_parse_or_with_and_grouping() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or (and [?e :tag ?t]) [?e :label ?t])])"#,
        );
        assert!(cmd.is_ok(), "parse with and grouping failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            let or_clause = &q.where_clauses[1];
            if let WhereClause::Or(branches) = or_clause {
                assert_eq!(branches.len(), 2);
                assert_eq!(branches[0].len(), 1);
                assert_eq!(branches[1].len(), 1);
            } else {
                panic!("expected Or clause");
            }
        }
    }

    #[test]
    fn test_parse_or_join_basic() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or-join [?e]
                                [?e :tag :red]
                                [?e :tag :blue])])"#,
        );
        assert!(cmd.is_ok(), "or-join parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert!(matches!(q.where_clauses[1], WhereClause::OrJoin { .. }));
        }
    }

    #[test]
    fn test_parse_or_safety_mismatched_new_vars_is_error() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or [?e :a ?x] [?e :b ?y])])"#,
        );
        assert!(
            cmd.is_err(),
            "should fail: branches introduce different vars"
        );
        let err = cmd.unwrap_err();
        assert!(err.contains("same set of new variables"));
    }

    #[test]
    fn test_parse_or_join_unbound_join_var_is_error() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (or-join [?unbound]
                                [?unbound :tag :red])])"#,
        );
        assert!(cmd.is_err(), "should fail: unbound join var");
        let err = cmd.unwrap_err();
        assert!(err.contains("not bound"));
    }

    #[test]
    fn test_or_inside_not_is_parse_error() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?e
                       :where [?e :name ?n]
                              (not (or [?e :a true] [?e :b true]))])"#,
        );
        assert!(cmd.is_err(), "or inside not should be a parse error");
        let err = cmd.unwrap_err();
        assert!(err.contains("or") || err.contains("not"));
    }
}
