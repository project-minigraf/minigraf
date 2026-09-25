//! Structured runtime error codes surfaced through `MinigrafError`.
//!
//! See `docs/superpowers/specs/2026-08-25-structured-error-codes-design.md`
//! and `docs/ERROR_REFERENCE.md` for the design and the full code registry.

use std::fmt;

/// Coarse-grained error category — the type library consumers match on.
///
/// `#[non_exhaustive]`: a future category can be added without that being a
/// breaking change. Downstream `match` expressions must include a wildcard
/// `_` arm. Has no effect within this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorCategory {
    /// Datalog/EDN syntax errors (`PRS-` codes).
    Parser,
    /// Query planning/execution errors (`QRY-` codes).
    Query,
    /// On-disk storage/file-format errors (`STG-` codes).
    Storage,
    /// Write-ahead log errors (`WAL-` codes).
    Wal,
    /// Public `Minigraf` API misuse or internal sequencing errors (`API-` codes).
    Api,
    /// Unclassified internal invariant violations (`INT-` codes).
    Internal,
}

/// Crate-internal handle for one documented `docs/ERROR_REFERENCE.md` code.
///
/// Never exposed to library consumers — only the `&'static str` code string
/// it resolves to via [`REGISTRY`] is public (through [`MinigrafError::code`]).
/// Kept internal so this enum is free to be reorganized without breaking any
/// downstream `match`.
///
/// Deliberately does NOT derive `Debug`: this enum's variant count tracks
/// every documented error code (186, as of #277's final API+INT category PR
/// — 130 originally documented `PRS`/`QRY`/`STG`/`WAL`/`API` codes plus 55
/// `INT-0xx` codes assigned to the crate's remaining previously-uncoded
/// call sites, plus the original `INT-000` catch-all), and a derived
/// `Debug` impl generates a per-variant string-literal match arm that's
/// pure binary-size cost with no runtime use anywhere in this crate
/// (nothing formats an `ErrorCode` with `{:?}` — [`CodedError`]'s own
/// hand-written `Debug` below only needs the already-formatted `message`
/// and the code string, not this enum). Keeps the project's binary
/// size goal (see PHILOSOPHY.md) affordable as the registry grows.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorCode {
    Prs001,
    Prs002,
    Prs003,
    Prs004,
    Prs005,
    Prs006,
    Prs007,
    Prs008,
    Prs009,
    Prs010,
    Prs011,
    Prs012,
    Prs013,
    Prs014,
    Prs015,
    Prs016,
    Prs017,
    Prs018,
    Prs019,
    Prs020,
    Prs021,
    Prs022,
    Prs023,
    Prs024,
    Prs025,
    Prs026,
    Prs027,
    Prs028,
    Prs029,
    Prs030,
    Prs031,
    Prs032,
    Prs033,
    Prs034,
    Prs035,
    Prs036,
    Prs037,
    Prs038,
    Prs039,
    Prs040,
    Prs041,
    Prs042,
    Prs043,
    Prs044,
    Prs045,
    Prs046,
    Prs047,
    Prs048,
    Prs049,
    Prs050,
    Prs051,
    Prs052,
    Prs053,
    Prs054,
    Prs055,
    Prs056,
    Prs057,
    Prs058,
    Prs059,
    Prs060,
    Prs061,
    Prs062,
    Prs063,
    Prs064,
    Prs065,
    Prs066,
    Prs067,
    Prs068,
    Prs069,
    Prs070,
    Prs071,
    Prs072,
    Prs073,
    Prs074,
    Prs075,
    Prs076,
    Prs077,
    Prs078,
    Prs079,
    Qry001,
    Qry002,
    Qry003,
    Qry004,
    Qry005,
    Qry006,
    Qry007,
    Qry008,
    Qry009,
    Int000,
    Stg001,
    Stg002,
    Stg003,
    Stg004,
    Stg005,
    Stg006,
    Stg007,
    Stg008,
    Stg009,
    Stg010,
    Stg011,
    Stg012,
    Stg013,
    Stg014,
    Stg015,
    Stg016,
    Stg017,
    Stg018,
    Stg019,
    Stg020,
    Stg021,
    Stg022,
    Stg023,
    Stg024,
    Stg025,
    Stg026,
    Stg027,
    Wal001,
    Wal002,
    Wal003,
    Wal004,
    Wal005,
    Wal006,
    Api001,
    Api002,
    Api003,
    Api004,
    Api005,
    Api006,
    Api007,
    Api008,
    Api009,
    Int001,
    Int002,
    Int003,
    Int004,
    Int005,
    Int006,
    Int007,
    Int008,
    Int009,
    Int010,
    Int011,
    Int012,
    Int013,
    Int014,
    Int015,
    Int016,
    Int017,
    Int018,
    Int019,
    Int020,
    Int021,
    Int022,
    Int023,
    Int024,
    Int025,
    Int026,
    Int027,
    Int028,
    Int029,
    Int030,
    Int031,
    Int032,
    Int033,
    Int034,
    Int035,
    Int036,
    Int037,
    Int038,
    Int039,
    Int040,
    Int041,
    Int042,
    Int043,
    Int044,
    Int045,
    Int046,
    Int047,
    Int048,
    Int049,
    Int050,
    Int051,
    Int052,
    Int053,
    Int054,
    Int055,
}

