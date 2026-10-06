# Golden-file compatibility corpus

Database files written by released versions of Minigraf, each with a manifest of
the results it must give (#391). `tests/golden_corpus_test.rs` opens a copy of every
file with the current build and checks its manifest:

1. The committed files still match the CRC32 values in the manifest.
2. After open, `tx_count` and every query result match the manifest.
3. After a checkpoint and a reopen, they still match.
4. One more write moves `tx_count` past the file's value and survives a reopen.

On v3 and later, a v7 file must also have a v8 meta page after its first open
(it is migrated in place), give the same results when the migrated copy is
reopened, and lose its WAL at the checkpoint.

Manifests for a newer format than the reader supports are skipped.

## Rules

- **Golden files are never regenerated, only added.** A `.graph` or `.wal` file here
  is a record of what a released version wrote. CI (`policy.yml`, job
  `golden-corpus`) fails a pull request that modifies, deletes or renames one.
- **Every release that changes on-disk behavior adds golden files** for that
  behavior, with a recipe in a generator crate.
- **Expected rows are written by hand from the recipe**, not recorded from a read.
  A recorded read would freeze a bug as expected output.
- `"stale_wal": true` marks a WAL whose entries are all already in the file. A
  checkpoint then has nothing to do, so the WAL stays until the next write is
  checkpointed; the test does not require it to be gone.
- A query that a release is known to get wrong carries `"min_reader"` (the first
  major version that gets it right) and `"known_issue"`.

## Manifest format

```json
{
  "file": "v7_basic.graph",
  "wal": null,
  "format": 7,
  "written_by": "minigraf 2.0.3 (crates.io), tests/golden/gen/v7 recipe basic",
  "crc32": { "graph": "0x368e4f45", "wal": null },
  "tx_count": 4,
  "queries": [
    { "name": "eavt lookup",
      "query": "(query [:find ?a :where [:alice :person/age ?a]])",
      "rows": [["31"]] }
  ]
}
```

Rows are compared as sets. Each value is written as canonical text: strings as
JSON-escaped literals in quotes, integers in decimal, floats as Rust `{}` output,
`true`/`false`, keywords as `:ns/name`, and `nil` for null. Queries never return
entity ids, because ids are not a stable output.

## Files

| File | Written by | Shape |
|---|---|---|
| `v7_basic` | 2.0.3 | Shared dataset (every value type, refs, Unicode, a 2,000-byte string, valid-time windows, a retraction), one checkpoint |
| `v7_multi_checkpoint` | 2.0.3 | Shared dataset with a checkpoint after each transaction, plus 300 filler entities over three more checkpoints |
| `v7_index_rebuilt` | 2.0.3 | `v7_multi_checkpoint` with a damaged `index_checksum`, opened once by 2.0.3, which rebuilt the indexes and rewrote the header. Stale pages remain past `page_count` |
| `v7_pending_wal` | 2.0.3 | Shared dataset checkpointed, then two transactions held only in a v1 WAL (the session ended without running destructors, as a crash would) |
| `v7_stale_wal` | 2.0.3 | Same as `v7_pending_wal`, but the session closed normally: the facts are in the file, and the WAL next to it holds only entries already checkpointed (#447 shape) |
| `v7_multivalue` | 2.0.3 | Same-transaction multi-values and a batched retract (#371). v2.x reads these wrong |
| `v8_basic` | `v3` at 23aa57c | Shared dataset, one checkpoint |
| `v8_multi_checkpoint` | `v3` at 23aa57c | As `v7_multi_checkpoint`: seven copy-on-write checkpoints, alternating meta slots, reused free pages |
| `v8_pending_wal` | `v3` at 23aa57c | As `v7_pending_wal`, with a v2 WAL (records its base generation) |
| `v8_stale_wal` | `v3` at 23aa57c | As `v7_stale_wal`, with a v2 WAL |
| `v8_multivalue` | `v3` at 23aa57c | The #371 shape, written natively |
| `v8_migrated_from_v7` | `v3` at 23aa57c | `v7_multi_checkpoint` migrated in place (v8 spec §9) |
| `v8_large_values` | `v3` at 23aa57c | Shared dataset plus a 4,068-byte string stored twice (deduplicated) with one copy retracted, 64- and 65-byte strings (inline and value-page boundary), a 1,000-byte attribute and a 1,000-byte keyword |

## Generators

`gen/v7/` is a standalone crate pinned to `minigraf = "=2.0.3"` from crates.io,
with a committed `Cargo.lock`. It records how each v7 file was made. It is not part
of the workspace and is never re-run to replace a file: it refuses to write over an
existing one.

`gen/v8/` is the same for the v8 files, pinned by git rev to the `v3` branch at
`23aa57c5e46e7e78fa823425cc74cff3764f1763`, after the v8 format froze and before
v3.0.0 was published. There is no v8 `index_rebuilt` file: v8 never rebuilds an
index on open.

```sh
cd tests/golden/gen/v7 && cargo run -- <empty-dir>
cd tests/golden/gen/v8 && cargo run -- <empty-dir>
```
