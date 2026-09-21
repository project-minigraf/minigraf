# Index key value collision (#371, #287) — Design

**Date**: 2026-09-21
**Issues**: #371 (same-transaction multi-valued facts read back as one value), #287 (`EavtKey`/`AevtKey` carry no value bytes)
**Target**: file format v8, v3.0.0 milestone

## Problem

When one `transact` writes two values of the same attribute for one entity, both facts share
`(entity, attribute, valid_from, valid_to, tx_count)`. Two layers then discard one of them:

1. **Index keys.** `EavtKey` and `AevtKey` have no value bytes. Colliding entries overwrite each
   other in the pending `BTreeMap`s. At checkpoint, `build_sorted_index_entries` sorts them with
   `sort_unstable_by`, so which colliding entry survives can differ between EAVT and AEVT.
   Reads through the two paths can return different values for the same entity.
2. **Query-time dedup.** `selective_fact_fetch` (`src/query/datalog/executor.rs`) dedups fetched
   facts on `(entity, attribute, tx_count, asserted)`. That key has no value either, so even with
   correct indexes it keeps only the first of the colliding facts. For a batched retract, the
   second retraction never reaches `net_asserted_facts`, so its assertion is never cancelled.

A full scan (no bound entity or attribute) reads the fact pages directly and returns both values,
which is why the paths disagree.

## Scope

In scope:

- Add value bytes and `asserted` to `EavtKey` and `AevtKey`.
- Add `asserted` to `AvetKey` and `VaetKey`. Both already distinguish different values (AVET has
  `value_bytes`, VAET has the ref target), but an assertion and a retraction of the same value at
  the same `tx_count` share a key. This is inferred from the key definitions; a test written first
  must demonstrate it before the change is applied.
- File format v7 → v8, with migration.
- Add value bytes to the `seen` key in `selective_fact_fetch`.

Out of scope:

- Dropping v1–v6 migration code (a separate PR at v3.0.0 cut time). v1–v6 migrations stay and
  chain into v8.
- Version bump, tag, and release.
- The `seen` set in `executor.rs` `Or`-branch evaluation. It keys on bound values and is not
  affected.

## Design

### Key layout

The new fields are appended to the end of each key, so the existing sort prefix and range-scan
order are unchanged.

| Key  | Order (v8)                                                                  |
|------|-----------------------------------------------------------------------------|
| EAVT | entity, attribute, valid_from, valid_to, tx_count, **value_bytes, asserted** |
| AEVT | attribute, entity, valid_from, valid_to, tx_count, **value_bytes, asserted** |
| AVET | attribute, value_bytes, valid_from, valid_to, entity, tx_count, **asserted** |
| VAET | ref_target, attribute, valid_from, valid_to, source_entity, tx_count, **asserted** |

`value_bytes` is `encode_value(&fact.value)`, as AVET already uses.

Range bounds (`get_facts_by_entity`, `get_facts_by_attribute`, `lookup_eavt_*`, `lookup_aevt_*`,
and B+tree range tests) use `value_bytes: vec![]` and `asserted: false` as the minimum. Existing
upper bounds are exclusive next-entity or next-attribute keys, so they need only the same minimum
values. Inclusive upper bounds built from `tx_count: u64::MAX` (`lookup_eavt_entity_attr`) cannot
name a maximal `value_bytes`, so they would drop entries whose `tx_count` is `u64::MAX`. Those
helpers are rewritten as exclusive bounds on the next attribute, and a unit test covers the edge.

Alternative rejected: placing `value_bytes` directly after the attribute, as AVET does. It changes
the sort order and hurts temporal range scans on entity and attribute.

### Query-time dedup

`selective_fact_fetch` `seen` key becomes
`(entity, attribute, tx_count, asserted, encode_value(&value))`. Both the entity-driven and
attribute-driven loops use it.

### Format v8 and migration

- `FORMAT_VERSION` becomes 8. The 84-byte header layout is unchanged; only `version` differs.
- On open, a v7 header forces the existing `needs_rebuild` path in `PersistentFactStorage::load`.
  That path re-reads packed fact pages with real `FactRef`s (`read_all_with_refs`), rebuilds all
  four B+trees with the new keys, and writes a v8 header. Fact pages are not modified.
- Idempotent under crash: the rebuild overwrites index pages before writing the header. A crash in
  between leaves a v7 header whose index checksum no longer matches, so the next open rebuilds
  again.
- `FileHeader::validate` still rejects versions above `FORMAT_VERSION` (`STG-006`).
- Header checksum validation applies to `version >= 7` as today.

## Testing

Regression tests are written first and shown failing before any fix:

1. Issue reproduction: two values of one attribute in one transact; read through EAVT, AEVT and
   full scan; after checkpoint and reopen; over several graph shapes (filler entities with two
   attributes each). All three paths return both values.
2. Batched retract of both values, then checkpoint and reopen: no path returns either value.
3. Same value asserted and retracted in one `WriteTransaction`: both facts survive in all four
   indexes (demonstrates the `asserted` collision, then the fix).
4. v7 → v8 migration: build a v7 fixture containing colliding values (from the current code, before
   the change), open with the new code, assert v8 header and correct reads.
5. Unit tests: key ordering, range-bound minima, `encode_value` in the key, `selective_fact_fetch`
   dedup with two values sharing everything but the value.
6. Update existing assertions on `FORMAT_VERSION == 7` and header version.

Tests follow the project convention: no `{:?}` of `Result`/`Fact`/`Value` in assert messages.

## Documentation and process

- Update `CHANGELOG.md`, `ROADMAP.md` (move #287 work out of planned scope), `README.md` if it
  cites the format version, `docs/TEST_COVERAGE.md`, `CLAUDE.md` (test count, format version), and
  wiki `Architecture.md` (key layout, format v8). Wiki is committed and pushed separately.
- PR closes #371 and #287 only. No version bump or tag in this PR.
- Work in worktree `.worktrees/fix-371-index-value-collision`, branch
  `fix/371-index-value-collision`. Own the PR until CI is green; do not merge without explicit
  confirmation.

## Risks

- Larger keys (value bytes) increase index size and lower B+tree fanout for EAVT/AEVT. Measure
  file size and benchmark impact on the existing suite before claiming no regression.
- Long string values inflate keys. The existing `MAX_FACT_BYTES` bound applies to facts; check
  whether a key can exceed the B+tree node page size and, if so, decide on truncation with a
  fact-page tiebreak or a size error.
- v8 files are unreadable by v2.x. This is the intended breaking change for v3.0.0.
