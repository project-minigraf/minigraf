# Golden-file compatibility corpus

Database files written by released versions of Minigraf, each with a manifest of
the results it must give (#391). `tests/golden_corpus_test.rs` opens a copy of every
file with the current build and checks its manifest:

1. The committed files still match the CRC32 values in the manifest.
2. After open, `tx_count` and every query result match the manifest.
3. After a checkpoint and a reopen, they still match.
4. One more write moves `tx_count` past the file's value and survives a reopen.

Manifests for a newer format than the reader supports are skipped.

## Rules

- **Golden files are never regenerated, only added.** A `.graph` or `.wal` file here
  is a record of what a released version wrote. CI (`policy.yml`, job
  `golden-corpus`) fails a pull request that modifies, deletes or renames one.
- **Every release that changes on-disk behavior adds golden files** for that
  behavior, with a recipe in a generator crate.
- **Expected rows are written by hand from the recipe**, not recorded from a read.
  A recorded read would freeze a bug as expected output.
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

## Generators

`gen/v7/` is a standalone crate pinned to `minigraf = "=2.0.3"` from crates.io,
with a committed `Cargo.lock`. It records how each v7 file was made. It is not part
of the workspace and is never re-run to replace a file: it refuses to write over an
existing one.

```sh
cd tests/golden/gen/v7 && cargo run -- <empty-dir>
```
