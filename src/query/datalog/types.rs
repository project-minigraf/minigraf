use crate::graph::types::Value;
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
    /// Map: {:key val ...}
    Map(Vec<(EdnValue, EdnValue)>),
    /// Null/nil
    Nil,
    /// A named bind slot: `$identifier`.
    /// Only valid in a `PreparedQuery` template AST — must be replaced by
    /// `substitute()` before the query reaches the executor.
    BindSlot(String),
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

    /// Check if this is a map
    pub fn is_map(&self) -> bool {
        matches!(self, EdnValue::Map(_))
    }

    /// Get the map contents if this is a map
    pub fn as_map(&self) -> Option<&Vec<(EdnValue, EdnValue)>> {
        match self {
            EdnValue::Map(m) => Some(m),
            _ => None,
        }
    }
}

/// Window aggregate functions usable inside an `:over` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowFunc {
    Sum,
    Count,
    Min,
    Max,
    Avg,
    Rank,
    RowNumber,
    /// Any function name not recognised at parse time — resolved against
    /// `FunctionRegistry` at query execution time.
    Udf(String),
}

/// Sort direction for the `:order-by` key in a window spec.
#[derive(Debug, Clone, PartialEq)]
pub enum Order {
    Asc,
    Desc,
}

/// The `:over (...)` clause of a window function expression.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSpec {
    pub func: WindowFunc,
    /// The variable to accumulate. None for rank/row-number (position-based).
    pub var: Option<String>,
    /// Partition variable; None means the whole result set is one partition.
    pub partition_by: Option<String>,
    /// Sort key variable (required — parser enforces this).
    pub order_by: String,
    pub order: Order,
}

impl WindowSpec {
    /// Returns the FunctionRegistry key name for this function.
    pub fn func_name(&self) -> String {
        match &self.func {
            WindowFunc::Sum => "sum".to_string(),
            WindowFunc::Count => "count".to_string(),
            WindowFunc::Min => "min".to_string(),
            WindowFunc::Max => "max".to_string(),
            WindowFunc::Avg => "avg".to_string(),
            WindowFunc::Rank => "rank".to_string(),
            WindowFunc::RowNumber => "row-number".to_string(),
            WindowFunc::Udf(name) => name.clone(),
        }
    }
}

/// Binary operators for expression clauses.
/// Manual impl for PartialEq/Eq/Hash because `Matches` holds a `regex_lite::Regex`
/// which doesn't implement these traits.
#[derive(Debug, Clone)]
pub enum BinOp {
    // Comparisons — return Boolean
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    Neq,
    // Arithmetic — return numeric Value (Integer or Float, with int/float promotion)
    Add,
    Sub,
    Mul,
    Div,
    // String predicates — return Boolean
    StartsWith,
    EndsWith,
    Contains,
    /// Pattern must be a string literal validated at parse time via regex-lite.
    /// The compiled `regex_lite::Regex` is stored alongside the original string pattern.
    Matches {
        regex: regex_lite::Regex,
        pattern: String,
    },
}

impl PartialEq for BinOp {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (BinOp::Matches { pattern: p1, .. }, BinOp::Matches { pattern: p2, .. }) => p1 == p2,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }
}

impl Eq for BinOp {}

impl std::hash::Hash for BinOp {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            BinOp::Matches { pattern, .. } => {
                core::hash::Hash::hash(&0u8, state);
                core::hash::Hash::hash(pattern, state);
            }
            _ => core::hash::Hash::hash(&std::mem::discriminant(self), state),
        }
    }
}

/// Unary type-predicate operators — always return Boolean.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    StringQ,
    IntegerQ,
    FloatQ,
    BooleanQ,
    NilQ,
    /// A UDF predicate name not in the built-in whitelist — resolved against
    /// `FunctionRegistry` at query execution time.
    Udf(String),
}

/// Composable expression tree for `WhereClause::Expr`.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Logic variable reference: `?v`
    Var(String),
    /// Literal value: `100`, `"foo"`, `true`
    Lit(Value),
    /// Named bind slot: `$name` — substituted to `Expr::Lit` before execution.
    Slot(String),
    BinOp(BinOp, Box<Expr>, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
}

/// A single element in the :find clause: either a plain variable, an aggregate, or a window function.
#[derive(Debug, Clone, PartialEq)]
pub enum FindSpec {
    /// A plain logic variable: ?name
    Variable(String),
    /// A regular aggregate: (count ?e), (sum ?salary), etc.
    /// `func` is the hyphenated name registered in `FunctionRegistry`.
    Aggregate { func: String, var: String },
    /// A window function: (sum ?salary :over (:order-by ?hire-date))
    Window(WindowSpec),
}