/// Single source of truth: (code, code string, message template, category).
///
/// The message template is verified against `docs/ERROR_REFERENCE.md`'s
/// "Error text" lines by the sync test in this module.
pub(crate) const REGISTRY: &[(ErrorCode, &str, &str, ErrorCategory)] = &[
    (
        ErrorCode::Prs001,
        "PRS-001",
        "Unexpected end of input",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs002,
        "PRS-002",
        "Unexpected character: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs003,
        "PRS-003",
        "Unexpected token: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs004,
        "PRS-004",
        "Unclosed vector",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs005,
        "PRS-005",
        "Unclosed list",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs006,
        "PRS-006",
        "Unterminated map: missing '}'",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs007,
        "PRS-007",
        "String exceeds maximum length of {} bytes",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs008,
        "PRS-008",
        "Keyword exceeds maximum length of {} bytes",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs009,
        "PRS-009",
        "Tagged literal exceeds maximum length of {} bytes",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs010,
        "PRS-010",
        "Expected command symbol",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs011,
        "PRS-011",
        "Unknown command: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs012,
        "PRS-012",
        "Expected a list starting with a command symbol",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs013,
        "PRS-013",
        "Query requires a map argument",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs014,
        "PRS-014",
        ":as-of requires a value",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs015,
        "PRS-015",
        ":as-of counter must be non-negative, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs016,
        "PRS-016",
        ":as-of must be an integer (counter) or ISO 8601 string, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs017,
        "PRS-017",
        ":valid-at requires a value",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs018,
        "PRS-018",
        ":valid-at must be an ISO 8601 string or :any-valid-time, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs019,
        "PRS-019",
        ":valid-from must be an ISO 8601 string, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs020,
        "PRS-020",
        ":valid-to must be an ISO 8601 string, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs021,
        "PRS-021",
        "':with' clause requires at least one aggregate in :find",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs022,
        "PRS-022",
        "':with' variable {} not bound in :where",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs023,
        "PRS-023",
        "Aggregate variable {} not bound in :where",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs024,
        "PRS-024",
        "Aggregate expression must have exactly 2 elements (func ?var), got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs025,
        "PRS-025",
        "Aggregate function name must be a symbol, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs026,
        "PRS-026",
        "Aggregate argument must be a variable (starting with ?)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs027,
        "PRS-027",
        "'{}' is a window function and requires an ':over (...)' clause",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs028,
        "PRS-028",
        "window expression cannot be empty",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs029,
        "PRS-029",
        "window function name must be a symbol",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs030,
        "PRS-030",
        "'{}' is not supported in this version; lag/lead are planned for a future release",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs031,
        "PRS-031",
        "'{}' is not window-compatible and cannot be used with ':over'",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs032,
        "PRS-032",
        "'{}' requires a variable argument (starting with ?) before ':over'",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs033,
        "PRS-033",
        "'{}' requires ':over' after the variable argument",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs034,
        "PRS-034",
        "'{}' requires ':over' immediately after the function name (no variable argument)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs035,
        "PRS-035",
        "':over' must be followed by a list, e.g., (:order-by ?var)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs036,
        "PRS-036",
        "unexpected tokens after ':over' clause in window expression",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs037,
        "PRS-037",
        "':partition-by' requires a variable (starting with ?)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs038,
        "PRS-038",
        "':order-by' requires a variable (starting with ?)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs039,
        "PRS-039",
        "unknown option in ':over' clause: '{}'",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs040,
        "PRS-040",
        "unexpected element in ':over' clause: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs041,
        "PRS-041",
        "Transact requires a vector of facts",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs042,
        "PRS-042",
        "Transact argument must be a vector of facts",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs043,
        "PRS-043",
        "Retract requires a vector of facts",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs044,
        "PRS-044",
        "Retract argument must be a vector of facts",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs045,
        "PRS-045",
        "Each fact must be a vector [e a v] or [e a v {opts}]",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs046,
        "PRS-046",
        "Fact must have at least 3 elements (E A V), got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs047,
        "PRS-047",
        "Optional 4th element of a fact must be a map {:valid-from ... :valid-to ...}, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs048,
        "PRS-048",
        "Transact with options requires a facts vector after the map",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs049,
        "PRS-049",
        "unexpected end of fact vector",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs050,
        "PRS-050",
        "Empty list in :where clause",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs051,
        "PRS-051",
        "(not ...) cannot appear inside another (not ...)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs052,
        "PRS-052",
        "(not) requires at least one clause",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs053,
        "PRS-053",
        "(or) requires at least one branch",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs054,
        "PRS-054",
        "(or-join) requires a join-vars vector and at least one branch",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs055,
        "PRS-055",
        "(or-join) first argument must be a vector of join variables",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs056,
        "PRS-056",
        "(or-join) join variables must be logic variables, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs057,
        "PRS-057",
        "all branches of (or ...) must introduce the same set of new variables",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs058,
        "PRS-058",
        "(and) inside or/or-join requires at least one clause",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs059,
        "PRS-059",
        "(not-join) requires a join-vars vector and at least one clause",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs060,
        "PRS-060",
        "Expected pattern vector or rule invocation in :where clause, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs061,
        "PRS-061",
        "Unexpected element in query: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs062,
        "PRS-062",
        "expression list cannot be empty",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs063,
        "PRS-063",
        "expression head must be a symbol, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs064,
        "PRS-064",
        "{} takes exactly 1 argument",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs065,
        "PRS-065",
        "{} takes exactly 2 arguments",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs066,
        "PRS-066",
        "matches? second argument must be a string literal",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs067,
        "PRS-067",
        "unknown expression operator: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs068,
        "PRS-068",
        "expression clause must be [(expr)] or [(expr) ?out], got {} elements",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs069,
        "PRS-069",
        "expression output must be a ?variable, got {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs070,
        "PRS-070",
        "unsupported expression argument: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs071,
        "PRS-071",
        "Expected UUID string after #uuid tag",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs072,
        "PRS-072",
        "Invalid UUID",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs073,
        "PRS-073",
        "Unknown tagged literal: #{}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs074,
        "PRS-074",
        "Bind slot name exceeds maximum length of {} bytes",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs075,
        "PRS-075",
        "transact takes (transact [facts]) or (transact {opts} [facts]); found {} unexpected trailing argument(s) — the valid-time options map must come BEFORE the facts vector, not after",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs076,
        "PRS-076",
        "retract takes (retract [facts]); found {} unexpected trailing argument(s)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs077,
        "PRS-077",
        "unexpected trailing input after a complete form: {}",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs078,
        "PRS-078",
        "query takes (query [...]); found {} unexpected trailing argument(s)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Prs079,
        "PRS-079",
        "rule takes (rule [...]); found {} unexpected trailing argument(s)",
        ErrorCategory::Parser,
    ),
    (
        ErrorCode::Qry001,
        "QRY-001",
        "Invalid entity: {}",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry002,
        "QRY-002",
        "Attribute must be a keyword",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry003,
        "QRY-003",
        "Cannot transact a pseudo-attribute",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry004,
        "QRY-004",
        "Invalid value: {}",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry005,
        "QRY-005",
        "Transaction failed: {}",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry006,
        "QRY-006",
        "Retraction failed: {}",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry007,
        "QRY-007",
        "unknown predicate: '{}'",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry008,
        "QRY-008",
        "functions lock poisoned",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Qry009,
        "QRY-009",
        "rules lock poisoned",
        ErrorCategory::Query,
    ),
    (
        ErrorCode::Int000,
        "INT-000",
        "unclassified internal error: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Stg001,
        "STG-001",
        "Invalid header: too short (got {} bytes, need 64)",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg002,
        "STG-002",
        "Invalid magic number: not a .graph file",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg003,
        "STG-003",
        "Invalid v4/v5/v6 header: expected at least 72 bytes, got {}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg004,
        "STG-004",
        "Invalid v6 header: expected 80 bytes, got {}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg005,
        "STG-005",
        "Invalid v7 header: expected 84 bytes, got {}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg006,
        "STG-006",
        "Unsupported format version: {} (supported: 1-{})",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg007,
        "STG-007",
        "page_count must be greater than 0",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg008,
        "STG-008",
        "eavt_root_page ({}) must be less than page_count ({})",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg009,
        "STG-009",
        "fact_page_count ({}) cannot exceed page_count ({})",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg010,
        "STG-010",
        "Failed to read header from existing file: {}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg011,
        "STG-011",
        "internal page has no children",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg012,
        "STG-012",
        "Expected index page at page {}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg013,
        "STG-013",
        "range_scan: expected leaf at page_id={}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg014,
        "STG-014",
        "Expected packed page (0x02), got 0x{}",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg015,
        "STG-015",
        "Record at slot {} extends beyond page boundary",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg016,
        "STG-016",
        "backend mutex poisoned",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg017,
        "STG-017",
        "page count overflow computing index_start",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg018,
        "STG-018",
        "page count overflow computing next_free",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg019,
        "STG-019",
        "page count overflow computing new_fact_start",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg020,
        "STG-020",
        "fact index {} exceeds u16::MAX",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg021,
        "STG-021",
        "page id overflow in checksum computation",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg022,
        "STG-022",
        "page id overflow writing fact pages",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg023,
        "STG-023",
        "page index {} exceeds u64::MAX",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg024,
        "STG-024",
        "pending fact count exceeds u64::MAX",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg025,
        "STG-025",
        "Database is already open in this process ({}). A second handle on one file would give each its own page table and corrupt both — reuse the existing handle instead. `Minigraf` is cheap to clone and all clones share the same database.",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg026,
        "STG-026",
        "Database is locked by another process ({}). The lock is held on the file itself and is released automatically when the holding process exits, so there is no lock file to clean up.",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Stg027,
        "STG-027",
        "Failed to lock database at {}: {}. This filesystem does not support file locking (common on NFSv3 without lockd, and on some FUSE mounts). Set `allow_unlocked` in `OpenOptions` to open anyway — that accepts the risk that concurrent writers corrupt the file.",
        ErrorCategory::Storage,
    ),
    (
        ErrorCode::Wal001,
        "WAL-001",
        "Invalid WAL magic number: not a .wal file",
        ErrorCategory::Wal,
    ),
    (
        ErrorCode::Wal002,
        "WAL-002",
        "Unsupported WAL version: {} (expected {})",
        ErrorCategory::Wal,
    ),
    (
        ErrorCode::Wal003,
        "WAL-003",
        "Fact serialised size {} bytes exceeds maximum {} bytes. Store large payloads externally and reference them with a Value::String URL/path or Value::Ref entity ID.",
        ErrorCategory::Wal,
    ),
    (
        ErrorCode::Wal004,
        "WAL-004",
        "fact serialised size {} exceeds u32 range",
        ErrorCategory::Wal,
    ),
    (
        ErrorCode::Wal005,
        "WAL-005",
        "WAL num_facts exceeds platform usize",
        ErrorCategory::Wal,
    ),
    (
        ErrorCode::Wal006,
        "WAL-006",
        "failed to delete WAL file {}: {}",
        ErrorCategory::Wal,
    ),
    (
        ErrorCode::Api001,
        "API-001",
        "write lock is poisoned; database may be in an inconsistent state",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api002,
        "API-002",
        "unexpected command variant in write path",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api003,
        "API-003",
        "attribute must be a keyword",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api004,
        "API-004",
        "cannot transact a pseudo-attribute",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api005,
        "API-005",
        "only (query ...) commands can be prepared; got transact",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api006,
        "API-006",
        "only (query ...) commands can be prepared; got retract",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api007,
        "API-007",
        "only (query ...) commands can be prepared; got rule",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api008,
        "API-008",
        "function registry lock poisoned: PoisonError { .. }",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Api009,
        "API-009",
        "WAL not initialized",
        ErrorCategory::Api,
    ),
    (
        ErrorCode::Int001,
        "INT-001",
        "a WriteTransaction is already in progress on this thread; use tx.execute() instead",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int002,
        "INT-002",
        "invalid entity: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int003,
        "INT-003",
        "invalid value: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int004,
        "INT-004",
        "invalid timestamp: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int005,
        "INT-005",
        "millisecond value {} is outside the supported datetime range",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int006,
        "INT-006",
        "internal parser error: expected {} token",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int007,
        "INT-007",
        "Float literal out of range: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int008,
        "INT-008",
        "Integer literal out of range: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int009,
        "INT-009",
        "Symbol exceeds maximum length of {} bytes",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int010,
        "INT-010",
        "Exceeded maximum recursion depth of {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int011,
        "INT-011",
        "unexpected end of {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int012,
        "INT-012",
        "duplicate {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int013,
        "INT-013",
        "{} requires a value",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int014,
        "INT-014",
        "{} must be >= 1",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int015,
        "INT-015",
        "{} must be a positive integer",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int016,
        "INT-016",
        "(or)/(or-join) cannot appear inside (not)/(not-join)",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int017,
        "INT-017",
        "pseudo-attribute {} is not valid in {} position",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int018,
        "INT-018",
        "{} {} in {} is not bound by any {} clause",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int019,
        "INT-019",
        "datalog parser: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int020,
        "INT-020",
        "evaluator: iteration or result limit exceeded: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int021,
        "INT-021",
        "evaluator: unsupported where-clause in evaluate_rule: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int022,
        "INT-022",
        "datalog evaluator: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int023,
        "INT-023",
        "packed page: fact record exceeds slot capacity: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int024,
        "INT-024",
        "packed page: malformed page data: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int025,
        "INT-025",
        "internal: unsubstituted :valid-at bind slot reached the executor",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int026,
        "INT-026",
        "temporal pseudo-attributes :db/valid-from, :db/valid-to, :db/tx-count, and :db/tx-id require :any-valid-time; add :any-valid-time to your query",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int027,
        "INT-027",
        "query has no :where clause, rules, or aggregates — nothing binds the variables. Add a :where clause (e.g., [:find ?e ?a ?v :where [?e ?a ?v]]) or use an aggregate.",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int028,
        "INT-028",
        "Rule invocation '{}' must have 1 or 2 arguments, got {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int029,
        "INT-029",
        "unknown aggregate function: '{}'",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int030,
        "INT-030",
        "unknown window function '{}' — register it with register_aggregate() before querying",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int031,
        "INT-031",
        "or-join variable {} is not bound in the incoming scope",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int032,
        "INT-032",
        "query executor: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int033,
        "INT-033",
        "missing bind value for slot '${}'",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int034,
        "INT-034",
        "bind slot '${}' is not permitted in attribute position; the query optimizer selects an index based on the attribute at prepare time and cannot handle a parameterised attribute",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int035,
        "INT-035",
        "slot '${}' in {} position requires {}, got {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int036,
        "INT-036",
        "header too short: need {}..{}, got {} bytes",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int037,
        "INT-037",
        "header: slice at {} not exactly {} bytes",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int038,
        "INT-038",
        "header too short for {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int039,
        "INT-039",
        "Invalid magic number",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int040,
        "INT-040",
        "{} '{}' is already registered",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int041,
        "INT-041",
        "sum: expected Integer, Float, or Null, got {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int042,
        "INT-042",
        "min/max: no non-null values in group",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int043,
        "INT-043",
        "{}: cannot compare {} and {} values",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int044,
        "INT-044",
        "{}: expected Integer, Float, String, or Null, got {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int045,
        "INT-045",
        "pending fact index {} out of bounds",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int046,
        "INT-046",
        "no CommittedFactReader but got committed FactRef (page_id={})",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int047,
        "INT-047",
        "into_backend: backend Arc has multiple owners",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int048,
        "INT-048",
        "storage: arithmetic overflow: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int049,
        "INT-049",
        "storage: internal invariant violation: {}",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int050,
        "INT-050",
        "{} lock poisoned",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int051,
        "INT-051",
        "Invalid page size: {} bytes (expected {})",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int052,
        "INT-052",
        "Page {} not found",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int053,
        "INT-053",
        "Header checksum mismatch: possible file corruption. Database may be damaged.",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int054,
        "INT-054",
        "unstratifiable: predicate '{}' is involved in a negative cycle through '{}'",
        ErrorCategory::Internal,
    ),
    (
        ErrorCode::Int055,
        "INT-055",
        "rule predicate '{}' disappeared during rollback",
        ErrorCategory::Internal,
    ),
];

