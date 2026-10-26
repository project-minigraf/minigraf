# Optional distinct query results (#493)

Status: approved by Claude on 2026-10-26 after two review rounds.

## Scope and compatibility

Keep the existing bag semantics: a nonaggregate query returns one row per binding,
including repeated `:find` tuples. Add an optional bare `:distinct` section flag:

```datalog
(transact [[:e1 :c 1] [:e1 :c 2] [:e2 :c 1]])
(query [:find ?e :where [?e :c ?v]])           ;; three rows
(query [:find ?e :where [?e :c ?v] :distinct]) ;; two rows
```

This follows the maintainer's decision in issue #493. Target `v3`, with an
unreleased v3.0.0 note, following the issue's current milestone. No v2 backport,
version bump, new dependency, storage change, or change to defaults. This aligns
with PHILOSOPHY.md: embedded, simple, self-contained and backwards compatible.

## Syntax

`:distinct` takes no argument and may appear between complete sections in any
section order, like `:any-valid-time`. It terminates the preceding section;
following nonkeyword items do not resume that section (`PRS-061`). Repeating
`:distinct` is `INT-012`, as with `:limit`/`:order-by`. Omitting the flag
means false. It is not a prepared bind slot. Extend both the parser and independent
pest grammar, including the grammar verifier's generated tokens.

## Semantics and execution

Deduplicate by the entire final `:find` tuple only. Variables absent from `:find`
do not participate. Order remains unspecified unless `:order-by` is present.
Use the equality of aggregate groups and `count-distinct` (`JoinKey::group`):
integer and float are different types, equal float zeros collapse, all NaNs are
one value, and a keyword is distinct from a reference to its named entity. Sets
use their existing equality. To keep output independent of encounter order,
the opt-in result uses a canonical `-0.0` for any float zero and `f64::NAN` for
any NaN, including float elements inside returned sets. This also applies to a
unique zero/NaN: canonicalization is part of the opt-in output contract. Other
values retain their original representation. Default bag output and `or` binding
deduplication keep their existing behavior.
Equal long strings must collapse across committed and pending storage.

Evaluate `:where` and rules, then aggregates and windows, then distinct, then
`:order-by`, then `:offset` and `:limit`. Distinct must not change aggregate
inputs, window peers/ranks, or `:with` grouping. Repeated final tuples produced by
different `:with` groups always collapse. Rows collapse exactly when every output
column is equal: even `row-number` repeats across partitions, while equal peer
ranks can repeat within one. Empty input and aggregate zero-row behavior
stay the same. `:max-results`/`:max-derived-facts` remain rule-derivation limits.

Extend the engine's metered `Distinct` operator with an explicit projection mode,
keyed by `plan.find`, after `Builder::finish` and before `Sort`/`Limit`. Binding
mode (`or`) keeps `JoinKey::of`; projection mode uses `JoinKey::group` and
canonicalizes returned floats/sets. Do not deduplicate whole bindings or
individual batches. Its state spans the stream, is charged to
`OpenOptions::query_memory_limit`, reports `QRY-012` when exhausted, appears in
engine operator counters, and is released by the existing cursor lifecycle.
Even with ordered top-k pagination, distinct holds one key per unique output
tuple encountered: unlike `Sort` alone, its memory is not bounded by the limit.

Add an internal default-false `DatalogQuery::distinct` field; the query AST is
private to the crate, so this is not a public Rust API change. Prepared cloning must
preserve it. Execute, cursor, prepared execute/cursor, write transactions, memory,
file, temporal and recursive query paths share the same plan.

## Acceptance and documentation

Regression tests must distinguish bag output from distinct output and cover whole
tuples, cross-batch duplicates, ordering/pagination, aggregates with `:with`, peer
windows, prepared queries, cursors, joins/rules/or, temporal queries, write
transactions, file checkpoint/reopen, typed scalar equality, long strings, sets,
memory limits and engine counter visibility. Exercise plan reordering and both
hashed/correlated subplan modes where available. Inject NaNs via prepared values
or test-only direct facts, since query text cannot spell NaN. Include long values
in chunked write transactions. For the memory bound, prove the bag query succeeds
under the same bound where its distinct equivalent fails with `QRY-012`.
Avoid debug-format assertion messages for values/UUID-containing types.

Synchronize CHANGELOG.md, ROADMAP.md, README.md, CLAUDE.md and
docs/TEST_COVERAGE.md, the generated docs/GRAMMAR.md and affected existing error
descriptions. In a separate minigraf-docs worktree/PR, document bag defaults
for all versions, the existing aggregate workaround on older releases, and gate
the new syntax/semantics from v3.0.0 using version directives. Audit architecture,
comparison and use-case pages and change only claims affected by this feature.
No release is being cut, so do not add a release tag or site.toml release entry.

## Review addendum: retained set payloads

Correction approved by Claude on 2026-10-26: the shared meter helpers currently
count a set's pointer but omit the retained set elements and their strings. This
predates #493 and would undermine its memory-limit contract. Extend `cell_bytes`,
`key_bytes` and `value_bytes` through one shared `set_bytes` estimate (set header,
element values and string payloads, Arc counters, and documented per-element tree
slack). The element's own `Value` size is counted once by `value_bytes`.
Keep existing scalar estimates unchanged. This closes the same hole in distinct,
aggregate and other holders that use these helpers, without changing query answers.
Add a file-backed regression showing a plain scan fits the limit while projected
distinct, aggregate groups and a window retaining the large sets fail with QRY-012.
Follow the existing per-entry logical accounting convention, even for shared Arcs.
Unit checks must exercise each estimator independently, including growth when only
an element's string payload grows, and a lower bound covering all element bytes.
