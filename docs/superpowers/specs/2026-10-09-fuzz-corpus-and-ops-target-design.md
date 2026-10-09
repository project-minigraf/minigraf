# Fuzz corpus, page targets and an operation-sequence target — Design

**Issues:** #387 (with #375), tracker #383
**Milestone:** v3.0.0, branch `v3`, plus a workflow-only PR to `main`

## 1. Problems

1. `.github/workflows/fuzz.yml` neither caches nor commits the corpus. Every night
   each target starts again from the seeds in `fuzz/corpus/` and spends its 55
   minutes finding the same coverage again.
2. Scheduled workflows run from the default branch's workflow file, and that file
   checks out `main`. The v3 code (format v8) has never been fuzzed at night.
3. On `v3`, three of the six targets no longer reach the code they are named for.
   They still write the pre-release single-header layout, which has no `"META"`
   magic:
   - `btree_page` and `fact_page` stop at STG-032 on every input (#375 was filed
     against an older layout; the problem is now worse).
   - `file_header` fuzzes page 0, but almost every input fails the meta CRC, so
     the meta field checks behind it are never reached.
4. Every target parses one input. No target fuzzes a sequence of storage
   operations or checks that the database returns correct data.

`wal_entry`, `datalog_parser` and `datalog_eval` are fine.

## 2. Delivery

| PR | Base | Contents |
|---|---|---|
| A | `v3` | §3 workflows, §4 page targets (#375), seeds |
| A′ | `main` | §3 workflows only, byte-identical to PR A's copies |
| B | `v3` | §5 `ops_sequence`, §6 `corrupt_file`, seeds |

All say "Refs #387" (PR A also "Refs #375"). No closing keywords: the tracker
closes issues. The workflow files are the same on both branches, so future
`main` → `v3` merges do not conflict on them.

No library code changes. The fuzz crate is outside the published package and
does not affect binary size.

## 3. Workflows

### 3.1 Nightly fuzz (`fuzz.yml`)

Same branch selection as `bench.yml`: two crons, and the cron string that fired
picks the ref.

```yaml
on:
  schedule:
    - cron: '0 4 * * *'   # main
    - cron: '0 16 * * *'  # v3
  workflow_dispatch:
    inputs:
      ref: { default: v3 }
      seconds: { default: '3300' }
env:
  FUZZ_REF: <dispatch input, else v3 for the 16:00 cron, else main>
```

Jobs:

1. **`targets`**: checks out `FUZZ_REF`, reads the `[[bin]]` names from
   `fuzz/Cargo.toml`, and outputs them as a JSON matrix. Each branch fuzzes the
   targets it has, so PR B's new targets join the v3 run without touching
   `main`'s workflow.
2. **`fuzz`** (matrix over targets, `fail-fast: false`, in parallel): checks out
   `FUZZ_REF`, restores the corpus cache, runs
   `cargo fuzz run <t> fuzz/corpus/<t> -- -max_total_time=$seconds`, saves the
   cache (`if: always()`), and uploads `fuzz/artifacts/<t>` plus the log as an
   artifact if the run failed. Running in parallel cuts the wall time from ~5.5 h
   to ~1 h; the repo is public, so the minutes are free.
3. **`report`** (`needs: fuzz`, `if: failure()`): downloads the failure
   artifacts and comments on, or opens, the "Nightly fuzz failure" issue as
   today, with the branch in the body. Then fails.

**Corpus cache.** `actions/cache/restore` and `actions/cache/save`, key
`fuzz-corpus-<ref>-<target>-<run_id>`, restore key prefix
`fuzz-corpus-<ref>-<target>-`. The newest run's corpus is restored and the run
saves a new entry. Committed seeds and the cached corpus share the directory;
libFuzzer reads both. GitHub evicts entries unused for 7 days and caps the repo at
10 GB. With nightly runs, older entries age out on their own.

### 3.2 Weekly corpus minimization (`fuzz-corpus.yml`)

Mondays, once per branch (same two-cron pattern, at 08:00 and 20:00), plus
`workflow_dispatch`. For each target in turn, in one job:

1. Restore the newest corpus cache for `<ref>-<target>`.
2. `cargo fuzz cmin <t> fuzz/corpus/<t>`.
3. Keep every `seed_*` file (regression seeds are never dropped), plus the
   minimized files cmin kept.
4. Save the minimized corpus as a new cache entry, so the nightly run starts from
   it.

Then commit `fuzz/corpus/` to the branch `bot/fuzz-corpus-<ref>` (force-pushed)
and open a PR to `<ref>` with `gh pr create`, or update the open one. A corpus
file larger than 64 KiB is left out of the commit (stays in the cache only).

Needs `contents: write` and `pull-requests: write`. **The repository setting
"Allow GitHub Actions to create and approve pull requests" is off today.** While
it is off, the job pushes the branch and prints a compare link in the job
summary instead of failing. PRs opened with `GITHUB_TOKEN` do not trigger CI.
That is fine for corpus-only changes, and a maintainer reviews and merges them.

## 4. Page targets (PR A, #375)

The fuzz crate sees only the public API. To reach a decoder behind a checksum,
a target builds a real file with the public API, overwrites one page with fuzzer
bytes, and recomputes that page's checksum itself (`crc32fast`; the layouts are
in `storage/page.rs` and `storage/meta.rs`, spec §4). The target never asserts
success: errors are expected, and only panics, hangs and OOMs count as findings.