pub(crate) fn registry_entry(code: ErrorCode) -> (&'static str, &'static str, ErrorCategory) {
    let entry = REGISTRY.iter().find(|(c, ..)| *c == code);
    // This is NOT exhaustively guaranteed: `registry_is_a_subset_of_error_reference_doc`
    // iterates over `REGISTRY`, not over `ErrorCode` variants, so it cannot catch a new
    // `ErrorCode` variant added without a matching `REGISTRY` row. The `debug_assert!`
    // below catches that mistake in debug/test builds only — nothing catches it in a
    // release build, where it silently falls back to the always-present INT-000 entry
    // below instead of panicking, since `unwrap`/`expect`/`panic!` are denied crate-wide.
    // `error::tests::every_error_code_variant_has_a_registry_entry` is the real
    // exhaustiveness guarantee: its `match` over `ErrorCode` fails to compile if a new
    // variant is added without also being handled there.
    debug_assert!(
        entry.is_some(),
        "every ErrorCode variant must have a REGISTRY entry"
    );
    match entry {
        Some((_, code_str, template, category)) => (*code_str, *template, *category),
        None => (
            "INT-000",
            "unclassified internal error: {}",
            ErrorCategory::Internal,
        ),
    }
}

/// Fill a message template's positional `{}` placeholders from `args`, in order.
///
/// Not `std::fmt`-based: the template is runtime data pulled from [`REGISTRY`],
/// and `format!` requires a compile-time string literal.
pub(crate) fn format_template(template: &str, args: &[&dyn fmt::Display]) -> String {
    let placeholder_count = template.matches("{}").count();
    debug_assert_eq!(
        placeholder_count,
        args.len(),
        "format_template: template has a different placeholder count than args supplied"
    );
    let mut out = String::with_capacity(template.len());
    let mut args = args.iter();
    let mut rest = template;
    while let Some(pos) = rest.find("{}") {
        out.push_str(&rest[..pos]);
        if let Some(arg) = args.next() {
            out.push_str(&arg.to_string());
        }
        rest = &rest[pos + 2..];
    }
    out.push_str(rest);
    out
}