impl FindSpec {
    /// Column header string used in QueryResult::QueryResults.vars.
    /// Variable("?name") → "?name"
    /// Aggregate { "count-distinct", "?e" } → "(count-distinct ?e)"
    pub fn display_name(&self) -> String {
        match self {
            FindSpec::Variable(v) => v.clone(),
            FindSpec::Aggregate { func, var } => format!("({} {})", func, var),
            FindSpec::Window(ws) => match &ws.var {
                Some(v) => format!("({} {} :over ...)", ws.func_name(), v),
                None => format!("({} :over ...)", ws.func_name()),
            },
        }
    }

    /// The logic variable this spec draws values from.
    /// For rank/row-number (no var), returns a synthetic placeholder.
    pub fn var(&self) -> &str {
        match self {
            FindSpec::Variable(v) => v.as_str(),
            FindSpec::Aggregate { var, .. } => var.as_str(),
            FindSpec::Window(ws) => ws.var.as_deref().unwrap_or("__window_var"),
        }
    }
}

/// Built-in pseudo-attributes — reserved `:db/*` keywords that bind fact metadata
/// rather than stored attribute values. Never stored in the fact database.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PseudoAttr {
    ValidFrom, // :db/valid-from → Value::Integer(fact.valid_from)
    ValidTo,   // :db/valid-to   → Value::Integer(fact.valid_to)
    TxCount,   // :db/tx-count   → Value::Integer(fact.tx_count as i64)
    TxId,      // :db/tx-id      → Value::Integer(fact.tx_id as i64)
    ValidAt,   // :db/valid-at   → query-level constant (Value::Integer or Value::Null)
}

impl PseudoAttr {
    /// Returns `Some(variant)` if `k` is a reserved `:db/*` pseudo-attribute keyword.
    pub fn from_keyword(k: &str) -> Option<Self> {
        match k {
            ":db/valid-from" => Some(PseudoAttr::ValidFrom),
            ":db/valid-to" => Some(PseudoAttr::ValidTo),
            ":db/tx-count" => Some(PseudoAttr::TxCount),
            ":db/tx-id" => Some(PseudoAttr::TxId),
            ":db/valid-at" => Some(PseudoAttr::ValidAt),
            _ => None,
        }
    }

    /// Returns the canonical keyword string for this pseudo-attribute.
    pub fn as_keyword(&self) -> &'static str {
        match self {
            PseudoAttr::ValidFrom => ":db/valid-from",
            PseudoAttr::ValidTo => ":db/valid-to",
            PseudoAttr::TxCount => ":db/tx-count",
            PseudoAttr::TxId => ":db/tx-id",
            PseudoAttr::ValidAt => ":db/valid-at",
        }
    }

    /// True for the four per-fact pseudo-attributes (all except `ValidAt`).
    pub fn is_per_fact(&self) -> bool {
        !matches!(self, PseudoAttr::ValidAt)
    }
}

/// Attribute position in a `Pattern` — either a real stored attribute keyword
/// or a built-in pseudo-attribute that binds fact metadata.
#[derive(Debug, Clone, PartialEq)]
pub enum AttributeSpec {
    Real(EdnValue),
    Pseudo(PseudoAttr),
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
    pub attribute: AttributeSpec,
    pub value: EdnValue,
    /// Per-fact valid-time override (millis since epoch). None = use transaction-level default.
    pub valid_from: Option<i64>,
    /// Per-fact valid-time override (millis since epoch). None = use transaction-level default.
    pub valid_to: Option<i64>,
}

impl Pattern {
    pub fn new(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Real(attribute),
            value,
            valid_from: None,
            valid_to: None,
        }
    }

    /// Create a pattern with a pseudo-attribute.
    pub fn pseudo(entity: EdnValue, pseudo: PseudoAttr, value: EdnValue) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Pseudo(pseudo),
            value,
            valid_from: None,
            valid_to: None,
        }
    }

    /// Create a pattern with a real (stored) attribute.
    pub fn real(entity: EdnValue, attribute: EdnValue, value: EdnValue) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Real(attribute),
            value,
            valid_from: None,
            valid_to: None,
        }
    }

    /// Create a pattern with explicit per-fact valid-time overrides.
    pub fn with_valid_time(
        entity: EdnValue,
        attribute: EdnValue,
        value: EdnValue,
        valid_from: Option<i64>,
        valid_to: Option<i64>,
    ) -> Self {
        Pattern {
            entity,
            attribute: AttributeSpec::Real(attribute),
            value,
            valid_from,
            valid_to,
        }
    }

    /// Parse a pattern from an EDN vector (exactly 3 elements, no per-fact map).
    pub fn from_edn(vector: &[EdnValue]) -> Result<Self, String> {
        if vector.len() != 3 {
            return Err(format!(
                "Pattern must have exactly 3 elements (E A V), got {}",
                vector.len()
            ));
        }

        Ok(Pattern {
            entity: vector[0].clone(),
            attribute: AttributeSpec::Real(vector[1].clone()),
            value: vector[2].clone(),
            valid_from: None,
            valid_to: None,
        })
    }
}

