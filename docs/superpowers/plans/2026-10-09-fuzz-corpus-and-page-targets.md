# Fuzz corpus and page targets (PR A) — Plan

Spec: `docs/superpowers/specs/2026-10-09-fuzz-corpus-and-ops-target-design.md` §3–§4.
Branch `fuzz/387-corpus-and-page-targets` off `origin/v3`; PR to `v3`, "Refs #387, #375".
Then PR A′: the two workflow files only, to `main`.

## Tasks

1. **Shared template helper** — `fuzz/fuzz_targets/common/mod.rs` (included with
   `#[path]`): build the v8 template once (`OnceLock`) through the public API, list its
   pages by type from the page headers, `fix_page_crc`, `fix_meta_crc`, and run the
   standard probe queries plus `verify()`. Add `crc32fast` to `fuzz/Cargo.toml`.
2. **`btree_page`** — rewrite per spec §4.1.
3. **`fact_page`** — rewrite on `tests/golden/v7_basic.graph` per §4.2.
4. **`file_header`** — rewrite per §4.3.
5. **Seeds** — a `fuzz/seedgen` example? No: a `#[test]`-free binary is overkill.
   Generate seeds once with a throwaway program in the scratchpad that uses the
   same helper, and commit the bytes. Note how they were made in
   `fuzz/corpus/README.md`.
6. **Acceptance for #375** — temporary `eprintln!` in `node.rs` leaf/internal decode
   (not committed): run `btree_page` on the leaf seed, internal seed and a
   truncated seed; confirm decode reached and the leaf seed's query returns rows.
   Run each target for 60 s locally; no crashes.
7. **Workflows** — `fuzz.yml` (two crons, `targets` → matrix `fuzz` → `report`,
   corpus cache), `fuzz-corpus.yml` (weekly cmin, seeds kept, cache save, bot
   branch + PR, compare link fallback). Lint with `actionlint` if available.
   Trigger `workflow_dispatch` on the PR branch with `seconds=60` to smoke-test.
8. **Docs** — `docs/TEST_COVERAGE.md` fuzz section, CLAUDE.md only if counts change
   (fuzz targets are not counted as tests).
9. **PR A** to `v3`; monitor CI. **PR A′** to `main` (workflows only).