/// An `anyhow`-compatible error carrying a compile-time-checked [`ErrorCode`].
///
/// Built exclusively through [`bail_coded!`]/[`err_coded!`] and wrapped as an
/// `anyhow::Error`, so it rides through the crate's existing `anyhow::Result`
/// plumbing (`?`, `.context()`) unchanged. Recovered at the public API
/// boundary by [`MinigrafError::from`]`(anyhow::Error)` walking the error
/// chain for the first `CodedError` via `downcast_ref`.
pub(crate) struct CodedError {
    code: ErrorCode,
    message: String,
}

impl CodedError {
    /// `#[cold]`/`#[inline(never)]`: `bail_coded!`/`err_coded!` expand to a
    /// call to this at every one of the (roughly 300, now that #277's six
    /// category PRs have all landed) error call sites across the crate.
    /// Error construction is by definition the cold path, so forcing
    /// it out of line keeps the lookup + `format_template` call from being
    /// duplicated inline at each one — pure binary-size cost for a path
    /// that, unlike the argument-array construction immediately at the call
    /// site (which still inlines, and is cheap), has no per-call-site
    /// specialization to gain from inlining.
    #[cold]
    #[inline(never)]
    pub(crate) fn new(code: ErrorCode, args: &[&dyn fmt::Display]) -> Self {
        let (_, template, _) = registry_entry(code);
        CodedError {
            code,
            message: format_template(template, args),
        }
    }

