use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ============================================================================
// Datalog EAV Model (Phase 3+)
// ============================================================================

/// Transaction ID type - milliseconds since UNIX epoch
///
/// We use timestamps as transaction IDs for natural chronological ordering
/// and consistency with bi-temporal valid_time (Phase 4). Millisecond precision
/// is sufficient for Phase 3's single-threaded usage.
pub type TxId = u64;

/// Get current timestamp as transaction ID (milliseconds since UNIX epoch)
pub fn tx_id_now() -> TxId {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_millis() as u64
}

/// Create a TxId from a SystemTime
pub fn tx_id_from_system_time(time: SystemTime) -> TxId {
    time.duration_since(UNIX_EPOCH)
        .expect("System time before UNIX epoch")
        .as_millis() as u64
}

/// Convert a TxId back to a SystemTime
pub fn tx_id_to_system_time(tx_id: TxId) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_millis(tx_id)
}

/// Entity ID type - using UUID for unique entity identification
pub type EntityId = Uuid;

/// Attribute name - namespace-qualified keywords like ":person/name" or ":friend"
pub type Attribute = String;

/// Value types for Datalog facts
///
/// The Value enum represents all possible value types that can be stored in facts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Value {
    /// String value
    String(String),
    /// 64-bit integer
    Integer(i64),
    /// 64-bit floating point
    Float(f64),
    /// Boolean value
    Boolean(bool),
    /// Reference to another entity (for relationships)
    Ref(EntityId),
    /// Keyword (e.g., ":status/active", ":person")
    Keyword(String),
    /// Null/None value
    Null,
}

impl Eq for Value {}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Ordering for `Value` variants.
///
/// # NaN Semantics
/// - `NaN` is ordered as **Greater** than all other float values (including positive infinity)
/// - `NaN` equals `NaN` (two NaN values are considered equal)
/// - When comparing a float against a non-float, the float's NaN status determines ordering
///
/// # Cross-Variant Ordering
/// Values of different variants are ordered by discriminant:
/// - String (0) < Integer (1) < Float (2) < Boolean (3) < Ref (4) < Keyword (5) < Null (6)
impl Ord for Value {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Simple variant ordering using match
        match (self, other) {
            // Same variant - compare inner values
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Integer(a), Value::Integer(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                if a.is_nan() && b.is_nan() {
                    std::cmp::Ordering::Equal
                } else if a.is_nan() {
                    std::cmp::Ordering::Greater
                } else if b.is_nan() {
                    std::cmp::Ordering::Less
                } else {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                }
            }
            (Value::Boolean(a), Value::Boolean(b)) => a.cmp(b),
            (Value::Ref(a), Value::Ref(b)) => a.cmp(b),
            (Value::Keyword(a), Value::Keyword(b)) => a.cmp(b),
            (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
            // Different variants — order by a stable integer discriminant so
            // the total order is deterministic and does not depend on stack
            // addresses (the previous implementation used pointer values here,
            // which are non-deterministic and violate Ord's contract).
            _ => {
                fn discriminant(v: &Value) -> u8 {
                    match v {
                        Value::String(_) => 0,
                        Value::Integer(_) => 1,
                        Value::Float(_) => 2,
                        Value::Boolean(_) => 3,
                        Value::Ref(_) => 4,
                        Value::Keyword(_) => 5,
                        Value::Null => 6,
                    }
                }
                discriminant(self).cmp(&discriminant(other))
            }
        }
    }
}

