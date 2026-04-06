//! Public `Minigraf` facade with `WriteTransaction` and `OpenOptions`.
//!
//! This module provides the primary user-facing API for Minigraf:
//! - `Minigraf::open()` / `Minigraf::in_memory()` for database creation
//! - `Minigraf::execute()` for implicit (self-contained) transactions
//! - `Minigraf::begin_write()` / `WriteTransaction` for explicit transactions
//! - `Minigraf::checkpoint()` for manual WAL compaction

use crate::graph::types::{Fact, VALID_TIME_FOREVER};

/// Sentinel value used in `materialize_transaction` to signal "no explicit `valid_from`
/// was provided; use the transaction timestamp at commit time."
///
/// `i64::MIN` is chosen because it is not a representable Unix millisecond timestamp
/// in any practical context, avoiding the collision that `0` would have with the Unix
/// epoch (1970-01-01T00:00:00Z), which is a legitimate `valid_from` value.
const VALID_FROM_USE_TX_TIME: i64 = i64::MIN;
use crate::graph::types::Value;
use crate::graph::FactStorage;
use crate::query::datalog::evaluator::DEFAULT_MAX_DERIVED_FACTS;
use crate::query::datalog::evaluator::DEFAULT_MAX_RESULTS;
use crate::query::datalog::executor::DatalogExecutor;
use crate::query::datalog::executor::QueryResult;
use crate::query::datalog::functions::{
    AggImpl, AggregateDesc, FunctionRegistry, PredicateDesc, UdfFinaliseFn, UdfOps, UdfStepFn,
};
use crate::query::datalog::parser::parse_datalog_command;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{AttributeSpec, DatalogCommand, Transaction};
use crate::storage::backend::file::FileBackend;
use crate::storage::backend::MemoryBackend;
use crate::storage::persistent_facts::PersistentFactStorage;
use crate::wal::WalWriter;
use anyhow::{bail, Result};
use std::any::Any;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

// ─── Thread-local reentrant-write detection ─────────────────────────────────

thread_local! {
    /// Set to `true` while a `WriteTransaction` is active on this thread.
    /// Prevents same-thread deadlock when `db.execute()` is called while
    /// a `WriteTransaction` is in progress.
    static WRITE_TX_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn set_write_tx_active(val: bool) {
    WRITE_TX_ACTIVE.with(|f| f.set(val));
}

fn is_write_tx_active() -> bool {
    WRITE_TX_ACTIVE.with(|f| f.get())
}

// ─── OpenOptions ─────────────────────────────────────────────────────────────

/// Configuration options for opening a `Minigraf` database.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Number of WAL entries committed before an automatic checkpoint is triggered.
    ///
    /// Defaults to 1000. Lower values mean more frequent checkpoints (smaller WAL,
    /// more I/O). Higher values mean less frequent checkpoints (larger WAL, less I/O).
    pub wal_checkpoint_threshold: usize,
    /// Number of pages to hold in the LRU page cache. Default: 256 (= 1MB at 4KB pages).
    pub page_cache_size: usize,
    /// Maximum facts that can be derived per recursive rule iteration.
    /// Defaults to 100_000. Use to prevent runaway recursive rules.
    pub max_derived_facts: usize,
    /// Maximum total query results. Defaults to 1_000_000.
    pub max_results: usize,
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            wal_checkpoint_threshold: 1000,
            page_cache_size: 256,
            max_derived_facts: DEFAULT_MAX_DERIVED_FACTS,
            max_results: DEFAULT_MAX_RESULTS,
        }
    }
}

impl OpenOptions {
    /// Create a new `OpenOptions` with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the number of pages to hold in the LRU page cache.
    ///
    /// Each page is 4KB, so the default of 256 pages uses ~1MB of memory.
    pub fn page_cache_size(mut self, size: usize) -> Self {
        self.page_cache_size = size;
        self
    }

    /// Set the maximum facts that can be derived per recursive rule iteration.
    ///
    /// Defaults to 100_000. Use lower values to prevent runaway recursive rules
    /// from consuming excessive memory.
    pub fn max_derived_facts(mut self, n: usize) -> Self {
        self.max_derived_facts = n;
        self
    }

    /// Set the maximum total query results.
    ///
    /// Defaults to 1_000_000. Use lower values to limit result set size.
    pub fn max_results(mut self, n: usize) -> Self {
        self.max_results = n;
        self
    }

