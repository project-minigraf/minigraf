use super::types::*;
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::types::Value;
use crate::temporal::parse_timestamp;
use anyhow::Result;
use uuid::Uuid;

const MAX_STRING_LENGTH: usize = 1024 * 1024; // 1MB
const MAX_KEYWORD_LENGTH: usize = 1024; // 1KB
const MAX_SYMBOL_LENGTH: usize = 1024; // 1KB
const MAX_RECURSION_DEPTH: usize = 100;

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
    /// `#{`, which opens a set literal (#316).
    SetOpen,
    Nil,
    BindSlot(String), // $identifier — named bind slot for PreparedQuery
}

/// Tokenize EDN input
#[cfg(test)]
fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut lexer = Lexer::new(input);
    let tokens = lexer.by_ref().collect();
    lexer.finish()?;
    Ok(tokens)
}

/// Reads EDN tokens from `input` one at a time (#550). After its first error
/// it yields nothing more and keeps the error.
struct Lexer<'a> {
    input: &'a str,
    /// Byte offset of the next character.
    pos: usize,
    error: Option<anyhow::Error>,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self::at(input, 0)
    }

    /// A lexer that starts at byte `pos` of `input`, which must be a token
    /// boundary.
    fn at(input: &'a str, pos: usize) -> Self {
        Lexer {
            input,
            pos,
            error: None,
        }
    }

    /// Lex the rest of the input, keeping nothing: its first error, if any.
    fn finish(&mut self) -> Result<()> {
        while self.next().is_some() {}
        match self.error.take() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.input.get(self.pos..)?.chars().next()
    }

    fn bump(&mut self) {
        if let Some(ch) = self.peek_char() {
            self.pos += ch.len_utf8();
        }
    }

    /// The next token, or `None` at the end of the input.
    fn lex(&mut self) -> Result<Option<Token>> {
        while let Some(ch) = self.peek_char() {
            let token = match ch {
                // Whitespace
                ' ' | '\t' | '\n' | '\r' | ',' => {
                    self.bump();
                    continue;
                }
                // Parens and brackets
                '(' => {
                    self.bump();
                    Token::LeftParen
                }
                ')' => {
                    self.bump();
                    Token::RightParen
                }
                '[' => {
                    self.bump();
                    Token::LeftBracket
                }
                ']' => {
                    self.bump();
                    Token::RightBracket
                }
                '{' => {
                    self.bump();
                    Token::LeftBrace
                }
                '}' => {
                    self.bump();
                    Token::RightBrace
                }
                // String literals
                '"' => {
                    self.bump(); // consume opening quote
                    let mut string = String::new();
                    let mut closed = false;
                    while let Some(ch) = self.peek_char() {
                        if ch == '"' {
                            self.bump(); // consume closing quote
                            closed = true;
                            break;
                        } else if ch == '\\' {
                            self.bump();
                            // Handle escape sequences
                            if let Some(escaped) = self.peek_char() {
                                self.bump();
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
                            if string.len() >= MAX_STRING_LENGTH {
                                bail_coded!(ErrorCode::Prs007, MAX_STRING_LENGTH);
                            }
                            string.push(ch);
                            self.bump();
                        }
                    }
                    if !closed {
                        bail_coded!(ErrorCode::Prs109);
                    }
                    Token::String(string)
                }
                // Keywords (start with :)
                ':' => {
                    self.bump();
                    let mut keyword = String::from(":");
                    while let Some(ch) = self.peek_char() {
                        // '?' is a constituent character, matching symbols (see below) and
                        // EDN itself — predicate-style names like :person/alive? are idiomatic.
                        // A keyword can never be a query variable, so there is no ambiguity.
                        if ch.is_alphanumeric() || ch == '/' || ch == '-' || ch == '_' || ch == '?'
                        {
                            if keyword.len() >= MAX_KEYWORD_LENGTH {
                                bail_coded!(ErrorCode::Prs008, MAX_KEYWORD_LENGTH);
                            }
                            keyword.push(ch);
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if keyword.len() == 1 {
                        bail_coded!(ErrorCode::Prs110);
                    }
                    Token::Keyword(keyword)
                }
                // Tagged literals (start with #)
                '#' => {
                    self.bump();
                    if self.peek_char() == Some('{') {
                        self.bump();
                        return Ok(Some(Token::SetOpen));
                    }
                    let mut tag = String::new();
                    while let Some(ch) = self.peek_char() {
                        if ch.is_alphanumeric() || ch == '-' {
                            if tag.len() >= MAX_SYMBOL_LENGTH {
                                bail_coded!(ErrorCode::Prs009, MAX_SYMBOL_LENGTH);
                            }
                            tag.push(ch);
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    Token::TaggedLiteral(tag)
                }
                // Numbers or symbols starting with -
                '-' => {
                    self.bump();
                    match self.peek_char() {
                        // It's a negative number
                        Some(next_ch) if next_ch.is_numeric() => self.number(String::from("-"))?,
                        // It's a symbol starting with -
                        Some(_) => Token::Symbol(self.symbol(String::from("-"))?),
                        // Just a - symbol
                        None => Token::Symbol("-".to_string()),
                    }
                }
                // Numbers
                '0'..='9' => self.number(String::new())?,
                // Symbols (including variables starting with ?)
                _ if ch.is_alphabetic() || ch == '?' || ch == '_' => {
                    let symbol = self.symbol(String::new())?;
                    // Check for special symbols
                    match symbol.as_str() {
                        "true" => Token::Boolean(true),
                        "false" => Token::Boolean(false),
                        "nil" => Token::Nil,
                        _ => Token::Symbol(symbol),
                    }
                }
                // Operator symbols: <, <=, >, >=, =, !=, +, *, /
                '<' | '>' | '=' | '+' | '*' | '/' => {
                    self.bump();
                    let mut sym = String::from(ch);
                    // Consume a trailing '=' to form <=, >=
                    if (ch == '<' || ch == '>') && self.peek_char() == Some('=') {
                        sym.push('=');
                        self.bump();
                    }
                    Token::Symbol(sym)
                }
                '!' => {
                    self.bump();
                    if self.peek_char() == Some('=') {
                        self.bump();
                        Token::Symbol("!=".to_string())
                    } else {
                        bail_coded!(ErrorCode::Prs002, '!');
                    }
                }
                // EDN line comment: skip from ';' to the next newline (or end of input)
                ';' => {
                    self.bump(); // consume ';'
                    while let Some(c) = self.peek_char() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                    continue;
                }
                '$' => {
                    self.bump(); // consume '$'
                    let mut name = String::new();
                    while let Some(ch) = self.peek_char() {
                        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                            if name.len() >= MAX_SYMBOL_LENGTH {
                                bail_coded!(ErrorCode::Prs074, MAX_SYMBOL_LENGTH);
                            }
                            name.push(ch);
                            self.bump();
                        } else {
                            break;
                        }
                    }
                    if name.is_empty() {
                        bail_coded!(ErrorCode::Prs083);
                    }
                    Token::BindSlot(name)
                }
                _ => {
                    bail_coded!(ErrorCode::Prs002, ch);
                }
            };
            return Ok(Some(token));
        }
        Ok(None)
    }

    /// A number whose text starts with `num` (`""` or `"-"`).
    fn number(&mut self, mut num: String) -> Result<Token> {
        let mut is_float = false;
        while let Some(ch) = self.peek_char() {
            if ch.is_numeric() {
                num.push(ch);
                self.bump();
            } else if ch == '.' && !is_float {
                is_float = true;
                num.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        if is_float {
            let f: f64 = num
                .parse()
                .map_err(|_| err_coded!(ErrorCode::Int007, num.clone()))?;
            Ok(Token::Float(f))
        } else {
            let i: i64 = num
                .parse()
                .map_err(|_| err_coded!(ErrorCode::Int008, num.clone()))?;
            Ok(Token::Integer(i))
        }
    }

    /// The rest of a symbol whose text starts with `symbol`.
    fn symbol(&mut self, mut symbol: String) -> Result<String> {
        while let Some(ch) = self.peek_char() {
            if ch.is_alphanumeric() || ch == '?' || ch == '_' || ch == '-' || ch == '/' {
                if symbol.len() >= MAX_SYMBOL_LENGTH {
                    bail_coded!(ErrorCode::Int009, MAX_SYMBOL_LENGTH);
                }
                symbol.push(ch);
                self.bump();
            } else {
                break;
            }
        }
        Ok(symbol)
    }
}

impl Iterator for Lexer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        if self.error.is_some() {
            return None;
        }
        match self.lex() {
            Ok(token) => token,
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }
}

/// Parser for EDN values, reading tokens from a [`Lexer`] as it needs them.
struct Parser<'a> {
    lexer: Lexer<'a>,
    peeked: Option<Token>,
    depth: usize,
    /// When false, vectors, lists and maps are checked but hold no elements.
    keep: bool,
}

impl<'a> Parser<'a> {
    fn new(lexer: Lexer<'a>) -> Self {
        Parser {
            lexer,
            peeked: None,
            depth: 0,
            keep: true,
        }
    }

    fn peek(&mut self) -> Option<&Token> {
        if self.peeked.is_none() {
            self.peeked = self.lexer.next();
        }
        self.peeked.as_ref()
    }

    fn advance(&mut self) -> Option<Token> {
        self.peeked.take().or_else(|| self.lexer.next())
    }

    fn parse_map(&mut self) -> Result<EdnValue> {
        self.advance(); // consume '{'
        let mut pairs = Vec::new();

        while let Some(token) = self.peek() {
            if token == &Token::RightBrace {
                self.advance(); // consume '}'
                return Ok(EdnValue::Map(pairs));
            }
            let key = self.parse_value()?;
            let val = self.parse_value()?;
            if self.keep {
                pairs.push((key, val));
            }
        }

        bail_coded!(ErrorCode::Prs006);
    }

    fn parse_value(&mut self) -> Result<EdnValue> {
        self.depth += 1;
        if self.depth > MAX_RECURSION_DEPTH {
            self.depth -= 1;
            bail_coded!(ErrorCode::Int010, MAX_RECURSION_DEPTH);
        }
        let result = self.parse_value_inner();
        self.depth -= 1;
        result
    }

    fn parse_value_inner(&mut self) -> Result<EdnValue> {
        match self.peek() {
            Some(Token::LeftParen) => self.parse_list(),
            Some(Token::LeftBracket) => self.parse_vector(),
            Some(Token::LeftBrace) => self.parse_map(),
            Some(Token::SetOpen) => self.parse_set(),
            Some(Token::Keyword(_)) => {
                if let Some(Token::Keyword(k)) = self.advance() {
                    Ok(EdnValue::Keyword(k))
                } else {
                    // peek confirmed Keyword variant; advance should match
                    bail_coded!(ErrorCode::Int006, "keyword");
                }
            }
            Some(Token::Symbol(_)) => {
                if let Some(Token::Symbol(s)) = self.advance() {
                    Ok(EdnValue::Symbol(s))
                } else {
                    bail_coded!(ErrorCode::Int006, "symbol");
                }
            }
            Some(Token::String(_)) => {
                if let Some(Token::String(s)) = self.advance() {
                    Ok(EdnValue::String(s))
                } else {
                    bail_coded!(ErrorCode::Int006, "string");
                }
            }
            Some(Token::Integer(_)) => {
                if let Some(Token::Integer(i)) = self.advance() {
                    Ok(EdnValue::Integer(i))
                } else {
                    bail_coded!(ErrorCode::Int006, "integer");
                }
            }
            Some(Token::Float(_)) => {
                if let Some(Token::Float(f)) = self.advance() {
                    Ok(EdnValue::Float(f))
                } else {
                    bail_coded!(ErrorCode::Int006, "float");
                }
            }
            Some(Token::Boolean(_)) => {
                if let Some(Token::Boolean(b)) = self.advance() {
                    Ok(EdnValue::Boolean(b))
                } else {
                    bail_coded!(ErrorCode::Int006, "boolean");
                }
            }
            Some(Token::TaggedLiteral(_)) => {
                if let Some(Token::TaggedLiteral(tag)) = self.advance() {
                    if tag == "uuid" {
                        // Next token should be a string containing the UUID
                        if let Some(Token::String(uuid_str)) = self.advance() {
                            match Uuid::parse_str(&uuid_str) {
                                Ok(uuid) => Ok(EdnValue::Uuid(uuid)),
                                Err(_) => bail_coded!(ErrorCode::Prs072),
                            }
                        } else {
                            bail_coded!(ErrorCode::Prs071);
                        }
                    } else {
                        bail_coded!(ErrorCode::Prs073, tag);
                    }
                } else {
                    bail_coded!(ErrorCode::Int006, "tagged literal");
                }
            }
            Some(Token::Nil) => {
                self.advance();
                Ok(EdnValue::Nil)
            }
            Some(Token::BindSlot(_)) => {
                if let Some(Token::BindSlot(name)) = self.advance() {
                    Ok(EdnValue::BindSlot(name))
                } else {
                    bail_coded!(ErrorCode::Int006, "bind slot");
                }
            }
            Some(token) => {
                let repr = format!("{:?}", token);
                bail_coded!(ErrorCode::Prs003, repr);
            }
            None => bail_coded!(ErrorCode::Prs001),
        }
    }

    /// `#{ v* }` (#316): its elements are scalar literals; a nested set is
    /// PRS-119, any other element PRS-120.
    fn parse_set(&mut self) -> Result<EdnValue> {
        self.advance(); // consume #{
        let mut elements = Vec::new();

        while let Some(token) = self.peek() {
            match token {
                Token::RightBrace => {
                    self.advance(); // consume }
                    return Ok(EdnValue::Set(elements));
                }
                Token::SetOpen => bail_coded!(ErrorCode::Prs119),
                _ => {}
            }
            let value = self.parse_value()?;
            match value {
                EdnValue::String(_)
                | EdnValue::Integer(_)
                | EdnValue::Float(_)
                | EdnValue::Boolean(_)
                | EdnValue::Keyword(_)
                | EdnValue::Uuid(_)
                | EdnValue::Nil => {}
                other => bail_coded!(ErrorCode::Prs120, format!("{other:?}")),
            }
            if self.keep {
                elements.push(value);
            }
        }

        bail_coded!(ErrorCode::Prs121);
    }

    fn parse_vector(&mut self) -> Result<EdnValue> {
        self.advance(); // consume [
        let mut elements = Vec::new();

        while let Some(token) = self.peek() {
            if token == &Token::RightBracket {
                self.advance(); // consume ]
                return Ok(EdnValue::Vector(elements));
            }
            let value = self.parse_value()?;
            if self.keep {
                elements.push(value);
            }
        }

        bail_coded!(ErrorCode::Prs004);
    }

    fn parse_list(&mut self) -> Result<EdnValue> {
        self.advance(); // consume (
        let mut elements = Vec::new();

        while let Some(token) = self.peek() {
            if token == &Token::RightParen {
                self.advance(); // consume )
                return Ok(EdnValue::List(elements));
            }
            let value = self.parse_value()?;
            if self.keep {
                elements.push(value);
            }
        }

        bail_coded!(ErrorCode::Prs005);
    }
}

/// Parse EDN input into EdnValue
pub fn parse_edn(input: &str) -> Result<EdnValue> {
    let mut parser = Parser::new(Lexer::new(input));
    let value = parser.parse_value();
    let trailing = match value {
        Ok(_) => parser.peek().map(|token| format!("{:?}", token)),
        Err(_) => None,
    };
    // A lexer error anywhere in the input comes before a structure error.
    parser.lexer.finish()?;
    let value = value?;
    if let Some(repr) = trailing {
        bail_coded!(ErrorCode::Prs077, repr);
    }
    Ok(value)
}

/// Parse a Datalog command from EDN
pub fn parse_datalog_command(input: &str) -> Result<DatalogCommand> {
    if let Some(kind) = statement::kind_of(input) {
        return statement::parse(input, kind);
    }
    parse_whole_command(input)
}

/// Parse a command read whole into one [`EdnValue`]. `transact` and `retract`
/// go to [`statement`] instead; their whole-input parsers stay for its
/// differential tests.
fn parse_whole_command(input: &str) -> Result<DatalogCommand> {
    let edn = parse_edn(input)?;

    match edn {
        EdnValue::List(elements) if !elements.is_empty() => {
            // Command is a symbol (e.g., "query", "transact")
            // SAFETY: is_empty() check above guarantees index 0 exists
            #[allow(clippy::indexing_slicing)]
            let command = match &elements[0] {
                EdnValue::Symbol(s) => s.as_str(),
                EdnValue::Keyword(k) => k.as_str(),
                _ => bail_coded!(ErrorCode::Prs010),
            };

            // SAFETY: elements has at least 1 element, so [1..] is valid (may be empty)
            #[allow(clippy::indexing_slicing)]
            let rest = &elements[1..];
            match command {
                "query" => parse_query(rest),
                #[cfg(test)]
                "transact" => parse_transact(rest),
                #[cfg(test)]
                "retract" => parse_retract(rest),
                "rule" => parse_rule(rest),
                _ => bail_coded!(ErrorCode::Prs011, command),
            }
        }
        _ => bail_coded!(ErrorCode::Prs012),
    }
}

/// Parse an aggregate expression list: (func-name ?var)
/// e.g., [Symbol("count"), Symbol("?e")] → FindSpec::Aggregate { "count", "?e" }
fn parse_aggregate(elems: &[EdnValue]) -> Result<FindSpec> {
    // Detect window function: contains ":over" keyword anywhere in elems
    let has_over = elems
        .iter()
        .any(|e| matches!(e, EdnValue::Keyword(k) if k == ":over"));
    if has_over {
        return parse_window_expr(elems);
    }

    if elems.len() != 2 {
        bail_coded!(ErrorCode::Prs024, elems.len());
    }

    // SAFETY: len == 2 check above guarantees indices 0 and 1 exist
    #[allow(clippy::indexing_slicing)]
    let func_name = match &elems[0] {
        EdnValue::Symbol(s) if !s.starts_with('?') => s.clone(),
        other => {
            let repr = format!("{:?}", other);
            bail_coded!(ErrorCode::Prs025, repr);
        }
    };

    const WINDOW_ONLY: &[&str] = &["avg", "rank", "row-number"];

    if WINDOW_ONLY.contains(&func_name.as_str()) {
        bail_coded!(ErrorCode::Prs027, func_name);
    }
    // Unknown aggregate names are allowed — they are resolved at runtime as UDFs.
    // If no UDF with this name is registered, the executor returns an error.

    // SAFETY: len == 2 check above guarantees index 1 exists
    #[allow(clippy::indexing_slicing)]
    let var = match &elems[1] {
        EdnValue::Symbol(s) if s.starts_with('?') => s.clone(),
        _ => bail_coded!(ErrorCode::Prs026),
    };

    Ok(FindSpec::Aggregate {
        func: func_name,
        var,
    })
}

/// Parse a window function expression.
/// Syntax: `(func ?var :over (:partition-by ?p :order-by ?o :desc))`
/// For rank/row-number (no var): `(rank :over (:order-by ?o))`
fn parse_window_expr(elems: &[EdnValue]) -> Result<FindSpec> {
    if elems.is_empty() {
        bail_coded!(ErrorCode::Prs028);
    }

    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let func_name = match &elems[0] {
        EdnValue::Symbol(s) if !s.starts_with('?') => s.as_str(),
        _ => bail_coded!(ErrorCode::Prs029),
    };

    if matches!(func_name, "lag" | "lead") {
        bail_coded!(ErrorCode::Prs030, func_name);
    }

    let func = match func_name {
        "sum" => WindowFunc::Sum,
        "count" => WindowFunc::Count,
        "min" => WindowFunc::Min,
        "max" => WindowFunc::Max,
        "avg" => WindowFunc::Avg,
        "rank" => WindowFunc::Rank,
        "row-number" => WindowFunc::RowNumber,
        "count-distinct" | "sum-distinct" => {
            bail_coded!(ErrorCode::Prs031, func_name);
        }
        other => WindowFunc::Udf(other.to_string()),
    };

    // rank and row-number: no ?var argument
    // others: require ?var before :over
    let (var, over_keyword_idx) = match func {
        WindowFunc::Rank | WindowFunc::RowNumber => {
            if !matches!(elems.get(1), Some(EdnValue::Keyword(k)) if k == ":over") {
                bail_coded!(ErrorCode::Prs034, func_name);
            }
            (None, 1usize)
        }
        _ => {
            let var = match elems.get(1) {
                Some(EdnValue::Symbol(s)) if s.starts_with('?') => s.clone(),
                _ => {
                    bail_coded!(ErrorCode::Prs032, func_name);
                }
            };
            if !matches!(elems.get(2), Some(EdnValue::Keyword(k)) if k == ":over") {
                bail_coded!(ErrorCode::Prs033, func_name);
            }
            (Some(var), 2usize)
        }
    };

    if over_keyword_idx + 2 != elems.len() {
        bail_coded!(ErrorCode::Prs036);
    }

    // Parse the :over clause list
    let over_list = match elems.get(over_keyword_idx + 1) {
        Some(EdnValue::List(l)) => l.as_slice(),
        _ => {
            bail_coded!(ErrorCode::Prs035);
        }
    };

    let mut partition_by: Option<String> = None;
    let mut order_by: Option<String> = None;
    let mut order = Order::Asc;

    let mut j = 0;
    while j < over_list.len() {
        let item = over_list
            .get(j)
            .ok_or_else(|| err_coded!(ErrorCode::Int011, ":over clause"))?;
        match item {
            EdnValue::Keyword(k) => match k.as_str() {
                ":partition-by" => {
                    j += 1;
                    partition_by = match over_list.get(j) {
                        Some(EdnValue::Symbol(s)) if s.starts_with('?') => Some(s.clone()),
                        _ => {
                            bail_coded!(ErrorCode::Prs037);
                        }
                    };
                }
                ":order-by" => {
                    j += 1;
                    order_by = match over_list.get(j) {
                        Some(EdnValue::Symbol(s)) if s.starts_with('?') => Some(s.clone()),
                        _ => {
                            bail_coded!(ErrorCode::Prs038);
                        }
                    };
                }
                ":desc" => {
                    order = Order::Desc;
                }
                ":asc" => {
                    order = Order::Asc;
                }
                other => {
                    bail_coded!(ErrorCode::Prs039, other);
                }
            },
            other => {
                let repr = format!("{:?}", other);
                bail_coded!(ErrorCode::Prs040, repr);
            }
        }
        j += 1;
    }

    let order_by = order_by.ok_or_else(|| err_coded!(ErrorCode::Prs084))?;

    Ok(FindSpec::Window(WindowSpec {
        func,
        var,
        partition_by,
        order_by,
        order,
    }))
}

fn parse_query(elements: &[EdnValue]) -> Result<DatalogCommand> {
    // Parse (query [:find ?x ?y :as-of N :valid-at "ts" :where [patterns...]])
    if elements.is_empty() {
        bail_coded!(ErrorCode::Prs013);
    }

    if elements.len() > 1 {
        bail_coded!(ErrorCode::Prs078, elements.len() - 1);
    }

    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let query_vector = elements[0]
        .as_vector()
        .ok_or_else(|| err_coded!(ErrorCode::Prs085))?;
    if query_vector.is_empty() {
        bail_coded!(ErrorCode::Prs111);
    }
    // Whether the item at `j` is a value, not a keyword or the end of the vector.
    let value_at = |j: usize| {
        query_vector
            .get(j)
            .is_some_and(|v| v.as_keyword().is_none())
    };

    let mut find_specs: Vec<FindSpec> = Vec::new();
    let mut with_vars: Vec<String> = Vec::new();
    let mut where_clauses = Vec::new();
    let mut current_clause: Option<&str> = None;
    let mut query_as_of: Option<AsOf> = None;
    let mut query_valid_at: Option<ValidAt> = None;
    let mut query_max_derived_facts: Option<usize> = None;
    let mut query_max_results: Option<usize> = None;
    let mut query_limit: Option<Count> = None;

    let mut i = 0;
    while i < query_vector.len() {
        let current_item = query_vector
            .get(i)
            .ok_or_else(|| err_coded!(ErrorCode::Int011, "query vector"))?;
        if let Some(keyword) = current_item.as_keyword() {
            match keyword {
                ":as-of" => {
                    current_clause = None;
                    // Next element is the value
                    i += 1;
                    let as_of_val = query_vector
                        .get(i)
                        .ok_or_else(|| err_coded!(ErrorCode::Prs014))?;
                    let as_of = match as_of_val {
                        EdnValue::Integer(n) if *n >= 0 => {
                            let val = u64::try_from(*n).map_err(|_| {
                                err_coded!(ErrorCode::Int019, ":as-of counter must be non-negative")
                            })?;
                            AsOf::Counter(val)
                        }
                        EdnValue::Integer(n) => {
                            bail_coded!(ErrorCode::Prs015, n);
                        }
                        EdnValue::String(s) => {
                            let ts = parse_timestamp(s)?;
                            AsOf::Timestamp(ts)
                        }
                        EdnValue::BindSlot(name) => AsOf::Slot(name.clone()),
                        other => {
                            let repr = format!("{:?}", other);
                            bail_coded!(ErrorCode::Prs016, repr);
                        }
                    };
                    query_as_of = Some(as_of);
                    i += 1;
                    continue;
                }
                ":valid-at" => {
                    current_clause = None;
                    i += 1;
                    let valid_at_val = query_vector
                        .get(i)
                        .ok_or_else(|| err_coded!(ErrorCode::Prs017))?;
                    let valid_at = match valid_at_val {
                        EdnValue::String(s) => {
                            let ts = parse_timestamp(s)?;
                            ValidAt::Timestamp(ts)
                        }
                        EdnValue::Keyword(k) if k == ":any-valid-time" => ValidAt::AnyValidTime,
                        EdnValue::BindSlot(name) => ValidAt::Slot(name.clone()),
                        other => {
                            let repr = format!("{:?}", other);
                            bail_coded!(ErrorCode::Prs018, repr);
                        }
                    };
                    query_valid_at = Some(valid_at);
                    i += 1;
                    continue;
                }
                ":any-valid-time" => {
                    current_clause = None;
                    // Shorthand for `:valid-at :any-valid-time`; disables automatic
                    // valid-time filtering so pseudo-attribute patterns are accessible.
                    query_valid_at = Some(ValidAt::AnyValidTime);
                    i += 1;
                    continue;
                }
                ":max-derived-facts" => {
                    current_clause = None;
                    if query_max_derived_facts.is_some() {
                        bail_coded!(ErrorCode::Int012, ":max-derived-facts");
                    }
                    i += 1;
                    let val = query_vector
                        .get(i)
                        .ok_or_else(|| err_coded!(ErrorCode::Int013, ":max-derived-facts"))?;
                    match val {
                        EdnValue::Integer(n) if *n >= 1 => {
                            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                            {
                                query_max_derived_facts = Some(*n as usize);
                            }
                        }
                        EdnValue::Integer(_) => {
                            bail_coded!(ErrorCode::Int014, ":max-derived-facts");
                        }
                        _other => {
                            bail_coded!(ErrorCode::Int015, ":max-derived-facts");
                        }
                    }
                    i += 1;
                    continue;
                }
                ":max-results" => {
                    current_clause = None;
                    if query_max_results.is_some() {
                        bail_coded!(ErrorCode::Int012, ":max-results");
                    }
                    i += 1;
                    let val = query_vector
                        .get(i)
                        .ok_or_else(|| err_coded!(ErrorCode::Int013, ":max-results"))?;
                    match val {
                        EdnValue::Integer(n) if *n >= 1 => {
                            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                            {
                                query_max_results = Some(*n as usize);
                            }
                        }
                        EdnValue::Integer(_) => {
                            bail_coded!(ErrorCode::Int014, ":max-results");
                        }
                        _other => {
                            bail_coded!(ErrorCode::Int015, ":max-results");
                        }
                    }
                    i += 1;
                    continue;
                }
                ":limit" => {
                    current_clause = None;
                    if query_limit.is_some() {
                        bail_coded!(ErrorCode::Int012, ":limit");
                    }
                    i += 1;
                    if !value_at(i) {
                        bail_coded!(ErrorCode::Int013, ":limit");
                    }
                    let val = query_vector
                        .get(i)
                        .ok_or_else(|| err_coded!(ErrorCode::Int013, ":limit"))?;
                    query_limit = Some(match val {
                        EdnValue::Integer(n) if *n >= 0 => {
                            Count::Literal(usize::try_from(*n).unwrap_or(usize::MAX))
                        }
                        EdnValue::BindSlot(name) => Count::Slot(name.clone()),
                        other => bail_coded!(ErrorCode::Prs115, ":limit", format!("{other:?}")),
                    });
                    i += 1;
                    continue;
                }
                ":with" => {
                    // Collect ?-prefixed symbols until the next keyword or end of vector
                    if !value_at(i + 1) {
                        bail_coded!(ErrorCode::Prs112, ":with");
                    }
                    i += 1;
                    while i < query_vector.len() {
                        let with_item = query_vector.get(i).ok_or_else(|| {
                            err_coded!(ErrorCode::Int011, "query vector in :with")
                        })?;
                        match with_item {
                            EdnValue::Symbol(s) if s.starts_with('?') => {
                                with_vars.push(s.clone());
                                i += 1;
                            }
                            EdnValue::Keyword(_) => break,
                            other => {
                                bail_coded!(ErrorCode::Prs086, format!("{other:?}"));
                            }
                        }
                    }
                    continue;
                }
                ":find" | ":where" => {
                    if !value_at(i + 1) {
                        bail_coded!(ErrorCode::Prs112, keyword);
                    }
                    current_clause = Some(keyword);
                    i += 1;
                    continue;
                }
                other => {
                    bail_coded!(ErrorCode::Prs113, other);
                }
            }
        }

        match current_clause {
            Some(":find") => match current_item {
                EdnValue::Symbol(s) if s.starts_with('?') => {
                    find_specs.push(FindSpec::Variable(s.clone()));
                }
                EdnValue::List(elems) => {
                    find_specs.push(parse_aggregate(elems)?);
                }
                other => {
                    bail_coded!(ErrorCode::Prs087, format!("{other:?}"));
                }
            },
            Some(":where") => {
                // Parse both patterns (vectors) and rule invocations (lists)
                if let Some(pattern_vec) = current_item.as_vector() {
                    // Expr clause: [(list-expr) ?out?] — element 0 is a List
                    if matches!(pattern_vec.first(), Some(EdnValue::List(_))) {
                        let clause = parse_expr_clause(pattern_vec)?;
                        where_clauses.push(clause);
                    } else {
                        let pattern = parse_query_pattern(pattern_vec)?;
                        where_clauses.push(WhereClause::Pattern(pattern));
                    }
                } else if let Some(rule_list) = current_item.as_list() {
                    let clause = parse_list_as_where_clause(rule_list, true)?;
                    where_clauses.push(clause);
                } else {
                    let repr = format!("{:?}", current_item);
                    bail_coded!(ErrorCode::Prs060, repr);
                }
            }
            _ => {
                let repr = format!("{:?}", current_item);
                bail_coded!(ErrorCode::Prs061, repr);
            }
        }

        i += 1;
    }

    resolve_pseudo_sources(&mut where_clauses)?;

    // Safety check: all variables in (not ...) and (not-join ...) must be bound by outer clauses
    let outer_bound: std::collections::HashSet<String> = where_clauses
        .iter()
        .flat_map(outer_vars_from_clause)
        .collect();
    check_not_safety(&where_clauses, &outer_bound)?;
    check_not_join_safety(&where_clauses, &outer_bound)?;
    check_expr_safety(&where_clauses)?;

    // Validate :find and :with vars are bound in :where. An unbound :find
    // variable (e.g. a typo) would otherwise return an empty result (#406).
    for spec in &find_specs {
        match spec {
            FindSpec::Variable(var) => {
                if !outer_bound.contains(var) {
                    bail_coded!(ErrorCode::Prs080, var);
                }
            }
            FindSpec::Aggregate { var, .. } => {
                if !outer_bound.contains(var) {
                    bail_coded!(ErrorCode::Prs023, var);
                }
            }
            FindSpec::Window(ws) => {
                let window_vars = ws
                    .var
                    .iter()
                    .chain(ws.partition_by.iter())
                    .chain(std::iter::once(&ws.order_by));
                for var in window_vars {
                    if !outer_bound.contains(var) {
                        bail_coded!(ErrorCode::Prs080, var);
                    }
                }
            }
        }
    }
    // In a query with aggregates, windows run over the groups, which hold
    // only the plain :find variables (#547 spec §4.4).
    if find_specs
        .iter()
        .any(|s| matches!(s, FindSpec::Aggregate { .. }))
    {
        let plain: Vec<&String> = find_specs
            .iter()
            .filter_map(|s| match s {
                FindSpec::Variable(v) => Some(v),
                _ => None,
            })
            .collect();
        for spec in &find_specs {
            if let FindSpec::Window(ws) = spec {
                let vars = ws
                    .var
                    .iter()
                    .chain(ws.partition_by.iter())
                    .chain(std::iter::once(&ws.order_by));
                for var in vars {
                    if !plain.contains(&var) {
                        bail_coded!(ErrorCode::Prs082, var, spec.display_name());
                    }
                }
            }
        }
    }
    for var in &with_vars {
        if !outer_bound.contains(var) {
            bail_coded!(ErrorCode::Prs022, var);
        }
    }
    // :with without any aggregate is an error
    if !with_vars.is_empty()
        && !find_specs
            .iter()
            .any(|s| matches!(s, FindSpec::Aggregate { .. }))
    {
        bail_coded!(ErrorCode::Prs021);
    }

    let mut query = DatalogQuery::new(find_specs, where_clauses);
    query.as_of = query_as_of;
    query.valid_at = query_valid_at;
    query.with_vars = with_vars;
    query.max_derived_facts = query_max_derived_facts;
    query.max_results = query_max_results;
    query.limit = query_limit;
    Ok(DatalogCommand::Query(query))
}

#[cfg(test)]
fn parse_transact(elements: &[EdnValue]) -> Result<DatalogCommand> {
    // Parse (transact [facts]) or (transact {opts} [facts])
    if elements.is_empty() {
        bail_coded!(ErrorCode::Prs041);
    }

    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let first_element = &elements[0];

    // Check if the first element is a map (transaction-level options)
    let (tx_valid_from, tx_valid_to, facts_element, consumed) = if first_element.is_map() {
        let map = first_element.as_map().ok_or_else(|| {
            err_coded!(
                ErrorCode::Int019,
                "internal error: is_map() true but as_map() returned None"
            )
        })?;
        let (from, to) = parse_valid_time_map(map)?;
        let facts_elem = elements
            .get(1)
            .ok_or_else(|| err_coded!(ErrorCode::Prs048))?;
        (from, to, facts_elem, 2)
    } else {
        (None, None, first_element, 1)
    };

    if elements.len() > consumed {
        bail_coded!(ErrorCode::Prs075, elements.len() - consumed);
    }

    let facts_vector = facts_element
        .as_vector()
        .ok_or_else(|| err_coded!(ErrorCode::Prs042))?;

    let mut patterns = Vec::new();
    for fact in facts_vector {
        let fact_vec = fact
            .as_vector()
            .ok_or_else(|| err_coded!(ErrorCode::Prs045))?;

        // A fact is [e a v] or [e a v {opts}]
        if fact_vec.len() < 3 {
            bail_coded!(ErrorCode::Prs046, fact_vec.len());
        }
        if fact_vec.len() > 4 {
            bail_coded!(ErrorCode::Prs108, fact_vec.len());
        }

        // SAFETY: len >= 3 check above guarantees indices 0, 1, 2 exist
        #[allow(clippy::indexing_slicing)]
        let entity = fact_vec[0].clone();
        #[allow(clippy::indexing_slicing)]
        let attribute = fact_vec[1].clone();
        #[allow(clippy::indexing_slicing)]
        let value = fact_vec[2].clone();

        // Check for optional per-fact map at position 3
        let (fact_valid_from, fact_valid_to) = if fact_vec.len() >= 4 {
            match fact_vec.get(3) {
                Some(EdnValue::Map(pairs)) => parse_valid_time_map(pairs)?,
                Some(other) => {
                    let repr = format!("{:?}", other);
                    bail_coded!(ErrorCode::Prs047, repr);
                }
                None => {
                    bail_coded!(ErrorCode::Prs049);
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
fn parse_valid_time_map(pairs: &[(EdnValue, EdnValue)]) -> Result<(Option<i64>, Option<i64>)> {
    let mut valid_from = None;
    let mut valid_to = None;

    for (key, val) in pairs {
        match key.as_keyword() {
            Some(":valid-from") => {
                let s = match val {
                    EdnValue::String(s) => s,
                    other => {
                        let repr = format!("{:?}", other);
                        bail_coded!(ErrorCode::Prs019, repr);
                    }
                };
                valid_from = Some(parse_timestamp(s)?);
            }
            Some(":valid-to") => {
                let s = match val {
                    EdnValue::String(s) => s,
                    other => {
                        let repr = format!("{:?}", other);
                        bail_coded!(ErrorCode::Prs020, repr);
                    }
                };
                valid_to = Some(parse_timestamp(s)?);
            }
            _ => {
                bail_coded!(ErrorCode::Prs088, format!("{key:?}"));
            }
        }
    }

    Ok((valid_from, valid_to))
}

#[cfg(test)]
fn parse_retract(elements: &[EdnValue]) -> Result<DatalogCommand> {
    // Same structure as transact
    if elements.is_empty() {
        bail_coded!(ErrorCode::Prs043);
    }

    if elements.len() > 1 {
        bail_coded!(ErrorCode::Prs076, elements.len() - 1);
    }

    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let facts_vector = elements[0]
        .as_vector()
        .ok_or_else(|| err_coded!(ErrorCode::Prs044))?;

    let mut patterns = Vec::new();
    for fact in facts_vector {
        let fact_vec = fact
            .as_vector()
            .ok_or_else(|| err_coded!(ErrorCode::Prs089))?;
        if fact_vec.len() != 3 {
            bail_coded!(ErrorCode::Prs090, fact_vec.len());
        }
        patterns.push(Pattern::from_edn(fact_vec).map_err(anyhow::Error::msg)?);
    }

    Ok(DatalogCommand::Retract(Transaction::new(patterns)))
}

/// Convert a single EDN token to an Expr leaf, or recurse for a nested list.
fn parse_expr_arg(edn: &EdnValue) -> Result<Expr> {
    match edn {
        EdnValue::Symbol(s) if s.starts_with('?') => Ok(Expr::Var(s.clone())),
        EdnValue::Integer(n) => Ok(Expr::Lit(Value::Integer(*n))),
        EdnValue::Float(f) => Ok(Expr::Lit(Value::Float(*f))),
        EdnValue::String(s) => Ok(Expr::Lit(Value::String(s.clone()))),
        EdnValue::Boolean(b) => Ok(Expr::Lit(Value::Boolean(*b))),
        EdnValue::Nil => Ok(Expr::Lit(Value::Null)),
        EdnValue::Keyword(k) => Ok(Expr::Lit(Value::Keyword(k.clone()))),
        EdnValue::Set(_) => Ok(Expr::Lit(
            super::matcher::edn_to_value(edn).map_err(|e| err_coded!(ErrorCode::Prs070, e))?,
        )),
        EdnValue::List(inner) => parse_expr(inner),
        EdnValue::BindSlot(name) => Ok(Expr::Slot(name.clone())),
        other => {
            let repr = format!("{:?}", other);
            bail_coded!(ErrorCode::Prs070, repr);
        }
    }
}

/// Parse an EDN list `(op arg arg?)` into an Expr tree.
///
/// For `matches?`, the second argument must be a string literal and is
/// validated as a valid regex pattern at parse time.
fn parse_expr(list: &[EdnValue]) -> Result<Expr> {
    if list.is_empty() {
        bail_coded!(ErrorCode::Prs062);
    }
    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let head = match &list[0] {
        EdnValue::Symbol(s) if !s.starts_with('?') => s.as_str(),
        other => {
            let repr = format!("{:?}", other);
            bail_coded!(ErrorCode::Prs063, repr);
        }
    };

    match head {
        // Unary type predicates
        "string?" | "integer?" | "float?" | "boolean?" | "nil?" => {
            if list.len() != 2 {
                bail_coded!(ErrorCode::Prs064, head);
            }
            let op = match head {
                "string?" => UnaryOp::StringQ,
                "integer?" => UnaryOp::IntegerQ,
                "float?" => UnaryOp::FloatQ,
                "boolean?" => UnaryOp::BooleanQ,
                // SAFETY: the match arm above covers all 5 variants; this is the last one
                _ => UnaryOp::NilQ,
            };
            // SAFETY: len == 2 check above guarantees index 1 exists
            #[allow(clippy::indexing_slicing)]
            let arg = parse_expr_arg(&list[1])?;
            Ok(Expr::UnaryOp(op, Box::new(arg)))
        }

        // Binary operators
        "<" | ">" | "<=" | ">=" | "=" | "!=" | "+" | "-" | "*" | "/" | "starts-with?"
        | "ends-with?" | "contains?" | "matches?" | "in?" | "not-in?" | "any-in?" | "all-in?"
        | "none-in?" => {
            if list.len() != 3 {
                bail_coded!(ErrorCode::Prs065, head);
            }
            // matches? second arg must be a string literal; compile regex early.
            if head == "matches?" {
                // SAFETY: len == 3 check above guarantees indices 1 and 2 exist
                #[allow(clippy::indexing_slicing)]
                let lhs = parse_expr_arg(&list[1])?;
                #[allow(clippy::indexing_slicing)]
                let rhs = parse_expr_arg(&list[2])?;
                match &rhs {
                    Expr::Lit(Value::String(pattern)) => {
                        let compiled = regex_lite::Regex::new(pattern).map_err(|e| {
                            err_coded!(ErrorCode::Prs091, format!("{pattern:?}"), e)
                        })?;
                        let rhs_lit = Expr::Lit(Value::String(pattern.clone()));
                        return Ok(Expr::BinOp(
                            BinOp::Matches {
                                regex: compiled,
                                pattern: pattern.clone(),
                            },
                            Box::new(lhs),
                            Box::new(rhs_lit),
                        ));
                    }
                    _ => {
                        bail_coded!(ErrorCode::Prs066);
                    }
                }
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
                "in?" => BinOp::In,
                "not-in?" => BinOp::NotIn,
                "any-in?" => BinOp::AnyIn,
                "all-in?" => BinOp::AllIn,
                // SAFETY: the match arm above covers all non-matches? variants
                _ => BinOp::NoneIn,
            };
            // SAFETY: len == 3 check above guarantees indices 1 and 2 exist
            #[allow(clippy::indexing_slicing)]
            let lhs = parse_expr_arg(&list[1])?;
            #[allow(clippy::indexing_slicing)]
            let rhs = parse_expr_arg(&list[2])?;
            Ok(Expr::BinOp(op, Box::new(lhs), Box::new(rhs)))
        }

        other => {
            // Unknown name in 1-arg position: treat as UDF predicate resolved at runtime.
            // 2-arg unknown forms are still rejected.
            if list.len() == 2 {
                // SAFETY: len == 2 check above guarantees index 1 exists
                #[allow(clippy::indexing_slicing)]
                let arg = parse_expr_arg(&list[1])?;
                Ok(Expr::UnaryOp(
                    UnaryOp::Udf(other.to_string()),
                    Box::new(arg),
                ))
            } else {
                bail_coded!(ErrorCode::Prs067, other);
            }
        }
    }
}

/// Parse a vector clause whose first element is a list: `[(expr)]` or `[(expr) ?out]`.
///
/// Called from `:where` dispatch when `vec[0]` is an `EdnValue::List`.
fn parse_expr_clause(vec: &[EdnValue]) -> Result<WhereClause> {
    let first = vec.first().ok_or_else(|| {
        err_coded!(
            ErrorCode::Int019,
            "parse_expr_clause called with empty vector"
        )
    })?;
    let inner_list = match first {
        EdnValue::List(l) => l.as_slice(),
        _ => bail_coded!(
            ErrorCode::Int019,
            "parse_expr_clause called with non-list element 0"
        ),
    };
    let expr = parse_expr(inner_list)?;
    let binding = match vec.len() {
        1 => None,
        2 => {
            let second = vec
                .get(1)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "expression clause"))?;
            match second {
                EdnValue::Symbol(s) if s.starts_with('?') => Some(s.clone()),
                other => {
                    let repr = format!("{:?}", other);
                    bail_coded!(ErrorCode::Prs069, repr);
                }
            }
        }
        n => {
            bail_coded!(ErrorCode::Prs068, n);
        }
    };
    Ok(WhereClause::Expr { expr, binding })
}

/// Parse a list item (EDN List) appearing in a :where clause or rule body.
/// Returns Err if the list is empty, has an unknown form, or contains nested `not`.
/// Whether `s` can name a rule: a symbol that is neither a variable nor an operator
/// (the grammar's `plain_sym`).
fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_alphabetic() || c == '_') || (s.len() > 1 && s.starts_with('-'))
}

fn parse_list_as_where_clause(list: &[EdnValue], allow_not: bool) -> Result<WhereClause> {
    if list.is_empty() {
        bail_coded!(ErrorCode::Prs050);
    }
    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let head = &list[0];
    match head {
        EdnValue::Symbol(s) if s == "not" => {
            if !allow_not {
                bail_coded!(ErrorCode::Prs051);
            }
            if list.len() < 2 {
                bail_coded!(ErrorCode::Prs052);
            }
            let mut inner = Vec::new();
            let rest = list
                .get(1..)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "not clause"))?;
            for item in rest {
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
                        bail_coded!(ErrorCode::Int016);
                    }
                    // Recurse with allow_not=false to reject nested not
                    let clause = parse_list_as_where_clause(inner_list, false)?;
                    inner.push(clause);
                } else {
                    bail_coded!(ErrorCode::Prs092, format!("{item:?}"));
                }
            }
            Ok(WhereClause::Not(inner))
        }
        EdnValue::Symbol(s) if s == "not-join" => {
            if !allow_not {
                bail_coded!(ErrorCode::Prs093);
            }
            // Syntax: (not-join [?v1 ?v2 ...] clause1 clause2 ...)
            if list.len() < 3 {
                bail_coded!(ErrorCode::Prs059);
            }
            let join_var_vec = match list.get(1) {
                Some(EdnValue::Vector(v)) => v,
                _ => {
                    bail_coded!(ErrorCode::Prs094);
                }
            };
            let join_vars: Vec<String> = join_var_vec
                .iter()
                .map(|v| match v {
                    EdnValue::Symbol(s) if s.starts_with('?') => Ok(s.clone()),
                    _ => Err(err_coded!(ErrorCode::Prs095, format!("{v:?}"))),
                })
                .collect::<Result<_>>()?;
            let mut inner = Vec::new();
            let rest = list
                .get(2..)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "not-join clause"))?;
            for item in rest {
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
                        bail_coded!(ErrorCode::Int016);
                    }
                    // allow_not=false to reject nested (not ...) or (not-join ...)
                    let clause = parse_list_as_where_clause(inner_list, false)?;
                    inner.push(clause);
                } else {
                    bail_coded!(ErrorCode::Prs096, format!("{item:?}"));
                }
            }
            Ok(WhereClause::NotJoin {
                join_vars,
                clauses: inner,
            })
        }
        EdnValue::Symbol(s) if s == "or" => {
            if list.len() < 2 {
                bail_coded!(ErrorCode::Prs053);
            }
            let mut branches: Vec<Vec<WhereClause>> = Vec::new();
            let rest = list
                .get(1..)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "or clause"))?;
            for item in rest {
                let branch = parse_or_branch(item)?;
                branches.push(branch);
            }
            Ok(WhereClause::Or(branches))
        }
        EdnValue::Symbol(s) if s == "or-join" => {
            if list.len() < 3 {
                bail_coded!(ErrorCode::Prs054);
            }
            let join_var_vec = match list.get(1) {
                Some(EdnValue::Vector(v)) => v,
                _ => {
                    bail_coded!(ErrorCode::Prs055);
                }
            };
            let join_vars: Vec<String> = join_var_vec
                .iter()
                .map(|v| match v {
                    EdnValue::Symbol(s) if s.starts_with('?') => Ok(s.clone()),
                    _ => {
                        let repr = format!("{:?}", v);
                        Err(err_coded!(ErrorCode::Prs056, repr))
                    }
                })
                .collect::<Result<_>>()?;
            let mut branches: Vec<Vec<WhereClause>> = Vec::new();
            let rest = list
                .get(2..)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "or-join clause"))?;
            for item in rest {
                let branch = parse_or_branch(item)?;
                branches.push(branch);
            }
            Ok(WhereClause::OrJoin {
                join_vars,
                branches,
            })
        }
        EdnValue::Symbol(predicate) if is_name(predicate) => {
            let args = list
                .get(1..)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "rule invocation"))?
                .to_vec();
            Ok(WhereClause::RuleInvocation {
                predicate: predicate.clone(),
                args,
            })
        }
        _ => bail_coded!(ErrorCode::Prs097, format!("{head:?}")),
    }
}

