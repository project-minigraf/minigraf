use uuid::Uuid;

/// EDN (Extensible Data Notation) value types
/// This represents the core data types in EDN/Datalog syntax
#[derive(Debug, Clone, PartialEq)]
pub enum EdnValue {
    /// Keyword: :person/name, :find, :where
    Keyword(String),
    /// Symbol: ?name, ?e (logic variables)
    Symbol(String),
    /// String literal: "Alice"
    String(String),
    /// Integer: 42
    Integer(i64),
    /// Float: 3.14
    Float(f64),
    /// Boolean: true, false
    Boolean(bool),
    /// UUID: #uuid "550e8400-e29b-41d4-a716-446655440000"
    Uuid(Uuid),
    /// Vector: [...]
    Vector(Vec<EdnValue>),
    /// List: (...)
    List(Vec<EdnValue>),
    /// Null/nil
    Nil,
}

impl EdnValue {
    /// Check if this is a logic variable (symbol starting with ?)
    pub fn is_variable(&self) -> bool {
        matches!(self, EdnValue::Symbol(s) if s.starts_with('?'))
    }

    /// Get the variable name if this is a variable
    pub fn as_variable(&self) -> Option<&str> {
        match self {
            EdnValue::Symbol(s) if s.starts_with('?') => Some(s),
            _ => None,
        }
    }

    /// Check if this is a keyword
    pub fn is_keyword(&self) -> bool {
        matches!(self, EdnValue::Keyword(_))
    }

    /// Get the keyword value if this is a keyword
    pub fn as_keyword(&self) -> Option<&str> {
        match self {
            EdnValue::Keyword(k) => Some(k),
            _ => None,
        }
    }

    /// Check if this is a vector
    pub fn is_vector(&self) -> bool {
        matches!(self, EdnValue::Vector(_))
    }

    /// Get the vector contents if this is a vector
    pub fn as_vector(&self) -> Option<&Vec<EdnValue>> {
        match self {
            EdnValue::Vector(v) => Some(v),
            _ => None,
        }
    }

    /// Check if this is a list
    pub fn is_list(&self) -> bool {
        matches!(self, EdnValue::List(_))
    }

    /// Get the list contents if this is a list
    pub fn as_list(&self) -> Option<&Vec<EdnValue>> {
        match self {
            EdnValue::List(l) => Some(l),
            _ => None,
        }
    }
}

/// A Datalog pattern: [Entity Attribute Value]
/// Variables start with ?, constants are literal values
///
/// Examples:
/// - [?e :person/name "Alice"]  - Find entity with name "Alice"
/// - [?e :person/name ?name]    - Find all entity-name pairs
/// - [:alice :friend ?friend]   - Find all friends of Alice
#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub entity: EdnValue,
    pub attribute: EdnValue,
    pub value: EdnValue,
}

impl Pattern {
    pub fn new(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Self {
        Pattern {
            entity,
            attribute,
            value,
        }
    }

    /// Parse a pattern from an EDN vector
    pub fn from_edn(vector: &[EdnValue]) -> Result<Self, String> {
        if vector.len() != 3 {
            return Err(format!(
                "Pattern must have exactly 3 elements (E A V), got {}",
                vector.len()
            ));
        }

        Ok(Pattern {
            entity: vector[0].clone(),
            attribute: vector[1].clone(),
            value: vector[2].clone(),
        })
    }
}

/// A Datalog query with :find and :where clauses
///
/// Example:
/// ```datalog
/// (query [:find ?name ?age
///         :where [?e :person/name ?name]
///                [?e :person/age ?age]])
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DatalogQuery {
    /// Variables to return (from :find clause)
    pub find: Vec<String>,
    /// Patterns to match (from :where clause)
    pub patterns: Vec<Pattern>,
}

impl DatalogQuery {
    pub fn new(find: Vec<String>, patterns: Vec<Pattern>) -> Self {
        DatalogQuery { find, patterns }
    }
}

/// A Datalog rule definition
///
/// Example:
/// ```datalog
/// (rule [(reachable ?from ?to)
///        [?from :connected ?to]])
///
/// (rule [(reachable ?from ?to)
///        [?from :connected ?intermediate]
///        (reachable ?intermediate ?to)])
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// The rule head: (predicate ?var1 ?var2)
    pub head: Vec<EdnValue>,
    /// The rule body: list of patterns and rule invocations
    pub body: Vec<EdnValue>,
}