/// A clause in the :where section of a query
///
/// Can be either a fact pattern, a rule invocation, or a negation.
#[derive(Debug, Clone, PartialEq)]
pub enum WhereClause {
    /// A fact pattern: [?e :person/name ?name]
    Pattern(Pattern),
    /// A rule invocation: (reachable ?from ?to)
    RuleInvocation {
        /// Predicate name (e.g., "reachable")
        predicate: String,
        /// Arguments (variables, constants, or UUIDs)
        args: Vec<EdnValue>,
    },
    /// Negation as failure: (not clause1 clause2 ...)
    /// Succeeds when none of the inner clauses match.
    Not(Vec<WhereClause>),
    /// not-join: explicit join variables + existentially quantified body.
    /// Succeeds when no assignment to non-join variables satisfies all inner clauses
    /// when join variables are substituted from the outer binding.
    NotJoin {
        join_vars: Vec<String>,
        clauses: Vec<WhereClause>,
    },
    /// Expression clause: `[(expr) ?out?]`
    ///
    /// `binding = None`  → filter: keep binding iff `expr` evaluates to truthy.
    /// `binding = Some`  → bind the result `Value` to the named variable.
    ///
    /// Type mismatches and division by zero silently drop the row.
    Expr { expr: Expr, binding: Option<String> },
    /// Disjunction: (or branch1 branch2 ...) — succeeds if any branch matches.
    /// Each branch is a Vec<WhereClause>. A single clause is a one-element branch.
    Or(Vec<Vec<WhereClause>>),
    /// or-join: (or-join [?v1 ?v2] branch1 branch2 ...)
    /// join_vars are visible to the outer query; branch-private vars are existential.
    OrJoin {
        join_vars: Vec<String>,
        branches: Vec<Vec<WhereClause>>,
    },
}

impl WhereClause {
    /// Collect all rule invocation predicate names, recursively (including inside Not bodies).
    pub fn rule_invocations(&self) -> Vec<&str> {
        match self {
            WhereClause::Pattern(_) => vec![],
            WhereClause::RuleInvocation { predicate, .. } => vec![predicate.as_str()],
            WhereClause::Not(clauses) => {
                clauses.iter().flat_map(|c| c.rule_invocations()).collect()
            }
            WhereClause::NotJoin { clauses, .. } => {
                clauses.iter().flat_map(|c| c.rule_invocations()).collect()
            }
            WhereClause::Expr { .. } => vec![],
            WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => branches
                .iter()
                .flat_map(|b| b.iter().flat_map(|c| c.rule_invocations()))
                .collect(),
        }
    }

    /// True if this clause is a Not or NotJoin containing at least one RuleInvocation.
    pub fn has_negated_invocation(&self) -> bool {
        match self {
            WhereClause::Not(clauses) | WhereClause::NotJoin { clauses, .. } => clauses
                .iter()
                .any(|c| matches!(c, WhereClause::RuleInvocation { .. })),
            WhereClause::Pattern(_)
            | WhereClause::RuleInvocation { .. }
            | WhereClause::Expr { .. }
            | WhereClause::Or(_)
            | WhereClause::OrJoin { .. } => false,
        }
    }
}

/// A Datalog query with :find and :where clauses
///
/// Example with patterns:
/// ```datalog
/// (query [:find ?name ?age
///         :where [?e :person/name ?name]
///                [?e :person/age ?age]])
/// ```
///
/// Example with rule invocation:
/// ```datalog
/// (query [:find ?to
///         :where (reachable :alice ?to)])
/// ```
///
/// Mixed example:
/// ```datalog
/// (query [:find ?name
///         :where (reachable :alice ?person)
///                [?person :person/name ?name]])
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DatalogQuery {
    /// Variables to return (from :find clause)
    pub find: Vec<FindSpec>,
    /// Where clauses: patterns and rule invocations
    pub where_clauses: Vec<WhereClause>,
    /// Optional transaction-time snapshot (:as-of)
    pub as_of: Option<AsOf>,
    /// Optional valid-time filter (:valid-at)
    pub valid_at: Option<ValidAt>,
    /// Grouping variables that participate in grouping but are excluded from output rows.
    pub with_vars: Vec<String>,
}

impl DatalogQuery {
    pub fn new(find: Vec<FindSpec>, where_clauses: Vec<WhereClause>) -> Self {
        DatalogQuery {
            find,
            where_clauses,
            as_of: None,
            valid_at: None,
            with_vars: Vec::new(),
        }
    }

