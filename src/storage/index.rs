//! Keys of the in-memory pending indexes, and the canonical value encoding.
//!
//! Facts written since the last checkpoint are indexed in memory by entity
//! (EAVT) and by attribute (AEVT) over their UUIDs and strings, mapping to the
//! fact's position in `FactStorage`'s pending list. Committed facts are read
//! through the on-disk covering indexes instead ([`crate::storage::keys`]).

use crate::graph::types::{Attribute, EntityId, Fact, Value};

// ─── Canonical Value Encoding ───────────────────────────────────────────────

/// Encode a `Value` to bytes that preserve sort order across all variants.
///
/// Discriminant assignment (first byte):
///   0x00 = Null, 0x01 = Boolean, 0x02 = Integer, 0x03 = Float,
///   0x04 = String, 0x05 = Keyword, 0x06 = Ref
///
/// Within each type, big-endian layout ensures byte-wise comparison matches
/// the natural order of the type.
pub fn encode_value(v: &Value) -> Vec<u8> {
    match v {
        Value::Null => vec![0x00],
        Value::Boolean(b) => vec![0x01, *b as u8],
        Value::Integer(n) => {
            let mut bytes = Vec::with_capacity(9);
            bytes.push(0x02);
            // Flip the sign bit so that negative numbers sort before positive
            // after unsigned byte comparison: MIN..=-1 maps to 0..0x7FFF...,
            // 0..=MAX maps to 0x8000...=0xFFFF...
            let bits = (*n).cast_unsigned() ^ 0x8000_0000_0000_0000;
            bytes.extend_from_slice(&bits.to_be_bytes());
            bytes
        }
        Value::Float(f) => {
            let mut bytes = Vec::with_capacity(9);
            bytes.push(0x03);
            let bits = if f.is_nan() {
                // Canonicalize all NaN to a single bit pattern (quiet NaN, positive)
                0x7FF8_0000_0000_0000u64
            } else {
                let raw = f.to_bits();
                if raw >> 63 == 0 {
                    raw ^ 0x8000_0000_0000_0000 // positive: flip sign bit
                } else {
                    !raw // negative: flip all bits
                }
            };
            bytes.extend_from_slice(&bits.to_be_bytes());
            bytes
        }
        Value::String(s) => {
            let mut bytes = Vec::with_capacity(s.len().saturating_add(1));
            bytes.push(0x04);
            bytes.extend_from_slice(s.as_bytes());
            bytes
        }
        Value::Keyword(k) => {
            let mut bytes = Vec::with_capacity(k.len().saturating_add(1));
            bytes.push(0x05);
            bytes.extend_from_slice(k.as_bytes());
            bytes
        }
        Value::Ref(id) => {
            let mut bytes = Vec::with_capacity(17);
            bytes.push(0x06);
            bytes.extend_from_slice(id.as_bytes());
            bytes
        }
    }
}

// ─── Index Key Types ─────────────────────────────────────────────────────────

/// EAVT: sort by (Entity, Attribute, ValidFrom, ValidTo, TxCount, ValueBytes, Asserted)
///
/// `value_bytes` and `asserted` come last so entity/attribute range scans keep
/// their order; they keep facts that differ only in value or assert/retract
/// from colliding (#371, #287).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EavtKey {
    pub entity: EntityId,
    pub attribute: Attribute,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
    pub value_bytes: Vec<u8>,
    pub asserted: bool,
}

/// AEVT: sort by (Attribute, Entity, ValidFrom, ValidTo, TxCount, ValueBytes, Asserted)
///
/// `value_bytes` and `asserted` come last so entity/attribute range scans keep
/// their order; they keep facts that differ only in value or assert/retract
/// from colliding (#371, #287).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AevtKey {
    pub attribute: Attribute,
    pub entity: EntityId,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
    pub value_bytes: Vec<u8>,
    pub asserted: bool,
}

impl EavtKey {
    pub fn from_fact(f: &Fact) -> Self {
        EavtKey {
            entity: f.entity,
            attribute: f.attribute.clone(),
            valid_from: f.valid_from,
            valid_to: f.valid_to,
            tx_count: f.tx_count,
            value_bytes: encode_value(&f.value),
            asserted: f.asserted,
        }
    }

