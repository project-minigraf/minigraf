# Minigraf Error Reference

This document covers every user-facing error produced by the core Minigraf Rust library.
Errors surface as `anyhow::Error` values returned from `db.execute()`, `db.prepare()`,
and related API methods.

**Reference codes** (e.g. `PRS-001`) are documentation-only identifiers. They do not
appear in runtime output today — runtime codes are tracked in
[#277](https://github.com/project-minigraf/minigraf/issues/277).

**Out of scope**: FFI/binding errors from `minigraf-python`, `minigraf-node`, and
`minigraf-wasm` are handled in those repos.

---

## Quick Reference Table

| Code | Error (prefix) | Category |
|------|----------------|----------|
<!-- filled in Task 12 -->

---

## PRS — Parser Errors

Parser errors occur when Minigraf cannot parse the Datalog/EDN input string.
They are returned immediately from `db.execute()` before any fact is read or written.

See the [Datalog Reference](../../.wiki/Datalog-Reference.md) for syntax guidance.

<!-- entries added in Tasks 3–7 -->

---

## QRY — Query Execution Errors

Query execution errors occur after parsing succeeds, during pattern matching,
predicate evaluation, or fact transacting.

<!-- entries added in Task 8 -->

---

## STG — Storage Errors

Storage errors relate to reading or writing the `.graph` file. They typically
indicate a corrupted, truncated, or incompatible database file.

See the [file format section in README](../README.md#file-format) for version history.

<!-- entries added in Task 9 -->

---

## WAL — Write-Ahead Log Errors

WAL errors relate to the sidecar `.wal` file written alongside the `.graph` file.
The WAL is replayed on open and deleted on checkpoint.

<!-- entries added in Task 10 -->

---

## API — Database API Errors

API errors indicate a violated contract in how the public `Minigraf` or
`WriteTransaction` API is used.

<!-- entries added in Task 11 -->

---

## Appendix: Internal Errors

The following error strings indicate a bug in the Minigraf library itself, not a
user mistake. If you encounter one, please
[open a GitHub issue](https://github.com/project-minigraf/minigraf/issues/new)
with the full error message and the input that triggered it.

- `internal parser error: expected keyword token`
- `internal parser error: expected symbol token`
- `internal parser error: expected string token`
- `internal parser error: expected integer token`
- `internal parser error: expected float token`
- `internal parser error: expected boolean token`
- `internal parser error: expected tagged literal token`
- `internal parser error: expected bind slot token`
