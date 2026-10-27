# Batched UUID projection (#530)

Approved after Claude spec/plan review, 2026-10-27. Targets `v3`, format v8.

## Problem

The streaming engine carries DICT ids until projection, but a scan returning a
different entity in every row still performs one root-to-leaf DICT lookup per
UUID. At `80120758`, the 1M-fact envelope attribute scan spends about 58% of
sampled user cycles under `DictReader::uuid_of` on the idle bench machine.

## Design

`Project` resolves up to 1,024 committed entity cache misses from its returned
columns as one sorted, distinct batch. Duplicate cells are compacted when the
fixed buffer fills so they do not consume its distinct-id bound. Its temporary UUID hints live on the
stack. Pending/provisional ids and existing cache hits are excluded using the
same routing predicate as scalar `IdSpace::uuid`. One distinct miss, no misses,
and in-memory queries stay on the scalar path.

`KeySource::uuids_of` is an additive default method: strictly increasing ids,
callbacks in that order, empty input reads nothing. Its default implementation
calls scalar `uuid_of`. `OnDiskReader` overrides it using the query's same pinned
DICT root, never a newer database state. `DictReader` encodes the ids into a
bounded fixed-size key buffer, then calls `btree::get_many`.

`get_many` groups sorted keys by each internal node's child range, visits every
required page once, and calls today's `leaf_get` for each requested key. It
retains the scalar leaf restart-point behavior rather than decoding full leaves.
Sparse keys do not cause more backend page reads than scalar lookups. Recursion
has scalar `get`'s 32-level bound. Empty and invalid input do no reads. Unsorted
or duplicate keys are the existing internal invariant violation, `INT-049`.
Missing or malformed requested UUIDs are scalar's existing `STG-036`; unrelated
UUID values are never parsed. Fixed DICT keys preserve the existing byte format,
including integer-length boundaries.

Hints do not populate `IdSpace` or charge its meter. When each returned cell is
converted, `uuid_with_hint` uses its hint or the scalar lookup and performs the
original charge, allocation and map insertion in row/column order. A failed or
incomplete speculative read discards all hints. Scalar errors remain authoritative,
so a later bad id or budget refusal cannot displace an earlier value error.
`end_pull` still releases committed lookup maps. No fields or retained caches are
added to the stream, reader or id space. Stack buffers are bounded independently
of database size; the heap memory meter continues counting retained query state.

## Boundaries and philosophy

The change optimizes projected entity/ref cells. Joins, collect-set, sorting and
aggregate value conversions retain their existing lookup behavior. It adds no
configuration, dependencies, file-format changes or release bump. Sharing exact
lookups while preserving scalar behavior follows the embedded, portable,
reliability-first design. It does not introduce a cache that grows with a query's
whole answer.

## Acceptance

- Dense lookup batches read shared tree paths once; sparse batches read no more
  pages than scalar; empty/invalid inputs, missing keys, callbacks and cycles are
  covered by deterministic tests with a disabled page cache.
- Requested UUIDs match scalar over sparse and multi-chunk subsets; unrelated
  malformed values do not fail the batch; fixed encodings keep byte order.
- Projection batches committed ids even when pending/provisional ids share a
  batch; repeated refs, duplicate columns, row order, fallback, memory charging
  and earlier scalar errors remain correct.
- A scan which does not project entities, including a count, performs no entity
  conversion before cache release. Existing multi-batch and generation-pin tests
  continue to pass.
- Default and `test-hooks` suites and CI clippy run on the bench. Idle sequential
  A/B measurements pinned to one NUMA node cover the original 1M scan, point reads, sparse entities and
  random/repeated ref projections; profiles and timings run separately.
- Claude reviews the implementation before a PR targeting `v3`. Bench scratch
  files are removed after retaining local verification evidence.