    /// Smallest possible key for `entity`: every EAVT key of that entity sorts at or after it.
    pub fn entity_start(entity: EntityId) -> Self {
        EavtKey {
            entity,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
            value_bytes: Vec::new(),
            asserted: false,
        }
    }

    /// Smallest possible key for the `(entity, attribute)` pair.
    pub fn entity_attribute_start(entity: EntityId, attribute: &str) -> Self {
        EavtKey {
            attribute: attribute.to_string(),
            ..EavtKey::entity_start(entity)
        }
    }
}

impl AevtKey {
    pub fn from_fact(f: &Fact) -> Self {
        AevtKey {
            attribute: f.attribute.clone(),
            entity: f.entity,
            valid_from: f.valid_from,
            valid_to: f.valid_to,
            tx_count: f.tx_count,
            value_bytes: encode_value(&f.value),
            asserted: f.asserted,
        }
    }

    /// Smallest possible key for `attribute`.
    pub fn attribute_start(attribute: &str) -> Self {
        AevtKey {
            attribute: attribute.to_string(),
            entity: EntityId::nil(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            tx_count: 0,
            value_bytes: Vec::new(),
            asserted: false,
        }
    }
}

// ─── Indexes ─────────────────────────────────────────────────────────────────

/// The pending (uncheckpointed) facts' indexes: each key maps to the fact's
/// position in `FactStorage`'s pending list.
#[derive(Default, Clone)]
pub struct Indexes {
    pub(crate) eavt: std::collections::BTreeMap<EavtKey, usize>,
    pub(crate) aevt: std::collections::BTreeMap<AevtKey, usize>,
}

impl Indexes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Index the pending fact at position `pos`.
    pub fn insert(&mut self, fact: &Fact, pos: usize) {
        self.eavt.insert(EavtKey::from_fact(fact), pos);
        self.aevt.insert(AevtKey::from_fact(fact), pos);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Fact, VALID_TIME_FOREVER, Value};
    use uuid::Uuid;

    fn fact_at(entity: Uuid, attr: &str, value: Value, asserted: bool) -> Fact {
        let mut f = Fact::with_valid_time(
            entity,
            attr.to_string(),
            value,
            100,
            7,
            100,
            VALID_TIME_FOREVER,
        );
        f.asserted = asserted;
        f
    }

    /// #371: two values of one attribute in one transaction must both be indexed,
    /// and an assertion and a retraction of one value must not collide.
    #[test]
    fn same_tx_values_and_assert_retract_keep_distinct_entries() {
        let e = Uuid::from_u128(1);
        let mut idx = Indexes::new();
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(Uuid::from_u128(10)), true),
            0,
        );
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(Uuid::from_u128(11)), true),
            1,
        );
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(Uuid::from_u128(11)), false),
            2,
        );
        assert_eq!(idx.eavt.len(), 3, "EAVT keeps every value and op");
        assert_eq!(idx.aevt.len(), 3, "AEVT keeps every value and op");
    }

    #[test]
    fn entity_and_attribute_start_keys_sort_before_every_matching_key() {
        let e = Uuid::from_u128(5);
        let f = fact_at(e, ":a", Value::Null, false);
        assert!(EavtKey::entity_start(e) <= EavtKey::from_fact(&f));
        assert!(EavtKey::entity_attribute_start(e, ":a") <= EavtKey::from_fact(&f));
        assert!(AevtKey::attribute_start(":a") <= AevtKey::from_fact(&f));
        assert!(EavtKey::entity_start(Uuid::from_u128(6)) > EavtKey::from_fact(&f));
    }

    #[test]
    fn test_encode_value_sort_order() {
        let ints = [i64::MIN, -1, 0, 1, i64::MAX].map(|n| encode_value(&Value::Integer(n)));
        assert!(ints.windows(2).all(|w| w[0] < w[1]), "integers in order");
        let floats = [f64::NEG_INFINITY, -1.0, 0.0, 1.0, f64::INFINITY]
            .map(|f| encode_value(&Value::Float(f)));
        assert!(floats.windows(2).all(|w| w[0] < w[1]), "floats in order");
        let null = encode_value(&Value::Null);
        let b = encode_value(&Value::Boolean(false));
        assert!(null < b && b < ints[2], "cross-type order");
        assert_eq!(
            encode_value(&Value::Float(f64::NAN)),
            encode_value(&Value::Float(-f64::NAN)),
            "NaN is canonical"
        );
    }
}