    /// Set the path for a file-backed database.
    pub fn path(self, path: impl AsRef<Path>) -> OpenOptionsWithPath {
        OpenOptionsWithPath {
            opts: self,
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Open an in-memory (non-persistent) database.
    ///
    /// Uses the options set on the builder. WAL-related options are ignored.
    pub fn open_memory(self) -> Result<Minigraf> {
        Minigraf::in_memory_with_options(self)
    }
}

/// `OpenOptions` combined with a file path, ready to open.
pub struct OpenOptionsWithPath {
    opts: OpenOptions,
    path: PathBuf,
}

impl OpenOptionsWithPath {
    /// Open or create the file-backed database.
    pub fn open(self) -> Result<Minigraf> {
        Minigraf::open_with_options(self.path, self.opts)
    }
}

// ─── WriteContext ─────────────────────────────────────────────────────────────

/// Internal write context: distinguishes in-memory from file-backed databases.
enum WriteContext {
    /// In-memory database: no WAL, no persistence.
    Memory,
    /// File-backed database: has a WAL sidecar and a persistent storage layer.
    File {
        pfs: PersistentFactStorage<FileBackend>,
        /// WAL writer. `None` after a checkpoint until the next write.
        wal: Option<WalWriter>,
        db_path: PathBuf,
        /// Count of WAL entries written since the last checkpoint (or since open).
        wal_entry_count: usize,
    },
}

// ─── Inner ────────────────────────────────────────────────────────────────────

struct Inner {
    /// The shared in-memory fact store. Cloning is cheap (Arc-based).
    fact_storage: FactStorage,
    /// Shared rule registry, persists across all `execute()` calls.
    rules: Arc<RwLock<RuleRegistry>>,
    /// Function registry for aggregates and window functions.
    /// `RwLock` is used in anticipation of the 7.7b `register_aggregate`/`register_predicate` mutation API.
    functions: Arc<RwLock<FunctionRegistry>>,
    /// Serialises all writes. Holds `WriteContext` which contains the PFS/WAL
    /// for file-backed databases.
    write_lock: Mutex<WriteContext>,
    /// Configuration options.
    options: OpenOptions,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // On clean close, perform a best-effort checkpoint to reduce WAL size.
        // Errors are silently ignored (can't propagate from Drop).
        // Skip if wal_checkpoint_threshold is usize::MAX — that sentinel suppresses
        // all checkpointing (used by benchmarks to keep WAL entries pending).
        if self.options.wal_checkpoint_threshold == usize::MAX {
            return;
        }
        if let Ok(mut ctx) = self.write_lock.lock() {
            let _ = Minigraf::do_checkpoint(&self.fact_storage, &mut ctx);
        }
    }
}

// ─── Fact size validation ─────────────────────────────────────────────────────

/// Validate that every fact in `facts` can fit in a single packed-page slot.
///
/// Called before writing to the WAL so that oversized facts are rejected at
/// insertion time rather than at checkpoint time.  Only invoked for file-backed
/// databases — in-memory databases have no page size constraint.
fn check_fact_sizes(facts: &[Fact]) -> anyhow::Result<()> {
    use crate::storage::packed_pages::MAX_FACT_BYTES;
    for fact in facts {
        let bytes = postcard::to_allocvec(fact)
            .map_err(|e| anyhow::anyhow!("Failed to serialise fact for size check: {}", e))?;
        if bytes.len() > MAX_FACT_BYTES {
            anyhow::bail!(
                "Fact serialised size {} bytes exceeds maximum {} bytes. \
                 Store large payloads externally and reference them with a \
                 Value::String URL/path or Value::Ref entity ID.",
                bytes.len(),
                MAX_FACT_BYTES
            );
        }
    }
    Ok(())
}

// ─── Minigraf ─────────────────────────────────────────────────────────────────

/// The primary embedded graph database handle.
///
/// `Minigraf` is cheap to clone — all clones share the same underlying database.
///
/// # File-backed usage
/// ```no_run
/// use minigraf::db::Minigraf;
///
/// let db = Minigraf::open("mydb.graph").unwrap();
/// db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
/// ```
///
/// # In-memory usage
/// ```
/// use minigraf::db::Minigraf;
///
/// let db = Minigraf::in_memory().unwrap();
/// db.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
/// ```
///
/// # Fact Size Limit (file-backed databases only)
///
/// Each fact persisted to a `.graph` file must serialise to at most
/// [`crate::storage::packed_pages::MAX_FACT_BYTES`] bytes (currently 4 080).
///
/// In practice, `Value::String` content is limited to roughly **3 900–4 000 bytes**
/// depending on entity and attribute name lengths.
///
/// Facts that exceed this limit are rejected at insertion time with a descriptive
/// error. This check does **not** apply to `Minigraf::in_memory()`.
///
/// ## Workarounds for large payloads
///
/// - **External blob reference** — store the payload in a file or object store
///   and record its path, URL, or content-addressed hash as a `Value::String`:
///   ```text
///   (transact [[:doc123 :blob/sha256 "a3f5c9..."]])
///   ```
/// - **Entity decomposition** — split large values across multiple facts using
///   a continuation-entity pattern.
/// - **In-memory database** — `Minigraf::in_memory()` has no fact size limit
///   and is suitable for workloads that do not require persistence.
#[derive(Clone)]
pub struct Minigraf {
    inner: Arc<Inner>,
}

impl Minigraf {
    // ── Constructors ─────────────────────────────────────────────────────────

    /// Open or create a file-backed database with default options.
    ///
    /// A sidecar WAL file (`<path>.wal`) is created alongside the main file.
    /// Any existing WAL from a previous crash is replayed automatically.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_options(path, OpenOptions::default())
    }

    /// Open or create a file-backed database with custom options.
    pub fn open_with_options(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self> {
        let db_path = path.as_ref().to_path_buf();

        // Open the main .graph file
        let backend = FileBackend::open(&db_path)?;
        let pfs = PersistentFactStorage::new(backend, opts.page_cache_size)?;

        // Share the fact storage
        let fact_storage = pfs.storage().clone();

        // Derive WAL path: "<db_path>.wal"
        let wal_path = Self::wal_path_for(&db_path);

        // Replay any existing WAL entries before opening the writer
        let wal_entry_count = Self::replay_wal(&wal_path, &fact_storage, &pfs)?;

        // Open the WAL writer only if the WAL file already exists from a previous session.
        // Otherwise, create it lazily on the first write.
        let wal = if wal_path.exists() {
            Some(WalWriter::open_or_create(&wal_path)?)
        } else {
            None
        };

        let ctx = WriteContext::File {
            pfs,
            wal,
            db_path,
            wal_entry_count,
        };

        Ok(Minigraf {
            inner: Arc::new(Inner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
                write_lock: Mutex::new(ctx),
                options: opts,
            }),
        })
    }

    /// Create an in-memory database (no WAL, no persistence). Suitable for tests and REPL.
    pub fn in_memory() -> Result<Self> {
        Self::in_memory_with_options(OpenOptions::default())
    }