**Template.** Built once per process (`OnceLock`) in a temp dir: 90 entities
with a number, a name and a ref to the next entity, a long string (a value
page), a fact-level valid-time window, a keyword and a retraction. The draft is
then rewritten through `LogWriter` with fixed tx times (`10^12 + 1000 ×
tx_count`). The bytes are then the same in every process, so seeds cut from them
are still valid when the target runs. `LogWriter` commits once (one batch):
every tree page is reachable from the meta page, and nothing is freed. 90
entities give the four fact indexes two leaves and an internal root each, and
keep a full scan cheap. Generation 1 is in slot A and generation 2 in slot B.
Each run copies the template bytes to a fresh file.

**One probe per run.** Under ASan, running all seven probe queries and
`verify()` on every input held the targets to ~8 executions/s. Instead, one
input byte picks one probe (`pick % 8`: seven queries that between them scan
EAVT, AEVT, AVET, VAET, the DICT, `:valid-at` and `:as-of`, or `verify()`). The
fuzzer varies that byte like any other.

### 4.1 `btree_page`

- Byte 0 picks the node: bit 7 picks an internal node (else a leaf), and bits
  0–6 pick which one.
- Byte 1: bit 0 flips the type byte between leaf and internal, bit 1 opens
  read-only. Byte 2 picks the probe.
- The rest overwrites the node's `count` field (offset 2) and body (offset 24).
  The page id and generation stay, and the page CRC is recomputed, so the page
  passes the header check and reaches the node decoder.

### 4.2 `fact_page`

v7 fact pages are read only when a v7 file is opened. The target uses
`tests/golden/v7_basic.graph` as its template (`include_bytes!`), overwrites
fact page 1 with the input, and opens. Byte 0 bit 0 picks read-only (in-memory
load) or read-write (migration to v8). Then a full scan, a name query and
`verify()`. The v7 header checksum covers only the header.

### 4.3 `file_header`

Byte 0: bit 0 replaces slot B (else slot A); bit 1 writes the correct meta CRC
over the result, so the field checks (page count, roots, free list, feature
bits) are reached; bit 2 zeroes the other slot, so the fuzzed one is the only
candidate; bit 3 keeps the magic and version (bytes 0..16) and overwrites from
the generation field on; bit 4 opens read-only. Byte 1 picks the probe. The
rest overwrites the slot.

### 4.4 Seeds

Each target gets seeds that reach its decoder. `btree_page`: a valid leaf, the
same leaf opened read-only, two valid internal nodes, a leaf whose `count`
overruns its truncated slot directory, and a leaf with its type flipped to
internal. `fact_page`: the v7 fact page, read-only and read-write.
`file_header`: each slot's own meta page, slot B alone, and slot B's fields
only, read-only. A seed generator is not committed: seeds are cut from the
template with the input layout above. Stale seeds
built for the old layout (`seed_v7_3page.bin`, `seed_v5_large_page_count.bin`,
…) stay, since a seed costs little, and new ones are added next to them.

**Acceptance (#375).** A run on the valid-leaf seed reaches leaf decoding and the
query returns the template's entries. A truncated-slot seed reaches the decoder
and gets an error. Both were checked once with a temporary trace in
`storage/node.rs` (not committed): the valid leaf and internal seeds return all
rows and `verify()` passes, while the truncated and type-flipped seeds decode
the fuzzed page and fail with INT-049.

## 5. `ops_sequence` (PR B)

A differential target: storage operations on a file-backed database, compared
with the reference model in `tests/reference_model/mod.rs` that #385 and #386
use.

- The fuzz crate includes the model with
  `#[path = "../../tests/reference_model/mod.rs"] mod reference_model;` and adds
  `proptest` and `uuid` as fuzz-crate dependencies. The check helpers of
  `tests/model_based_test.rs` (`check_full_rows`, `check_shapes`, `check`) move
  into the shared module so the target and the test use one copy.
- The input is decoded with `arbitrary::Unstructured` into at most 32 ops:
  transact, retract, a write transaction (commit or rollback), checkpoint,
  reopen (default, small cache, auto-checkpoint) and crash (open a copy of the
  file and WAL). The pools and grid are the model's, so statements hit each
  other often.
- After each op, it compares the full rows, now and at a fuzzer-chosen `:as-of`,
  and one fuzzer-chosen query shape with the model. A statement the model
  rejects must fail with the model's error code. A mismatch panics with the step
  number and the query. The message carries pool indices, never a `Uuid`.
- Seeds: a few encoded op lists covering every op kind.

The proptest test explores randomly; the fuzzer adds coverage guidance and a
persistent corpus. Both use the same oracle.

## 6. `corrupt_file` (PR B)

Opening a damaged file must return an error or correct data, never wrong data.

- **Template** (once per process): a file built from fixed statements over
  three checkpoints, with no WAL left. For each checkpoint generation, the
  expected answers of a fixed set of queries are recorded.
- **Input:** up to 16 `(offset, xor)` edits (offset modulo file length, nonzero
  xor) and an optional truncation to a page multiple or an arbitrary length.
  Checksums are **not** recomputed: the CRCs are the contract under test.
- Open read-only or read-write (from one input byte), then run the fixed
  queries. Each query must either fail or return exactly the answer of the last
  generation, or of the one before it (the meta fallback when the active meta
  page is damaged). The whole open must match one generation, not a different
  generation per query. A mismatch panics.
- `verify()` runs too. It may report problems or fail but must not panic.
- Seeds: no edits, a flip in each page type, and a truncation at the last page.

Any finding gets a regression test in `tests/` and a seed, as for every fuzz
find.

## 7. Out of scope

- Fuzzing the browser or IndexedDB backend.
- Byte edits with recomputed CRCs. CRC32 does not protect against deliberate
  forgery, and that is not part of the format's contract.
- WAL corruption beyond what `wal_entry` covers.
