use super::types::*;
use uuid::Uuid;

/// Tokenizer for EDN syntax
#[derive(Debug, Clone, PartialEq)]
enum Token {
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
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

    fn parse_value(&mut self) -> Result<EdnValue, String> {
        match self.peek() {
            Some(Token::LeftParen) => self.parse_list(),
            Some(Token::LeftBracket) => self.parse_vector(),
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
    // Parse (query [:find ?x ?y :where [patterns...]])
    if elements.is_empty() {
        return Err("Query requires a map argument".to_string());
    }

    let query_vector = elements[0]
        .as_vector()
        .ok_or("Query argument must be a vector")?;

    let mut find_vars = Vec::new();
    let mut patterns = Vec::new();
    let mut current_clause = None;

    let mut i = 0;
    while i < query_vector.len() {
        if let Some(keyword) = query_vector[i].as_keyword() {
            current_clause = Some(keyword);
            i += 1;
            continue;
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
                // Parse patterns
                if let Some(pattern_vec) = query_vector[i].as_vector() {
                    patterns.push(Pattern::from_edn(pattern_vec)?);
                } else {
                    return Err(format!(
                        "Expected pattern vector in :where clause, got {:?}",
                        query_vector[i]
                    ));
                }
            }
            _ => {
                return Err(format!("Unexpected element in query: {:?}", query_vector[i]));
            }
        }

        i += 1;
    }

    Ok(DatalogCommand::Query(DatalogQuery::new(
        find_vars, patterns,
    )))
}

fn parse_transact(elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // Parse (transact [[e a v] [e a v] ...])
    if elements.is_empty() {
        return Err("Transact requires a vector of facts".to_string());
    }

    let facts_vector = elements[0]
        .as_vector()
        .ok_or("Transact argument must be a vector")?;

    let mut patterns = Vec::new();
    for fact in facts_vector {
        let fact_vec = fact
            .as_vector()
            .ok_or("Each fact must be a vector [e a v]")?;
        patterns.push(Pattern::from_edn(fact_vec)?);
    }

    Ok(DatalogCommand::Transact(Transaction::new(patterns)))
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

fn parse_rule(_elements: &[EdnValue]) -> Result<DatalogCommand, String> {
    // TODO: Implement rule parsing in later phase
    Err("Rule parsing not yet implemented".to_string())
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
        let tokens = tokenize("42 3.14 -5 -2.5").unwrap();
        assert_eq!(tokens[0], Token::Integer(42));
        assert_eq!(tokens[1], Token::Float(3.14));
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
                assert_eq!(q.patterns.len(), 1);
                assert_eq!(
                    q.patterns[0].attribute,
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
                assert_eq!(
                    tx.facts[0].entity,
                    EdnValue::Keyword(":alice".to_string())
                );
                assert_eq!(
                    tx.facts[0].value,
                    EdnValue::String("Alice".to_string())
                );
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
        let input = r#"(query [:find ?name ?age :where [?e :person/name ?name] [?e :person/age ?age]])"#;
        let cmd = parse_datalog_command(input).unwrap();

        match cmd {
            DatalogCommand::Query(q) => {
                assert_eq!(q.find, vec!["?name", "?age"]);
                assert_eq!(q.patterns.len(), 2);
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
}
