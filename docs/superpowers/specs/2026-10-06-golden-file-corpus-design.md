# Golden-file compatibility corpus (#391)

**Issue:** #391 (parent #383, milestone v3.0.0)
**Status:** design
**Branches:** PR A on `main` (v7 corpus), PR B on `v3` (migration checks + v8 corpus)

## 1. Problem

`tests/fixtures/` holds `compat.graph`, a two-fact v7 file. On `v3` there is also
`v7_multivalue.graph` (#371). No committed file covers the shapes real users have
on disk: several checkpoints, an index rebuilt on open, a leftover WAL. v3.0.0
migrates v7 to v8 one way, so the v7 shapes must be frozen with expected results
before the migration ships. v8 must be frozen too, now that its layout is settled
(v8 spec §11).

## 2. Scope

- **Supported versions.** On `v3`, v1–v6 are rejected with STG-028
  (`migration_matrix_test.rs` covers this), so "every supported version" means v7
  and v8. Real-data v1–v6 files are out of scope. Those versions only matter to
  v2.x, which gets data-integrity fixes only.
- **`main` (v2.x)** gets the v7 corpus with read, checkpoint and reopen checks. That
  guards v2.x patch releases against breaking their own format.
- **`v3`** gets the same v7 files (through the regular `main` → `v3` merge), checks
  them after migration, and adds a v8 corpus.

## 3. Layout

```
tests/golden/
  README.md                     rules + how each file was made
  v7_basic.graph                + v7_basic.json
  v7_multi_checkpoint.graph     + .json
  v7_index_rebuilt.graph        + .json
  v7_pending_wal.graph          + v7_pending_wal.graph.wal + .json
  v7_stale_wal.graph            + v7_stale_wal.graph.wal + .json
  v7_multivalue.graph           + .json
  v8_*.graph / .wal / .json     (PR B)
  gen/v7/                       standalone crate, minigraf = "=2.0.3" (crates.io)
  gen/v8/                       standalone crate, minigraf = { git, rev = <v3 sha> } (PR B)
tests/golden_corpus_test.rs     one harness, runs every manifest
```

Each generator crate has its own `[workspace]` table and a committed `Cargo.lock`.
It is listed in the root `workspace.exclude`, so `cargo build` and `cargo test` never
build it. `cargo package` skips directories that have their own `Cargo.toml`.

## 4. Generators

A generator records how a file was made. It is never re-run to replace a committed
file. `cargo run -- <out-dir>` writes every file into `<out-dir>`. It refuses to run
if a target already exists, then prints each file's CRC32 and final `tx_count` for
the manifest.

Shared dataset (`base`), three transactions:

1. Entities `:alice`, `:bob` and `:carol` with every value type: string (ASCII,
   Unicode `"Zoë 🚀"`, a 2,000-byte string), integer (including `i64::MIN` and
   `i64::MAX`), float, boolean, keyword, and ref (`:alice :person/friend :bob`).
2. Valid-time facts: `:alice :employment/status` with two non-overlapping
   `{:valid-from :valid-to}` windows, plus one open-ended window.
3. Retract `[:alice :person/age 30]`, assert `[:alice :person/age 31]`.

v7 files (gen/v7, minigraf 2.0.3):

| File | Recipe |
|---|---|
| `v7_basic` | `base`, one checkpoint |
| `v7_multi_checkpoint` | `base` with a checkpoint after each transaction, then 300 filler entities in 3 transacts with checkpoints between them, so the facts span several fact pages and saves |
| `v7_index_rebuilt` | Same as `v7_multi_checkpoint`, then flip byte 64 (`index_checksum`) and re-seal the header CRC (bytes 80..84), then open with 2.0.3, which rebuilds the indexes and rewrites the header (#370 path), then close |
| `v7_pending_wal` | `base` checkpointed, then 2 more transacts (one with a retraction) under `wal_checkpoint_threshold(usize::MAX)`, then `mem::forget` on the handle, as a crash would: the transactions live only in a v1 WAL |
| `v7_stale_wal` | As `v7_pending_wal`, but the handle is dropped. 2.0.3's `PersistentFactStorage` saves on drop even under `usize::MAX`, so the facts reach the file and the WAL holds only already-checkpointed entries (#447 shape; also what a crash between a checkpoint's save and its WAL delete leaves) |
| `v7_multivalue` | The #371 shape: `[:t/x :kind :k/a] [:t/x :kind :k/b]` plus 30 fillers in one transact, then `[:t/y :tag :g/a] [:t/y :tag :g/b]` and a batched retract of both, then checkpoint |

v8 files (gen/v8, PR B, `v3` pinned at a commit after #452):

| File | Recipe |
|---|---|
| `v8_basic` | `base`, one checkpoint |
| `v8_multi_checkpoint` | As for v7. COW checkpoints reuse pages from the free list and alternate meta slots A/B |
| `v8_pending_wal` | As for v7. WAL version 2 with `base_generation` |
| `v8_stale_wal` | As for v7, with a v2 WAL |
| `v8_multivalue` | The #371 shape, written natively |
| `v8_migrated_from_v7` | A copy of `v7_multi_checkpoint` opened by the pinned `v3` build, which migrates it, then closed |
| `v8_large_values` | Values that need value pages, near `MAX_VALUE_BYTES` |

## 5. Manifest

One JSON file per golden file, read with `serde_json` (already a dev-dependency).
The expected rows are **written by hand from the recipe**, not recorded from a
read. A recorded read would freeze a bug such as #371.

```json
{
  "file": "v7_pending_wal.graph",
  "wal": "v7_pending_wal.graph.wal",
  "format": 7,
  "written_by": "minigraf 2.0.3 (crates.io), tests/golden/gen/v7 recipe pending_wal",
  "crc32": { "graph": "0x1a2b3c4d", "wal": "0x5e6f7081" },
  "tx_count": 5,
  "queries": [
    { "name": "ages now",
      "query": "(query [:find ?n ?a :where [?e :person/name ?n] [?e :person/age ?a]])",
      "rows": [["\"Alice\"", "31"], ["\"Bob\"", "42"]] },
    { "name": "multi-value via EAVT",
      "query": "(query [:find ?v :where [:t/x :kind ?v]])",
      "rows": [[":k/a"], [":k/b"]],
      "min_reader": 3 }
  ]
}
```

- Each value in `rows` is written in canonical text: strings as JSON-escaped literals
  in quotes, integers in decimal, floats as Rust `{}` output, `true`/`false`,
  keywords as `:ns/name`, `nil` for null. Queries never return entity ids or refs.
  They join through them instead, because ids are not a stable output.
- Rows are compared as **sets**, since the v8 spec allows a different row order.
- `min_reader` skips a query on older readers. It is used only where a release
  is known to be wrong (#371 on v2.x). Each use carries a `"known_issue"` field.
- Queries cover: every value type; EAVT, AEVT and AVET lookups; joins through refs;
  `:as-of N` for each `tx_count`; `:valid-at` inside, between and outside windows;
  `:any-valid-time`; `not`; and `count` aggregation.

## 6. Harness (`tests/golden_corpus_test.rs`)

`const READER: u32` is 2 on `main` and 3 on `v3`. The harness finds every
`tests/golden/*.json` and, for each one:

1. **Frozen check.** The CRC32 of the committed `.graph` (and `.wal`) equals the
   manifest value.
2. **Skip.** A manifest whose `format` this reader cannot open is skipped: v8 on
   `main`. On `v3` nothing is skipped.
3. **Open.** Copy the file (and WAL) into a tempdir and open it. Check
   `current_tx_count() == tx_count` and run every query.
4. **Persist.** Checkpoint, close, reopen, and run the queries again. On `v3`, the
   header must now be v8 and the WAL must be gone. (v2.x keeps an already-checkpointed
   WAL after a checkpoint, because replay counted no new entries. The data is correct,
   so `main` does not check this.)
5. **Write after.** Transact one fact. `tx_count` becomes `tx_count + 1`. Reopen:
   the new fact is present and the counter has not rewound (#447).
6. **Untouched.** The committed fixture's bytes are unchanged. The harness only
   ever writes to the tempdir copy.

PR B adds these `v3`-only checks (as delivered, the WAL-gone check skips manifests marked `"stale_wal": true`: v3 also keeps an already-checkpointed WAL until the next write is checkpointed):

- The first open of each v7 file migrates it. Opening the same copy again
  without a checkpoint gives the same results.
- `compat.graph` and `tests/fixtures/v7_multivalue.graph` stay where they are and
  keep their current tests. They are not moved, because moving them breaks
  `include_bytes!` users (`src/browser/mod.rs`).

One `#[test]` per manifest would need a macro or build script. Instead, a single
test walks every manifest, collects all failures, and reports each one by manifest
and query name, without `{:?}` on `Value`/`Result` (CodeQL rule). One failing
manifest does not hide the others.

## 7. Add-only rule

- **CI.** A new `golden-corpus` job in `policy.yml` runs on pull requests. It fails
  if `git diff --diff-filter=MDR origin/<base>...HEAD -- 'tests/golden/*.graph'
  'tests/golden/*.wal'` lists anything. Manifests may gain queries, and reviewers
  check edits to existing expectations.
- **CRC.** The CRC in the manifest catches a changed file even when the CI job
  is bypassed.
- **Docs.** `tests/golden/README.md` and CONTRIBUTING.md: every release that
  changes on-disk behavior adds golden files with a generator recipe. Golden files
  are never regenerated, only added.

## 8. Delivery

**PR A (`main`)**: `tests/golden/` with the v7 files, manifests, README and
`gen/v7`; the harness with `READER = 2`; `workspace.exclude`; the policy job; and
docs (CONTRIBUTING, TEST_COVERAGE, CHANGELOG Unreleased, CLAUDE.md test count). No
release needed. This is test-only.

**PR B (`v3`)**, after `main` is merged into `v3`: `READER = 3`; the migration
checks; `gen/v8` and the v8 files; v3 docs. The format is frozen from this point:
a change to v8 needs a new golden file and a new feature bit, never an edit.

## 9. Risks

- **The 2.0.3 crate differs from the `v2.0.3` tag.** crates.io holds the published
  source, which is what users ran. The generator records the version, and its
  `Cargo.lock` pins the dependency tree.
- **Floats in manifests.** Only literals with an exact decimal form are used
  (`1.5`, `-0.25`), so formatting cannot drift.
- **Wall-clock `tx_id`.** Files hold real timestamps. Queries use `:as-of N`
  (tx_count) and explicit valid-time strings, never "now"-relative times. Facts
  asserted without `:valid-from` are valid from their `tx_id`, so `:valid-at`
  queries use dates after 2026-10-06 or explicit windows only.