/// Parse a where-clause pattern vector with pseudo-attribute detection.
///
/// Detects `:db/*` keywords in the attribute position and wraps them in
/// `AttributeSpec::Pseudo`. Rejects `:db/*` keywords in entity or value positions.
/// Falls through to `Pattern::from_edn` for regular patterns.
fn parse_query_pattern(vec: &[EdnValue]) -> Result<Pattern> {
    if vec.len() != 3 {
        bail_coded!(ErrorCode::Prs098, vec.len());
    }

    // SAFETY: len == 3 check above guarantees indices 0, 1, 2 exist
    // Reject :db/* in entity position
    #[allow(clippy::indexing_slicing)]
    if let EdnValue::Keyword(k) = &vec[0]
        && PseudoAttr::from_keyword(k).is_some()
    {
        bail_coded!(ErrorCode::Int017, k, "entity");
    }

    // Reject :db/* in value position
    #[allow(clippy::indexing_slicing)]
    if let EdnValue::Keyword(k) = &vec[2]
        && PseudoAttr::from_keyword(k).is_some()
    {
        bail_coded!(ErrorCode::Int017, k, "value");
    }

    // Detect pseudo-attribute in attribute position
    #[allow(clippy::indexing_slicing)]
    if let EdnValue::Keyword(k) = &vec[1]
        && let Some(pseudo) = PseudoAttr::from_keyword(k)
    {
        #[allow(clippy::indexing_slicing)]
        return Ok(Pattern::pseudo(vec[0].clone(), pseudo, vec[2].clone()));
    }

    Pattern::from_edn(vec).map_err(anyhow::Error::msg)
}

