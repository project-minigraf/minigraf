# Set values and set-membership predicates (#316) — Design

**Issue:** #316
**Milestone:** v3.0.0, branch `v3` (format v8 gains a value tag before it is frozen; one new `Value`
variant; five predicates, one binding function, new error codes)
**Builds on:** the v8 storage format (`2026-10-05-v8-storage-format-design.md` §6.2, §6.3), the streaming
engine (#473), `value_order` (#547)
**Overlaps:** #306 (`2026-10-24-limit-order-by-design.md`) only in `value_order` and small `parser.rs` and
grammar hunks (§10)

## 1. Decisions

Settled with the maintainer on 2026-10-24:

1. **Key encoding: inline, spilled when long.** A new type tag `0x08` sorts after Ref. A set whose element
   encoding is at most 64 bytes is stored in the key. A longer one goes to a value page with the
   long-string scheme (§4).
2. **Element equality is `Value`'s total order** (`Ord`/`Hash`): `-0.0` equals `0.0`, every NaN is one
   element, a keyword never equals a ref, `1` never equals `1.0`. Dedup and membership both use it. Of
   equal floats, the least by `value_order` is kept (`-0.0`), as a float group is in #547.
3. **API: `Value::Set(ValueSet)`, and `Value` becomes `#[non_exhaustive]`.** `ValueSet` is a newtype that
   cannot hold a set (§2). Prepared queries use `BindValue::Val(Value::Set(..))`; `BindValue` gets no
   variant.
4. **`collect-set` is defined by a pattern.** `[(collect-set ?e :a) ?s]` binds, per row, the set of `?v`
   over `[?e :a ?v]` under the query's temporal view, and `#{}` when there are none (§7).

Deviation from the brainstorm: an unbound entity in `collect-set` is the existing `INT-018` raised for
unbound expression variables, not a new parse code (§7.1).

## 2. Terms

- **Scalar**: a `Value` that is not a set (String, Integer, Float, Boolean, Ref, Keyword, Null).
- **Set**: a `ValueSet`, a finite set of scalars.
- **Set equality**: two elements are equal when `Value::cmp` returns `Equal`.
- **Canonical order** of a set: its elements in `Value::cmp` order. Iteration, display, JSON output and
  the pending encoding (§4.3) use it.

## 3. Value model

```rust
/// A finite set of scalar values. It never holds a set.
#[derive(Clone, Debug, Default)]
pub struct ValueSet(BTreeSet<Value>);

impl ValueSet {
    pub fn new(values: impl IntoIterator<Item = Value>) -> Result<Self>; // API-021 for a set element
    pub fn iter(&self) -> impl Iterator<Item = &Value>; // canonical order
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn contains(&self, v: &Value) -> bool;
}

#[non_exhaustive]
pub enum Value { String(..), Integer(..), Float(..), Boolean(..), Ref(..), Keyword(..), Null, Set(ValueSet) }
```

- **Inserting an equal float.** `BTreeSet::insert` keeps the element already there. `new` replaces it
  instead when the new one is less by `value_order`, so `#{0.0 -0.0}` and `#{-0.0 0.0}` both hold `-0.0`.
  One NaN is kept, whichever bits came first. NaN payloads are not observable through the query
  language.
- **`Ord`/`Hash`/`PartialEq`/`Eq` for `Value`.** Set has discriminant 7, after Null. Two sets compare by
  their canonical element sequences lexicographically (a proper prefix is less). `Hash` hashes the length,
  then each element. `PartialEq` for `ValueSet` is `cmp == Equal`, so a set holding a NaN equals itself.
  The derived `PartialEq` of `Value` uses it for the Set arm.
- **`value_order`** (`aggregate.rs`): Set is a new class after `nil`. Two sets compare by their elements
  sorted by `value_order`, lexicographically, a proper prefix less. `min`/`max`, window sorts and #306's
  `:order-by` then order set values. `sum`, `avg` and the other numeric aggregates treat a set as they
  treat a keyword.
- **Serde** (the WAL's postcard): `Value::Set` is appended as the last variant, so no existing encoding
  changes. `ValueSet` serializes as a sequence in canonical order. It deserializes through `new`, so the
  order is restored and a nested set is an error. An error while decoding a WAL entry is handled as it is
  today.
- **Size of a stored set.** `set_bytes(s)` is the v8 element encoding's length (§4.1), counting each
  keyword as 6 bytes and each ref as 10, their largest v8 encodings, since ids are not yet assigned.
  The WAL rejects a transacted or retracted set with `set_bytes > MAX_VALUE_BYTES` with `WAL-003`
  (today's check in `wal.rs` for a long string), so a set the WAL takes always fits a value page at
  checkpoint. Sets in query text or bind values are not stored and have no limit.

## 4. Storage

### 4.1 v8 keys (`keys.rs`)

`TAG_SET = 0x08`, after `TAG_REF`, so the cross-type order becomes Null < Boolean < Integer < Float <
String < Keyword < Ref < Set. That matches `Value::cmp` for Set (last) but not for the other types, as
today.

The **element encoding** of a set is the concatenation of each element's existing key encoding (`put_value`
of its `KeyValue`; a keyword is its iid, a ref its eid), sorted by those bytes. A string element is always
written in full with the short-string form (escaped, `00 00`), whatever its length, so an element never
holds a value ref. Each element encoding is prefix-free, so sorting and concatenating gives one canonical
byte string per set for a given DICT.

| set | key bytes after `08` |
|---|---|
| element encoding ≤ 64 B | the element encoding escaped (`00` → `00 FF`), then `00 00` |
| element encoding > 64 B | the first 32 bytes escaped, then `00 01`, `hash64` of the whole element encoding, value ref (`page u64`, `slot u16`) |

This is §6.2's long-string layout with the element encoding in place of the string bytes. Its
properties carry over: equality is exact (seek to `08 ‖ prefix ‖ marker ‖ hash`, compare full element
encodings), and the order among sets is by bytes, which no query relies on (sets have no range
predicates). `KeyValue` gains `Set(Vec<KeyValue>)` and `LongSet { prefix, hash, vref }`.

### 4.2 Value pages and DICT (`value_pages.rs`, `dict.rs`)

A spilled set's value-page record is its element encoding. Dedup is §6.3's: DICT tag `0x06` candidates
with the same `hash64`, full bytes compared. A set and a string with the same bytes may share a record;
the key's tag says how to read it. `verify` (#373) finds set records through index keys, as for long
strings, and checks that each decodes as a set's element encoding.

### 4.3 Pending encoding (`index.rs`)

`encode_value` gains `Value::Set` → `0x07`, then for each element in canonical order a 4-byte big-endian
length and the element's `encode_value`. Lengths make it injective; the engine's relations (#548) key
derived rows on it, and the pending EAVT/AEVT keys hold it.

### 4.4 Kani (`index/verification.rs`, `keys/verification.rs`)

The encoder harnesses (#627) get the set tag: `encode_value` of a set of at most two small elements is
injective; the v8 element encoding of two-element sets is canonical (insertion order does not change the
bytes). Per the CLAUDE.md warning, no new decoder or string harness without a bench-machine run first.

## 5. Syntax (`parser.rs`, `tests/grammar/grammar.pest`)

- The tokenizer reads `#` followed immediately by `{` as `Token::SetOpen`; `#uuid` and other tags are
  unchanged. Today `#{` is `PRS-073` (unknown tagged literal `#`).
- `#{ v* }` reads to `EdnValue::Set(Vec<EdnValue>)`. Its elements are the scalar literals `edn_to_value`
  takes (strings, numbers, booleans, `nil`, keywords, `#uuid`). A nested `#{}` is `PRS-119`; a variable,
  vector, list, map or bind slot inside is `PRS-120`. Duplicates are allowed and removed.
- A set literal is accepted wherever a value literal is: the value of a transacted or retracted fact, a
  pattern's value constant, an expression operand, a rule-invocation argument. In an entity or attribute
  position it is the existing error for a non-entity or non-keyword there.
- `parse_expr_arg` maps `EdnValue::Set` to `Expr::Lit(Value::Set(..))`. A `$slot` operand stays
  `Expr::Slot`, and `BindValue::Val(Value::Set(..))` substitutes like any value.
- The grammar gains `set_lit = { "#{" ~ edn_value* ~ "}" }` in `edn_value`. The verifier (#643) gains `#{`
  in `TOKENS`; `PRS-119` and `PRS-120` get `SEMANTIC` entries with witnesses, since the grammar accepts
  `#{#{}}` and `#{?x}`. `KNOWN_DRIFT` stays empty.

Codes are the next free ones when the PR opens; #306 holds PRS-115–118.

## 6. Predicates

Five `BinOp`s, each taking exactly two operands (`PRS-065` otherwise):

| op | operands | true when |
|---|---|---|
| `in?` | scalar `x`, set `S` | `x ∈ S` |
| `not-in?` | scalar `x`, set `S` | `x ∉ S` |
| `any-in?` | set `A`, set `B` | `A ∩ B ≠ ∅` |
| `all-in?` | set `A`, set `B` | `A ⊆ B` |
| `none-in?` | set `A`, set `B` | `A ∩ B = ∅` |

- Membership is set equality (§2). `in?` with `S = #{}` is false, `not-in?` with `S = #{}` is true,
  `all-in? #{} B` and `none-in? #{} B` are true, `any-in? #{} B` is false.
- A wrong operand shape (a set as `in?`'s first operand, a scalar where a set is needed) is a type
  mismatch: `eval_binop` returns `Err(())`, so a filter drops the row and a binding form binds nothing,
  as for `(< "a" 1)`. `not-in?` with a non-set second operand also drops the row; it is not true.
- They work as filters `[(in? ?x #{..})]` and as Boolean bindings `[(in? ?x #{..}) ?b]`.
- `contains?`, `starts-with?`, `ends-with?` are unchanged; `contains?` stays string-only.
- `=` and `!=` between sets use `Value`'s `PartialEq` (§3). A set never equals a scalar.
- Pushdown: none beyond today's. A filter is not turned into an index probe.

## 7. `collect-set`

```datalog
[(collect-set ?e :user/scope) ?scopes]
```

### 7.1 Parse

- Exactly two operands and a binding (`PRS-065`, `PRS-068` as for other forms). Without a binding it is
  `PRS-121` (`collect-set` produces a value; bind it to a variable).
- The first operand is a variable; the second is a keyword literal (`PRS-122` otherwise, naming what was
  given). A `$slot` is not accepted in either.
- The variable must be bound by an earlier clause: `check_expr_safety` checks it with the expression
  variables, so an unbound one is `INT-018`, as for `[(> ?x 1)]`.
- It may appear wherever an expression clause may: `:where`, rule bodies, `not`/`not-join` bodies, `or`
  branches. It is not part of a larger expression: `(in? 1 (collect-set ..))` is `PRS-067` (unknown operator), as `collect-set` is read only at clause level.
- Stratification: it reads stored facts only, never a rule's relation, so it adds no edge. Magic sets
  treat it as an expression clause.

### 7.2 Meaning

For each input row with entity cell `e`, `?s` is bound to the set of values `v` such that `[e :a v]`
matches under the query's snapshot, `:as-of` and `:valid-at`, with net-assert and retractions applied
exactly as for that pattern. No matches give `#{}`; the row is kept. `e` resolves as the pattern's entity
position resolves its bound variable (an entity cell, or a keyword naming an entity). A cell that cannot
be an entity (an integer, a string) gives `#{}`, as the pattern would match nothing.

If any matching value is a set, the query fails with `QRY-014` (`collect-set ?s over {attribute} found a
set value; bind a set-valued attribute with a pattern instead`). It does not skip or flatten it.

### 7.3 Engine

A new `Step::CollectSet { entity: usize, attr, out: usize }` in `filter.rs`. Per row it runs a one-entity
EAVT probe for `(e, a)` (the probe `IndexNestedLoopJoin` uses: committed `live_keys`, then pending runs),
collects the values into a `ValueSet`, and writes `Cell::Val(Scalar::Set(Arc<ValueSet>))`. If `out` is
already bound it keeps the rows where the two are equal, as `Bind` does (#596). The set it builds is
charged to the query's `Meter` while it is built, `QRY-012` over `query_memory_limit` (#549).

## 8. Engine cells

- `Scalar::Set(Arc<ValueSet>)` holds every set, decoded at scan. A spilled set is read from its value page
  when its key is scanned. Keyword and ref elements are translated through DICT at decode, so a cell
  never holds a `ValueRef` for a set and a set cell never join-equals a long-string cell.
- `JoinKey::Set(Arc<ValueSet>)`: joins and `or`/`not` keys on set columns compare by set equality. The
  keyword-equals-ref rule for scalar join keys does not apply inside a set (decision 2).
- A set constant in a pattern's value position is `ValueConst::Exact(Value::Set(..))`, scanned as an
  AVET equality on the encoded constant (§4.1) when the planner reads AVET, else matched per fact.
- `project.rs` turns the cell back into `Value::Set`. Aggregate groups and `Distinct` key on `Value`'s
  `Ord`, which already covers sets.

## 9. Output

- REPL and `Display`: `#{e1 e2 ..}` in canonical order, elements as today; `#{}` when empty.
- JSON (`browser/mod.rs` `value_to_json`, `BrowserCursor`): an array of the elements' JSON in canonical
  order.
- Bindings: every binding (Python, C, Node, Java, Swift, Android) has its own `value_to_json`. Each gets
  the same array arm and a wildcard arm (now that `Value` is `#[non_exhaustive]`), on its `v3` branch after
  core merges, with a consumer test that queries a set. No binding exposes prepared queries, so none gets a
  bind type.

## 10. Overlap with #306

`value_order` gets the Set class here; #306's `Sort` uses `value_order`, so `:order-by` of a set column
works whichever lands first, with a test added by the second. `parser.rs` and `grammar.pest` hunks are
independent (new token and value rule here, new query sections there); the second PR rebases. Error codes
are taken in PR order.

## 11. Errors

| code | when |
|---|---|
| `API-021` (new) | `ValueSet::new` given a set element |
| `PRS-119` (new) | a set literal inside a set literal |
| `PRS-120` (new) | a set element that is not a scalar literal |
| `PRS-121` (new) | `collect-set` without a binding |
| `PRS-122` (new) | `collect-set` operands not a variable and a keyword literal |
| `QRY-014` (new) | `collect-set` matched a set value |
| `WAL-003` (message widened) | a stored set over `MAX_VALUE_BYTES` (§3) |

`docs/ERROR_REFERENCE.md` lists each, and `error.rs`'s registry test keeps the two in step.

## 12. Tests

- **Value model**: `ValueSet::new` dedup (including `-0.0`/`0.0` either order keeping `-0.0`, several
  NaNs), nested set rejected, `Ord`/`Hash`/`Eq` consistency (property test: `a == b ⇒ hash a == hash b`,
  `cmp` total and antisymmetric over random sets of scalars), `value_order` total over sets, serde round
  trip, non-canonical and nested postcard input.
- **Storage**: key encode/decode round trip of inline and spilled sets, the 64-byte boundary, dedup of an
  equal spilled set (no new value page), a set and a string with equal bytes, `set_bytes` at
  `MAX_VALUE_BYTES` and one over (`WAL-003`), keyword/ref elements across checkpoint and reopen, `verify`
  on a file with spilled sets, golden-file corpus entry with sets, crash-recovery replay of a WAL entry
  holding sets.
- **Parser**: literals in each accepted position, `#uuid` and `#{` adjacent, empty set, duplicates,
  each new parse error; grammar verifier with the new token and `SEMANTIC` entries.
- **Predicates**: the table in §6 for each scalar type including keywords, refs, NaN and `-0.0`; every
  empty-set case; each wrong operand shape in filter and binding form; `not-in?` with a non-set; literal,
  stored-attribute, `collect-set` and bind-slot operands; `contains?` unchanged.
- **`collect-set`**: missing attribute gives `#{}` and keeps the row; dedup; retractions; `:as-of` and
  `:valid-at`; pending and committed facts mixed; a set-valued attribute is `QRY-014`; unbound entity
  `INT-018`; in a rule body, a `not` body and an `or` branch; out variable already bound; `QRY-012` under
  a small `query_memory_limit`.
- **Prepared**: `BindValue::Val(Value::Set)` in an operand slot and a value slot, reused across executes.
- **Engine**: joins on a set column, set constant in a pattern (AVET and per-fact paths), `count-distinct`
  and grouping by a set, `min`/`max` over sets; model-based test (#385) generator gains set values.
- **Bindings**: consumer test per binding returning a set (§9).

## 13. Documentation

`CHANGELOG.md` (Added: set values, predicates, `collect-set`; Changed: `Value` is `#[non_exhaustive]`;
format v8 value tag `0x08`), `docs/ERROR_REFERENCE.md`, `README.md`, `ROADMAP.md`, `CLAUDE.md` (data
model, test count), `docs/TEST_COVERAGE.md`, the v8 storage spec's §6.2 table is not edited (frozen
record) but `content/architecture.md` on the docs site gets the Set row. Docs site
`content/datalog-reference.md` (set literals, the five predicates, `collect-set`, `@since v3.0.0`) and
`content/comparison.md`.

## 14. Delivery (PRs into `v3`)

1. **Set values**: `ValueSet`, `Value::Set`, `#[non_exhaustive]`, `Ord`/`Hash`/`value_order`, serde, the
   parser literal and grammar, v8 keys, value pages, pending encoding, Kani, WAL size check, engine cells
   and joins, output; transact, retract, query by exact value; `API-021`, `PRS-119`, `PRS-120`.
2. **Predicates**: the five `BinOp`s and prepared-query sets.
3. **`collect-set`**: parse, `Step::CollectSet`, `PRS-121`, `PRS-122`, `QRY-014`.
4. **Docs site PR**, then one PR per binding (§9).

Each PR updates the docs of §13 that its change touches.