    pub(crate) fn code(&self) -> ErrorCode {
        self.code
    }
}

impl fmt::Display for CodedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

// Hand-written rather than `#[derive(Debug)]`: a derive would require
// `ErrorCode: Debug`, which is deliberately not implemented (see the
// binary-size comment on `ErrorCode`). `std::error::Error` only requires
// `Debug` to exist, not that it echo every field, so this reuses the
// already-formatted `message` plus the stable code string instead.
impl fmt::Debug for CodedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (code_str, ..) = registry_entry(self.code);
        f.debug_struct("CodedError")
            .field("code", &code_str)
            .field("message", &self.message)
            .finish()
    }
}

impl std::error::Error for CodedError {}

/// Return early with a coded, `anyhow`-compatible error.
///
/// `bail_coded!(ErrorCode::Prs001)` for a static message, or
/// `bail_coded!(ErrorCode::Prs002, ch)` to fill the template's `{}`
/// placeholders positionally. A drop-in replacement for `anyhow::bail!`.
macro_rules! bail_coded {
    ($code:expr $(,)?) => {
        return Err(::anyhow::Error::new($crate::error::CodedError::new($code, &[])))
    };
    ($code:expr, $($arg:expr),+ $(,)?) => {
        return Err(::anyhow::Error::new($crate::error::CodedError::new(
            $code,
            &[$(&$arg as &dyn ::std::fmt::Display),+],
        )))
    };
}

/// Build (without returning) a coded, `anyhow`-compatible error.
///
/// For use where an `anyhow::Error` value is needed directly, e.g.
/// `.ok_or_else(|| err_coded!(ErrorCode::Api009))`.
macro_rules! err_coded {
    ($code:expr $(,)?) => {
        ::anyhow::Error::new($crate::error::CodedError::new($code, &[]))
    };
    ($code:expr, $($arg:expr),+ $(,)?) => {
        ::anyhow::Error::new($crate::error::CodedError::new(
            $code,
            &[$(&$arg as &dyn ::std::fmt::Display),+],
        ))
    };
}

pub(crate) use bail_coded;
pub(crate) use err_coded;

/// The error type returned from Minigraf's public API.
///
/// Carries a matchable [`category`](MinigrafError::category) and a stable
/// [`code`](MinigrafError::code) (e.g. `"PRS-001"`) documented in
/// `docs/ERROR_REFERENCE.md`. `Display` renders as `[CODE] message`.
#[derive(Debug)]
pub struct MinigrafError {
    category: ErrorCategory,
    code: &'static str,
    message: String,
    source: anyhow::Error,
}

impl MinigrafError {
    /// The coarse-grained category — the type to `match` on.
    pub fn category(&self) -> ErrorCategory {
        self.category
    }

