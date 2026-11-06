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
| `max_derived_facts` | 1,000,000 | query text: `:max-derived-facts N` | Replaces the database value for that query, up or down |
| `max_results` (deprecated) | no limit of its own | query text: `:max-results N` | Alone, replaces the database value; with `:max-derived-facts`, the lower applies |
| `query_memory_limit` | no limit | — (query keyword and `PreparedQuery` setter planned, #870) | — |
| round limit | 1000, fixed | — (`OpenOptions::max_iterations` and `:max-iterations` planned, #861) | — |
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
keywords, such as `:query-memory-limit` (#870, not in `DEFAULTS`) and
`:max-iterations` (#861, in `DEFAULTS`).
