# Net-assert on index keys (O(live) committed reads) — Design

**Issue:** #379 (follow-up to #323), tracker #383
**Milestone:** v3.0.0, branch `v3`, file format v8 (no format change)
**Builds on:** `2026-10-05-v8-storage-format-design.md` §5.3, §7

> **Superseded in part by #435** (`2026-10-06-one-window-per-triple-design.md`): net-assert now
> keeps each triple's newest transaction, not the newest assertion per window, so the walk
> stops at the first transaction group of each triple. The per-window rule below is history.

## 1. Problem

Every non-`:as-of` query fetches facts, then applies `net_asserted_facts` (keep an
assertion only if it is the newest in its `(e, a, v, vf, vt)` window and newer than every
retraction of `(e, a, v)`). On v8, a committed read decodes every index entry of the
range and translates it into a `Fact` through the dictionary (UUID, attribute name,
timestamp, maybe a long-value page), including every superseded and retracted record.
The cost is O(history), and the history is thrown away a moment later.

`:as-of N` queries are worse: they never take the selective path and always scan and
translate every fact in the file.

## 2. Idea

v8 keys are ordered `e a v tx↓ vf vt op` (EAVT; AEVT is `a e v …`). The history of one
triple is contiguous and newest first. Net-assert can therefore be decided on the key
bytes while walking, and only surviving entries are decoded and translated:

- Walk the triple's entries in `tx` descending order, one transaction group at a time.
- If a group holds a retraction, its assertions and every older entry of the triple are
  dead. They are passed over one by one; after 8 in a row the walk calls
  `seek(triple ‖ 0xFF)` (`0xFF` sorts after every `tx↓` byte). Short histories, the
  common case, pay no seek; long ones pay O(log) page visits.
- Otherwise each assertion survives if its `(vf, vt)` bytes have not been seen in this
  triple; the first one seen is the newest of its window.
- `:as-of N`: entries newer than `N` are passed over the same way; the seek target is
  `triple ‖ tx↓(N)`, the triple's newest entry at or before `N`.

## 3. Why prefiltering committed facts is exact

The executor still applies `net_asserted_facts` to committed and pending facts together.
Prefiltering is exact because every pending fact has a larger `tx_count` than every
committed one, and net-assert only ever lets a newer record hide an older one:

- A committed assertion hidden by a committed record stays hidden in the union.
- A dropped committed retraction can hide only committed assertions older than it, and
  those are already gone. It cannot hide a pending assertion, which is newer.
- `:as-of N` applies the same argument to the records with `tx_count ≤ N`.

A randomized test checks `net(prefilter(committed) ∪ pending) = net(committed ∪ pending)`
for random histories, with and without `:as-of`.

## 4. Changes

- `keys::triple_len(index, key)`: bytes of the leading `(e, a, v)` (or index-order)
  triple, without decoding the value.
- `CommittedReader::live_facts(scan, as_of)` with `scan` one of all, entity,
  entity + attribute, attribute. The default implementation (history, then `as_of`
  filter, then `net_asserted_facts`) is the reference; `OnDiskReader` overrides it with
  the key walk.
- `FactStorage::get_live_facts(scan, as_of)`: pending facts (filtered by `as_of`) plus
  committed live facts.
- Executor `filter_facts_for_query`: when there is no transaction overlay, the
  `None` and `:as-of N` (counter) cases use live reads, selective when patterns bind
  entities or attributes and a full live scan otherwise. `:as-of` timestamps, which
  have no key form, and the transaction overlay keep their current path.

## 5. Testing

- `triple_len` agrees with a full decode for every value type and index.
- Randomized histories (asserts, retracts, re-asserts, valid-time windows, several
  values per attribute, as-of points): `live_facts` from disk equals the reference for
  every scan kind, before and after reopen.
- Query results with and without the prefilter are identical on the existing suites;
  new point-query tests for retract/re-assert and `:as-of` on committed data.
- A counting backend shows that a point query on an entity with long history reads
  fewer leaves than a history read.

## 6. Philosophy

Aligned: no format change, no API change, no dependency; less work per query.
