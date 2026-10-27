# #530: batch projected UUID lookups

Date: 2026-10-27. Base: `v3` at `80120758`.

## Problem

The streaming engine keeps internal ids through scans and operators, but returned
entity/ref cells still cause a root-to-leaf DICT lookup for each distinct id.
The original 1M-fact attribute scan spends most of its time in this conversion.
Resolve projected ids together without retaining a whole-query dictionary cache.

## Design

`Project` collects at most 1,024 distinct committed UUID-cache misses from the
returned cells. A preflight avoids initializing hints when fewer than two ids
need conversion. A fixed stack buffer maintains sorted, distinct ids using
binary search and insertion; repeated ids move nothing. Once full, remaining
ids use ordinary scalar conversion. Pending/provisional ids never read DICT.

`KeySource::uuids_of` is an internal required method. The pinned `OnDiskReader`
forwards to `DictReader::uuids_of`; no public API or retained reader fields change.
The dictionary checks sorted/distinct input and processes chunks of at most
1,024 ids. A chunk with at least 64 ids and numeric span at most twice its count
uses the existing `LeafCursor`: seek once to its first key, then advance through
leaves, decode integer keys and parse UUIDs only for requested ids. There is no
per-id key encoding; transient leaf decoding is bounded to one leaf plus its
path. Stop at the last requested id or the end of the eid
prefix. The density bound limits the skipped entries. Sparse/small chunks use
existing exact-key B+tree reads, preserving restart-point leaf searches. Both
paths skip the scalar reader's temporary memo map and share UUID decoding.
There are no production changes to the B+tree or key encoding.

Hints do not populate `IdSpace` or charge its meter. Each returned cell consumes
its hint or scalar lookup in the original row/column order, then performs the
original memory charge, allocation and cache insertion. Any failed or incomplete
speculative read discards all hints. Scalar errors remain authoritative: a later
bad id or budget refusal cannot displace an earlier value error. A dense walk can
encounter corruption in skipped leaves; its error causes scalar fallback rather
than changing the query result. `verify()` remains responsible for unrelated
corruption. `end_pull` still releases committed lookup maps.

No fields or retained caches are added to the stream, reader or id space. Stack
buffers are bounded independently of database size; the heap memory meter counts
retained query state. A 256 KiB worker-stack probe covers the projection cases.

## Boundaries and philosophy

This optimizes projected entity/ref cells. Joins, collect-set, sorting and
aggregate value conversions keep their existing lookup behavior. It adds no
configuration, dependencies, file-format changes or version bump. Reusing
existing tree primitives with bounded temporary memory follows the embedded,
portable, reliability-first philosophy. The unchanged 1,230 KiB binary-size cap
and coverage floors are acceptance requirements.

## Acceptance

- Dense batches share tree/leaf reads; sparse batches read no more pages than
  scalar. Compare results and raw reads with cache disabled at 63/64 ids and
  spans of `2c`/`2c+1`, as well as sparse and multi-chunk subsets.
- Cover empty/invalid input, requested missing/malformed UUIDs, unrelated malformed
  values, callback errors after partial output, wrong page types and cycles.
- Cover committed/pending/repeated projection, duplicate columns, row order,
  fallback, memory charging and earlier scalar errors. Check the sorted buffer
  against a set for ascending, descending, repeated and shuffled inputs.
- Queries without projected entities, including count, do no entity conversion
  before cache release. Existing multi-batch/generation-pin tests still pass.
- Run default and `test-hooks` suites, clippy, release size and coverage gates on
  the bench. Run idle, sequential A/B timings pinned to one NUMA node, including
  the original 1M scan, point reads, sparse/dense-threshold and descending/random/
  repeated refs. Profile separately from timing.
- Obtain Claude spec/plan and final reviews. Target `v3`; keep benchmark scratch
  until CI is green, retain reports locally, then remove task-owned bench files.