impl Value {
    /// Extract string value if this is a String variant
    pub fn as_string(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Extract integer value if this is an Integer variant
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Extract float value if this is a Float variant
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Extract boolean value if this is a Boolean variant
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Extract entity reference if this is a Ref variant
    pub fn as_ref(&self) -> Option<EntityId> {
        match self {
            Value::Ref(id) => Some(*id),
            _ => None,
        }
    }

    /// Extract keyword if this is a Keyword variant
    pub fn as_keyword(&self) -> Option<&str> {
        match self {
            Value::Keyword(k) => Some(k),
            _ => None,
        }
    }

    /// Check if this value is Null
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// Sentinel value for open-ended valid time (a fact is valid "forever").
/// Used as the default `valid_to` when no end time is specified.
pub const VALID_TIME_FOREVER: i64 = i64::MAX;

/// A Datalog fact: (Entity, Attribute, Value) triple with transaction metadata
///
/// This is the core data structure for Phase 3+. Facts are immutable and versioned
/// by transaction ID. Facts are never deleted, only retracted (asserted=false).
///
/// Phase 4 adds bi-temporal fields:
/// - `tx_count`: monotonically incrementing batch counter within a transaction
/// - `valid_from`: when the fact became valid in the real world (millis since epoch)
/// - `valid_to`: when the fact stopped being valid (`VALID_TIME_FOREVER` = open-ended)
///
/// # Examples
/// ```
/// use minigraf::{Fact, Value};
/// use uuid::Uuid;
///
/// // Fact: Alice's name is "Alice"
/// let alice_id = Uuid::new_v4();
/// let fact = Fact::new(
///     alice_id,
///     ":person/name".to_string(),
///     Value::String("Alice".to_string()),
///     1, // transaction ID
/// );
///
/// // Fact: Alice is friends with Bob (reference)
/// let bob_id = Uuid::new_v4();
/// let friendship = Fact::new(
///     alice_id,
///     ":friend".to_string(),
///     Value::Ref(bob_id),
///     2,
/// );
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Fact {
    /// The entity this fact is about
    pub entity: EntityId,
    /// The attribute/property name (namespace-qualified, e.g., ":person/name")
    pub attribute: Attribute,
    /// The value of this attribute
    pub value: Value,
    /// Transaction ID that asserted or retracted this fact
    pub tx_id: TxId,
    /// Monotonically incrementing batch counter within a transaction (Phase 4)
    pub tx_count: u64,
    /// Valid-time start: when the fact became valid in the real world (millis since epoch).
    /// Defaults to `tx_id as i64` (wall-clock time of the transaction).
    pub valid_from: i64,
    /// Valid-time end: when the fact stopped being valid (millis since epoch).
    /// `VALID_TIME_FOREVER` means the fact is open-ended (still valid).
    pub valid_to: i64,
    /// True if this fact is asserted, false if retracted.
    /// Retractions are used instead of deletions to maintain history.
    pub asserted: bool,
}

impl Fact {
    /// Create a new asserted fact with default valid time (valid_from=tx_id, valid_to=FOREVER).
    pub fn new(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: tx_id as i64,
            valid_to: VALID_TIME_FOREVER,
            asserted: true,
        }
    }

    /// Create an asserted fact with explicit valid time and tx_count.
    pub fn with_valid_time(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
        tx_count: u64,
        valid_from: i64,
        valid_to: i64,
    ) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count,
            valid_from,
            valid_to,
            asserted: true,
        }
    }

    /// Create a retraction with default valid time.
    pub fn retract(entity: EntityId, attribute: Attribute, value: Value, tx_id: TxId) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: tx_id as i64,
            valid_to: VALID_TIME_FOREVER,
            asserted: false,
        }
    }

    /// Create a fact with explicit asserted flag and default valid time.
    pub fn with_asserted(
        entity: EntityId,
        attribute: Attribute,
        value: Value,
        tx_id: TxId,
        asserted: bool,
    ) -> Self {
        Fact {
            entity,
            attribute,
            value,
            tx_id,
            tx_count: 0,
            valid_from: tx_id as i64,
            valid_to: VALID_TIME_FOREVER,
            asserted,
        }
    }

    /// Check if this is an assertion (not a retraction)
    pub fn is_asserted(&self) -> bool {
        self.asserted
    }

    /// Check if this is a retraction
    pub fn is_retracted(&self) -> bool {
        !self.asserted
    }
}

/// Options for controlling valid time on a transact/retract call.
///
/// When `valid_from` is `None`, defaults to the transaction timestamp.
/// When `valid_to` is `None`, defaults to `VALID_TIME_FOREVER` (open-ended).
#[derive(Debug, Clone, Default)]
pub struct TransactOptions {
    /// Override the valid-time start (millis since epoch). `None` = use tx timestamp.
    pub valid_from: Option<i64>,
    /// Override the valid-time end (millis since epoch). `None` = open-ended (FOREVER).
    pub valid_to: Option<i64>,
}