    /// Create an in-memory database with custom options.
    ///
    /// Note: WAL-related options are ignored for in-memory databases.
    pub fn in_memory_with_options(opts: OpenOptions) -> Result<Self> {
        let backend = MemoryBackend::new();
        let pfs = PersistentFactStorage::new(backend, opts.page_cache_size)?;
        let fact_storage = pfs.storage().clone();

        // For in-memory databases we don't need the PFS beyond initialisation;
        // we just use the shared FactStorage directly.
        drop(pfs);

        Ok(Minigraf {
            inner: Arc::new(Inner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                functions: Arc::new(RwLock::new(FunctionRegistry::with_builtins())),
                write_lock: Mutex::new(WriteContext::Memory),
                options: opts,
            }),
        })
    }

    // ── WAL replay helper ────────────────────────────────────────────────────

    /// Replay any WAL entries that are newer than the main file's checkpoint.
    ///
    /// Returns the number of entries replayed (used to seed `wal_entry_count`).
    fn replay_wal(
        wal_path: &Path,
        fact_storage: &FactStorage,
        pfs: &PersistentFactStorage<FileBackend>,
    ) -> Result<usize> {
        if !wal_path.exists() {
            return Ok(0);
        }

        let mut reader = crate::wal::WalReader::open(wal_path)?;
        let entries = reader.read_entries()?;
        let last_checkpointed = pfs.last_checkpointed_tx_count();

        let mut replayed = 0;
        for entry in &entries {
            if entry.tx_count <= last_checkpointed {
                // Already present in the main file; skip.
                continue;
            }
            for fact in &entry.facts {
                let _ = fact_storage.load_fact(fact.clone())?;
            }
            replayed += 1;
        }

        // Re-synchronise tx_counter to the maximum tx_count across all facts
        fact_storage.restore_tx_counter()?;

        Ok(replayed)
    }

    // ── Execute ──────────────────────────────────────────────────────────────

    /// Execute a Datalog command as a self-contained implicit transaction.
    ///
    /// For file-backed databases, the WAL entry is written **before** facts are
    /// applied to the in-memory store. A successful return means the facts are in
    /// both the WAL and the in-memory store; a crash after this call returns will
    /// replay the facts on next open.
    ///
    /// If the WAL write fails, an error is returned and the in-memory store is
    /// left unchanged. The database remains consistent for subsequent in-process
    /// operations.
    ///
    /// Returns `Err` if called from the same thread that holds an active
    /// `WriteTransaction` (use `tx.execute()` instead).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - A `WriteTransaction` is already active on **this thread** (deadlock prevention).
    /// - Parsing fails.
    /// - Execution fails.
    /// - WAL write fails (file-backed databases).
    pub fn execute(&self, input: &str) -> Result<QueryResult> {
        // Detect same-thread reentrant write (would deadlock on the Mutex).
        if is_write_tx_active() {
            bail!(
                "a WriteTransaction is already in progress on this thread; use tx.execute() instead"
            );
        }

        let cmd = parse_datalog_command(input).map_err(|e| anyhow::anyhow!("{}", e))?;

        // Determine if this is a read-only command (query / rule registration).
        let is_write = matches!(
            cmd,
            DatalogCommand::Transact(_) | DatalogCommand::Retract(_)
        );

        if is_write {
            let mut ctx = self.inner.write_lock.lock().map_err(|_| {
                anyhow::anyhow!("write lock is poisoned; database may be in an inconsistent state")
            })?;

            // Handle write commands with correct WAL-first ordering:
            // 1. Materialize facts (no storage mutation yet)
            // 2. Allocate tx_count + tx_id and stamp facts
            // 3. Write WAL entry FIRST — if this fails, FactStorage is unchanged
            // 4. Apply facts to shared FactStorage
            let (stamped, is_retract) = match &cmd {
                DatalogCommand::Transact(tx) => (Minigraf::materialize_transaction(tx)?, false),
                DatalogCommand::Retract(tx) => (Minigraf::materialize_retraction(tx)?, true),
                _ => unreachable!("is_write guarantees Transact or Retract"),
            };

            let tx_count = self.inner.fact_storage.allocate_tx_count();
            let tx_id = crate::graph::types::tx_id_now();

            let stamped: Vec<Fact> = stamped
                .into_iter()
                .map(|mut f| {
                    f.tx_id = tx_id;
                    f.tx_count = tx_count;
                    // Fix valid_from if it was left as the sentinel
                    if f.asserted && f.valid_from == VALID_FROM_USE_TX_TIME {
                        f.valid_from = tx_id as i64;
                    }
                    f
                })
                .collect();

            // For file-backed databases, reject oversized facts before touching the WAL.
            if matches!(*ctx, WriteContext::File { .. }) {
                check_fact_sizes(&stamped)?;
            }

            // Write WAL BEFORE applying to shared FactStorage.
            // If this fails, FactStorage is still unchanged — clean rollback.
            let should_checkpoint = WriteTransaction::wal_write_stamped_batch(
                &mut ctx,
                &self.inner.options,
                tx_count,
                &stamped,
            )?;

            // WAL succeeded — now apply facts to shared FactStorage.
            for fact in &stamped {
                let _ = self.inner.fact_storage.load_fact(fact.clone())?;
            }

            // Trigger auto-checkpoint AFTER facts are in FactStorage so the
            // checkpoint captures the newly written facts.
            if should_checkpoint {
                Minigraf::do_checkpoint(&self.inner.fact_storage, &mut ctx)?;
            }

            // Return the same QueryResult the executor would have returned.
            if is_retract {
                Ok(QueryResult::Retracted(tx_id))
            } else {
                Ok(QueryResult::Transacted(tx_id))
            }
        } else {
            // Read-only: no lock needed
            let mut executor = DatalogExecutor::new_with_rules_and_functions(
                self.inner.fact_storage.clone(),
                self.inner.rules.clone(),
                self.inner.functions.clone(),
            );
            executor.set_limits(
                self.inner.options.max_derived_facts,
                self.inner.options.max_results,
            );
            executor.execute(cmd)
        }
    }

    // ── Explicit transaction ──────────────────────────────────────────────────

    /// Begin an explicit write transaction.
    ///
    /// Acquires the write lock; held until `commit()`, `rollback()`, or drop.
    ///
    /// # Errors
    ///
    /// Returns an error if a `WriteTransaction` is already active on **this thread**.
    pub fn begin_write(&self) -> Result<WriteTransaction<'_>> {
        if is_write_tx_active() {
            bail!(
                "a WriteTransaction is already in progress on this thread; use tx.execute() instead"
            );
        }
        let guard = self.inner.write_lock.lock().map_err(|_| {
            anyhow::anyhow!("write lock is poisoned; database may be in an inconsistent state")
        })?;
        // Set flag only after successfully acquiring the lock
        set_write_tx_active(true);
        Ok(WriteTransaction {
            guard,
            inner: &self.inner,
            pending_facts: Vec::new(),
            committed: false,
        })
    }

    // ── Checkpoint ───────────────────────────────────────────────────────────

    /// Manually trigger a checkpoint: flush all in-memory facts to the main file
    /// and delete the WAL sidecar.
    ///
    /// No-op for in-memory databases.
    pub fn checkpoint(&self) -> Result<()> {
        let mut ctx = self.inner.write_lock.lock().map_err(|_| {
            anyhow::anyhow!("write lock is poisoned; database may be in an inconsistent state")
        })?;
        Self::do_checkpoint(&self.inner.fact_storage, &mut ctx)
    }

    /// Internal checkpoint logic (operates on an already-held write-lock guard).
    fn do_checkpoint(_fact_storage: &FactStorage, ctx: &mut WriteContext) -> Result<()> {
        match ctx {
            WriteContext::Memory => {
                // No-op for in-memory databases.
            }
            WriteContext::File {
                pfs,
                wal,
                db_path,
                wal_entry_count,
            } => {
                // Force a full save even if no new writes since last checkpoint.
                pfs.force_dirty();
                pfs.save()?;

                // Derive WAL path and delete the sidecar.
                let wal_path = Self::wal_path_for(db_path);

                // Close the WAL writer (drop it) before deleting the file.
                *wal = None;

                if wal_path.exists() {
                    WalWriter::delete_file(&wal_path)?;
                }

                // WAL writer will be recreated lazily on the next write.
                *wal_entry_count = 0;
            }
        }
        Ok(())
    }

    /// Parse and plan a query once; bind slots (`$name`) are left unresolved.
    ///
    /// Returns a [`crate::query::datalog::prepared::PreparedQuery`] that can be executed
    /// many times with different bind values via
    /// [`crate::query::datalog::prepared::PreparedQuery::execute`].
    ///
    /// # Errors
    /// - Parse failure.
    /// - A bind slot appears in an attribute position (rejected at prepare time).
    /// - The command is not a `(query ...)` — `transact`, `retract`, and `rule`
    ///   are not preparable.
    pub fn prepare(
        &self,
        query_str: &str,
    ) -> Result<crate::query::datalog::prepared::PreparedQuery> {
        use crate::query::datalog::prepared::prepare_query;

        let cmd = parse_datalog_command(query_str).map_err(|e| anyhow::anyhow!("{}", e))?;

        let query = match cmd {
            DatalogCommand::Query(q) => q,
            DatalogCommand::Transact(_) => {
                anyhow::bail!("only (query ...) commands can be prepared; got transact")
            }
            DatalogCommand::Retract(_) => {
                anyhow::bail!("only (query ...) commands can be prepared; got retract")
            }
            DatalogCommand::Rule(_) => {
                anyhow::bail!("only (query ...) commands can be prepared; got rule")
            }
        };

        prepare_query(
            query,
            self.inner.fact_storage.clone(),
            self.inner.rules.clone(),
            self.inner.functions.clone(),
        )
    }

    /// Returns a clone of the underlying `FactStorage` for use by the REPL.
    ///
    /// Cloning is cheap — `FactStorage` is `Arc`-backed.
    ///
    /// # Warning
    /// This method bypasses the WAL and the write lock. It is intended **only** for
    /// the built-in REPL (`src/main.rs`). External callers should use
    /// [`Minigraf::execute`] or [`Minigraf::begin_write`] to ensure crash safety.
    #[doc(hidden)]
    pub fn inner_fact_storage(&self) -> crate::graph::FactStorage {
        self.inner.fact_storage.clone()
    }

    /// Compute the WAL sidecar path for a given database path.
    fn wal_path_for(db_path: &Path) -> PathBuf {
        let mut p = db_path.to_path_buf();
        let name = p
            .file_name()
            .map(|n| {
                let mut s = n.to_os_string();
                s.push(".wal");
                s
            })
            .unwrap_or_else(|| std::ffi::OsString::from("db.graph.wal"));
        p.set_file_name(name);
        p
    }

    // ── Materialize helpers ───────────────────────────────────────────────────

    /// Convert a `Transaction` into a list of assertion `Fact`s (tx_id and tx_count
    /// are set to 0 as placeholders; they are assigned at commit time).
    fn materialize_transaction(tx: &Transaction) -> Result<Vec<Fact>> {
        use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
        use crate::query::datalog::types::EdnValue;

        let tx_valid_from = tx.valid_from;
        let tx_valid_to = tx.valid_to;
        let mut facts = Vec::new();

        for pattern in &tx.facts {
            let entity = edn_to_entity_id(&pattern.entity)
                .map_err(|e| anyhow::anyhow!("invalid entity: {}", e))?;

            let attr = match &pattern.attribute {
                AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
                AttributeSpec::Real(_) => anyhow::bail!("attribute must be a keyword"),
                AttributeSpec::Pseudo(_) => anyhow::bail!("cannot transact a pseudo-attribute"),
            };

            let value = edn_to_value(&pattern.value)
                .map_err(|e| anyhow::anyhow!("invalid value: {}", e))?;

            let valid_from = pattern
                .valid_from
                .or(tx_valid_from)
                .unwrap_or(VALID_FROM_USE_TX_TIME);
            let valid_to = pattern
                .valid_to
                .or(tx_valid_to)
                .unwrap_or(VALID_TIME_FOREVER);

            facts.push(Fact::with_valid_time(
                entity, attr, value, 0, 0, valid_from, valid_to,
            ));
        }

        Ok(facts)
    }

    /// Convert a `Transaction` into a list of retraction `Fact`s.
    fn materialize_retraction(tx: &Transaction) -> Result<Vec<Fact>> {
        use crate::query::datalog::matcher::{edn_to_entity_id, edn_to_value};
        use crate::query::datalog::types::EdnValue;

        let mut facts = Vec::new();

        for pattern in &tx.facts {
            let entity = edn_to_entity_id(&pattern.entity)
                .map_err(|e| anyhow::anyhow!("invalid entity: {}", e))?;

            let attr = match &pattern.attribute {
                AttributeSpec::Real(EdnValue::Keyword(k)) => k.clone(),
                AttributeSpec::Real(_) => anyhow::bail!("attribute must be a keyword"),
                AttributeSpec::Pseudo(_) => anyhow::bail!("cannot transact a pseudo-attribute"),
            };

            let value = edn_to_value(&pattern.value)
                .map_err(|e| anyhow::anyhow!("invalid value: {}", e))?;

            let mut f = Fact::retract(entity, attr, value, 0);
            f.tx_count = 0;
            facts.push(f);
        }

        Ok(facts)
    }

    // ── UDF registration ─────────────────────────────────────────────────────

    /// Register a custom aggregate function.
    ///
    /// `Acc` is any `Send + 'static` type that serves as the accumulator.
    /// It is type-erased internally. The function is usable in both `:find`
    /// grouping position and `:over` (window) position.
    ///
    /// Returns `Err` if `name` is already registered (built-in or UDF).
    ///
    /// # Example
    /// ```
    /// # use minigraf::db::Minigraf;
    /// # use minigraf::graph::types::Value;
    /// let db = Minigraf::in_memory().unwrap();
    /// db.register_aggregate(
    ///     "mysum",
    ///     || 0i64,
    ///     |acc: &mut i64, v: &Value| { if let Value::Integer(i) = v { *acc += i; } },
    ///     |acc: &i64, _n: usize| Value::Integer(*acc),
    /// ).unwrap();
    /// ```
    pub fn register_aggregate<Acc>(
        &self,
        name: &str,
        init: impl Fn() -> Acc + Send + Sync + 'static,
        step: impl Fn(&mut Acc, &Value) + Send + Sync + 'static,
        finalise: impl Fn(&Acc, usize) -> Value + Send + Sync + 'static,
    ) -> Result<()>
    where
        Acc: Any + Send + 'static,
    {
        let init_boxed: Arc<dyn Fn() -> Box<dyn Any + Send> + Send + Sync> =
            Arc::new(move || Box::new(init()) as Box<dyn Any + Send>);
        let step_boxed: UdfStepFn = Arc::new(move |acc, v| {
            // SAFETY: `init_boxed` always creates `Box<Acc>`, so downcast is infallible.
            step(
                acc.downcast_mut::<Acc>()
                    .expect("UDF accumulator type mismatch"),
                v,
            );
        });
        let finalise_boxed: UdfFinaliseFn = Arc::new(move |acc, n| {
            finalise(
                acc.downcast_ref::<Acc>()
                    .expect("UDF accumulator type mismatch"),
                n,
            )
        });

        let desc = AggregateDesc {
            impl_: AggImpl::Udf(UdfOps {
                init: init_boxed,
                step: step_boxed,
                finalise: finalise_boxed,
            }),
            is_builtin: false,
        };
        self.inner
            .functions
            .write()
            .map_err(|e| anyhow::anyhow!("function registry lock poisoned: {}", e))?
            .register_aggregate_desc(name.to_string(), desc)
    }

    /// Register a custom single-argument filter predicate.
    ///
    /// The predicate is usable in `[(name? ?var)]` `:where` clauses.
    /// Returns `Err` if `name` is already registered (built-in or UDF).
    ///
    /// # Example
    /// ```
    /// # use minigraf::db::Minigraf;
    /// # use minigraf::graph::types::Value;
    /// let db = Minigraf::in_memory().unwrap();
    /// db.register_predicate(
    ///     "email?",
    ///     |v: &Value| matches!(v, Value::String(s) if s.contains('@')),
    /// ).unwrap();
    /// ```
    pub fn register_predicate(
        &self,
        name: &str,
        f: impl Fn(&Value) -> bool + Send + Sync + 'static,
    ) -> Result<()> {
        let desc = PredicateDesc {
            f: Arc::new(f),
            is_builtin: false,
        };
        self.inner
            .functions
            .write()
            .map_err(|e| anyhow::anyhow!("function registry lock poisoned: {}", e))?
            .register_predicate_desc(name.to_string(), desc)
    }
}

