# Golden-file corpus: implementation plan (#391)

Spec: `docs/superpowers/specs/2026-10-06-golden-file-corpus-design.md`.

## PR A: `main` (branch `test/391-golden-corpus`)

1. **Generator crate** `tests/golden/gen/v7/` with `minigraf = "=2.0.3"` and
   `crc32fast`, its own `[workspace]`, and a committed `Cargo.lock`. Add the
   crate to root `workspace.exclude`. It holds one function per recipe:
   `basic`, `multi_checkpoint`, `index_rebuilt`, `pending_wal` and `multivalue`.
   It refuses to overwrite existing files and prints the CRC32 and `tx_count`
   of each file.
2. **Generate** into `tests/golden/`. Check the version bytes (`xxd -l 8`) and
   that `v7_pending_wal.graph.wal` exists.
3. **Manifests**: write the five `.json` files by hand from the recipes, as in
   spec §5.
4. **Harness** `tests/golden_corpus_test.rs` with `READER = 2`, implementing
   the steps in spec §6. Check that it is green, then that it catches a broken
   manifest: change one expected row and one CRC, see both failures reported,
   then revert.
5. **Policy job** `golden-corpus` in `.github/workflows/policy.yml`. This job
   lets PRs add golden files but never modify or delete them.
6. **Docs**: `tests/golden/README.md`, CONTRIBUTING.md (the add-only rule),
   `docs/TEST_COVERAGE.md`, CHANGELOG Unreleased, and the CLAUDE.md test count.
7. Run fmt, clippy, and `cargo test`. Open the PR to `main` and own CI until it
   passes.

## PR B: `v3` (after `main` is merged into `v3`)

1. Set `READER = 3`. Add the migration checks: the header is v8 after open, and
   the v1 WAL is replayed and then removed.
2. **Generator crate** `tests/golden/gen/v8/` pinned to a `v3` git rev. It
   writes `basic`, `multi_checkpoint`, `pending_wal`, `multivalue`,
   `migrated_from_v7` and `large_values`.
3. Write the v8 manifests and update the v3 docs. Open the PR to `v3`.
