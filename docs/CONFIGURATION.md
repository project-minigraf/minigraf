# Configuration

Minigraf needs no configuration: `Minigraf::open("app.graph")` uses safe
defaults. `OpenOptions` changes them for one handle. Nothing here is stored
in the `.graph` file; each open sets it.

This page lists every `OpenOptions` setting, where else it can be changed,
and how the embedder controls which settings query text may override
(`allow_overrides`, #862).

## Settings and where they can be changed

"Query text" means keywords in the query vector, such as
`:max-derived-facts 1000`. Query text is gated by
[`allow_overrides`](#allow_overrides). Settings changed from application
code are not gated.

| `OpenOptions` field | Default | Also changed by | Semantics of an override |
|---|---|---|---|
| `max_derived_facts` | 1,000,000 | query text: `:max-derived-facts N` or `$slot`; code: `PreparedQuery::with_max_derived_facts` | Replaces the database value for that query, up or down; a keyword overrides the setter |
| `max_results` (deprecated) | no limit of its own | query text: `:max-results N` or `$slot` | Alone, replaces the database value; with `:max-derived-facts`, the lower applies |
| `query_memory_limit` | no limit | query text: `:query-memory-limit N` or `$slot` (not in `DEFAULTS`); code: `PreparedQuery::with_query_memory_limit` | Replaces the database value for that query, up or down; a keyword overrides the setter |
| `max_iterations` | 1,000,000 rounds per stratum | query text: `:max-iterations N` or `$slot`; code: `PreparedQuery::with_max_iterations` | Replaces the database value for that query, up or down; a keyword overrides the setter |
| `transaction_chunk_facts` | 65,536 | — (per-transaction `TxOptions` planned, #871) | — |
| `synchronous` | `SyncMode::Full` | — (per-transaction `TxOptions` planned, #871) | — |
| `wal_checkpoint_threshold` | 1000 | open only | Database-wide checkpoint policy |
| `page_cache_size` | 256 pages | open only | Shared cache |
| `allow_unlocked` | `false` | open only | Decided when the file is opened |
| `read_only` | `false` | open only | Decided when the file is opened |
| `allow_overrides` | `Overridable::DEFAULTS` | open only | Policy for query text |

`BrowserDb` takes no options: it always runs with the defaults, including
`Overridable::DEFAULTS`.

## `allow_overrides`

`OpenOptions::allow_overrides` lists the query keywords that may override the
database's settings, as `Overridable` variants:

| Variant | Keyword | In `DEFAULTS` |
|---|---|---|
| `Overridable::MaxDerivedFacts` | `:max-derived-facts` | yes |
| `Overridable::MaxResults` | `:max-results` | yes |
| `Overridable::QueryMemoryLimit` | `:query-memory-limit` | no |
| `Overridable::MaxIterations` | `:max-iterations` | yes |

The default, `Overridable::DEFAULTS`, allows every keyword that could
override a setting before the list existed, so existing queries keep
working.

A query that uses a keyword the list leaves out is refused with
[`API-025`](ERROR_REFERENCE.md#api-025-query-option-override-not-allowed)
before it runs. The check depends only on whether the keyword is present,
never on its value, and the override is never ignored or clamped. It applies
to `Minigraf::execute`, `Minigraf::query`, `Minigraf::prepare` (a refused
query yields no `PreparedQuery`) and `WriteTransaction::execute` (a refused
query leaves the transaction usable).

The list **replaces** the defaults; it is not merged with them. Duplicates
have no effect.

```rust
use minigraf::{OpenOptions, Overridable};

// Query text may not override any limit, e.g. when it comes from users or an LLM.
let db = OpenOptions::new()
    .allow_overrides([])
    .path("app.graph")
    .open()?;

// Only :max-results; :max-derived-facts is refused with API-025.
let db = OpenOptions::new()
    .allow_overrides([Overridable::MaxResults])
    .open_memory()?;

// The defaults plus another variant (for keywords added later).
let opts = OpenOptions::new().allow_overrides(
    Overridable::DEFAULTS.iter().copied().chain([/* Overridable::… */]),
);
```

`Overridable` is `#[non_exhaustive]`: later releases add variants for new
keywords. `MaxIterations` (#861) joined `DEFAULTS` in v3.0.0, so its keyword
works without configuration.

`QueryMemoryLimit` is left out of `DEFAULTS` because the keyword can raise the
limit that keeps a query from exhausting memory. List it to let query text
change the limit:

```rust
use minigraf::{OpenOptions, Overridable};

let db = OpenOptions::new()
    .query_memory_limit(64 << 20)
    .allow_overrides(Overridable::DEFAULTS.iter().copied().chain([Overridable::QueryMemoryLimit]))
    .open_memory()?;
db.execute("(query [:find ?x :where [?e :v ?x] :query-memory-limit 1048576])")?;
```

## Limit keywords

`:max-derived-facts N`, `:max-results N`, `:max-iterations N` and
`:query-memory-limit N` take an integer or a `$slot` that a prepared query fills with
`BindValue::Val(Value::Integer(n))`:

| Keyword | Range | Out of range | Other value |
|---|---|---|---|
| `:max-derived-facts`, `:max-results`, `:max-iterations` | `n >= 1` | `INT-014` | `INT-015` (literal), `API-024` (bind value) |
| `:query-memory-limit` | `n >= 0` (bytes) | `INT-015` | `INT-015` (literal), `API-024` (bind value) |

A missing bind value is `API-022`. `Minigraf::execute` and `Minigraf::query`
refuse a query with an unbound slot (`API-010`).

`:query-memory-limit 0` lets a query run only if it holds no state: no hash
join, aggregate, sort, distinct set or rule relation, and no entity ids to
resolve. Uncheckpointed facts and returned entities need ids, so in practice
only a query over checkpointed facts that returns values runs under 0; any
other query fails at once with `QRY-012`.

`max_iterations` counts the rounds of each stratum of a rule query
separately, not their total. A stratum starts another round only after
deriving a new fact, so its rounds never exceed the facts it derives plus one:
at the defaults (both 1,000,000) `max_derived_facts` is reached first, and
`max_iterations` matters only when lowered. `0` fails every rule query with
`INT-020`.

## `PreparedQuery` setters

`PreparedQuery::with_max_derived_facts(n)`,
`PreparedQuery::with_max_iterations(n)` and
`PreparedQuery::with_query_memory_limit(bytes)` replace the database values
that were copied when the query was prepared, for that statement. They are
application code, so `allow_overrides` does not gate them. An allowed
keyword in the prepared text, literal or slot, still overrides the setter.
`usize::MAX` means no limit.

```rust
let pq = db
    .prepare("(query [:find ?x ?y :where (reach ?x ?y)])")?
    .with_max_derived_facts(10_000)
    .with_query_memory_limit(16 << 20);
let rows = pq.execute(&[])?;
```