impl TransactOptions {
    pub fn new(valid_from: Option<i64>, valid_to: Option<i64>) -> Self {
        TransactOptions {
            valid_from,
            valid_to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_id_timestamp() {
        use std::time::SystemTime;

        // Test tx_id_now() returns a reasonable timestamp
        let tx1 = tx_id_now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let tx2 = tx_id_now();

        // tx2 should be after tx1
        assert!(
            tx2 > tx1,
            "Transaction IDs should be chronologically ordered"
        );

        // Difference should be at least 5ms (we slept for 5ms)
        assert!(tx2 - tx1 >= 5, "Expected at least 5ms difference");

        // Test round-trip conversion
        let now = SystemTime::now();
        let tx_id = tx_id_from_system_time(now);
        let recovered = tx_id_to_system_time(tx_id);

        // Should be within 1ms (we lose precision converting to millis)
        let diff = recovered
            .duration_since(now)
            .unwrap_or_else(|e| e.duration());
        assert!(
            diff.as_millis() < 1,
            "Round-trip conversion should preserve timestamp within 1ms"
        );
    }

    #[test]
    fn test_tx_id_ordering() {
        // TxIds created sequentially should be ordered
        let mut tx_ids = vec![];
        for _ in 0..5 {
            tx_ids.push(tx_id_now());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        // Verify chronological order
        for i in 1..tx_ids.len() {
            assert!(
                tx_ids[i] > tx_ids[i - 1],
                "TxIds should be strictly increasing"
            );
        }
    }

    #[test]
    fn test_value_creation_and_accessors() {
        // String value
        let string_val = Value::String("Alice".to_string());
        assert_eq!(string_val.as_string(), Some("Alice"));
        assert_eq!(string_val.as_integer(), None);
        assert!(!string_val.is_null());

        // Integer value
        let int_val = Value::Integer(42);
        assert_eq!(int_val.as_integer(), Some(42));
        assert_eq!(int_val.as_string(), None);

        // Float value
        let float_val = Value::Float(4.5);
        assert_eq!(float_val.as_float(), Some(4.5));

        // Boolean value
        let bool_val = Value::Boolean(true);
        assert_eq!(bool_val.as_boolean(), Some(true));

        // Reference value
        let ref_id = Uuid::new_v4();
        let ref_val = Value::Ref(ref_id);
        assert_eq!(ref_val.as_ref(), Some(ref_id));
        assert_eq!(ref_val.as_string(), None);

        // Keyword value
        let keyword_val = Value::Keyword(":person".to_string());
        assert_eq!(keyword_val.as_keyword(), Some(":person"));

        // Null value
        let null_val = Value::Null;
        assert!(null_val.is_null());
        assert_eq!(null_val.as_string(), None);
    }

    #[test]
    fn test_fact_creation() {
        let entity = Uuid::new_v4();
        let fact = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        assert_eq!(fact.entity, entity);
        assert_eq!(fact.attribute, ":person/name");
        assert_eq!(fact.value, Value::String("Alice".to_string()));
        assert_eq!(fact.tx_id, 1);
        assert!(fact.is_asserted());
        assert!(!fact.is_retracted());
    }

    #[test]
    fn test_fact_retraction() {
        let entity = Uuid::new_v4();
        let fact = Fact::retract(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            2,
        );

        assert_eq!(fact.entity, entity);
        assert_eq!(fact.attribute, ":person/name");
        assert_eq!(fact.tx_id, 2);
        assert!(!fact.is_asserted());
        assert!(fact.is_retracted());
    }

    #[test]
    fn test_fact_with_ref_value() {
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();

        // Fact: Alice is friends with Bob
        let friendship = Fact::new(alice, ":friend".to_string(), Value::Ref(bob), 1);

        assert_eq!(friendship.entity, alice);
        assert_eq!(friendship.attribute, ":friend");
        assert_eq!(friendship.value.as_ref(), Some(bob));
        assert!(friendship.is_asserted());
    }

    #[test]
    fn test_fact_equality() {
        let entity = Uuid::new_v4();

        let fact1 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        let fact2 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
        );

        assert_eq!(fact1, fact2);

        // Different transaction ID = different fact
        let fact3 = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            2,
        );

        assert_ne!(fact1, fact3);

        // Different tx_count = different fact
        let fact4 = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
            99, // tx_count differs from fact1 (which has tx_count=0)
            1,
            VALID_TIME_FOREVER,
        );

        assert_ne!(fact1, fact4);

        // Different valid_from = different fact
        let fact5 = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
            0,
            9999, // valid_from differs from fact1 (which has valid_from=tx_id=1)
            VALID_TIME_FOREVER,
        );

        assert_ne!(fact1, fact5);