// ─── WriteTransaction ─────────────────────────────────────────────────────────

/// An explicit write transaction. Holds the write lock for its lifetime.
///
/// # Usage
/// ```no_run
/// use minigraf::db::Minigraf;
///
/// let db = Minigraf::in_memory().unwrap();
/// let mut tx = db.begin_write().unwrap();
/// tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#).unwrap();
/// tx.execute(r#"(transact [[:alice :person/age 30]])"#).unwrap();
/// tx.commit().unwrap();
/// ```
///
/// Dropping without committing performs an implicit rollback.
pub struct WriteTransaction<'a> {
    guard: MutexGuard<'a, WriteContext>,
    inner: &'a Inner,
    /// Facts buffered in this transaction (not yet committed to FactStorage).
    pending_facts: Vec<Fact>,
    /// Set to `true` after a successful `commit()` to suppress rollback in `Drop`.
    committed: bool,
}

impl<'a> WriteTransaction<'a> {
    /// Execute a Datalog command within this transaction.
    ///
    /// - **Writes** (transact / retract): buffered in-memory; not durable until `commit()`.
    ///   Returns `Ok(QueryResult::Ok)` immediately (not `Transacted`/`Retracted`).
    /// - **Reads** (query): see committed facts in `FactStorage` **plus** all facts
    ///   buffered in this transaction (read-your-own-writes).
    /// - **Rules**: registered immediately into the shared rule registry.
    pub fn execute(&mut self, input: &str) -> Result<QueryResult> {
        let cmd = parse_datalog_command(input).map_err(|e| anyhow::anyhow!("{}", e))?;

        match cmd {
            DatalogCommand::Transact(tx) => {
                let new_facts = Minigraf::materialize_transaction(&tx)?;
                self.pending_facts.extend(new_facts);
                Ok(QueryResult::Ok)
            }
            DatalogCommand::Retract(tx) => {
                let new_facts = Minigraf::materialize_retraction(&tx)?;
                self.pending_facts.extend(new_facts);
                Ok(QueryResult::Ok)
            }
            DatalogCommand::Query(_) | DatalogCommand::Rule(_) => {
                // For queries: build a temporary FactStorage that includes
                // committed facts + buffered pending facts (read-your-own-writes).
                let view = self.build_query_view()?;
                let executor = DatalogExecutor::new_with_rules_and_functions(
                    view,
                    self.inner.rules.clone(),
                    self.inner.functions.clone(),
                );
                executor.execute(cmd)
            }
        }
    }

