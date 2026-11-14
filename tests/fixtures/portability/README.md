# Portability fixtures (#441)

The same database, built from `commands.txt`, must answer every query in
`queries.txt` (tab, then the JSON rows, sorted) in every mode and on every
platform.

- `native_v8.graph`: written by the native build (checkpointed, no WAL).
  Regenerate with `cargo test --test opfs_portability_test -- --ignored`.
  The browser tests import it into OPFS mode.
- `opfs_export.graph`: `exportGraph()` of an OPFS-mode database built in a
  browser. `tests/opfs_portability_test.rs` opens it with `Minigraf::open`,
  checks `verify()` and the answers.

- `native_v7.graph`: real format v7, built from the same `commands.txt` with
  the legacy writer used by migration tests. Regenerate with
  `cargo test --lib regenerate_portability_v7_fixture -- --ignored`.
  Native migration and OPFS import answer every query in `queries.txt`,
  including long values, retraction, valid time and transaction time.