impl Rule {
    pub fn new(head: Vec<EdnValue>, body: Vec<EdnValue>) -> Self {
        Rule { head, body }
    }
}

/// A transaction: list of facts to assert or retract
///
/// Example:
/// ```datalog
/// (transact [[:alice :person/name "Alice"]
///            [:alice :person/age 30]
///            [:alice :friend :bob]])
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Transaction {
    /// List of fact triples to assert
    pub facts: Vec<Pattern>,
}

impl Transaction {
    pub fn new(facts: Vec<Pattern>) -> Self {
        Transaction { facts }
    }
}

/// A Datalog command (top-level form)
#[derive(Debug, Clone, PartialEq)]
pub enum DatalogCommand {
    /// Execute a query
    Query(DatalogQuery),
    /// Define a rule
    Rule(Rule),
    /// Transact facts
    Transact(Transaction),
    /// Retract facts
    Retract(Transaction),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edn_value_variable() {
        let var = EdnValue::Symbol("?name".to_string());
        assert!(var.is_variable());
        assert_eq!(var.as_variable(), Some("?name"));

        let not_var = EdnValue::Symbol("name".to_string());
        assert!(!not_var.is_variable());
        assert_eq!(not_var.as_variable(), None);
    }

    #[test]
    fn test_edn_value_keyword() {
        let keyword = EdnValue::Keyword(":person/name".to_string());
        assert!(keyword.is_keyword());
        assert_eq!(keyword.as_keyword(), Some(":person/name"));

        let not_keyword = EdnValue::String(":not-a-keyword".to_string());
        assert!(!not_keyword.is_keyword());
    }

    #[test]
    fn test_pattern_creation() {
        let pattern = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        );

        assert!(pattern.entity.is_variable());
        assert!(pattern.attribute.is_keyword());
    }

    #[test]
    fn test_pattern_from_edn() {
        let vector = vec![
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        ];

        let pattern = Pattern::from_edn(&vector).unwrap();
        assert_eq!(pattern.entity, EdnValue::Symbol("?e".to_string()));
        assert_eq!(
            pattern.attribute,
            EdnValue::Keyword(":person/name".to_string())
        );
        assert_eq!(pattern.value, EdnValue::String("Alice".to_string()));
    }

    #[test]
    fn test_pattern_from_edn_invalid_length() {
        let vector = vec![
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
        ];

        let result = Pattern::from_edn(&vector);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must have exactly 3 elements"));
    }

    #[test]
    fn test_datalog_query_creation() {
        let query = DatalogQuery::new(
            vec!["?name".to_string(), "?age".to_string()],
            vec![
                Pattern::new(
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::Symbol("?name".to_string()),
                ),
                Pattern::new(
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Keyword(":person/age".to_string()),
                    EdnValue::Symbol("?age".to_string()),
                ),
            ],
        );

        assert_eq!(query.find.len(), 2);
        assert_eq!(query.patterns.len(), 2);
    }

    #[test]
    fn test_transaction_creation() {
        let tx = Transaction::new(vec![
            Pattern::new(
                EdnValue::Keyword(":alice".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::String("Alice".to_string()),
            ),
            Pattern::new(
                EdnValue::Keyword(":alice".to_string()),
                EdnValue::Keyword(":person/age".to_string()),
                EdnValue::Integer(30),
            ),
        ]);

        assert_eq!(tx.facts.len(), 2);
    }
}