    /// The stable reference code, e.g. `"PRS-001"`. See `docs/ERROR_REFERENCE.md`.
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for MinigrafError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for MinigrafError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

impl From<anyhow::Error> for MinigrafError {
    fn from(err: anyhow::Error) -> Self {
        for cause in err.chain() {
            if let Some(coded) = cause.downcast_ref::<CodedError>() {
                let (code_str, _, category) = registry_entry(coded.code());
                return MinigrafError {
                    category,
                    code: code_str,
                    message: coded.to_string(),
                    source: err,
                };
            }
        }
        let message_text = err.to_string();
        let (code_str, template, category) = registry_entry(ErrorCode::Int000);
        let message = format_template(template, &[&message_text as &dyn fmt::Display]);
        MinigrafError {
            category,
            code: code_str,
            message,
            source: err,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_template_no_placeholders() {
        assert_eq!(
            format_template("no placeholders here", &[]),
            "no placeholders here"
        );
    }

    #[test]
    fn format_template_one_placeholder() {
        let arg = 42;
        assert_eq!(
            format_template("value: {}", &[&arg as &dyn std::fmt::Display]),
            "value: 42"
        );
    }

    #[test]
    fn format_template_multiple_placeholders() {
        let a = "x";
        let b = 7;
        assert_eq!(
            format_template(
                "{} then {}",
                &[&a as &dyn std::fmt::Display, &b as &dyn std::fmt::Display]
            ),
            "x then 7"
        );
    }

    /// Exhaustively verifies every `ErrorCode` variant resolves to a real
    /// `REGISTRY` entry, not `registry_entry`'s INT-000 fallback: `ALL` is
    /// checked at runtime, and `_assert_all_variants_covered`'s `match` must
    /// list every `ErrorCode` variant explicitly (no `_` arm) so that adding
    /// a new variant without also adding it to both `ALL` and that match is
    /// a compile error — the real exhaustiveness guarantee `registry_entry`'s
    /// `debug_assert!` cannot provide on its own (it's compiled out in
    /// release builds, and `registry_is_a_subset_of_error_reference_doc`
    /// only walks `REGISTRY`, not `ErrorCode`).
    #[test]
    fn every_error_code_variant_has_a_registry_entry() {
        fn assert_has_registry_entry(code: ErrorCode) {
            let (code_str, _, _) = registry_entry(code);
            assert_ne!(
                code_str, "",
                "ErrorCode variant resolved to an empty registry code"
            );
            assert!(
                REGISTRY.iter().any(|(c, ..)| *c == code),
                "ErrorCode variant has no REGISTRY entry"
            );
        }

        // No `_` arm: adding a new `ErrorCode` variant without listing it
        // here is a compile error, which is the actual exhaustiveness
        // guarantee this test provides.
        fn exhaustive(code: ErrorCode) {
            match code {
                ErrorCode::Int000
                | ErrorCode::Prs001
                | ErrorCode::Prs002
                | ErrorCode::Prs003
                | ErrorCode::Prs004
                | ErrorCode::Prs005
                | ErrorCode::Prs006
                | ErrorCode::Prs007
                | ErrorCode::Prs008
                | ErrorCode::Prs009
                | ErrorCode::Prs010
                | ErrorCode::Prs011
                | ErrorCode::Prs012
                | ErrorCode::Prs013
                | ErrorCode::Prs014
                | ErrorCode::Prs015
                | ErrorCode::Prs016
                | ErrorCode::Prs017
                | ErrorCode::Prs018
                | ErrorCode::Prs019
                | ErrorCode::Prs020
                | ErrorCode::Prs021
                | ErrorCode::Prs022
                | ErrorCode::Prs023
                | ErrorCode::Prs024
                | ErrorCode::Prs025
                | ErrorCode::Prs026
                | ErrorCode::Prs027
                | ErrorCode::Prs028
                | ErrorCode::Prs029
                | ErrorCode::Prs030
                | ErrorCode::Prs031
                | ErrorCode::Prs032
                | ErrorCode::Prs033
                | ErrorCode::Prs034
                | ErrorCode::Prs035
                | ErrorCode::Prs036
                | ErrorCode::Prs037
                | ErrorCode::Prs038
                | ErrorCode::Prs039
                | ErrorCode::Prs040
                | ErrorCode::Prs041
                | ErrorCode::Prs042
                | ErrorCode::Prs043
                | ErrorCode::Prs044
                | ErrorCode::Prs045
                | ErrorCode::Prs046
                | ErrorCode::Prs047
                | ErrorCode::Prs048
                | ErrorCode::Prs049
                | ErrorCode::Prs050
                | ErrorCode::Prs051
                | ErrorCode::Prs052
                | ErrorCode::Prs053
                | ErrorCode::Prs054
                | ErrorCode::Prs055
                | ErrorCode::Prs056
                | ErrorCode::Prs057
                | ErrorCode::Prs058
                | ErrorCode::Prs059
                | ErrorCode::Prs060
                | ErrorCode::Prs061
                | ErrorCode::Prs062
                | ErrorCode::Prs063
                | ErrorCode::Prs064
                | ErrorCode::Prs065
                | ErrorCode::Prs066
                | ErrorCode::Prs067
                | ErrorCode::Prs068
                | ErrorCode::Prs069
                | ErrorCode::Prs070
                | ErrorCode::Prs071
                | ErrorCode::Prs072
                | ErrorCode::Prs073
                | ErrorCode::Prs074
                | ErrorCode::Prs075
                | ErrorCode::Prs076
                | ErrorCode::Prs077
                | ErrorCode::Prs078
                | ErrorCode::Prs079
                | ErrorCode::Qry001
                | ErrorCode::Qry002
                | ErrorCode::Qry003
                | ErrorCode::Qry004
                | ErrorCode::Qry005
                | ErrorCode::Qry006
                | ErrorCode::Qry007
                | ErrorCode::Qry008
                | ErrorCode::Qry009
                | ErrorCode::Stg001
                | ErrorCode::Stg002
                | ErrorCode::Stg003
                | ErrorCode::Stg004
                | ErrorCode::Stg005
                | ErrorCode::Stg006
                | ErrorCode::Stg007
                | ErrorCode::Stg008
                | ErrorCode::Stg009
                | ErrorCode::Stg010
                | ErrorCode::Stg011
                | ErrorCode::Stg012
                | ErrorCode::Stg013
                | ErrorCode::Stg014
                | ErrorCode::Stg015
                | ErrorCode::Stg016
                | ErrorCode::Stg017
                | ErrorCode::Stg018
                | ErrorCode::Stg019
                | ErrorCode::Stg020
                | ErrorCode::Stg021
                | ErrorCode::Stg022
                | ErrorCode::Stg023
                | ErrorCode::Stg024
                | ErrorCode::Stg025
                | ErrorCode::Stg026
                | ErrorCode::Stg027
                | ErrorCode::Wal001
                | ErrorCode::Wal002
                | ErrorCode::Wal003
                | ErrorCode::Wal004
                | ErrorCode::Wal005
                | ErrorCode::Wal006
                | ErrorCode::Api001
                | ErrorCode::Api002
                | ErrorCode::Api003
                | ErrorCode::Api004
                | ErrorCode::Api005
                | ErrorCode::Api006
                | ErrorCode::Api007
                | ErrorCode::Api008
                | ErrorCode::Api009
                | ErrorCode::Int001
                | ErrorCode::Int002
                | ErrorCode::Int003
                | ErrorCode::Int004
                | ErrorCode::Int005
                | ErrorCode::Int006
                | ErrorCode::Int007
                | ErrorCode::Int008
                | ErrorCode::Int009
                | ErrorCode::Int010
                | ErrorCode::Int011
                | ErrorCode::Int012
                | ErrorCode::Int013
                | ErrorCode::Int014
                | ErrorCode::Int015
                | ErrorCode::Int016
                | ErrorCode::Int017
                | ErrorCode::Int018
                | ErrorCode::Int019
                | ErrorCode::Int020
                | ErrorCode::Int021
                | ErrorCode::Int022
                | ErrorCode::Int023
                | ErrorCode::Int024
                | ErrorCode::Int025
                | ErrorCode::Int026
                | ErrorCode::Int027
                | ErrorCode::Int028
                | ErrorCode::Int029
                | ErrorCode::Int030
                | ErrorCode::Int031
                | ErrorCode::Int032
                | ErrorCode::Int033
                | ErrorCode::Int034
                | ErrorCode::Int035
                | ErrorCode::Int036
                | ErrorCode::Int037
                | ErrorCode::Int038
                | ErrorCode::Int039
                | ErrorCode::Int040
                | ErrorCode::Int041
                | ErrorCode::Int042
                | ErrorCode::Int043
                | ErrorCode::Int044
                | ErrorCode::Int045
                | ErrorCode::Int046
                | ErrorCode::Int047
                | ErrorCode::Int048
                | ErrorCode::Int049
                | ErrorCode::Int050
                | ErrorCode::Int051
                | ErrorCode::Int052
                | ErrorCode::Int053
                | ErrorCode::Int054
                | ErrorCode::Int055 => assert_has_registry_entry(code),
            }
        }

        for code in [
            ErrorCode::Int000,
            ErrorCode::Prs001,
            ErrorCode::Prs002,
            ErrorCode::Prs003,
            ErrorCode::Prs004,
            ErrorCode::Prs005,
            ErrorCode::Prs006,
            ErrorCode::Prs007,
            ErrorCode::Prs008,
            ErrorCode::Prs009,
            ErrorCode::Prs010,
            ErrorCode::Prs011,
            ErrorCode::Prs012,
            ErrorCode::Prs013,
            ErrorCode::Prs014,
            ErrorCode::Prs015,
            ErrorCode::Prs016,
            ErrorCode::Prs017,
            ErrorCode::Prs018,
            ErrorCode::Prs019,
            ErrorCode::Prs020,
            ErrorCode::Prs021,
            ErrorCode::Prs022,
            ErrorCode::Prs023,
            ErrorCode::Prs024,
            ErrorCode::Prs025,
            ErrorCode::Prs026,
            ErrorCode::Prs027,
            ErrorCode::Prs028,
            ErrorCode::Prs029,
            ErrorCode::Prs030,
            ErrorCode::Prs031,
            ErrorCode::Prs032,
            ErrorCode::Prs033,
            ErrorCode::Prs034,
            ErrorCode::Prs035,
            ErrorCode::Prs036,
            ErrorCode::Prs037,
            ErrorCode::Prs038,
            ErrorCode::Prs039,
            ErrorCode::Prs040,
            ErrorCode::Prs041,
            ErrorCode::Prs042,
            ErrorCode::Prs043,
            ErrorCode::Prs044,
            ErrorCode::Prs045,
            ErrorCode::Prs046,
            ErrorCode::Prs047,
            ErrorCode::Prs048,
            ErrorCode::Prs049,
            ErrorCode::Prs050,
            ErrorCode::Prs051,
            ErrorCode::Prs052,
            ErrorCode::Prs053,
            ErrorCode::Prs054,
            ErrorCode::Prs055,
            ErrorCode::Prs056,
            ErrorCode::Prs057,
            ErrorCode::Prs058,
            ErrorCode::Prs059,
            ErrorCode::Prs060,
            ErrorCode::Prs061,
            ErrorCode::Prs062,
            ErrorCode::Prs063,
            ErrorCode::Prs064,
            ErrorCode::Prs065,
            ErrorCode::Prs066,
            ErrorCode::Prs067,
            ErrorCode::Prs068,
            ErrorCode::Prs069,
            ErrorCode::Prs070,
            ErrorCode::Prs071,
            ErrorCode::Prs072,
            ErrorCode::Prs073,
            ErrorCode::Prs074,
            ErrorCode::Prs075,
            ErrorCode::Prs076,
            ErrorCode::Prs077,
            ErrorCode::Prs078,
            ErrorCode::Prs079,
            ErrorCode::Qry001,
            ErrorCode::Qry002,
            ErrorCode::Qry003,
            ErrorCode::Qry004,
            ErrorCode::Qry005,
            ErrorCode::Qry006,
            ErrorCode::Qry007,
            ErrorCode::Qry008,
            ErrorCode::Qry009,
            ErrorCode::Stg001,
            ErrorCode::Stg002,
            ErrorCode::Stg003,
            ErrorCode::Stg004,
            ErrorCode::Stg005,
            ErrorCode::Stg006,
            ErrorCode::Stg007,
            ErrorCode::Stg008,
            ErrorCode::Stg009,
            ErrorCode::Stg010,
            ErrorCode::Stg011,
            ErrorCode::Stg012,
            ErrorCode::Stg013,
            ErrorCode::Stg014,
            ErrorCode::Stg015,
            ErrorCode::Stg016,
            ErrorCode::Stg017,
            ErrorCode::Stg018,
            ErrorCode::Stg019,
            ErrorCode::Stg020,
            ErrorCode::Stg021,
            ErrorCode::Stg022,
            ErrorCode::Stg023,
            ErrorCode::Stg024,
            ErrorCode::Stg025,
            ErrorCode::Stg026,
            ErrorCode::Stg027,
            ErrorCode::Wal001,
            ErrorCode::Wal002,
            ErrorCode::Wal003,
            ErrorCode::Wal004,
            ErrorCode::Wal005,
            ErrorCode::Wal006,
            ErrorCode::Api001,
            ErrorCode::Api002,
            ErrorCode::Api003,
            ErrorCode::Api004,
            ErrorCode::Api005,
            ErrorCode::Api006,
            ErrorCode::Api007,
            ErrorCode::Api008,
            ErrorCode::Api009,
            ErrorCode::Int001,
            ErrorCode::Int002,
            ErrorCode::Int003,
            ErrorCode::Int004,
            ErrorCode::Int005,
            ErrorCode::Int006,
            ErrorCode::Int007,
            ErrorCode::Int008,
            ErrorCode::Int009,
            ErrorCode::Int010,
            ErrorCode::Int011,
            ErrorCode::Int012,
            ErrorCode::Int013,
            ErrorCode::Int014,
            ErrorCode::Int015,
            ErrorCode::Int016,
            ErrorCode::Int017,
            ErrorCode::Int018,
            ErrorCode::Int019,
            ErrorCode::Int020,
            ErrorCode::Int021,
            ErrorCode::Int022,
            ErrorCode::Int023,
            ErrorCode::Int024,
            ErrorCode::Int025,
            ErrorCode::Int026,
            ErrorCode::Int027,
            ErrorCode::Int028,
            ErrorCode::Int029,
            ErrorCode::Int030,
            ErrorCode::Int031,
            ErrorCode::Int032,
            ErrorCode::Int033,
            ErrorCode::Int034,
            ErrorCode::Int035,
            ErrorCode::Int036,
            ErrorCode::Int037,
            ErrorCode::Int038,
            ErrorCode::Int039,
            ErrorCode::Int040,
            ErrorCode::Int041,
            ErrorCode::Int042,
            ErrorCode::Int043,
            ErrorCode::Int044,
            ErrorCode::Int045,
            ErrorCode::Int046,
            ErrorCode::Int047,
            ErrorCode::Int048,
            ErrorCode::Int049,
            ErrorCode::Int050,
            ErrorCode::Int051,
            ErrorCode::Int052,
            ErrorCode::Int053,
            ErrorCode::Int054,
            ErrorCode::Int055,
        ] {
            exhaustive(code);
        }
    }

    #[test]
    fn registry_entry_resolves_int000() {
        let (code_str, template, category) = registry_entry(ErrorCode::Int000);
        assert_eq!(code_str, "INT-000");
        assert_eq!(template, "unclassified internal error: {}");
        assert_eq!(category, ErrorCategory::Internal);
    }

    #[test]
    fn coded_error_display_uses_template() {
        let e = CodedError::new(ErrorCode::Int000, &[&"boom" as &dyn std::fmt::Display]);
        assert_eq!(e.to_string(), "unclassified internal error: boom");
    }

    #[test]
    fn bail_coded_returns_anyhow_error_wrapping_coded_error() {
        fn inner() -> anyhow::Result<()> {
            bail_coded!(ErrorCode::Int000, "boom");
        }
        let err = inner().unwrap_err();
        let coded = err
            .downcast_ref::<CodedError>()
            .expect("expected a CodedError");
        assert!(
            coded.code() == ErrorCode::Int000,
            "expected ErrorCode::Int000"
        );
        assert_eq!(coded.to_string(), "unclassified internal error: boom");
    }

    #[test]
    fn err_coded_builds_anyhow_error_without_returning() {
        let err = err_coded!(ErrorCode::Int000, "boom");
        let coded = err
            .downcast_ref::<CodedError>()
            .expect("expected a CodedError");
        assert!(
            coded.code() == ErrorCode::Int000,
            "expected ErrorCode::Int000"
        );
    }

    #[test]
    fn minigraf_error_display_format() {
        let anyhow_err = err_coded!(ErrorCode::Int000, "boom");
        let e: MinigrafError = anyhow_err.into();
        assert_eq!(e.to_string(), "[INT-000] unclassified internal error: boom");
        assert_eq!(e.code(), "INT-000");
        assert_eq!(e.category(), ErrorCategory::Internal);
    }

    #[test]
    fn minigraf_error_finds_coded_error_through_context_chain() {
        let anyhow_err = err_coded!(ErrorCode::Int000, "boom").context("extra context");
        let e: MinigrafError = anyhow_err.into();
        assert_eq!(e.code(), "INT-000");
        assert_eq!(e.to_string(), "[INT-000] unclassified internal error: boom");
    }

    #[test]
    fn minigraf_error_finds_coded_error_through_two_context_hops() {
        let anyhow_err = err_coded!(ErrorCode::Int000, "boom")
            .context("first context")
            .context("second context");
        let e: MinigrafError = anyhow_err.into();
        assert_eq!(e.code(), "INT-000");
        assert_eq!(e.to_string(), "[INT-000] unclassified internal error: boom");
    }

    // A genuine arg-count mismatch trips the `debug_assert_eq!` in
    // `format_template` — active under `cargo test`'s debug build, which is
    // what this test exercises. In a release build (`debug_assert!` compiled
    // out) the same call does not panic: the loop below the assert simply
    // drops the text of any `{}` placeholder with no matching arg.
    #[test]
    #[should_panic(expected = "different placeholder count")]
    fn format_template_arg_count_mismatch_debug_asserts() {
        format_template("value: {}", &[]);
    }

    #[test]
    fn minigraf_error_falls_back_to_int000_for_uncoded_anyhow_error() {
        let anyhow_err = anyhow::anyhow!("plain uncoded error");
        let e: MinigrafError = anyhow_err.into();
        assert_eq!(e.code(), "INT-000");
        assert_eq!(e.category(), ErrorCategory::Internal);
        assert_eq!(
            e.to_string(),
            "[INT-000] unclassified internal error: plain uncoded error"
        );
    }

    #[test]
    fn minigraf_error_implements_std_error_source() {
        let anyhow_err = err_coded!(ErrorCode::Int000, "boom");
        let e: MinigrafError = anyhow_err.into();
        let err: &dyn std::error::Error = &e;
        assert!(err.source().is_some());
    }

    /// `docs/ERROR_REFERENCE.md` and `REGISTRY` must now be in full
    /// bidirectional agreement: every `REGISTRY` entry's code+template
    /// appears in the doc verbatim, AND every documented `### CODE` section
    /// with an "Error text" line has a matching `REGISTRY` entry. This is
    /// only safe now that #277's final category PR (API+INT) has audited
    /// and coded every remaining `bail!`/`anyhow!` call site crate-wide —
    /// before that, a documented code with no registry entry just meant
    /// that call site hadn't been migrated yet (see the design spec's
    /// "Directionality note").
    #[test]
    fn registry_matches_error_reference_doc_bidirectionally() {
        let doc = include_str!("../docs/ERROR_REFERENCE.md");
        let mut doc_entries: std::collections::HashMap<String, String> = Default::default();
        let mut lines = doc.lines().peekable();
        while let Some(line) = lines.next() {
            let Some(rest) = line.strip_prefix("### ") else {
                continue;
            };
            let code = rest.split_whitespace().next().unwrap_or("").to_string();
            if !is_error_reference_code(&code) {
                continue;
            }
            let mut error_text = None;
            while let Some(&next_line) = lines.peek() {
                if next_line.starts_with("### ") || next_line.starts_with("## ") {
                    break;
                }
                if let Some(text) = next_line.strip_prefix("**Error text**: `") {
                    error_text = text.strip_suffix('`').map(str::to_string);
                    lines.next();
                    break;
                }
                lines.next();
            }
            if let Some(text) = error_text {
                doc_entries.insert(code, text);
            }
        }

        // Direction 1: every REGISTRY entry's code+template appears in the doc verbatim.
        for (_, code_str, template, _) in REGISTRY {
            match doc_entries.get(*code_str) {
                Some(doc_text) => assert_eq!(
                    doc_text, template,
                    "REGISTRY template for a documented code doesn't match its ERROR_REFERENCE.md Error text"
                ),
                None => panic!("REGISTRY has a code with no ERROR_REFERENCE.md entry at all"),
            }
        }

        // Direction 2: every documented code has a REGISTRY row.
        let registry_codes: std::collections::HashSet<&str> =
            REGISTRY.iter().map(|(_, code_str, ..)| *code_str).collect();
        for doc_code in doc_entries.keys() {
            assert!(
                registry_codes.contains(doc_code.as_str()),
                "ERROR_REFERENCE.md documents a code with no REGISTRY entry"
            );
        }
    }

    fn is_error_reference_code(s: &str) -> bool {
        let bytes = s.as_bytes();
        bytes.len() == 7
            && bytes[..3].iter().all(u8::is_ascii_uppercase)
            && bytes[3] == b'-'
            && bytes[4..].iter().all(u8::is_ascii_digit)
    }
}