    /// Helper: Create a query with only patterns (for backward compatibility)
    pub fn from_patterns(find: Vec<FindSpec>, patterns: Vec<Pattern>) -> Self {
        DatalogQuery {
            find,
            where_clauses: patterns.into_iter().map(WhereClause::Pattern).collect(),
            as_of: None,
            valid_at: None,
            with_vars: Vec::new(),
        }
    }

    /// Helper: Get all patterns from where clauses
    pub fn get_patterns(&self) -> Vec<Pattern> {
        self.where_clauses
            .iter()
            .filter_map(|clause| match clause {
                WhereClause::Pattern(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }

    /// Recursively collect all (predicate, args) pairs from rule invocations,
    /// including those nested inside Not bodies at any depth.
    fn collect_rule_invocations_recursive(clauses: &[WhereClause]) -> Vec<(String, Vec<EdnValue>)> {
        let mut result = Vec::new();
        for clause in clauses {
            match clause {
                WhereClause::RuleInvocation { predicate, args } => {
                    result.push((predicate.clone(), args.clone()));
                }
                WhereClause::Not(inner) | WhereClause::NotJoin { clauses: inner, .. } => {
                    result.extend(Self::collect_rule_invocations_recursive(inner));
                }
                WhereClause::Pattern(_) => {}
                WhereClause::Expr { .. } => {}
                WhereClause::Or(branches) | WhereClause::OrJoin { branches, .. } => {
                    for branch in branches {
                        result.extend(Self::collect_rule_invocations_recursive(branch));
                    }
                }
            }
        }
        result
    }

    /// Helper: Get all rule invocations from where clauses, including inside Not bodies
    pub fn get_rule_invocations(&self) -> Vec<(String, Vec<EdnValue>)> {
        Self::collect_rule_invocations_recursive(&self.where_clauses)
    }

    /// Get only top-level rule invocations — those NOT nested inside a Not body.
    ///
    /// Used by execute_query_with_rules to build positive patterns from rule heads;
    /// rule invocations inside `not` are handled by the not-post-filter, not here.
    pub fn get_top_level_rule_invocations(&self) -> Vec<(String, Vec<EdnValue>)> {
        self.where_clauses
            .iter()
            .filter_map(|c| match c {
                WhereClause::RuleInvocation { predicate, args } => {
                    Some((predicate.clone(), args.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Check if this query uses any rules (including inside Not bodies at any depth)
    pub fn uses_rules(&self) -> bool {
        self.where_clauses
            .iter()
            .any(|c| !c.rule_invocations().is_empty())
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
    /// The rule body: typed where clauses (patterns, rule invocations, not)
    pub body: Vec<WhereClause>,
}

impl Rule {
    pub fn new(head: Vec<EdnValue>, body: Vec<WhereClause>) -> Self {
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
    /// Optional transaction-level default valid_from (millis since epoch)
    pub valid_from: Option<i64>,
    /// Optional transaction-level default valid_to (millis since epoch)
    pub valid_to: Option<i64>,
}

impl Transaction {
    pub fn new(facts: Vec<Pattern>) -> Self {
        Transaction {
            facts,
            valid_from: None,
            valid_to: None,
        }
    }
}

/// A point-in-time selector for transaction-time travel queries.
///
/// Used with `get_facts_as_of()` to snapshot the database at a past point.
#[derive(Debug, Clone, PartialEq)]
pub enum AsOf {
    /// Select facts whose `tx_count` is ≤ n (monotonic batch counter).
    Counter(u64),
    /// Select facts whose `tx_id` (wall-clock millis since epoch) is ≤ t.
    Timestamp(i64),
    /// Named bind slot: `$name` — resolved to `Counter` or `Timestamp` at execute time.
    Slot(String),
}

/// A point-in-time selector for valid-time travel queries.
///
/// Used with `get_facts_valid_at()` to see which facts were valid at a
/// specific moment in the real world.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidAt {
    /// Return facts where `valid_from <= ts < valid_to`.
    Timestamp(i64),
    /// Return all facts regardless of valid time (no valid-time filter).
    AnyValidTime,
    /// Named bind slot: `$name` — resolved to `Timestamp` or `AnyValidTime` at execute time.
    Slot(String),
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
        assert!(matches!(
            pattern.attribute,
            AttributeSpec::Real(EdnValue::Keyword(_))
        ));
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
            AttributeSpec::Real(EdnValue::Keyword(":person/name".to_string()))
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
            vec![
                FindSpec::Variable("?name".to_string()),
                FindSpec::Variable("?age".to_string()),
            ],
            vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::Symbol("?name".to_string()),
                )),
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?e".to_string()),
                    EdnValue::Keyword(":person/age".to_string()),
                    EdnValue::Symbol("?age".to_string()),
                )),
            ],
        );

        assert_eq!(query.find.len(), 2);
        assert_eq!(query.where_clauses.len(), 2);
        assert_eq!(query.get_patterns().len(), 2);
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

    #[test]
    fn test_datalog_query_with_temporal_fields() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?name".to_string())],
            vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":person/name".to_string()),
                EdnValue::Symbol("?name".to_string()),
            ))],
        );

        assert!(query.as_of.is_none());
        assert!(query.valid_at.is_none());

        let query_with_time = DatalogQuery {
            as_of: Some(AsOf::Counter(5)),
            valid_at: Some(ValidAt::AnyValidTime),
            ..query
        };

        assert!(matches!(query_with_time.as_of, Some(AsOf::Counter(5))));
        assert!(matches!(
            query_with_time.valid_at,
            Some(ValidAt::AnyValidTime)
        ));
    }

    #[test]
    fn test_transaction_with_valid_time() {
        let tx = Transaction {
            facts: vec![],
            valid_from: Some(1672531200000_i64),
            valid_to: None,
        };
        assert_eq!(tx.valid_from, Some(1672531200000_i64));
        assert!(tx.valid_to.is_none());
    }

    #[test]
    fn test_where_clause_not_variant_exists() {
        let not_clause = WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":banned".to_string()),
            EdnValue::Boolean(true),
        ))]);
        assert!(matches!(not_clause, WhereClause::Not(_)));
    }

    #[test]
    fn test_rule_invocations_pattern_returns_empty() {
        let clause = WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":a".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ));
        assert!(clause.rule_invocations().is_empty());
    }

    #[test]
    fn test_rule_invocations_rule_invocation_returns_predicate() {
        let clause = WhereClause::RuleInvocation {
            predicate: "blocked".to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        };
        assert_eq!(clause.rule_invocations(), vec!["blocked"]);
    }

    #[test]
    fn test_rule_invocations_recurses_into_not() {
        let clause = WhereClause::Not(vec![WhereClause::RuleInvocation {
            predicate: "blocked".to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }]);
        assert_eq!(clause.rule_invocations(), vec!["blocked"]);
    }

    #[test]
    fn test_has_negated_invocation_true_when_not_contains_rule_invocation() {
        let clause = WhereClause::Not(vec![WhereClause::RuleInvocation {
            predicate: "blocked".to_string(),
            args: vec![EdnValue::Symbol("?x".to_string())],
        }]);
        assert!(clause.has_negated_invocation());
    }

    #[test]
    fn test_has_negated_invocation_false_when_not_contains_only_pattern() {
        let clause = WhereClause::Not(vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?x".to_string()),
            EdnValue::Keyword(":banned".to_string()),
            EdnValue::Boolean(true),
        ))]);
        assert!(!clause.has_negated_invocation());
    }

    #[test]
    fn test_uses_rules_recurses_into_not_body() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?person".to_string())],
            vec![
                WhereClause::Pattern(Pattern::new(
                    EdnValue::Symbol("?person".to_string()),
                    EdnValue::Keyword(":person/name".to_string()),
                    EdnValue::Symbol("?name".to_string()),
                )),
                WhereClause::Not(vec![WhereClause::RuleInvocation {
                    predicate: "blocked".to_string(),
                    args: vec![EdnValue::Symbol("?person".to_string())],
                }]),
            ],
        );
        assert!(query.uses_rules());
    }

    #[test]
    fn test_get_rule_invocations_recurses_into_not_body() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?person".to_string())],
            vec![WhereClause::Not(vec![WhereClause::RuleInvocation {
                predicate: "blocked".to_string(),
                args: vec![EdnValue::Symbol("?person".to_string())],
            }])],
        );
        let invocations = query.get_rule_invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].0, "blocked");
    }

    #[test]
    fn test_where_clause_not_join_variant_exists() {
        let nj = WhereClause::NotJoin {
            join_vars: vec!["?e".to_string()],
            clauses: vec![WhereClause::Pattern(Pattern::new(
                EdnValue::Symbol("?e".to_string()),
                EdnValue::Keyword(":tag".to_string()),
                EdnValue::Symbol("?tag".to_string()),
            ))],
        };
        assert!(matches!(nj, WhereClause::NotJoin { .. }));
    }

    #[test]
    fn test_rule_invocations_recurses_into_not_join() {
        let nj = WhereClause::NotJoin {
            join_vars: vec!["?e".to_string()],
            clauses: vec![WhereClause::RuleInvocation {
                predicate: "blocked".to_string(),
                args: vec![EdnValue::Symbol("?e".to_string())],
            }],
        };
        let invocations = nj.rule_invocations();
        assert_eq!(invocations, vec!["blocked"]);
    }

    #[test]
    fn test_has_negated_invocation_true_for_not_join_with_rule_invocation() {
        let nj = WhereClause::NotJoin {
            join_vars: vec!["?e".to_string()],
            clauses: vec![WhereClause::RuleInvocation {
                predicate: "blocked".to_string(),
                args: vec![EdnValue::Symbol("?e".to_string())],
            }],
        };
        assert!(nj.has_negated_invocation());
    }

    #[test]
    fn test_collect_rule_invocations_recurses_into_not_join() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?e".to_string())],
            vec![WhereClause::NotJoin {
                join_vars: vec!["?e".to_string()],
                clauses: vec![WhereClause::RuleInvocation {
                    predicate: "blocked".to_string(),
                    args: vec![EdnValue::Symbol("?e".to_string())],
                }],
            }],
        );
        let invocations = query.get_rule_invocations();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].0, "blocked");
    }

    #[test]
    fn test_get_top_level_rule_invocations_excludes_not_join_body() {
        // not-join body rule invocations are NOT top-level
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?e".to_string())],
            vec![
                WhereClause::RuleInvocation {
                    predicate: "reachable".to_string(),
                    args: vec![
                        EdnValue::Symbol("?e".to_string()),
                        EdnValue::Symbol("?x".to_string()),
                    ],
                },
                WhereClause::NotJoin {
                    join_vars: vec!["?e".to_string()],
                    clauses: vec![WhereClause::RuleInvocation {
                        predicate: "blocked".to_string(),
                        args: vec![EdnValue::Symbol("?e".to_string())],
                    }],
                },
            ],
        );
        let top_level = query.get_top_level_rule_invocations();
        // Only "reachable" is top-level; "blocked" is inside not-join
        assert_eq!(top_level.len(), 1);
        assert_eq!(top_level[0].0, "reachable");
    }

    #[test]
    fn test_find_spec_variable_display_and_var() {
        let spec = FindSpec::Variable("?name".to_string());
        assert_eq!(spec.display_name(), "?name");
        assert_eq!(spec.var(), "?name");
    }

    #[test]
    fn test_find_spec_aggregate_display_and_var() {
        let spec = FindSpec::Aggregate {
            func: "count-distinct".to_string(),
            var: "?e".to_string(),
        };
        assert_eq!(spec.display_name(), "(count-distinct ?e)");
        assert_eq!(spec.var(), "?e");
    }

    #[test]
    fn test_find_spec_all_agg_display_names() {
        let cases = [
            ("count", "?e", "(count ?e)"),
            ("count-distinct", "?e", "(count-distinct ?e)"),
            ("sum", "?v", "(sum ?v)"),
            ("sum-distinct", "?v", "(sum-distinct ?v)"),
            ("min", "?x", "(min ?x)"),
            ("max", "?x", "(max ?x)"),
        ];
        for (func, var, expected) in cases {
            let spec = FindSpec::Aggregate {
                func: func.to_string(),
                var: var.to_string(),
            };
            assert_eq!(spec.display_name(), expected);
        }
    }

    #[test]
    fn test_window_spec_func_name() {
        assert_eq!(
            WindowSpec {
                func: WindowFunc::Sum,
                var: Some("?v".to_string()),
                partition_by: None,
                order_by: "?x".to_string(),
                order: Order::Asc,
            }
            .func_name(),
            "sum"
        );
        assert_eq!(
            WindowSpec {
                func: WindowFunc::Rank,
                var: None,
                partition_by: None,
                order_by: "?x".to_string(),
                order: Order::Desc,
            }
            .func_name(),
            "rank"
        );
        assert_eq!(
            WindowSpec {
                func: WindowFunc::RowNumber,
                var: None,
                partition_by: None,
                order_by: "?x".to_string(),
                order: Order::Asc,
            }
            .func_name(),
            "row-number"
        );
        let ws_udf = WindowSpec {
            func: WindowFunc::Udf("geomean".to_string()),
            var: Some("?v".to_string()),
            partition_by: None,
            order_by: "?o".to_string(),
            order: Order::Asc,
        };
        assert_eq!(ws_udf.func_name(), "geomean".to_string());
    }

    #[test]
    fn test_find_spec_window_display_and_var() {
        let ws_with_var = WindowSpec {
            func: WindowFunc::Sum,
            var: Some("?salary".to_string()),
            partition_by: None,
            order_by: "?date".to_string(),
            order: Order::Asc,
        };
        let spec = FindSpec::Window(ws_with_var);
        assert_eq!(spec.display_name(), "(sum ?salary :over ...)");
        assert_eq!(spec.var(), "?salary");

        let ws_no_var = WindowSpec {
            func: WindowFunc::Rank,
            var: None,
            partition_by: None,
            order_by: "?date".to_string(),
            order: Order::Desc,
        };
        let spec2 = FindSpec::Window(ws_no_var);
        assert_eq!(spec2.display_name(), "(rank :over ...)");
        assert_eq!(spec2.var(), "__window_var");
    }

    #[test]
    fn test_binop_variants_exist() {
        let _ = BinOp::Lt;
        let _ = BinOp::Gt;
        let _ = BinOp::Lte;
        let _ = BinOp::Gte;
        let _ = BinOp::Eq;
        let _ = BinOp::Neq;
        let _ = BinOp::Add;
        let _ = BinOp::Sub;
        let _ = BinOp::Mul;
        let _ = BinOp::Div;
        let _ = BinOp::StartsWith;
        let _ = BinOp::EndsWith;
        let _ = BinOp::Contains;
    }

    #[test]
    fn test_unary_op_variants_exist() {
        let _ = UnaryOp::StringQ;
        let _ = UnaryOp::IntegerQ;
        let _ = UnaryOp::FloatQ;
        let _ = UnaryOp::BooleanQ;
        let _ = UnaryOp::NilQ;
        let _ = UnaryOp::Udf("test-fn".to_string());
    }

    #[test]
    fn test_expr_var_and_lit() {
        use crate::graph::types::Value;
        let e = Expr::Var("?x".to_string());
        assert!(matches!(e, Expr::Var(_)));
        let l = Expr::Lit(Value::Integer(42));
        assert!(matches!(l, Expr::Lit(_)));
    }

    #[test]
    fn test_expr_binop_nested() {
        use crate::graph::types::Value;
        let e = Expr::BinOp(
            BinOp::Add,
            Box::new(Expr::Var("?a".to_string())),
            Box::new(Expr::Lit(Value::Integer(1))),
        );
        assert!(matches!(e, Expr::BinOp(BinOp::Add, _, _)));
    }

    #[test]
    fn test_where_clause_expr_filter_variant() {
        use crate::graph::types::Value;
        let clause = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Lt,
                Box::new(Expr::Var("?v".to_string())),
                Box::new(Expr::Lit(Value::Integer(100))),
            ),
            binding: None,
        };
        assert!(matches!(clause, WhereClause::Expr { binding: None, .. }));
    }

    #[test]
    fn test_where_clause_expr_binding_variant() {
        let clause = WhereClause::Expr {
            expr: Expr::BinOp(
                BinOp::Add,
                Box::new(Expr::Var("?a".to_string())),
                Box::new(Expr::Var("?b".to_string())),
            ),
            binding: Some("?sum".to_string()),
        };
        assert!(matches!(
            clause,
            WhereClause::Expr {
                binding: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn test_expr_clause_rule_invocations_empty() {
        use crate::graph::types::Value;
        let clause = WhereClause::Expr {
            expr: Expr::Lit(Value::Boolean(true)),
            binding: None,
        };
        assert!(clause.rule_invocations().is_empty());
        assert!(!clause.has_negated_invocation());
    }

    #[test]
    fn test_where_clause_or_variant_exists() {
        let branch1 = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":a".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ))];
        let branch2 = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":b".to_string()),
            EdnValue::Symbol("?v".to_string()),
        ))];
        let or_clause = WhereClause::Or(vec![branch1, branch2]);
        assert!(matches!(or_clause, WhereClause::Or(_)));
    }

    #[test]
    fn test_where_clause_or_join_variant_exists() {
        let branch = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":tag".to_string()),
            EdnValue::Symbol("?tag".to_string()),
        ))];
        let oj = WhereClause::OrJoin {
            join_vars: vec!["?e".to_string()],
            branches: vec![branch],
        };
        assert!(matches!(oj, WhereClause::OrJoin { .. }));
    }

    #[test]
    fn test_rule_invocations_recurses_into_or_branches() {
        let branch = vec![WhereClause::RuleInvocation {
            predicate: "active".to_string(),
            args: vec![EdnValue::Symbol("?e".to_string())],
        }];
        let or_clause = WhereClause::Or(vec![branch]);
        assert_eq!(or_clause.rule_invocations(), vec!["active"]);
    }

    #[test]
    fn test_has_negated_invocation_false_for_or() {
        let branch = vec![WhereClause::Pattern(Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":a".to_string()),
            EdnValue::Boolean(true),
        ))];
        let or_clause = WhereClause::Or(vec![branch]);
        assert!(!or_clause.has_negated_invocation());
    }

    #[test]
    fn test_collect_rule_invocations_recurses_into_or_branches() {
        let query = DatalogQuery::new(
            vec![FindSpec::Variable("?e".to_string())],
            vec![WhereClause::Or(vec![
                vec![WhereClause::RuleInvocation {
                    predicate: "active".to_string(),
                    args: vec![EdnValue::Symbol("?e".to_string())],
                }],
                vec![WhereClause::RuleInvocation {
                    predicate: "pending".to_string(),
                    args: vec![EdnValue::Symbol("?e".to_string())],
                }],
            ])],
        );
        let invocations = query.get_rule_invocations();
        assert_eq!(invocations.len(), 2);
        let pred_names: Vec<&str> = invocations.iter().map(|(p, _)| p.as_str()).collect();
        assert!(pred_names.contains(&"active"));
        assert!(pred_names.contains(&"pending"));
    }

    #[test]
    fn test_pseudo_attr_from_keyword_known() {
        assert!(matches!(
            PseudoAttr::from_keyword(":db/valid-from"),
            Some(PseudoAttr::ValidFrom)
        ));
        assert!(matches!(
            PseudoAttr::from_keyword(":db/valid-to"),
            Some(PseudoAttr::ValidTo)
        ));
        assert!(matches!(
            PseudoAttr::from_keyword(":db/tx-count"),
            Some(PseudoAttr::TxCount)
        ));
        assert!(matches!(
            PseudoAttr::from_keyword(":db/tx-id"),
            Some(PseudoAttr::TxId)
        ));
        assert!(matches!(
            PseudoAttr::from_keyword(":db/valid-at"),
            Some(PseudoAttr::ValidAt)
        ));
    }

    #[test]
    fn test_pseudo_attr_from_keyword_unknown() {
        assert!(PseudoAttr::from_keyword(":person/name").is_none());
        assert!(PseudoAttr::from_keyword(":db/other").is_none());
        assert!(PseudoAttr::from_keyword("").is_none());
    }

    #[test]
    fn test_pseudo_attr_is_per_fact() {
        assert!(PseudoAttr::ValidFrom.is_per_fact());
        assert!(PseudoAttr::ValidTo.is_per_fact());
        assert!(PseudoAttr::TxCount.is_per_fact());
        assert!(PseudoAttr::TxId.is_per_fact());
        assert!(!PseudoAttr::ValidAt.is_per_fact());
    }

    #[test]
    fn test_attribute_spec_real_variant() {
        let spec = AttributeSpec::Real(EdnValue::Keyword(":person/name".to_string()));
        assert!(matches!(spec, AttributeSpec::Real(_)));
    }

    #[test]
    fn test_attribute_spec_pseudo_variant() {
        let spec = AttributeSpec::Pseudo(PseudoAttr::ValidFrom);
        assert!(matches!(spec, AttributeSpec::Pseudo(PseudoAttr::ValidFrom)));
    }

    #[test]
    fn test_pattern_new_wraps_real() {
        let p = Pattern::new(
            EdnValue::Symbol("?e".to_string()),
            EdnValue::Keyword(":person/name".to_string()),
            EdnValue::Symbol("?n".to_string()),
        );
        assert!(matches!(p.attribute, AttributeSpec::Real(_)));
    }

    #[test]
    fn test_pattern_real_constructor() {
        let p = Pattern::real(
            EdnValue::Keyword("alice".to_string()),
            EdnValue::Keyword("person/name".to_string()),
            EdnValue::String("Alice".to_string()),
        );
        assert!(
            matches!(p.attribute, AttributeSpec::Real(_)),
            "Pattern::real should wrap attribute in AttributeSpec::Real"
        );
    }

    #[test]
    fn test_pattern_pseudo_wraps_pseudo() {
        let p = Pattern::pseudo(
            EdnValue::Symbol("?e".to_string()),
            PseudoAttr::ValidFrom,
            EdnValue::Symbol("?vf".to_string()),
        );
        assert!(matches!(
            p.attribute,
            AttributeSpec::Pseudo(PseudoAttr::ValidFrom)
        ));
    }

    #[test]
    fn test_pseudo_attr_as_keyword() {
        assert_eq!(PseudoAttr::ValidFrom.as_keyword(), ":db/valid-from");
        assert_eq!(PseudoAttr::ValidTo.as_keyword(), ":db/valid-to");
        assert_eq!(PseudoAttr::TxCount.as_keyword(), ":db/tx-count");
        assert_eq!(PseudoAttr::TxId.as_keyword(), ":db/tx-id");
        assert_eq!(PseudoAttr::ValidAt.as_keyword(), ":db/valid-at");
    }

    #[test]
    fn test_bind_slot_edn_variant_exists() {
        let v = EdnValue::BindSlot("entity".to_string());
        assert!(matches!(v, EdnValue::BindSlot(_)));
        // BindSlot is not a logic variable — it is not a ?-prefixed symbol
        assert!(!v.is_variable());
        assert!(v.as_variable().is_none());
    }

    #[test]
    fn test_as_of_slot_variant_exists() {
        let a = AsOf::Slot("tx".to_string());
        assert!(matches!(a, AsOf::Slot(_)));
    }

    #[test]
    fn test_valid_at_slot_variant_exists() {
        let v = ValidAt::Slot("date".to_string());
        assert!(matches!(v, ValidAt::Slot(_)));
    }

    #[test]
    fn test_expr_slot_variant_exists() {
        let e = Expr::Slot("threshold".to_string());
        assert!(matches!(e, Expr::Slot(_)));
    }
}
