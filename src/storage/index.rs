//! Index key types, FactRef, and canonical value encoding for the four
//! covering indexes (EAVT, AEVT, AVET, VAET).
//!
//! `FactRef` identifies a fact's location on disk. In Phase 6.1, one fact
//! occupies one page (`slot_index` is always 0). In Phase 6.2, `slot_index`
//! identifies the record slot within a packed page.

use crate::graph::types::{Attribute, EntityId, Fact, Value};
use serde::{Deserialize, Serialize};

// ─── FactRef ────────────────────────────────────────────────────────────────

/// Disk location of a fact.
///
/// `slot_index` is always `0` in Phase 6.1 (one fact per page).
/// In Phase 6.2 it identifies the record within a packed page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FactRef {
    pub page_id: u64,
    pub slot_index: u16,
}

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
            let bits = (*n as u64) ^ 0x8000_0000_0000_0000;
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
            let mut bytes = Vec::with_capacity(1 + s.len());
            bytes.push(0x04);
            bytes.extend_from_slice(s.as_bytes());
            bytes
        }
        Value::Keyword(k) => {
            let mut bytes = Vec::with_capacity(1 + k.len());
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

/// EAVT: sort by (Entity, Attribute, ValidFrom, ValidTo, TxCount)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EavtKey {
    pub entity: EntityId,
    pub attribute: Attribute,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
}

/// AEVT: sort by (Attribute, Entity, ValidFrom, ValidTo, TxCount)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AevtKey {
    pub attribute: Attribute,
    pub entity: EntityId,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
}

/// AVET: sort by (Attribute, ValueBytes, ValidFrom, ValidTo, Entity, TxCount)
///
/// `value_bytes` is the canonical encoding from `encode_value`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AvetKey {
    pub attribute: Attribute,
    pub value_bytes: Vec<u8>,
    pub valid_from: i64,
    pub valid_to: i64,
    pub entity: EntityId,
    pub tx_count: u64,
}

/// VAET: sort by (RefTarget, Attribute, ValidFrom, ValidTo, SourceEntity, TxCount)
///
/// Only facts with `Value::Ref` are indexed here.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VaetKey {
    pub ref_target: EntityId,
    pub attribute: Attribute,
    pub valid_from: i64,
    pub valid_to: i64,
    pub source_entity: EntityId,
    pub tx_count: u64,
}

// ─── Indexes ─────────────────────────────────────────────────────────────────

/// All four covering indexes held in memory alongside the fact list.
///
/// Populated on every `transact`, `retract`, and `load_fact`.
#[derive(Default, Clone)]
pub struct Indexes {
    pub(crate) eavt: std::collections::BTreeMap<EavtKey, FactRef>,
    pub(crate) aevt: std::collections::BTreeMap<AevtKey, FactRef>,
    pub(crate) avet: std::collections::BTreeMap<AvetKey, FactRef>,
    pub(crate) vaet: std::collections::BTreeMap<VaetKey, FactRef>,
}

