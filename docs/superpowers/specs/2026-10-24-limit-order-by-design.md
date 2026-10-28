# `:limit`, `:offset` and `:order-by` (#306)

**Milestone:** v3.0.0, branch `v3`. No change to the `.graph` format, the WAL format or the public Rust API.
**Follows:** #473 (streaming engine; its `Limit`/`Offset` operators, §6.1 there, and `Sort`, §6.2 there, are built here), #547 (`value_order`).
**Leaves to #641 (v3.next):** skipping `Sort` when an index already returns the rows in order, and sort-grouped `Aggregate`.
**Follow-up, 2026-10-28:** #641 now targets v3.0.0; its approved three-slice scope is recorded in [the input-order design](2026-10-28-641-input-order-design.md). This document retains the original #306 decisions below.
**Status:** approved 2026-10-24; not implemented.

## 1. Decisions

1. **`:order-by` is added** with `:limit` and `:offset`. A `Sort` operator orders the rows; under a `:limit` it keeps only the best `offset + limit` rows.
2. **`:limit` without `:order-by` returns any `n` rows of the answer** and stops pulling from its input once it has them. Operators that hold their whole input before returning a row (`Aggregate`, `Window`, a `HashJoin`'s build side, a hashed `not`/`or` body) still read all of theirs; the early stop saves the rest (§4.1). Which rows is unspecified and may differ between plans. This is the one exception to the rule that every plan returns the same rows as a bag (#594/#596 design): the rows returned are always a sub-bag of that plan-independent answer, with `min(n, |answer|)` rows.
3. **`:offset` without `:order-by` is rejected** at parse (`PRS-116`). Without an order, pages could overlap or miss rows between plans and between writes. A caller reading a whole answer in pieces uses a cursor (`Minigraf::query`).
4. **`:order-by` names `:find` elements**: plain variables, and aggregate or window elements written as they appear in `:find`, so `(count ?e) :desc` orders by that column.

## 2. Terms

- **Answer**: the rows a query returns without `:limit`, `:offset` and `:order-by`, as a bag. Every plan returns the same answer.
- **Column**: one `:find` element of a returned row.
- **Order keys**: the `:order-by` entries, each a column and a direction.
- **Full order**: the order keys, then every column not named by an order key, in `:find` order, ascending. Columns compare by `value_order` (`src/query/datalog/aggregate.rs`, #547): numbers numerically (Integer before Float when equal, `-0.0` before `0.0`, NaN after all numbers), then strings, booleans, refs, keywords, `nil`. Two rows equal under the full order are equal in every column, so the full order decides every position of an ordered result.

## 3. Syntax

```clojure
[:find ?name (count ?e)
 :where [?e :person/name ?name]
 :order-by [(count ?e) :desc ?name]
 :limit 10
 :offset 20]
```

- `:order-by` takes one vector. Each entry is a `:find` element, optionally followed by `:asc` or `:desc` (default `:asc`). The vector is needed because `:asc`/`:desc` are keywords and every other section ends at the next keyword.
- An entry matches the `:find` element it equals as a `FindSpec` (`==`). A variable that is bound but not in `:find` does not match.
- `:limit` and `:offset` each take one integer `>= 0`, or a bind slot `$name`. `:limit 0` returns no rows. `:offset` beyond the answer returns no rows.
- `:order-by`, `:limit` and `:offset` may appear in any order among the other query sections, at most once each.
- They apply to the rows the query returns: after aggregates and windows, and to rule queries. They do not affect `:max-derived-facts`, `:max-results`, rule evaluation or `query_memory_limit`.
- Inside a window's `:over`, `:order-by` is unchanged.

### 3.1 Errors

| Code | When | Raised |
|------|------|--------|
| `INT-012` Duplicate query option (existing) | `:order-by`, `:limit` or `:offset` twice | parse |
| `INT-013` Query option requires a value (existing) | the section ends with no value | parse |
| `PRS-115` (new) `:limit`/`:offset` value is not a non-negative integer | negative, float, string, other | parse; at execute for a bind value `Val(Integer(n))` with `n < 0` |
| `PRS-116` (new) `:offset` without `:order-by` | | parse |
| `PRS-117` (new) `:order-by` entry is not a `:find` element | includes a variable bound in `:where` but not in `:find`, and anything not a variable or list | parse |
| `PRS-118` (new) `:order-by` is not a non-empty vector, or names the same element twice, or has `:asc`/`:desc` without an element before it, or another keyword | | parse |
| `INT-035` Bind value type mismatch (existing) | a `:limit`/`:offset` slot bound to anything but `Val(Integer)` | execute |

The INT codes are reused because `:max-results` and `:max-derived-facts` raise them for the same faults.

Slots: `collect_slot_names` (`prepared.rs`) adds a `:limit`/`:offset` slot, so `PreparedQuery::slot_names` reports it and `execute` rejects it unbound (`API-010`), as other slots are. `substitute` resolves it through a new typed resolver, as `resolve_as_of_slot` resolves `:as-of`: `Val(Integer(n))` with `n >= 0` gives `n`; `Val(Integer(n))` with `n < 0` is `PRS-115`; any other bind value is `INT-035` naming `:limit` or `:offset` and expecting `Val(Integer)`; a missing binding is `INT-033`. Today's `Val` path for value positions (`substitute_edn_value`) is not used for these slots.

The grammar `tests/grammar/grammar.pest` gains `order_by_section`, `limit_section` and `offset_section`. The verifier (`parser/verifier.rs`, #643) gains `:limit`, `:offset`, `:asc` and `:desc` in `TOKENS` (`:order-by` is there already) and the three sections in its query templates. The rejections the grammar accepts get `SEMANTIC` entries with witnesses: `PRS-115` for a negative integer (the grammar's `integer_lit` takes a sign), `PRS-116`, `PRS-117`, and `PRS-118` for a duplicate entry; `INT-012` for a repeated section, as for `:max-results`. Any shape the grammar rejects (`:order-by` not a vector, a leading direction) needs no witness. `KNOWN_DRIFT` stays empty.

## 4. Engine

`DatalogQuery` gains `order_by: Vec<(FindSpec, Direction)>`, `limit: Option<Count>`, `offset: Option<Count>`, where `Count` is a literal `usize` or a slot name. `prepared::substitute` replaces a slot as it replaces `:as-of` slots.

`plan_with` (`src/query/engine/plan.rs`) adds steps after `Aggregate`/`Window`, over the columns in `plan.find`:

- With `:order-by`: `Step::Sort { keys: Vec<(column, desc)>, keep: Option<usize> }` with the full order's keys, where `keep` is `offset + limit` (saturating) when `:limit` is given.
- With `:limit` or `:offset`: `Step::Limit { offset, limit }`.

### 4.1 `Limit`

Pipelined. It drops the first `offset` rows it receives, passes on the next `limit` rows, then closes its child and returns `None`. It holds no state, so it is not metered. After it closes its child, the counters of every operator below show no further rows read (#473 spec §10). The work saved is what the child had not yet read: for a pipelined plan (scans, index nested-loop joins, filters, a correlated body) that is everything past the batch holding row `offset + limit`; below an `Aggregate`, `Window`, `Sort` or a hash join's build side, the whole of that operator's input has been read already. With `limit = 0` it closes its child before pulling.

### 4.2 `Sort`

Holds each input row's cells and the values of its key columns (cells turned into values through `Ctx`, as `Window` does), charged to the query's `Meter` (`QRY-012` over `query_memory_limit`, #549):

- Without `keep`, it holds every row, sorts them by the full order when its input ends, and returns them in batches of `BATCH_ROWS`, releasing each as it is returned. Memory follows the answer.
- With `keep = k`, it holds at most `k` rows in a binary heap whose top is the worst row held; a new row better than the top replaces it. Memory follows `k`, not the answer.

With `keep > 0`, or without `keep`, the input is read in full; only #641 can avoid that. With `keep = 0` (`:limit 0`), `Sort` closes its child without pulling and returns `None`.

### 4.3 Where it is used

`execute`, `Minigraf::query` cursors, `PreparedQuery::execute`/`query`, rule queries and `BrowserDb.query` all run a `Stream`, so all get the three sections. Bindings pass the query text through; none changes.

## 5. Tests

- Parser unit tests: every row of §3.1, every section order, defaults, `:limit 0`.
- Grammar verifier: the three sections in the command templates; `KNOWN_DRIFT` stays empty.
- Engine unit tests: `Limit` closes its child after `n` rows (counters); `Sort` with and without `keep`, ties under the full order, every `value_order` class, `keep = 0`, `QRY-012` in `Sort`.
- `tests/limit_order_test.rs`: ordered queries return the same list with `optimizer::reorder()` on and off (`minigraf::__reorder`); unordered `:limit n` returns `min(n, |answer|)` rows that are a sub-bag of the answer, with reordering on and off; aggregates, windows, rule queries, `:as-of`, prepared slots, cursors.
- **Acceptance (moved from #473):** over 10M facts, `[:find ?e ?v :where [?e :a ?v] :limit 10]` returns in time independent of the fact count, and the scan's `rows_in` is at most one batch past 10; `:order-by [?v] :limit 10` holds memory independent of the fact count (`Sort`'s `bytes_held`). Recorded in the #394 envelope at 100K, 1M and 10M, with peak RSS (shapes `limit` and `top_k`).

## 6. Documentation

`CHANGELOG.md` (Added), `docs/ERROR_REFERENCE.md` (PRS-115–118), `README.md`, `ROADMAP.md`, `CLAUDE.md`, `docs/TEST_COVERAGE.md`; docs site `content/datalog-reference.md` (the three sections, `@since v3.0.0`) and `content/comparison.md`.

## 7. Delivery

Two PRs to `v3`, merge commits; #306 is closed by hand after PR 2.

1. **`:limit`**: parser and grammar for `:limit` (literal and slot), `PRS-115`, `Limit`, the unordered tests and the 10M `limit` acceptance shape.
2. **`:order-by` and `:offset`**: parser and grammar, `PRS-116`–`118`, `Sort` with top-k, the ordered tests and the `top_k` acceptance shape, docs site PR.
