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
- File format v7 → v8, with migration. v7 (the format written by v2.0.0) is the only older
  format still supported.
- Dropping support for formats v1–v6, as the v3.0.0 file-format policy in `ROADMAP.md` requires.
  Doing it here avoids adapting v5 migration code to the new key types only to delete it.
- Add value bytes to the `seen` key in `selective_fact_fetch`.

Out of scope:

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
values. `lookup_eavt_entity`, `lookup_eavt_entity_attr` and similar `Indexes` helpers build
upper bounds from `u64::MAX` sentinels (and one from a `"zzz…"` attribute). With `value_bytes`
appended, an inclusive `tx_count: u64::MAX` bound no longer covers every key, so these helpers
switch to the exclusive next-entity / next-attribute bound that `graph/storage.rs` already uses.
Several are `#[allow(dead_code)]`; if unused outside tests they are removed instead.

Alternative rejected: placing `value_bytes` directly after the attribute, as AVET does. It changes
the sort order and hurts temporal range scans on entity and attribute.

### Query-time dedup

`selective_fact_fetch` `seen` key becomes
`(entity, attribute, tx_count, asserted, encode_value(&value))`. Both the entity-driven and
attribute-driven loops use it.

### Format v8 and migration

- `FORMAT_VERSION` becomes 8. The 84-byte header layout is unchanged; only `version` differs.
- Keys are postcard-encoded in B+tree pages. Postcard is not self-describing, so a v7 key
  cannot be decoded as a v8 key. Every code path that decodes on-disk keys (`OnDiskIndexReader`
  range scans, `stream_all_entries` in `save()`) must therefore never see a pre-v8 tree.
- On open, a v7 header forces the existing `needs_rebuild` path in `PersistentFactStorage::load`,
  before the index reader is wired and before any `save()`. That path re-reads packed fact pages
  with real `FactRef`s (`read_all_with_refs`), rebuilds all four B+trees with v8 keys, and writes
  a v8 header (`FileHeader::new()` already uses `FORMAT_VERSION`). Fact pages are not modified.
- Crash safety: the rebuild overwrites index pages first and writes the header last. A crash in
  between leaves a v7 header, so the next open rebuilds again. This rests on the
  version check, not on the index checksum.
- `FileHeader::validate` still rejects versions above `FORMAT_VERSION` (`STG-006`), so v2.x opening
  a v8 file fails cleanly. The upgrade is one-way and happens on first open; the CHANGELOG must say
  so, since bindings users (e.g. temporal_reasoning) get the upgrade silently on open.
- WAL entries carry facts, not index keys, so a sidecar WAL on a v7 file replays unchanged after
  the rebuild.
- The browser backend (`src/browser/mod.rs`, `import_graph`) uses the same
  `PersistentFactStorage::load`, so it gets the migration with no separate code.

### Dropping v1–v6

- `FileHeader::validate` rejects versions below 7 with a new error code (next free `STG-0xx`),
  whose text names the version found and says the file must first be opened with Minigraf v2.x to
  upgrade it to v7. Versions above 8 keep `STG-006`, whose text changes from
  `supported: 1-{}` to `supported: 7-{}`. Both are added to `docs/ERROR_REFERENCE.md`.
- Removed: `src/storage/btree.rs` (v5 paged-blob indexes), `migrate_v1_to_v2`,
  `migrate_v5_to_v6`, `load_one_per_page_legacy`, the `fact_page_format` one-per-page branch in
  `load`, and the pre-v7 branches of header parsing and checksum handling (the `version >= 6`,
  `version >= 7` and fact-pages-only checksum fallbacks). Their tests go with them.
- Kept: every v7 read path, since v7 is still opened (and migrated).
- All hand-built key literals (`build_sorted_index_entries`, `Indexes::insert`,
  `graph/storage.rs` range bounds, tests) go through one constructor per key type
  (`EavtKey::from_fact` etc.) so a future field change cannot miss a site.
- Planning must list every removed function and test, and confirm nothing else reaches the removed
  code (fuzz targets, examples, benches, bindings).

## Testing

Regression tests are written first and shown failing before any fix:

1. Issue reproduction: two values of one attribute in one transact; read through EAVT
   (`[:e :attr ?v]`), AEVT (`[?e :attr ?v]`) and full scan (`[?e ?a ?v]`), in three states:
   before checkpoint (pending `BTreeMap`s), after reopen without checkpoint (WAL replay), and after
   checkpoint and reopen (on-disk B+tree). Run over several graph shapes, including filler entities
   with two attributes each, which is what made EAVT and AEVT disagree in the issue. All paths
   return both values in all states.
2. Batched retract of both values, then checkpoint and reopen: no path returns either value.
3. Same value asserted and retracted in one `WriteTransaction` with the same valid window: all
   four indexes keep both facts. This test must fail on the current code first; if it cannot be
   made to fail, `asserted` is not added to AVET/VAET and the spec is revised.
4. v7 → v8 migration, from real v7 files written by v2.0.0 code:
   - The existing `tests/fixtures/compat.graph` is v7. Its native test
     (`tests/cross_platform_compat_test.rs`) and browser test (`src/browser/mod.rs`) now cover
     the migration path; add assertions that the header is v8 after open and that a second open
     does not rebuild.
   - Add a second fixture with colliding values, generated on `main` (v2.0.0) with the existing
     `generate_compat_fixture` example pattern. After migration, both values read back through
     all paths. The generation steps are recorded in the fixture's test doc comment.
5. Files with versions 1–6 fail to open with the new error code; files with version 9 fail with
   `STG-006`. Neither is modified on disk.
6. Maximum-size value: a fact whose value is near `MAX_FACT_BYTES` checkpoints and reads back
   through all four indexes. AVET already stores full value bytes today, so this is not a new
   risk class, but EAVT/AEVT separators in internal nodes now carry values too.
7. Unit tests: key ordering, range-bound minima, per-key constructors, and `selective_fact_fetch`
   dedup with two facts that differ only in value.
8. Update existing assertions on `FORMAT_VERSION == 7` and header version 7.

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

- Larger EAVT/AEVT keys lower B+tree fanout and grow the file. Measure file size and run the
  benchmark suite (`cargo bench`, not `cargo test --release`) against `main` before claiming no
  regression, and record the numbers in the PR.
- Oversized entries: `build_btree` writes one entry per page when an entry exceeds the fill
  threshold, but an entry larger than a page would overflow `write_leaf_page`. Facts are capped at
  `MAX_FACT_BYTES` and AVET entries already carry full value bytes, so EAVT/AEVT entries have the
  same bound as AVET today. Test 6 confirms this rather than assuming it.
- `encode_value` in the `selective_fact_fetch` dedup key allocates per fetched fact. Acceptable
  for correctness; note it in the benchmark comparison.
- v8 files are unreadable by v2.x. This is the intended breaking change for v3.0.0.