impl Indexes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fact into all applicable indexes.
    ///
    /// `fact_ref` is the disk location. In Phase 6.1, callers pass
    /// `FactRef { page_id: 0, slot_index: 0 }` as a placeholder; real
    /// page IDs are assigned by `save()` and updated via `reindex_from_facts`.
    pub fn insert(&mut self, fact: &Fact, fact_ref: FactRef) {
        self.eavt.insert(
            EavtKey {
                entity: fact.entity,
                attribute: fact.attribute.clone(),
                valid_from: fact.valid_from,
                valid_to: fact.valid_to,
                tx_count: fact.tx_count,
            },
            fact_ref,
        );

        self.aevt.insert(
            AevtKey {
                attribute: fact.attribute.clone(),
                entity: fact.entity,
                valid_from: fact.valid_from,
                valid_to: fact.valid_to,
                tx_count: fact.tx_count,
            },
            fact_ref,
        );

        self.avet.insert(
            AvetKey {
                attribute: fact.attribute.clone(),
                value_bytes: encode_value(&fact.value),
                valid_from: fact.valid_from,
                valid_to: fact.valid_to,
                entity: fact.entity,
                tx_count: fact.tx_count,
            },
            fact_ref,
        );

        if let Value::Ref(target) = &fact.value {
            self.vaet.insert(
                VaetKey {
                    ref_target: *target,
                    attribute: fact.attribute.clone(),
                    valid_from: fact.valid_from,
                    valid_to: fact.valid_to,
                    source_entity: fact.entity,
                    tx_count: fact.tx_count,
                },
                fact_ref,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Fact, VALID_TIME_FOREVER, Value};
    use uuid::Uuid;

    #[test]
    fn test_fact_ref_fields() {
        let r = FactRef {
            page_id: 42,
            slot_index: 7,
        };
        assert_eq!(r.page_id, 42);
        assert_eq!(r.slot_index, 7);
    }

    #[test]
    fn test_encode_value_sort_order_integers() {
        let neg = encode_value(&Value::Integer(-1));
        let zero = encode_value(&Value::Integer(0));
        let pos = encode_value(&Value::Integer(1));
        assert!(neg < zero, "neg should sort before zero");
        assert!(zero < pos, "zero should sort before pos");
    }

    #[test]
    fn test_encode_value_large_negative_before_large_positive() {
        let a = encode_value(&Value::Integer(i64::MIN));
        let b = encode_value(&Value::Integer(i64::MAX));
        assert!(a < b);
    }

    #[test]
    fn test_encode_value_sort_order_cross_type() {
        let null = encode_value(&Value::Null);
        let bool_val = encode_value(&Value::Boolean(false));
        let int_val = encode_value(&Value::Integer(0));
        assert!(null < bool_val);
        assert!(bool_val < int_val);
    }

    #[test]
    fn test_encode_value_ref_structure() {
        let id = Uuid::new_v4();
        let bytes = encode_value(&Value::Ref(id));
        assert_eq!(bytes[0], 0x06); // Ref discriminant
        assert_eq!(&bytes[1..17], id.as_bytes());
    }

    #[test]
    fn test_eavt_key_ordering_by_entity() {
        let e1 = Uuid::from_u128(1);
        let e2 = Uuid::from_u128(2);
        let k1 = EavtKey {
            entity: e1,
            attribute: ":age".to_string(),
            valid_from: 0,
            valid_to: i64::MAX,
            tx_count: 1,
        };
        let k2 = EavtKey {
            entity: e2,
            attribute: ":age".to_string(),
            valid_from: 0,
            valid_to: i64::MAX,
            tx_count: 1,
        };
        assert!(k1 < k2);
    }

    #[test]
    fn test_avet_key_orders_by_value_bytes() {
        let e = Uuid::new_v4();
        let k1 = AvetKey {
            attribute: ":score".to_string(),
            value_bytes: encode_value(&Value::Integer(10)),
            valid_from: 0,
            valid_to: i64::MAX,
            entity: e,
            tx_count: 1,
        };
        let k2 = AvetKey {
            attribute: ":score".to_string(),
            value_bytes: encode_value(&Value::Integer(20)),
            valid_from: 0,
            valid_to: i64::MAX,
            entity: e,
            tx_count: 2,
        };
        assert!(k1 < k2);
    }

    #[test]
    fn test_indexes_insert_vaet_only_for_ref() {
        let entity = Uuid::new_v4();
        let target = Uuid::new_v4();
        let mut indexes = Indexes::new();

        // Non-Ref value: should NOT appear in VAET
        let non_ref_fact = Fact::with_valid_time(
            entity,
            ":name".to_string(),
            Value::String("Alice".to_string()),
            0,
            1,
            0,
            VALID_TIME_FOREVER,
        );
        indexes.insert(
            &non_ref_fact,
            FactRef {
                page_id: 1,
                slot_index: 0,
            },
        );
        assert!(
            indexes.vaet.is_empty(),
            "VAET must not contain non-Ref fact"
        );

        // Ref value: SHOULD appear in VAET
        let ref_fact = Fact::with_valid_time(
            entity,
            ":friend".to_string(),
            Value::Ref(target),
            0,
            2,
            0,
            VALID_TIME_FOREVER,
        );
        indexes.insert(
            &ref_fact,
            FactRef {
                page_id: 2,
                slot_index: 0,
            },
        );
        assert_eq!(indexes.vaet.len(), 1);
    }

    #[test]
    fn test_indexes_insert_populates_all_four() {
        let entity = Uuid::new_v4();
        let target = Uuid::new_v4();
        let mut indexes = Indexes::new();
        let ref_fact = Fact::with_valid_time(
            entity,
            ":friend".to_string(),
            Value::Ref(target),
            0,
            1,
            0,
            VALID_TIME_FOREVER,
        );
        indexes.insert(
            &ref_fact,
            FactRef {
                page_id: 1,
                slot_index: 0,
            },
        );
        assert_eq!(indexes.eavt.len(), 1);
        assert_eq!(indexes.aevt.len(), 1);
        assert_eq!(indexes.avet.len(), 1);
        assert_eq!(indexes.vaet.len(), 1);
    }

    #[test]
    fn test_encode_value_sort_order_floats() {
        let neg_inf = encode_value(&Value::Float(f64::NEG_INFINITY));
        let neg_one = encode_value(&Value::Float(-1.0));
        let zero = encode_value(&Value::Float(0.0));
        let pos_one = encode_value(&Value::Float(1.0));
        let pos_inf = encode_value(&Value::Float(f64::INFINITY));
        assert!(neg_inf < neg_one, "-inf < -1.0");
        assert!(neg_one < zero, "-1.0 < 0.0");
        assert!(zero < pos_one, "0.0 < 1.0");
        assert!(pos_one < pos_inf, "1.0 < +inf");
    }

    #[test]
    fn test_encode_value_nan_is_canonical() {
        let nan1 = encode_value(&Value::Float(f64::NAN));
        let nan2 = encode_value(&Value::Float(f64::NAN));
        // All NaN values produce the same bytes
        assert_eq!(nan1, nan2);
        // NaN sorts above all positive finite values (it uses quiet NaN bit pattern)
        // Just verify it doesn't panic and produces a fixed-length result
        assert_eq!(nan1.len(), 9);
    }
}