    /// Commit this transaction atomically.
    ///
    /// All buffered facts are stamped with a single `tx_count` / `tx_id`, then
    /// the WAL entry is written and fsynced **before** any fact is applied to the
    /// shared `FactStorage`.  This guarantees that if the WAL write fails the
    /// database is left completely unchanged (clean rollback — no cleanup needed).
    pub fn commit(mut self) -> Result<()> {
        let facts_to_commit = std::mem::take(&mut self.pending_facts);

        if !facts_to_commit.is_empty() {
            let tx_count = self.inner.fact_storage.allocate_tx_count();
            let tx_id = crate::graph::types::tx_id_now();

            // Stamp facts with tx_id and tx_count
            let stamped: Vec<Fact> = facts_to_commit
                .into_iter()
                .map(|mut f| {
                    f.tx_id = tx_id;
                    f.tx_count = tx_count;
                    // Fix valid_from if it was left as the sentinel (placeholder for "use tx time")
                    if f.valid_from == VALID_FROM_USE_TX_TIME && f.asserted {
                        f.valid_from = tx_id as i64;
                    }
                    f
                })
                .collect();

            // For file-backed databases, reject oversized facts before touching the WAL.
            if matches!(*self.guard, WriteContext::File { .. }) {
                check_fact_sizes(&stamped)?;
            }

            // Write WAL entry FIRST — if this fails, no facts have been applied
            // to shared FactStorage, so the database remains in a clean state.
            let should_checkpoint = Self::wal_write_stamped_batch(
                &mut self.guard,
                &self.inner.options,
                tx_count,
                &stamped,
            )?;

            // WAL succeeded — now apply facts to shared FactStorage.
            for fact in stamped {
                let _ = self.inner.fact_storage.load_fact(fact)?;
            }

            // Trigger auto-checkpoint AFTER facts are in FactStorage so the
            // checkpoint captures the newly written facts.
            if should_checkpoint {
                Minigraf::do_checkpoint(&self.inner.fact_storage, &mut self.guard)?;
            }
        }

        self.committed = true;
        set_write_tx_active(false);
        Ok(())
    }