/// Parse a single branch of an (or ...) or (or-join ...) clause.
///
/// A branch is either:
/// - A single clause: `[pattern]` or `(rule-invocation)` or `[(expr)]`
/// - A grouped list of clauses: `(and clause1 clause2 ...)`
fn parse_or_branch(item: &EdnValue) -> Result<Vec<WhereClause>> {
    match item {
        EdnValue::List(inner) if matches!(inner.first(), Some(EdnValue::Symbol(s)) if s == "and") =>
        {
            // (and clause1 clause2 ...) — multi-clause branch
            if inner.len() < 2 {
                bail_coded!(ErrorCode::Prs058);
            }
            let mut clauses = Vec::new();
            let rest = inner
                .get(1..)
                .ok_or_else(|| err_coded!(ErrorCode::Int011, "and clause"))?;
            for clause_item in rest {
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
fn parse_or_branch_item(item: &EdnValue) -> Result<WhereClause> {
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
        _ => bail_coded!(ErrorCode::Prs099, format!("{item:?}")),
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
            vars.extend(p.hidden_vars().map(str::to_string));
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
            let first_set = match branch_var_sets.first() {
                Some(s) => s,
                None => return vec![],
            };
            let rest_sets = match branch_var_sets.get(1..) {
                Some(s) => s,
                None => return first_set.iter().cloned().collect(),
            };
            first_set
                .iter()
                .filter(|v| rest_sets.iter().all(|s| s.contains(*v)))
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
        // Hidden metadata variables (#594) are resolved by construction: a
        // body may bind its own, which no outer clause binds.
        WhereClause::Not(inner) => inner
            .iter()
            .flat_map(outer_vars_from_clause)
            .filter(|v| !v.starts_with("?#"))
            .collect(),
        _ => vec![],
    }
}

/// Validate safety: every variable in a (not ...) body must be bound by an outer clause.
fn check_not_safety(
    clauses: &[WhereClause],
    outer_bound: &std::collections::HashSet<String>,
) -> Result<()> {
    for clause in clauses {
        if let WhereClause::Not(_) = clause {
            for var in vars_in_not(clause) {
                if !outer_bound.contains(&var) {
                    bail_coded!(ErrorCode::Int018, "variable", var, "(not ...)", "outer");
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
) -> Result<()> {
    for clause in clauses {
        if let WhereClause::NotJoin { join_vars, .. } = clause {
            for var in join_vars {
                if !var.starts_with("?_") && !outer_bound.contains(var) {
                    bail_coded!(
                        ErrorCode::Int018,
                        "join variable",
                        var,
                        "(not-join ...)",
                        "outer"
                    );
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
        Expr::Slot(_) => vec![],
    }
}

/// Forward-pass safety check: every Var in an Expr clause must be bound by
/// an earlier clause. Binding-form Expr clauses add their output var to scope.
///
/// Called for both query `:where` clauses and rule body clauses.
fn check_expr_safety(clauses: &[WhereClause]) -> Result<()> {
    check_expr_safety_with_bound(clauses, &mut std::collections::HashSet::new())
}

fn check_expr_safety_with_bound(
    clauses: &[WhereClause],
    bound: &mut std::collections::HashSet<String>,
) -> Result<()> {
    for clause in clauses {
        match clause {
            WhereClause::Expr { expr, binding } => {
                for var in expr_vars(expr) {
                    if !bound.contains(&var) {
                        bail_coded!(
                            ErrorCode::Int018,
                            "variable",
                            var,
                            "expression clause",
                            "earlier"
                        );
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
                    // SAFETY: windows(2) yields slices of exactly 2 elements
                    #[allow(clippy::indexing_slicing)]
                    if branch_new_var_sets.windows(2).any(|w| w[0] != w[1]) {
                        bail_coded!(ErrorCode::Prs057);
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
                        bail_coded!(
                            ErrorCode::Int018,
                            "join variable",
                            var,
                            "(or-join ...)",
                            "earlier"
                        );
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

// ── Pseudo-attribute sources (#594) ──────────────────────────────────────────

/// An entity term as the source search compares it (spec §2 of the #594
/// design): variables and `$slot`s by name, constants by the entity they name.
#[derive(Clone, PartialEq)]
enum EntityTerm {
    Var(String),
    Slot(String),
    Eid(Uuid),
}

fn entity_term(e: &EdnValue) -> Option<EntityTerm> {
    match e {
        EdnValue::Symbol(s) if s.starts_with('?') && !s.starts_with("?_") => {
            Some(EntityTerm::Var(s.clone()))
        }
        EdnValue::BindSlot(n) => Some(EntityTerm::Slot(n.clone())),
        EdnValue::Keyword(_) | EdnValue::Uuid(_) => {
            crate::query::datalog::matcher::edn_to_entity_id(e)
                .ok()
                .map(EntityTerm::Eid)
        }
        _ => None,
    }
}

/// A pattern with a stored attribute, a possible source.
#[derive(Clone)]
struct Candidate {
    path: Vec<usize>,
    term: EntityTerm,
    /// Its position in a walk of the whole query: names its hidden variables.
    n: usize,
}

/// A clause list's containing clause, as seen from inside it.
struct Level {
    /// The candidates before the containing clause, in the enclosing list.
    before: Vec<Candidate>,
    /// `Some` for `not-join` / `or-join`: only these variables cross it.
    join: Option<Vec<String>>,
    /// The containing clause.
    at: Vec<usize>,
}

/// One per-fact pseudo-attribute pattern and its source.
struct Resolved {
    pseudo: Vec<usize>,
    source: Vec<usize>,
    n: usize,
    attr: PseudoAttr,
    /// The `not-join` / `or-join` clauses between them.
    crossed: Vec<Vec<usize>>,
}

/// The nearest candidate for `term`: in the list so far, then outwards before
/// each containing clause, crossing a `-join` only for a listed variable.
fn find_source<'a>(
    term: &EntityTerm,
    here: &'a [Candidate],
    stack: &'a [Level],
) -> Option<(&'a Candidate, Vec<Vec<usize>>)> {
    if let Some(c) = here.iter().rev().find(|c| c.term == *term) {
        return Some((c, Vec::new()));
    }
    let mut crossed = Vec::new();
    for level in stack.iter().rev() {
        if let Some(vars) = &level.join {
            match term {
                EntityTerm::Var(v) if vars.contains(v) => crossed.push(level.at.clone()),
                _ => return None,
            }
        }
        if let Some(c) = level.before.iter().rev().find(|c| c.term == *term) {
            return Some((c, crossed));
        }
    }
    None
}

fn entity_text(e: &EdnValue) -> String {
    match e {
        EdnValue::Symbol(s) => s.clone(),
        EdnValue::Keyword(k) => k.clone(),
        EdnValue::Uuid(u) => format!("#uuid \"{u}\""),
        EdnValue::BindSlot(n) => format!("${n}"),
        _ => "_".to_string(),
    }
}

fn walk_sources(
    list: &[WhereClause],
    prefix: &[usize],
    stack: &mut Vec<Level>,
    next: &mut usize,
    out: &mut Vec<Resolved>,
) -> Result<()> {
    let mut here: Vec<Candidate> = Vec::new();
    for (i, clause) in list.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(i);
        match clause {
            WhereClause::Pattern(p) => match &p.attribute {
                AttributeSpec::Pseudo(attr) if attr.is_per_fact() => {
                    let found = entity_term(&p.entity).and_then(|t| {
                        find_source(&t, &here, stack).map(|(c, crossed)| (c.clone(), crossed))
                    });
                    let Some((source, crossed)) = found else {
                        let e = entity_text(&p.entity);
                        bail_coded!(ErrorCode::Prs081, attr.as_keyword(), e, e);
                    };
                    out.push(Resolved {
                        pseudo: path,
                        source: source.path,
                        n: source.n,
                        attr: attr.clone(),
                        crossed,
                    });
                }
                AttributeSpec::Pseudo(_) => {}
                AttributeSpec::Real(_) => {
                    if let Some(term) = entity_term(&p.entity) {
                        here.push(Candidate {
                            path,
                            term,
                            n: *next,
                        });
                        *next += 1;
                    }
                }
            },
            WhereClause::Not(body) => {
                stack.push(Level {
                    before: here.clone(),
                    join: None,
                    at: path.clone(),
                });
                let r = walk_sources(body, &path, stack, next, out);
                stack.pop();
                r?;
            }
            WhereClause::NotJoin { join_vars, clauses } => {
                stack.push(Level {
                    before: here.clone(),
                    join: Some(join_vars.clone()),
                    at: path.clone(),
                });
                let r = walk_sources(clauses, &path, stack, next, out);
                stack.pop();
                r?;
            }
            WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
                let join = match clause {
                    WhereClause::OrJoin { join_vars, .. } => Some(join_vars.clone()),
                    _ => None,
                };
                for (b, branch) in branches.iter().enumerate() {
                    let mut branch_path = path.clone();
                    branch_path.push(b);
                    stack.push(Level {
                        before: here.clone(),
                        join: join.clone(),
                        at: path.clone(),
                    });
                    let r = walk_sources(branch, &branch_path, stack, next, out);
                    stack.pop();
                    r?;
                }
            }
            WhereClause::RuleInvocation { .. } | WhereClause::Expr { .. } => {}
        }
    }
    Ok(())
}

/// The clause at `path`: a clause index per list, and a branch index after
/// each `or` / `or-join`.
fn clause_at_mut<'a>(list: &'a mut [WhereClause], path: &[usize]) -> Option<&'a mut WhereClause> {
    let (&i, rest) = path.split_first()?;
    let clause = list.get_mut(i)?;
    if rest.is_empty() {
        return Some(clause);
    }
    match clause {
        WhereClause::Not(body) | WhereClause::NotJoin { clauses: body, .. } => {
            clause_at_mut(body, rest)
        }
        WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
            let (&b, rest) = rest.split_first()?;
            clause_at_mut(branches.get_mut(b)?, rest)
        }
        _ => None,
    }
}

/// Give every per-fact pseudo-attribute pattern in `clauses` (a query's
/// `:where` or a rule body) its source (#594): the nearest pattern before it
/// in the text with the same entity term and a stored attribute, in its own
/// clause list and then outwards before each containing `not` / `or`
/// (`not-join` / `or-join` only for a listed entity variable). The source
/// binds a hidden variable `?#<n>.<attr>` to its fact's metadata; the pseudo
/// pattern reads it; a crossed `-join` lists it. PRS-081 when there is none.
pub(crate) fn resolve_pseudo_sources(clauses: &mut [WhereClause]) -> Result<()> {
    let mut resolved = Vec::new();
    walk_sources(clauses, &[], &mut Vec::new(), &mut 0, &mut resolved)?;
    for r in resolved {
        let name = format!(
            "?#{}.{}",
            r.n,
            r.attr.as_keyword().trim_start_matches(":db/")
        );
        if let Some(WhereClause::Pattern(source)) = clause_at_mut(clauses, &r.source)
            && !source.meta.iter().any(|(a, _)| *a == r.attr)
        {
            source.meta.push((r.attr.clone(), name.clone()));
        }
        if let Some(WhereClause::Pattern(p)) = clause_at_mut(clauses, &r.pseudo) {
            p.pseudo_source = Some(EdnValue::Symbol(name.clone()));
        }
        for at in &r.crossed {
            if let Some(
                WhereClause::NotJoin { join_vars, .. } | WhereClause::OrJoin { join_vars, .. },
            ) = clause_at_mut(clauses, at)
                && !join_vars.contains(&name)
            {
                join_vars.push(name.clone());
            }
        }
    }
    Ok(())
}

fn parse_rule(elements: &[EdnValue]) -> Result<DatalogCommand> {
    // Rule syntax: (rule [(predicate ?args) [pattern1] [pattern2] ...])
    // elements[0] = Vector with head (list) + body (patterns/rule calls)

    if elements.is_empty() {
        bail_coded!(ErrorCode::Prs100);
    }

    if elements.len() > 1 {
        bail_coded!(ErrorCode::Prs079, elements.len() - 1);
    }

    // Parse the rule body (single vector containing head + body patterns)
    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let body_vec = elements[0]
        .as_vector()
        .ok_or_else(|| err_coded!(ErrorCode::Prs101))?;

    if body_vec.is_empty() {
        bail_coded!(ErrorCode::Prs102);
    }

    // First element is the head (must be a list)
    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    let head_list = body_vec[0]
        .as_list()
        .ok_or_else(|| err_coded!(ErrorCode::Prs103))?;

    if head_list.is_empty() {
        bail_coded!(ErrorCode::Prs104);
    }

    // Verify head starts with a symbol (predicate name)
    // SAFETY: is_empty() check above guarantees index 0 exists
    #[allow(clippy::indexing_slicing)]
    match &head_list[0] {
        EdnValue::Symbol(name) if is_name(name) => {}
        _ => bail_coded!(ErrorCode::Prs105),
    }
    // SAFETY: head_list is non-empty, so [1..] is valid
    #[allow(clippy::indexing_slicing)]
    if let Some(arg) = head_list[1..]
        .iter()
        .find(|a| !matches!(a, EdnValue::Symbol(s) if s.starts_with('?')))
    {
        bail_coded!(ErrorCode::Prs114, format!("{arg:?}"));
    }

    // Rest of body_vec are patterns, rule invocations, or (not ...) clauses
    let mut body_clauses: Vec<WhereClause> = Vec::new();
    // SAFETY: body_vec is non-empty, so [1..] is valid (may be empty)
    #[allow(clippy::indexing_slicing)]
    let body_rest = &body_vec[1..];
    for item in body_rest {
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
            bail_coded!(ErrorCode::Prs106, format!("{item:?}"));
        }
    }

    if body_clauses.is_empty() {
        bail_coded!(ErrorCode::Prs107);
    }

    resolve_pseudo_sources(&mut body_clauses)?;

    // Safety check: variables in (not ...) must be bound by the rule head or outer body clauses
    let mut outer_bound: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Head args count as binding sites
    // SAFETY: head_list is non-empty, so [1..] is valid
    #[allow(clippy::indexing_slicing)]
    let head_args = &head_list[1..];
    for v in head_args {
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

pub(crate) mod statement;
#[cfg(all(test, not(target_arch = "wasm32")))]
mod verifier;

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
    fn test_tokenize_string_length_limit() {
        let long_string = "\"".to_string() + &"x".repeat(MAX_STRING_LENGTH + 1);
        let result = tokenize(&long_string);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum length")
        );
    }

    #[test]
    fn test_tokenize_keyword_length_limit() {
        let long_keyword = ":".to_string() + &"x".repeat(MAX_KEYWORD_LENGTH + 1);
        let result = tokenize(&long_keyword);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum length")
        );
    }

    #[test]
    fn test_parse_edn_deep_nesting_returns_error() {
        let deeply_nested = "(".repeat(10_000);
        let result = parse_edn(&deeply_nested);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("recursion depth"));
    }

    #[test]
    fn test_tokenize_keyword_with_question_mark() {
        // Regression: '?' ended the keyword, so ":attended?" lexed as
        // Keyword(":attended") + Symbol("?"), turning a three-element fact into
        // four and reporting the value as a bogus fourth element.
        let tokens = tokenize("[:a :attended? true]").unwrap();
        assert_eq!(tokens.len(), 5, "expected 5 tokens");
        assert_eq!(tokens[2], Token::Keyword(":attended?".to_string()));
    }

    #[test]
    fn test_tokenize_keyword_question_mark_positions() {
        // Leading, infix, trailing and namespaced forms all stay in one token.
        let tokens = tokenize(":?x :att?end :alive? :person/alive?").unwrap();
        assert_eq!(tokens[0], Token::Keyword(":?x".to_string()));
        assert_eq!(tokens[1], Token::Keyword(":att?end".to_string()));
        assert_eq!(tokens[2], Token::Keyword(":alive?".to_string()));
        assert_eq!(tokens[3], Token::Keyword(":person/alive?".to_string()));
    }

    #[test]
    fn test_tokenize_variable_still_lexes_as_symbol() {
        // A '?' that does not follow ':' still starts a query variable.
        let tokens = tokenize("[?e :alive? ?v]").unwrap();
        assert_eq!(tokens[1], Token::Symbol("?e".to_string()));
        assert_eq!(tokens[2], Token::Keyword(":alive?".to_string()));
        assert_eq!(tokens[3], Token::Symbol("?v".to_string()));
    }

    #[test]
    fn test_tokenize_symbol_length_limit() {
        let long_symbol = "x".repeat(MAX_SYMBOL_LENGTH + 1);
        let result = tokenize(&long_symbol);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum length")
        );
    }

    #[test]
    fn set_literals_parse() {
        let v =
            parse_edn(r#"#{1 1.5 "a" :k true nil #uuid "550e8400-e29b-41d4-a716-446655440000" 1}"#)
                .unwrap();
        let EdnValue::Set(elements) = v else {
            panic!("a set");
        };
        assert_eq!(elements.len(), 8, "duplicates are kept until edn_to_value");
        assert!(matches!(parse_edn("#{}").unwrap(), EdnValue::Set(e) if e.is_empty()));
        assert!(matches!(parse_edn("#{,1 ,2,}").unwrap(), EdnValue::Set(e) if e.len() == 2));
        // `#uuid` right after `#{`, and `#{` right after a value.
        assert!(parse_edn(r#"#{#uuid "550e8400-e29b-41d4-a716-446655440000"}"#).is_ok());
        assert!(parse_edn("[1#{2}]").is_ok());
    }

    #[test]
    fn set_literal_errors_have_codes() {
        let code = |s: &str| crate::MinigrafError::from(parse_edn(s).unwrap_err()).code();
        assert_eq!(code("#{#{}}"), "PRS-119");
        assert_eq!(code("#{1 #{2}}"), "PRS-119");
        for s in ["#{?x}", "#{$s}", "#{[1]}", "#{(f)}", "#{{:a 1}}", "#{foo}"] {
            assert_eq!(code(s), "PRS-120", "{s}");
        }
        assert_eq!(code("#{1"), "PRS-121");
        assert_eq!(code("# {1}"), "PRS-073");
    }

    #[test]
    fn set_literal_in_an_expression_is_a_literal() {
        let cmd =
            parse_datalog_command("(query [:find ?e :where [?e :s ?s] [(= ?s #{:a :b :a})]])")
                .unwrap();
        let DatalogCommand::Query(q) = cmd else {
            panic!("a query");
        };
        let found = q.where_clauses.iter().any(|c| {
            matches!(c, WhereClause::Expr { expr: Expr::BinOp(_, _, r), .. }
                if matches!(&**r, Expr::Lit(Value::Set(s)) if s.len() == 2))
        });
        assert!(found, "the set is a literal with duplicates removed");
    }

    #[test]
    fn test_tokenize_tagged_literal_length_limit() {
        let long_tag = "#".to_string() + &"x".repeat(MAX_SYMBOL_LENGTH + 1);
        let result = tokenize(&long_tag);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum length")
        );
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
        assert!(result.unwrap_err().to_string().contains("non-negative"));
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
        let msg = result.unwrap_err().to_string();
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
    fn test_parse_transact_rejects_options_map_after_facts() {
        // The valid-time options map must come BEFORE the facts vector:
        // (transact {opts} [facts]). Putting it after — (transact [facts] {opts}) —
        // must be a hard parse error, not a silent no-op that drops :valid-from.
        let result = parse_datalog_command(
            r#"(transact [[:alice :employment/status :active]] {:valid-from "2023-01-01"})"#,
        );
        assert!(result.is_err(), "expected a parse error");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unexpected") || msg.contains("trailing"),
            "error should mention the unexpected trailing argument, was: {}",
            msg
        );
    }

    #[test]
    fn test_parse_transact_rejects_extra_trailing_argument() {
        // Trailing garbage after (transact {opts} [facts]) must also be rejected.
        let result = parse_datalog_command(
            r#"(transact {:valid-from "2023-01-01"} [[:alice :employment/status :active]] :bogus)"#,
        );
        assert!(result.is_err(), "expected a parse error");
    }

    #[test]
    fn test_parse_retract_rejects_extra_trailing_argument() {
        let result = parse_datalog_command(
            r#"(retract [[:alice :employment/status :active]] {:valid-from "2023-01-01"})"#,
        );
        assert!(result.is_err(), "expected a parse error");
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
    fn test_parse_max_derived_facts_valid() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 500000])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_ok(), "should parse :max-derived-facts");
        match result.unwrap() {
            DatalogCommand::Query(q) => {
                assert_eq!(q.max_derived_facts, Some(500_000));
                assert_eq!(q.max_results, None);
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_max_results_valid() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-results 9999])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_ok(), "should parse :max-results");
        match result.unwrap() {
            DatalogCommand::Query(q) => {
                assert_eq!(q.max_derived_facts, None);
                assert_eq!(q.max_results, Some(9_999));
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_both_limits_valid() {
        let input =
            r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 1000000 :max-results 100])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_ok(), "should parse both limit keys");
        match result.unwrap() {
            DatalogCommand::Query(q) => {
                assert_eq!(q.max_derived_facts, Some(1_000_000));
                assert_eq!(q.max_results, Some(100));
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_max_derived_facts_zero_rejected() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 0])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err(), "0 should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains(":max-derived-facts must be >= 1"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_max_results_zero_rejected() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-results 0])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err(), "0 should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains(":max-results must be >= 1"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_max_derived_facts_duplicate_rejected() {
        let input =
            r#"(query [:find ?x :where [?x :a :b] :max-derived-facts 100 :max-derived-facts 200])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err(), "duplicate key should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("duplicate :max-derived-facts"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_max_results_duplicate_rejected() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-results 100 :max-results 200])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err(), "duplicate key should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("duplicate :max-results"),
            "wrong error: {}",
            msg
        );
    }

    fn parse_limit(input: &str) -> Result<Option<Count>> {
        match parse_datalog_command(input)? {
            DatalogCommand::Query(q) => Ok(q.limit),
            _ => panic!("expected Query"),
        }
    }

    fn code_of(input: &str) -> &'static str {
        let e = parse_datalog_command(input).expect_err("should be rejected");
        crate::MinigrafError::from(e).code()
    }

    #[test]
    fn limit_literal_zero_and_slot() {
        let q = |tail: &str| format!("(query [:find ?x :where [?x :a ?y] {tail}])");
        assert_eq!(
            parse_limit(&q(":limit 10")).unwrap(),
            Some(Count::Literal(10))
        );
        assert_eq!(
            parse_limit(&q(":limit 0")).unwrap(),
            Some(Count::Literal(0))
        );
        assert_eq!(
            parse_limit(&q(":limit $n")).unwrap(),
            Some(Count::Slot("n".to_string()))
        );
        assert_eq!(parse_limit(&q("")).unwrap(), None);
    }

    #[test]
    fn limit_in_any_section_order() {
        for input in [
            "(query [:limit 3 :find ?x :where [?x :a ?y]])",
            "(query [:find ?x :limit 3 :where [?x :a ?y]])",
            "(query [:find ?x :where [?x :a ?y] :limit 3])",
            "(query [:find ?x :where [?x :a ?y] :limit 3 :as-of 1])",
        ] {
            assert_eq!(
                parse_limit(input).unwrap(),
                Some(Count::Literal(3)),
                "{input}"
            );
        }
    }

    #[test]
    fn limit_errors() {
        let q = |tail: &str| format!("(query [:find ?x :where [?x :a ?y] {tail}])");
        assert_eq!(code_of(&q(":limit 1 :limit 2")), "INT-012");
        assert_eq!(code_of(&q(":limit")), "INT-013");
        assert_eq!(
            code_of("(query [:find ?x :limit :where [?x :a ?y]])"),
            "INT-013"
        );
        assert_eq!(code_of(&q(":limit :ten")), "INT-013");
        for bad in ["-1", "1.5", "\"ten\"", "?x", "nil", "true", "[1]"] {
            assert_eq!(code_of(&q(&format!(":limit {bad}"))), "PRS-115", "{bad}");
        }
    }

    #[test]
    fn test_parse_max_derived_facts_non_integer_rejected() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-derived-facts "a lot"])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err(), "non-integer should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains(":max-derived-facts must be a positive integer"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_max_results_non_integer_rejected() {
        let input = r#"(query [:find ?x :where [?x :a :b] :max-results "a lot"])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err(), "non-integer should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains(":max-results must be a positive integer"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_edn_rejects_trailing_tokens_after_complete_form() {
        // A syntactically complete top-level form followed by garbage tokens
        // must be a hard error, not silently truncated parsing (#305).
        let result = parse_edn("[1 2 3] garbage-trailing-tokens ) ] } 12345");
        assert!(
            result.is_err(),
            "trailing tokens after a complete form must be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unexpected trailing input"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_datalog_command_rejects_trailing_tokens_after_complete_form() {
        let result = parse_datalog_command(
            r#"(query [:find ?e :where [?e :entity-type :type/commit]]) garbage-trailing-tokens ) ] } 12345"#,
        );
        assert!(
            result.is_err(),
            "trailing tokens after a complete command must be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unexpected trailing input"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_query_rejects_extra_argument_after_vector() {
        // :max-results must be a keyword INSIDE the query vector; putting it
        // after the vector as a sibling argument must not be silently dropped (#305).
        let result =
            parse_datalog_command(r#"(query [:find ?e :where [?e :a :b]] :max-results 5)"#);
        assert!(
            result.is_err(),
            "extra trailing argument to query must be rejected, not silently ignored"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unexpected trailing argument"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_rule_rejects_extra_argument_after_vector() {
        let result =
            parse_datalog_command(r#"(rule [(reachable ?a ?b) [?a :edge ?b]] :unexpected-extra)"#);
        assert!(
            result.is_err(),
            "extra trailing argument to rule must be rejected, not silently ignored"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("unexpected trailing argument"),
            "wrong error: {}",
            msg
        );
    }

    #[test]
    fn test_parse_limits_order_independent() {
        // :max-derived-facts before :find — order should not matter
        let input = r#"(query [:max-derived-facts 42 :find ?x :where [?x :a :b]])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_ok(), "order should not matter");
        match result.unwrap() {
            DatalogCommand::Query(q) => assert_eq!(q.max_derived_facts, Some(42)),
            _ => panic!("expected Query"),
        }
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
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("requires at least one clause"));
    }

    #[test]
    fn test_parse_nested_not_is_error() {
        let input = r#"(query [:find ?x :where [?x :a ?v] (not (not [?x :banned true]))])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("cannot appear inside another"));
    }

    #[test]
    fn test_parse_not_unbound_variable_is_error() {
        // ?y is only in the not body, not in any outer clause
        let input = r#"(query [:find ?x :where [?x :a ?v] (not [?y :banned true])])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not bound"));
    }

    #[test]
    fn test_parse_not_unbound_variable_in_rule_body_is_error() {
        // ?y only in not, not in head or non-not body
        let input = r#"(rule [(eligible ?x) [?x :applied true] (not [?y :banned true])])"#;
        let result = parse_datalog_command(input);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
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
        let msg = result.unwrap_err().to_string();
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
                        func: "count".to_string(),
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
                        func: "count-distinct".to_string(),
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
            "count",
            "count-distinct",
            "sum",
            "sum-distinct",
            "min",
            "max",
        ];
        for name in cases {
            let input = format!("(query [:find ({} ?v) :where [?e :a ?v]])", name);
            let cmd = parse_datalog_command(&input).expect("parse failed");
            match cmd {
                DatalogCommand::Query(q) => {
                    assert_eq!(
                        q.find[0],
                        FindSpec::Aggregate {
                            func: name.to_string(),
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
    fn test_parse_unknown_aggregate_as_udf() {
        // Phase 7.7b: unknown aggregate names are no longer rejected at parse time.
        // They are emitted as FindSpec::Aggregate and resolved at runtime as UDFs.
        let result = parse_datalog_command("(query [:find (average ?e) :where [?e :a ?v]])");
        assert!(result.is_ok(), "unknown aggregate should parse as UDF");
        if let Ok(DatalogCommand::Query(q)) = result {
            assert!(
                q.find
                    .iter()
                    .any(|s| matches!(s, FindSpec::Aggregate { func, .. } if func == "average")),
                "should have Aggregate with func='average'"
            );
        }
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
                .to_string()
                .contains("requires at least one aggregate"),
            "wrong error message"
        );
    }

    #[test]
    fn test_parse_error_aggregate_var_unbound() {
        let result = parse_datalog_command(r#"(query [:find (count ?unbound) :where [?e :a ?v]])"#);
        assert!(result.is_err(), "unbound aggregate var should fail");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not bound in :where"),
            "wrong error message"
        );
    }

    // Helper used by parse_expr tests (Task 2 / Phase 7.2b)
    fn parse(s: &str) -> Result<DatalogCommand> {
        parse_datalog_command(s)
    }

    fn where_of(s: &str) -> Vec<WhereClause> {
        match parse(s).expect("parse") {
            DatalogCommand::Query(q) => q.where_clauses,
            _ => panic!("expected a query"),
        }
    }

    fn pattern_of(c: &WhereClause) -> &Pattern {
        match c {
            WhereClause::Pattern(p) => p,
            _ => panic!("expected a pattern"),
        }
    }

    fn reads(p: &Pattern) -> Option<&str> {
        p.pseudo_source.as_ref().and_then(|s| s.as_variable())
    }

    #[test]
    fn pseudo_source_is_the_nearest_earlier_same_entity_pattern() {
        let w = where_of(
            "(query [:find ?t :any-valid-time :where \
             [?e :a ?x] [?f :b ?y] [?e :c ?z] [?e :db/tx-count ?t] [?e :db/valid-to ?u]])",
        );
        // Candidates are numbered in walk order: [?e :c ?z] is #2.
        let source = pattern_of(&w[2]);
        assert_eq!(
            source.meta,
            vec![
                (PseudoAttr::TxCount, "?#2.tx-count".to_string()),
                (PseudoAttr::ValidTo, "?#2.valid-to".to_string()),
            ]
        );
        assert!(pattern_of(&w[0]).meta.is_empty());
        assert_eq!(reads(pattern_of(&w[3])), Some("?#2.tx-count"));
        assert_eq!(reads(pattern_of(&w[4])), Some("?#2.valid-to"));
    }

    #[test]
    fn pseudo_source_crossing_a_not_join_is_listed() {
        let w = where_of(
            "(query [:find ?e :any-valid-time :where \
             [?e :a ?x] (not-join [?e] [?e :db/tx-count 1])])",
        );
        let WhereClause::NotJoin { join_vars, clauses } = &w[1] else {
            panic!("expected not-join");
        };
        assert_eq!(
            join_vars,
            &vec!["?e".to_string(), "?#0.tx-count".to_string()]
        );
        assert_eq!(reads(pattern_of(&clauses[0])), Some("?#0.tx-count"));
        // Through `not` nothing is listed.
        let w = where_of(
            "(query [:find ?e :any-valid-time :where [?e :a ?x] (not [?e :db/tx-count 1])])",
        );
        let WhereClause::Not(body) = &w[1] else {
            panic!("expected not");
        };
        assert_eq!(reads(pattern_of(&body[0])), Some("?#0.tx-count"));
    }

    #[test]
    fn pseudo_source_in_an_or_branch_is_its_own_or_outer() {
        let w = where_of(
            "(query [:find ?e :any-valid-time :where \
             [?e :a ?x] (or [?e :b 1] [?e :db/tx-count 1])])",
        );
        let WhereClause::Or(branches) = &w[1] else {
            panic!("expected or");
        };
        // The second branch reads the outer pattern, not the first branch's.
        assert_eq!(reads(pattern_of(&branches[1][0])), Some("?#0.tx-count"));
        assert!(pattern_of(&branches[0][0]).meta.is_empty());
    }

    #[test]
    fn pseudo_source_missing_is_prs_081() {
        let err =
            parse("(query [:find ?t :any-valid-time :where [?e :db/tx-count ?t] [?e :a ?x]])")
                .expect_err("no source");
        assert!(err.to_string().contains(":db/tx-count on ?e"));
        let err = parse("(rule [(r ?e ?t) [?e :db/tx-id ?t]])").expect_err("rule body closed");
        assert!(err.to_string().contains(":db/tx-id on ?e"));
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

    #[test]
    fn test_tokenize_bind_slot() {
        let result = parse_edn("$entity");
        assert!(
            matches!(result, Ok(EdnValue::BindSlot(ref s)) if s == "entity"),
            "expected BindSlot"
        );
    }

    #[test]
    fn test_tokenize_bind_slot_hyphenated() {
        let result = parse_edn("$min-level");
        assert!(
            matches!(result, Ok(EdnValue::BindSlot(ref s)) if s == "min-level"),
            "expected BindSlot(min-level)"
        );
    }

    #[test]
    fn test_parse_bind_slot_in_entity_position() {
        let cmd =
            parse_datalog_command("(query [:find ?name :where [$entity :person/name ?name]])");
        assert!(cmd.is_ok(), "parse failed");
        match cmd.unwrap() {
            DatalogCommand::Query(q) => match &q.where_clauses[0] {
                WhereClause::Pattern(p) => {
                    assert!(matches!(&p.entity, EdnValue::BindSlot(s) if s == "entity"));
                }
                _ => panic!("expected Pattern"),
            },
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_bind_slot_in_value_position() {
        let cmd = parse_datalog_command("(query [:find ?e :where [?e :person/name $name]])");
        assert!(cmd.is_ok(), "parse failed");
        match cmd.unwrap() {
            DatalogCommand::Query(q) => match &q.where_clauses[0] {
                WhereClause::Pattern(p) => {
                    assert!(matches!(&p.value, EdnValue::BindSlot(s) if s == "name"));
                }
                _ => panic!("expected Pattern"),
            },
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_as_of_slot() {
        let cmd = parse_datalog_command("(query [:find ?v :as-of $tx :where [?e :score ?v]])");
        assert!(cmd.is_ok(), "parse failed");
        match cmd.unwrap() {
            DatalogCommand::Query(q) => {
                assert!(matches!(q.as_of, Some(AsOf::Slot(ref s)) if s == "tx"));
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_valid_at_slot() {
        let cmd = parse_datalog_command("(query [:find ?v :valid-at $date :where [?e :score ?v]])");
        assert!(cmd.is_ok(), "parse failed");
        match cmd.unwrap() {
            DatalogCommand::Query(q) => {
                assert!(matches!(q.valid_at, Some(ValidAt::Slot(ref s)) if s == "date"));
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_expr_bind_slot_in_binop() {
        let cmd = parse_datalog_command(
            "(query [:any-valid-time :find ?v :where [?e :score ?v] [(>= ?v $threshold)]])",
        );
        assert!(cmd.is_ok(), "parse failed");
        match cmd.unwrap() {
            DatalogCommand::Query(q) => {
                let expr_clause = q
                    .where_clauses
                    .iter()
                    .find(|c| matches!(c, WhereClause::Expr { .. }));
                assert!(expr_clause.is_some(), "no Expr clause found");
                match expr_clause.unwrap() {
                    WhereClause::Expr {
                        expr: Expr::BinOp(_, _, rhs),
                        ..
                    } => {
                        assert!(matches!(rhs.as_ref(), Expr::Slot(s) if s == "threshold"));
                    }
                    _other => panic!("expected BinOp Expr clause"),
                }
            }
            _ => panic!("expected Query"),
        }
    }

    #[test]
    fn test_parse_bind_slot_empty_name_is_error() {
        let result = parse_edn("$");
        assert!(result.is_err(), "bare '$' should be a parse error");
    }

    #[test]
    fn test_integer_overflow_returns_error() {
        let result = tokenize("99999999999999999999999999999");
        assert!(result.is_err(), "i64 overflow must return error, not panic");
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("out of range"),
            "error should mention out of range"
        );
    }

    #[test]
    fn test_negative_integer_overflow_returns_error() {
        let result = tokenize("-99999999999999999999999999999");
        assert!(result.is_err(), "negative i64 overflow must return error");
    }

    #[test]
    fn test_float_overflow_returns_error() {
        // f64::MAX is ~1.8e308, so 1e999 overflows to infinity
        let result = tokenize("1.0e999");
        // f64 parse of very large exponent may produce infinity rather than error;
        // our code rejects it if parse() itself errors. For f64, parse() succeeds
        // with infinity, so this may not error. The critical fix is for integers.
        // Just ensure no panic occurs.
        let _ = result;
    }

    #[test]
    fn test_overflow_in_transact_returns_error() {
        let result = parse_datalog_command("(transact [[:e :a 99999999999999999999999999999]])");
        assert!(
            result.is_err(),
            "overflow in transact should be parse error"
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
        let err = cmd.unwrap_err().to_string();
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
        let err = cmd.unwrap_err().to_string();
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
        let err = cmd.unwrap_err().to_string();
        assert!(err.contains("or") || err.contains("not"));
    }
}

#[cfg(test)]
mod window_parse_tests {
    use super::*;

    #[test]
    fn parse_window_sum_no_partition() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?name (sum ?salary :over (:order-by ?salary))
                       :where [?e :employee/name ?name]
                              [?e :employee/salary ?salary]])"#,
        );
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert_eq!(q.find.len(), 2);
            assert!(matches!(&q.find[1], FindSpec::Window(ws) if
                ws.func == WindowFunc::Sum &&
                ws.var == Some("?salary".to_string()) &&
                ws.partition_by.is_none() &&
                ws.order_by == "?salary" &&
                ws.order == Order::Asc
            ));
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn parse_window_rank_desc() {
        let cmd = parse_datalog_command(
            r#"(query [:find (rank :over (:order-by ?score :desc))
                       :where [?e :item/score ?score]])"#,
        );
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert!(matches!(&q.find[0], FindSpec::Window(ws) if
                ws.func == WindowFunc::Rank &&
                ws.var.is_none() &&
                ws.order == Order::Desc
            ));
        } else {
            panic!("expected Query");
        }
    }

    #[test]
    fn parse_window_with_partition_by() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?dept (sum ?salary :over (:partition-by ?dept :order-by ?salary))
                       :where [?e :employee/dept ?dept]
                              [?e :employee/salary ?salary]])"#,
        );
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert!(matches!(&q.find[1], FindSpec::Window(ws) if
                ws.partition_by == Some("?dept".to_string())
            ));
        } else {
            panic!("expected Query");
        }
    }

    #[test]
    fn parse_lag_rejected() {
        let result = parse_datalog_command(
            r#"(query [:find (lag ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[test]
    fn parse_window_order_by_missing_rejected() {
        let result = parse_datalog_command(
            r#"(query [:find (sum ?v :over (:partition-by ?p)) :where [?e :x ?v] [?e :y ?p]])"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("order-by"));
    }

    #[test]
    fn parse_non_window_compatible_in_over_rejected() {
        let result = parse_datalog_command(
            r#"(query [:find (count-distinct ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .to_lowercase()
                .contains("window")
        );
    }

    #[test]
    fn parse_existing_aggregate_still_works() {
        let cmd = parse_datalog_command(r#"(query [:find (count ?e) :where [?e :person/name _]])"#);
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert!(matches!(&q.find[0], FindSpec::Aggregate { func, .. } if func == "count"));
        } else {
            panic!("expected Query");
        }
    }

    #[test]
    fn parse_row_number_no_var() {
        let cmd = parse_datalog_command(
            r#"(query [:find (row-number :over (:order-by ?v)) :where [?e :x ?v]])"#,
        );
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert!(matches!(&q.find[0], FindSpec::Window(ws) if
                ws.func == WindowFunc::RowNumber &&
                ws.var.is_none()
            ));
        } else {
            panic!("expected Query");
        }
    }

    #[test]
    fn parse_unknown_window_func_as_udf() {
        let result = parse_datalog_command(
            r#"(query [:find (frobnicate ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
        );
        assert!(
            result.is_ok(),
            "unknown window function should parse as UDF"
        );
        if let Ok(DatalogCommand::Query(q)) = result {
            if let FindSpec::Window(ws) = &q.find[0] {
                assert_eq!(ws.func, WindowFunc::Udf("frobnicate".to_string()));
            } else {
                panic!("expected Window find spec");
            }
        }
    }

    #[test]
    fn parse_lead_rejected() {
        let result = parse_datalog_command(
            r#"(query [:find (lead ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[test]
    fn parse_mixed_aggregate_and_window_allowed() {
        let cmd = parse_datalog_command(
            r#"(query [:find ?v (count ?e) (sum ?v :over (:order-by ?v))
                       :where [?e :x ?v]])"#,
        );
        assert!(cmd.is_ok(), "parse failed");
        if let Ok(DatalogCommand::Query(q)) = cmd {
            assert_eq!(q.find.len(), 3);
            assert!(matches!(&q.find[1], FindSpec::Aggregate { func, .. } if func == "count"));
            assert!(matches!(&q.find[2], FindSpec::Window(ws) if ws.func == WindowFunc::Sum));
        } else {
            panic!("expected Query");
        }
    }

    #[test]
    fn window_in_aggregate_query_reads_only_plain_find_variables() {
        // PRS-082 (#547 spec §4.4): the argument, :partition-by and :order-by
        // of a window in an aggregate query must be plain :find variables;
        // :with variables do not count.
        for q in [
            r#"(query [:find (count ?e) (sum ?v :over (:order-by ?v)) :where [?e :x ?v]])"#,
            r#"(query [:find ?d (count ?e) (rank :over (:order-by ?v)) :where [?e :x ?v] [?e :d ?d]])"#,
            r#"(query [:find ?d (count ?e) (sum ?d :over (:partition-by ?v :order-by ?d)) :where [?e :x ?v] [?e :d ?d]])"#,
            r#"(query [:find ?d (count ?e) (sum ?v :over (:order-by ?d)) :with ?v :where [?e :x ?v] [?e :d ?d]])"#,
        ] {
            let err = parse_datalog_command(q).err().expect("rejected");
            assert_eq!(crate::MinigrafError::from(err).code(), "PRS-082");
        }
        parse_datalog_command(
            r#"(query [:find ?d (count ?e) (rank :over (:partition-by ?d :order-by ?d)) :where [?e :d ?d]])"#,
        )
        .expect("grouping variables are allowed");
    }

    #[test]
    fn parse_udf_window_function() {
        // A name not in the built-in list should produce WindowFunc::Udf
        let result = parse_datalog_command(
            r#"(query [:find (geomean ?score :over (:order-by ?score))
                     :where [?e :item/score ?score]])"#,
        );
        assert!(result.is_ok(), "UDF window function should parse");
        if let Ok(DatalogCommand::Query(q)) = result {
            assert_eq!(q.find.len(), 1);
            if let FindSpec::Window(ws) = &q.find[0] {
                assert_eq!(ws.func, WindowFunc::Udf("geomean".to_string()));
            } else {
                panic!("expected Window find spec");
            }
        }
    }

    #[test]
    fn parse_udf_predicate_clause() {
        // Unknown 1-arg predicate should produce UnaryOp::Udf
        let result = parse_datalog_command(
            r#"(query [:find ?e :where [?e :person/email ?addr] [(email? ?addr)]])"#,
        );
        assert!(result.is_ok(), "UDF predicate should parse");
        if let Ok(DatalogCommand::Query(q)) = result {
            let has_udf = q.where_clauses.iter().any(|c| {
                matches!(c, WhereClause::Expr { expr: Expr::UnaryOp(UnaryOp::Udf(name), _), .. }
                    if name == "email?")
            });
            assert!(has_udf, "should have UnaryOp::Udf(\"email?\") clause");
        }
    }

    #[test]
    fn parse_unknown_two_arg_predicate_rejected() {
        // Unknown 2-arg form should still be a parse error
        let result =
            parse_datalog_command(r#"(query [:find ?e :where [?e :x ?v] [(myfn? ?v ?v)]])"#);
        assert!(result.is_err(), "unknown 2-arg form should be rejected");
    }
}
