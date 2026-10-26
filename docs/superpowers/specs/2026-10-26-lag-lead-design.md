# Lag and lead window functions (#182)

## Scope and fit

Add two Datalog `:find` window functions on `v3` for v3.0.0, per the maintainer's decision on 2026-10-26. Move #182 from v3.next in the roadmap and release docs. They use the existing `Window` operator and `windows` evaluator. There is no file-format, storage, Rust API, dependency, or configuration change. This fits PHILOSOPHY.md: a compact read-only query operation for embedded temporal data, with bounded semantics and tests.

## Syntax and semantics

`(lag ?value :over (:order-by ?key))` and `(lead ?value :over (:order-by ?key))` accept the same optional `:partition-by ?partition` and `:asc`/`:desc` order modifiers as existing window functions. The value argument and `:order-by` are required. The built-ins require `:over`. The plain `(lag ?value)` and `(lead ?value)` forms continue to resolve as user-registered aggregates, preserving existing calls; without registration they fail with `QRY-010`. No offset or explicit default argument is included in #182; a future proposal can add those without changing the one-argument form.

For each partition, sort rows by the existing total window order: `:order-by` value, then every named variable in `:find` order, then input position (#547 spec §4.1). `lag` returns the value argument from the immediately preceding row; `lead` returns it from the immediately following row. At the first/last row respectively the result is `nil`. A neighboring row whose value is `nil` also yields `nil`. These are physical-row operations: peer groups do not collapse or extend the offset, unlike running aggregates. `:desc` reverses only the primary order key, as for `row-number`; in descending order, the preceding row has a higher or equal primary key.

The output retains input row order, as every existing window does. Multiple windows read the same input rows. In a query with a regular aggregate, `lag`/`lead` operate over the grouped rows and their argument, partition, and order variables must be plain `:find` variables (`PRS-082`). Existing `:as-of`, `:valid-at`, cursors, prepared queries, `:distinct`, outer `:order-by`, `:offset`, and `:limit` apply at their usual stages.

## Errors and compatibility

The parser recognizes `lag`/`lead` as window built-ins. Existing window-shape errors (`PRS-032` missing argument, `PRS-033` malformed argument or missing `:over`, `PRS-084` missing order key, `PRS-080` unbound variable, `PRS-082` unavailable grouped variable) apply. `PRS-030` remains registered for compatibility with consumers matching error codes, but is no longer emitted. The error reference marks it deprecated.

## Validation

Use end-to-end tests with values distinct from order keys, both functions together, unpartitioned and partitioned inputs, ascending/descending order, ties broken by another named variable, `nil` boundaries, singleton partitions, `nil`/mixed-type partition keys, long strings, refs, keywords and sets. Include mixed aggregate/window, `:distinct`, pagination after windows, prepared query and cursor, and a registered plain `lag` aggregate. Check plan-independent output with tied rows. Replace the three old tests that expect `PRS-030`. The pest grammar already accepts the generic window expression, so only its explanatory comment needs updating. Verify the complete Rust suite and binary-size gate. Update the Datalog reference, examples, changelog, roadmap, maintainer context, and coverage documentation.