    /// Write a pre-stamped batch of facts to the WAL.
    ///
    /// Accepts an already-computed `tx_count` and `facts` slice.
    /// Called while the write lock is held.
    ///
    /// Returns `true` if an auto-checkpoint should be triggered.  The caller is
    /// responsible for applying facts to `FactStorage` **before** triggering the
    /// checkpoint, so that the checkpoint captures the newly written facts.
    fn wal_write_stamped_batch(
        ctx: &mut WriteContext,
        opts: &OpenOptions,
        tx_count: u64,
        facts: &[Fact],
    ) -> Result<bool> {
        match ctx {
            WriteContext::Memory => Ok(false),
            WriteContext::File {
                pfs,
                wal,
                db_path,
                wal_entry_count,
            } => {
                // Lazily open the WAL writer if not already open.
                if wal.is_none() {
                    let wal_path = Minigraf::wal_path_for(db_path);
                    *wal = Some(WalWriter::open_or_create(&wal_path)?);
                }

                let wal_writer = wal.as_mut().unwrap();
                wal_writer.append_entry(tx_count, facts)?;
                pfs.mark_dirty();
                *wal_entry_count += 1;

                Ok(*wal_entry_count >= opts.wal_checkpoint_threshold)
            }
        }
    }

    /// Explicitly roll back the transaction. Also happens implicitly on drop.
    pub fn rollback(mut self) {
        self.pending_facts.clear();
        self.committed = true; // Suppress rollback in Drop
        set_write_tx_active(false);
    }

    /// Build a temporary `FactStorage` that merges committed facts with pending ones.
    ///
    /// Used to implement read-your-own-writes semantics during a transaction.
    ///
    /// This performs a **deep copy**: a brand-new `FactStorage` is created and
    /// populated with all committed facts followed by the pending buffered facts.
    /// Using `self.inner.fact_storage.clone()` would be an Arc-based shallow clone
    /// that shares the underlying storage, causing `load_fact()` calls to mutate
    /// the shared store and expose uncommitted facts to concurrent readers.
    ///
    /// Optimization: if there are no pending facts, we can use the storage directly
    /// without copying. Otherwise, we create an overlay that combines committed
    /// facts with pending facts without copying all committed facts.
    fn build_query_view(&self) -> Result<FactStorage> {
        // Fast path: no pending facts, use the original storage directly
        if self.pending_facts.is_empty() {
            return Ok(self.inner.fact_storage.clone());
        }

        // Slow path: need to combine committed + pending facts
        // Create overlay that reads from both sources without full copy
        let view = FactStorage::new();

        // Load committed facts - this is the expensive part
        for fact in self.inner.fact_storage.get_all_facts()? {
            view.load_fact(fact)?;
        }

        // Add pending facts
        for fact in &self.pending_facts {
            view.load_fact(fact.clone())?;
        }

        view.restore_tx_counter()?;
        Ok(view)
    }
}

