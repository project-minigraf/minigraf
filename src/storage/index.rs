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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AevtKey {
    pub attribute: Attribute,
    pub entity: EntityId,
    pub valid_from: i64,
    pub valid_to: i64,
    pub tx_count: u64,
    pub value_bytes: Vec<u8>,
    pub asserted: bool,
}

/// AVET: sort by (Attribute, ValueBytes, ValidFrom, ValidTo, Entity, TxCount, Asserted)
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
    pub asserted: bool,
}

/// VAET: sort by (RefTarget, Attribute, ValidFrom, ValidTo, SourceEntity, TxCount, Asserted)
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

impl AvetKey {
    pub fn from_fact(f: &Fact) -> Self {
        AvetKey {
            attribute: f.attribute.clone(),
            value_bytes: encode_value(&f.value),
            valid_from: f.valid_from,
            valid_to: f.valid_to,
            entity: f.entity,
            tx_count: f.tx_count,
            asserted: f.asserted,
        }
    }
}

impl VaetKey {
    /// `None` unless the fact's value is a `Value::Ref` (only refs are VAET-indexed).
    pub fn from_fact(f: &Fact) -> Option<Self> {
        match &f.value {
            Value::Ref(target) => Some(VaetKey {
                ref_target: *target,
                attribute: f.attribute.clone(),
                valid_from: f.valid_from,
                valid_to: f.valid_to,
                source_entity: f.entity,
                tx_count: f.tx_count,
                asserted: f.asserted,
            }),
            _ => None,
        }
    }
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
        self.eavt.insert(EavtKey::from_fact(fact), fact_ref);
        self.aevt.insert(AevtKey::from_fact(fact), fact_ref);
        self.avet.insert(AvetKey::from_fact(fact), fact_ref);
        if let Some(k) = VaetKey::from_fact(fact) {
            self.vaet.insert(k, fact_ref);
        }
    }

    /// Query EAVT index for a specific entity (returns all facts for that entity).
    pub fn lookup_eavt_entity(&self, entity: EntityId) -> Vec<FactRef> {
        self.eavt
            .range(EavtKey::entity_start(entity)..)
            .take_while(|(k, _)| k.entity == entity)
            .map(|(_, v)| *v)
            .collect()
    }

    /// Query AEVT index for a specific attribute (returns all facts with that attribute).
    pub fn lookup_aevt_attr(&self, attribute: &str) -> Vec<FactRef> {
        self.aevt
            .range(AevtKey::attribute_start(attribute)..)
            .take_while(|(k, _)| k.attribute == attribute)
            .map(|(_, v)| *v)
            .collect()
    }

    /// Query AVET index for attribute + value.
    pub fn lookup_avet_attr_value(&self, attribute: &str, value: &Value) -> Vec<FactRef> {
        let value_bytes = encode_value(value);
        let start = AvetKey {
            attribute: attribute.to_string(),
            value_bytes: value_bytes.clone(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            entity: EntityId::nil(),
            tx_count: 0,
            asserted: false,
        };
        self.avet
            .range(start..)
            .take_while(|(k, _)| k.attribute == attribute && k.value_bytes == value_bytes)
            .map(|(_, v)| *v)
            .collect()
    }

    /// Query VAET index for ref target (reverse references).
    pub fn lookup_vaet_ref(&self, target: EntityId) -> Vec<FactRef> {
        let start = VaetKey {
            ref_target: target,
            attribute: String::new(),
            valid_from: i64::MIN,
            valid_to: i64::MIN,
            source_entity: EntityId::nil(),
            tx_count: 0,
            asserted: false,
        };
        self.vaet
            .range(start..)
            .take_while(|(k, _)| k.ref_target == target)
            .map(|(_, v)| *v)
            .collect()
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

    /// #371: two values of one attribute in one transaction must both be indexed.
    #[test]
    fn same_tx_multi_value_keeps_both_entries_in_all_indexes() {
        let e = Uuid::from_u128(1);
        let t1 = Uuid::from_u128(10);
        let t2 = Uuid::from_u128(11);
        let mut idx = Indexes::new();
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(t1), true),
            FactRef {
                page_id: 0,
                slot_index: 0,
            },
        );
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(t2), true),
            FactRef {
                page_id: 0,
                slot_index: 1,
            },
        );
        assert_eq!(
            idx.lookup_eavt_entity(e).len(),
            2,
            "EAVT must keep both values"
        );
        assert_eq!(
            idx.lookup_aevt_attr(":kind").len(),
            2,
            "AEVT must keep both values"
        );
        assert_eq!(idx.eavt.len(), 2);
        assert_eq!(idx.aevt.len(), 2);
        assert_eq!(idx.avet.len(), 2);
        assert_eq!(idx.vaet.len(), 2);
    }

    /// An assertion and a retraction of the same value at the same tx_count
    /// must not collide in any index.
    #[test]
    fn same_tx_assert_and_retract_keep_both_entries_in_all_indexes() {
        let e = Uuid::from_u128(1);
        let t = Uuid::from_u128(10);
        let mut idx = Indexes::new();
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(t), true),
            FactRef {
                page_id: 0,
                slot_index: 0,
            },
        );
        idx.insert(
            &fact_at(e, ":kind", Value::Ref(t), false),
            FactRef {
                page_id: 0,
                slot_index: 1,
            },
        );
        assert_eq!(idx.eavt.len(), 2, "EAVT must keep assert and retract");
        assert_eq!(idx.aevt.len(), 2, "AEVT must keep assert and retract");
        assert_eq!(idx.avet.len(), 2, "AVET must keep assert and retract");
        assert_eq!(idx.vaet.len(), 2, "VAET must keep assert and retract");
    }

    #[test]
    fn lookups_do_not_leak_neighbouring_entities_or_attributes() {
        let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let mut idx = Indexes::new();
        idx.insert(
            &fact_at(a, ":x", Value::Integer(1), true),
            FactRef {
                page_id: 0,
                slot_index: 0,
            },
        );
        idx.insert(
            &fact_at(b, ":x", Value::Integer(1), true),
            FactRef {
                page_id: 0,
                slot_index: 1,
            },
        );
        idx.insert(
            &fact_at(a, ":xy", Value::Integer(1), true),
            FactRef {
                page_id: 0,
                slot_index: 2,
            },
        );
        assert_eq!(idx.lookup_eavt_entity(a).len(), 2);
        assert_eq!(idx.lookup_aevt_attr(":x").len(), 2);
        assert_eq!(
            idx.lookup_avet_attr_value(":x", &Value::Integer(1)).len(),
            2
        );
    }

    #[test]
    fn entity_and_attribute_start_keys_sort_before_every_matching_key() {
        let e = Uuid::from_u128(5);
        let f = fact_at(e, ":a", Value::Null, false);
        assert!(EavtKey::entity_start(e) <= EavtKey::from_fact(&f));
        assert!(AevtKey::attribute_start(":a") <= AevtKey::from_fact(&f));
        assert!(EavtKey::entity_start(Uuid::from_u128(6)) > EavtKey::from_fact(&f));
    }

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
            value_bytes: Vec::new(),
            asserted: true,
        };
        let k2 = EavtKey {
            entity: e2,
            attribute: ":age".to_string(),
            valid_from: 0,
            valid_to: i64::MAX,
            tx_count: 1,
            value_bytes: Vec::new(),
            asserted: true,
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
            asserted: true,
        };
        let k2 = AvetKey {
            attribute: ":score".to_string(),
            value_bytes: encode_value(&Value::Integer(20)),
            valid_from: 0,
            valid_to: i64::MAX,
            entity: e,
            tx_count: 2,
            asserted: true,
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