        // Different valid_to = different fact
        let fact6 = Fact::with_valid_time(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1,
            0,
            1,
            12345, // valid_to differs from fact1 (which has valid_to=VALID_TIME_FOREVER)
        );

        assert_ne!(fact1, fact6);
    }

    #[test]
    fn test_value_types() {
        let values = vec![
            Value::String("test".to_string()),
            Value::Integer(42),
            Value::Float(4.5),
            Value::Boolean(true),
            Value::Ref(Uuid::new_v4()),
            Value::Keyword(":status/active".to_string()),
            Value::Null,
        ];

        // All values should serialize/deserialize correctly
        for value in values {
            let serialized = serde_json::to_string(&value).unwrap();
            let deserialized: Value = serde_json::from_str(&serialized).unwrap();
            assert_eq!(value, deserialized);
        }
    }

    #[test]
    fn test_fact_has_valid_time_fields() {
        let entity = Uuid::new_v4();
        let fact = Fact::new(
            entity,
            ":person/name".to_string(),
            Value::String("Alice".to_string()),
            1000,
        );
        // Defaults: valid_from = tx_id as i64, valid_to = FOREVER, tx_count = 0
        assert_eq!(fact.valid_from, 1000_i64);
        assert_eq!(fact.valid_to, VALID_TIME_FOREVER);
        assert_eq!(fact.tx_count, 0);
    }

    #[test]
    fn test_fact_with_explicit_valid_time() {
        let entity = Uuid::new_v4();
        let fact = Fact::with_valid_time(
            entity,
            ":employment/status".to_string(),
            Value::Keyword(":active".to_string()),
            1000,
            1,
            1672531200000_i64, // 2023-01-01
            1685577600000_i64, // 2023-06-01
        );
        assert_eq!(fact.valid_from, 1672531200000_i64);
        assert_eq!(fact.valid_to, 1685577600000_i64);
        assert_eq!(fact.tx_count, 1);
    }

    #[test]
    fn test_valid_time_forever_constant() {
        assert_eq!(VALID_TIME_FOREVER, i64::MAX);
    }

    #[test]
    fn test_transact_options_defaults() {
        let opts = TransactOptions::default();
        assert!(opts.valid_from.is_none());
        assert!(opts.valid_to.is_none());
    }

    // Helper used by the cross-variant Ord tests: moves values into a fresh
    // stack frame so the two parameters are always at distinct, consistent
    // addresses independent of the call site.
    fn cmp_values(a: Value, b: Value) -> std::cmp::Ordering {
        a.cmp(&b)
    }

    #[test]
    fn value_ord_cross_variant_is_antisymmetric() {
        // The pointer-address bug causes both cmp_values(A, B) and
        // cmp_values(B, A) to return the same ordering (whichever parameter
        // slot has the lower address "wins" in both calls), violating the
        // antisymmetry requirement for Ord: cmp(a,b) must equal reverse(cmp(b,a)).
        let pairs: &[(Value, Value)] = &[
            (Value::String("hello".into()), Value::Integer(42)),
            (Value::Integer(1), Value::Float(1.0)),
            (Value::Boolean(true), Value::Null),
            (Value::Keyword(":k".into()), Value::String("x".into())),
        ];
        for (a, b) in pairs {
            let forward = cmp_values(a.clone(), b.clone());
            let backward = cmp_values(b.clone(), a.clone());
            assert_eq!(
                forward,
                backward.reverse(),
                "Value::Ord cross-variant comparison must be antisymmetric"
            );
            assert_ne!(
                forward,
                std::cmp::Ordering::Equal,
                "Values of different types must not compare as Equal"
            );
        }
    }

    #[test]
    fn value_ord_cross_variant_is_stable() {
        // Ordering between two fixed variant types must be stable: it should
        // not depend on what the inner value is, only on which variant it is.
        // With the pointer-address bug every allocation is at a different
        // address, so this constraint can be violated non-deterministically.
        let string_lt_integer = cmp_values(Value::String("a".into()), Value::Integer(0));
        for s in ["", "z", "hello world"] {
            for i in [i64::MIN, 0, i64::MAX] {
                let result = cmp_values(Value::String(s.into()), Value::Integer(i));
                assert_eq!(
                    result, string_lt_integer,
                    "Cross-variant ordering must depend only on the variant, not the inner value"
                );
            }
        }
    }
}