impl Drop for WriteTransaction<'_> {
    fn drop(&mut self) {
        if !self.committed {
            // Implicit rollback: pending facts are simply discarded.
            // No changes were made to the shared FactStorage during buffering,
            // so no cleanup is required there.
            self.pending_facts.clear();
            set_write_tx_active(false);
        }
    }
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── in_memory basic ─────────────────────────────────────────────────────

    #[test]
    fn test_in_memory_no_wal_file() {
        let db = Minigraf::in_memory().unwrap();

        db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
            .unwrap();
        db.execute(r#"(transact [[:alice :person/age 30]])"#)
            .unwrap();

        let facts = db.inner.fact_storage.get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 2, "expected 2 facts after 2 transacts");
    }

    // ── begin_write / commit ─────────────────────────────────────────────────

    #[test]
    fn test_begin_write_commit_facts_visible() {
        let db = Minigraf::in_memory().unwrap();

        {
            let mut tx = db.begin_write().unwrap();
            tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
                .unwrap();
            tx.execute(r#"(transact [[:alice :person/age 30]])"#)
                .unwrap();
            tx.commit().unwrap();
        }

        let facts = db.inner.fact_storage.get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 2, "committed facts must be visible");
    }

    // ── begin_write / rollback ────────────────────────────────────────────────

    #[test]
    fn test_begin_write_rollback_no_facts_visible() {
        let db = Minigraf::in_memory().unwrap();

        {
            let mut tx = db.begin_write().unwrap();
            tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
                .unwrap();
            tx.rollback();
        }

        let facts = db.inner.fact_storage.get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 0, "rolled-back facts must not be visible");
    }

    // ── drop without commit = rollback ────────────────────────────────────────

    #[test]
    fn test_drop_without_commit_is_rollback() {
        let db = Minigraf::in_memory().unwrap();

        {
            let mut tx = db.begin_write().unwrap();
            tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
                .unwrap();
            // tx dropped here without commit
        }

        let facts = db.inner.fact_storage.get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 0, "dropped transaction must act as rollback");
    }

    // ── read-your-own-writes ──────────────────────────────────────────────────

    #[test]
    fn test_write_transaction_read_your_own_writes() {
        let db = Minigraf::in_memory().unwrap();

        let mut tx = db.begin_write().unwrap();
        tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
            .unwrap();

        // Query within the same transaction should see the buffered fact.
        let result = tx
            .execute(r#"(query [:find ?name :where [?e :person/name ?name]])"#)
            .unwrap();

        match result {
            QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1, "buffered fact must be visible in query");
            }
            _ => panic!("expected QueryResults"),
        }

        tx.commit().unwrap();
    }

    // ── thread-local flag: same-thread reentrant error ────────────────────────

    #[test]
    fn test_same_thread_reentrant_write_returns_error() {
        let db = Minigraf::in_memory().unwrap();

        let _tx = db.begin_write().unwrap();

        // While _tx is active, db.execute() on the same thread should fail fast.
        let err = db
            .execute(r#"(transact [[:bob :person/name "Bob"]])"#)
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("WriteTransaction is already in progress"),
            "expected reentrant-write error, got: {}",
            err
        );
    }

    // ── thread-local flag cleared after commit ────────────────────────────────

    #[test]
    fn test_thread_local_flag_cleared_after_commit() {
        let db = Minigraf::in_memory().unwrap();

        {
            let mut tx = db.begin_write().unwrap();
            tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
                .unwrap();
            tx.commit().unwrap();
        }

        // After commit, begin_write should succeed again on the same thread.
        let result = db.begin_write();
        assert!(
            result.is_ok(),
            "begin_write must succeed after commit clears the flag"
        );
        result.unwrap().rollback();
    }

    // ── thread-local flag cleared after rollback ──────────────────────────────

    #[test]
    fn test_thread_local_flag_cleared_after_rollback() {
        let db = Minigraf::in_memory().unwrap();

        {
            let tx = db.begin_write().unwrap();
            tx.rollback();
        }

        let result = db.begin_write();
        assert!(
            result.is_ok(),
            "begin_write must succeed after rollback clears the flag"
        );
        result.unwrap().rollback();
    }

    // ── thread-local flag cleared after drop ─────────────────────────────────

    #[test]
    fn test_thread_local_flag_cleared_after_drop() {
        let db = Minigraf::in_memory().unwrap();

        {
            let mut tx = db.begin_write().unwrap();
            tx.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
                .unwrap();
            // dropped here without commit
        }

        let result = db.begin_write();
        assert!(
            result.is_ok(),
            "begin_write must succeed after drop clears the flag"
        );
        result.unwrap().rollback();
    }

    // ── in-memory checkpoint is a no-op ──────────────────────────────────────

    #[test]
    fn test_in_memory_checkpoint_is_noop() {
        let db = Minigraf::in_memory().unwrap();
        db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
            .unwrap();
        // Should not error
        db.checkpoint().unwrap();
        // Facts should still be present
        let facts = db.inner.fact_storage.get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 1);
    }

    // ── file-backed: open_with_options custom threshold ───────────────────────

    #[test]
    fn test_open_with_options_custom_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.graph");

        let opts = OpenOptions {
            wal_checkpoint_threshold: 5,
            page_cache_size: 256,
            max_derived_facts: 100_000,
            max_results: 1_000_000,
        };
        let db = Minigraf::open_with_options(&path, opts).unwrap();
        assert_eq!(db.inner.options.wal_checkpoint_threshold, 5);
    }

    // ── failed commit leaves database unchanged ───────────────────────────────

    #[test]
    #[cfg(unix)] // directory-as-WAL trick is Unix-specific; skipped on Windows
    fn test_failed_commit_leaves_database_unchanged() {
        fn count_results(result: QueryResult) -> usize {
            match result {
                QueryResult::QueryResults { results, .. } => results.len(),
                _ => 0,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.graph");
        let wal_path = {
            let mut p = db_path.as_os_str().to_owned();
            p.push(".wal");
            std::path::PathBuf::from(p)
        };

        // Open file-backed db and commit one fact so the main file + WAL both exist
        let db = Minigraf::open(&db_path).unwrap();
        db.execute("(transact [[:alice :name \"Alice\"]])").unwrap();

        // Checkpoint: flushes Alice to the main file, closes and deletes the WAL.
        // After this, WriteContext::File { wal: None } so the next commit will
        // try to create a new WAL file at wal_path.
        db.checkpoint().unwrap();
        assert!(!wal_path.exists(), "WAL must be gone after checkpoint");

        // Place a directory at the WAL path so WalWriter::open_or_create() fails
        // with EISDIR when it tries to open the path for writing.
        std::fs::create_dir(&wal_path).unwrap();

        // Begin a transaction and buffer a fact
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:bob :name \"Bob\"]])").unwrap();

        // Commit should fail because the WAL path is now a directory
        let result = tx.commit();

        // Restore the directory so tempdir cleanup works
        std::fs::remove_dir(&wal_path).unwrap();

        assert!(
            result.is_err(),
            "commit should fail when WAL path is a directory"
        );

        // Bob must NOT be visible (failed commit must not apply facts)
        let n = count_results(
            db.execute("(query [:find ?name :where [?e :name ?name]])")
                .unwrap(),
        );
        assert_eq!(
            n, 1,
            "only Alice should be visible; Bob's failed commit must be rolled back"
        );
    }

    // ── file-backed: checkpoint deletes WAL and updates main file ─────────────

    #[test]
    fn test_file_backed_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.graph");
        let wal_path = dir.path().join("test.graph.wal");

        let db = Minigraf::open(&path).unwrap();
        db.execute(r#"(transact [[:alice :person/name "Alice"]])"#)
            .unwrap();

        // WAL should exist before checkpoint
        assert!(wal_path.exists(), "WAL must exist after a write");

        db.checkpoint().unwrap();

        // WAL should be deleted after checkpoint
        assert!(!wal_path.exists(), "WAL must be deleted after checkpoint");

        // Main file should be present with facts
        let db2 = Minigraf::open(&path).unwrap();
        let facts = db2.inner.fact_storage.get_asserted_facts().unwrap();
        assert_eq!(facts.len(), 1, "facts must survive checkpoint");
    }

    #[test]
    fn test_materialize_transaction_non_keyword_real_attr_error() {
        // Exercises db.rs line 576: Real(_) bail! in materialize_transaction (non-keyword Real attr)
        use crate::query::datalog::types::EdnValue;
        use crate::query::datalog::types::{Pattern, Transaction};
        let tx = Transaction {
            facts: vec![Pattern::new(
                EdnValue::Keyword(":alice".to_string()),
                EdnValue::Integer(42), // Real(Integer) — not a keyword
                EdnValue::Integer(0),
            )],
            valid_from: None,
            valid_to: None,
        };
        let r = Minigraf::materialize_transaction(&tx);
        assert!(
            r.is_err(),
            "materialize_transaction with non-keyword Real attr must fail"
        );
    }

    #[test]
    fn test_materialize_retraction_non_keyword_real_attr_error() {
        // Exercises db.rs line 613: Real(_) bail! in materialize_retraction (non-keyword Real attr)
        use crate::query::datalog::types::EdnValue;
        use crate::query::datalog::types::{Pattern, Transaction};
        let tx = Transaction {
            facts: vec![Pattern::new(
                EdnValue::Keyword(":alice".to_string()),
                EdnValue::String("not-a-keyword".to_string()), // Real(String) — not a keyword
                EdnValue::Integer(0),
            )],
            valid_from: None,
            valid_to: None,
        };
        let r = Minigraf::materialize_retraction(&tx);
        assert!(
            r.is_err(),
            "materialize_retraction with non-keyword Real attr must fail"
        );
    }

    #[test]
    fn test_materialize_transaction_pseudo_attr_error() {
        // Exercises db.rs line ~577: Pseudo(_) bail! in materialize_transaction
        use crate::query::datalog::types::EdnValue;
        use crate::query::datalog::types::{Pattern, PseudoAttr, Transaction};
        let tx = Transaction {
            facts: vec![Pattern::pseudo(
                EdnValue::Keyword(":alice".to_string()),
                PseudoAttr::ValidFrom,
                EdnValue::Integer(0),
            )],
            valid_from: None,
            valid_to: None,
        };
        let r = Minigraf::materialize_transaction(&tx);
        assert!(
            r.is_err(),
            "materialize_transaction with pseudo-attr must fail"
        );
    }

    #[test]
    fn test_materialize_retraction_pseudo_attr_error() {
        // Exercises db.rs line ~614: Pseudo(_) bail! in materialize_retraction
        use crate::query::datalog::types::EdnValue;
        use crate::query::datalog::types::{Pattern, PseudoAttr, Transaction};
        let tx = Transaction {
            facts: vec![Pattern::pseudo(
                EdnValue::Keyword(":alice".to_string()),
                PseudoAttr::TxCount,
                EdnValue::Integer(0),
            )],
            valid_from: None,
            valid_to: None,
        };
        let r = Minigraf::materialize_retraction(&tx);
        assert!(
            r.is_err(),
            "materialize_retraction with pseudo-attr must fail"
        );
    }

    // ── begin_write flag not leaked on lock failure ────────────────────────────────

    #[test]
    fn test_begin_write_flag_not_leaked_on_lock_failure() {
        // This test verifies that the thread-local flag is not set if lock acquisition fails.
        // We can't easily simulate lock failure in normal test, but we can verify the
        // flag is correctly managed: set after lock acquired, cleared on drop.

        let db = Minigraf::in_memory().unwrap();

        // Normal flow: begin_write succeeds, flag should be set
        {
            let _tx = db.begin_write().unwrap();
            assert!(
                is_write_tx_active(),
                "flag should be set during active transaction"
            );
        }
        // After drop, flag should be cleared
        assert!(
            !is_write_tx_active(),
            "flag should be cleared after transaction ends"
        );

        // Multiple sequential transactions should work
        {
            let _tx = db.begin_write().unwrap();
        }
        assert!(
            !is_write_tx_active(),
            "flag should be cleared after second transaction"
        );

        {
            let _tx = db.begin_write().unwrap();
        }
        assert!(
            !is_write_tx_active(),
            "flag should be cleared after third transaction"
        );
    }

    // ── query complexity limits ───────────────────────────────────────────────────

    #[test]
    fn test_max_derived_facts_limit_enforced() {
        // Recursive rule will derive many facts
        // Low limit should trigger error
        let low_opts = OpenOptions::default()
            .max_derived_facts(5)
            .max_results(1_000_000);
        let db_low = Minigraf::in_memory_with_options(low_opts).unwrap();

        // Add base edges in a chain
        db_low.execute("(transact [[:a :edge :b] [:b :edge :c] [:c :edge :d] [:d :edge :e] [:e :edge :f]])").unwrap();

        // Register recursive rule
        db_low
            .execute(r#"(rule [(reachable ?x ?y) [?x :edge ?y]])"#)
            .unwrap();
        db_low
            .execute(r#"(rule [(reachable ?x ?y) [?x :edge ?z] (reachable ?z ?y)])"#)
            .unwrap();

        let result = db_low.execute("(query [:find ?to :where (reachable :a ?to)])");
        assert!(
            result.is_err(),
            "Query should fail with max_derived_facts limit"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("derived") || err_msg.contains("limit"),
            "Error should mention derived facts limit, got: {}",
            err_msg
        );

        // Same query with higher limit should succeed
        let high_opts = OpenOptions::default()
            .max_derived_facts(100_000)
            .max_results(1_000_000);
        let db_high = Minigraf::in_memory_with_options(high_opts).unwrap();

        db_high.execute("(transact [[:a :edge :b] [:b :edge :c] [:c :edge :d] [:d :edge :e] [:e :edge :f]])").unwrap();
        db_high
            .execute(r#"(rule [(reachable ?x ?y) [?x :edge ?y]])"#)
            .unwrap();
        db_high
            .execute(r#"(rule [(reachable ?x ?y) [?x :edge ?z] (reachable ?z ?y)])"#)
            .unwrap();

        let result = db_high.execute("(query [:find ?to :where (reachable :a ?to)])");
        assert!(result.is_ok(), "Query should succeed with higher limit");
    }
}
