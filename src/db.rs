//! Public `Minigraf` facade with `WriteTransaction` and `OpenOptions`.
//!
//! This module provides the primary user-facing API for Minigraf:
//! - `Minigraf::open()` / `Minigraf::in_memory()` for database creation
//! - `Minigraf::execute()` for implicit (self-contained) transactions
//! - `Minigraf::begin_write()` / `WriteTransaction` for explicit transactions
//! - `Minigraf::checkpoint()` for manual WAL compaction

use crate::graph::types::{Fact, VALID_TIME_FOREVER};
use crate::graph::FactStorage;
use crate::query::datalog::executor::DatalogExecutor;
use crate::query::datalog::parser::parse_datalog_command;
use crate::query::datalog::rules::RuleRegistry;
use crate::query::datalog::types::{DatalogCommand, Transaction};
use crate::query::datalog::executor::QueryResult;
use crate::storage::backend::file::FileBackend;
use crate::storage::backend::MemoryBackend;
use crate::storage::persistent_facts::PersistentFactStorage;
use crate::wal::WalWriter;
use anyhow::{bail, Result};
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
}

impl Default for OpenOptions {
    fn default() -> Self {
        OpenOptions {
            wal_checkpoint_threshold: 1000,
        }
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
        if let Ok(mut ctx) = self.write_lock.lock() {
            let _ = Minigraf::do_checkpoint(&self.fact_storage, &mut ctx);
        }
    }
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
        let pfs = PersistentFactStorage::new(backend)?;

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
                write_lock: Mutex::new(ctx),
                options: opts,
            }),
        })
    }

    /// Create an in-memory database (no WAL, no persistence). Suitable for tests and REPL.
    pub fn in_memory() -> Result<Self> {
        let backend = MemoryBackend::new();
        let pfs = PersistentFactStorage::new(backend)?;
        let fact_storage = pfs.storage().clone();

        // For in-memory databases we don't need the PFS beyond initialisation;
        // we just use the shared FactStorage directly.
        drop(pfs);

        Ok(Minigraf {
            inner: Arc::new(Inner {
                fact_storage,
                rules: Arc::new(RwLock::new(RuleRegistry::new())),
                write_lock: Mutex::new(WriteContext::Memory),
                options: OpenOptions::default(),
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
                fact_storage.load_fact(fact.clone())?;
            }
            replayed += 1;
        }

        // Re-synchronise tx_counter to the maximum tx_count across all facts
        fact_storage.restore_tx_counter()?;

        Ok(replayed)
    }

    // ── Execute ──────────────────────────────────────────────────────────────

    /// Execute a Datalog command as an implicit (self-contained) transaction.
    ///
    /// For file-backed databases, writes are WAL-durable before this method returns.
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
            bail!("a WriteTransaction is already in progress on this thread; use tx.execute() instead");
        }

        let cmd = parse_datalog_command(input).map_err(|e| anyhow::anyhow!("{}", e))?;

        // Determine if this is a read-only command (query / rule registration).
        let is_write = matches!(cmd, DatalogCommand::Transact(_) | DatalogCommand::Retract(_));

        if is_write {
            let mut ctx = self.inner.write_lock.lock().unwrap();
            let executor = DatalogExecutor::new_with_rules(
                self.inner.fact_storage.clone(),
                self.inner.rules.clone(),
            );
            let result = executor.execute(cmd)?;
            // WAL write + optional checkpoint
            Self::maybe_wal_write_and_checkpoint(
                &self.inner.fact_storage,
                &mut ctx,
                &self.inner.options,
            )?;
            Ok(result)
        } else {
            // Read-only: no lock needed
            let executor = DatalogExecutor::new_with_rules(
                self.inner.fact_storage.clone(),
                self.inner.rules.clone(),
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
            bail!("a WriteTransaction is already in progress on this thread; use tx.execute() instead");
        }
        set_write_tx_active(true);
        let guard = self.inner.write_lock.lock().unwrap();
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
        let mut ctx = self.inner.write_lock.lock().unwrap();
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

    // ── WAL write + auto-checkpoint helper ───────────────────────────────────

    /// Write the most recent transaction batch to the WAL sidecar and optionally
    /// trigger an automatic checkpoint.
    ///
    /// Called while the write lock is held.
    fn maybe_wal_write_and_checkpoint(
        fact_storage: &FactStorage,
        ctx: &mut WriteContext,
        opts: &OpenOptions,
    ) -> Result<()> {
        let should_checkpoint = match ctx {
            WriteContext::Memory => {
                // Nothing to do for in-memory databases.
                false
            }
            WriteContext::File {
                pfs,
                wal,
                db_path,
                wal_entry_count,
            } => {
                let tx_count = pfs.storage().current_tx_count();
                let batch: Vec<Fact> = pfs
                    .storage()
                    .get_all_facts()?
                    .into_iter()
                    .filter(|f| f.tx_count == tx_count)
                    .collect();

                // Lazily open the WAL writer if not already open.
                if wal.is_none() {
                    let wal_path = Self::wal_path_for(db_path);
                    *wal = Some(WalWriter::open_or_create(&wal_path)?);
                }

                let wal_writer = wal.as_mut().unwrap();
                wal_writer.append_entry(tx_count, &batch)?;
                pfs.mark_dirty();
                *wal_entry_count += 1;

                *wal_entry_count >= opts.wal_checkpoint_threshold
            }
        };

        if should_checkpoint {
            Minigraf::do_checkpoint(fact_storage, ctx)?;
        }

        Ok(())
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
                EdnValue::Keyword(k) => k.clone(),
                _ => anyhow::bail!("attribute must be a keyword"),
            };

            let value = edn_to_value(&pattern.value)
                .map_err(|e| anyhow::anyhow!("invalid value: {}", e))?;

            let valid_from = pattern.valid_from.or(tx_valid_from).unwrap_or(0);
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
                EdnValue::Keyword(k) => k.clone(),
                _ => anyhow::bail!("attribute must be a keyword"),
            };

            let value = edn_to_value(&pattern.value)
                .map_err(|e| anyhow::anyhow!("invalid value: {}", e))?;

            let mut f = Fact::retract(entity, attr, value, 0);
            f.tx_count = 0;
            facts.push(f);
        }

        Ok(facts)
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
                let executor = DatalogExecutor::new_with_rules(view, self.inner.rules.clone());
                executor.execute(cmd)
            }
        }
    }

    /// Commit this transaction atomically.
    ///
    /// All buffered facts are applied to the shared `FactStorage` in a single
    /// batch (one `tx_count`), then a WAL entry is written and fsynced.
    ///
    /// On failure, buffered facts are rolled back and the database is left
    /// in the same state as before `begin_write()` was called.
    pub fn commit(mut self) -> Result<()> {
        // Apply buffered facts to shared FactStorage
        let facts_to_commit = std::mem::take(&mut self.pending_facts);

        if !facts_to_commit.is_empty() {
            // Assign a single tx_count for the entire batch
            let tx_count = self.inner.fact_storage.allocate_tx_count();
            let tx_id = crate::graph::types::tx_id_now();

            // Load each fact with the assigned tx_id and tx_count
            for mut fact in facts_to_commit {
                fact.tx_id = tx_id;
                fact.tx_count = tx_count;
                // Fix valid_from if it was left as 0 (placeholder for "use tx time")
                if fact.valid_from == 0 && fact.asserted {
                    fact.valid_from = tx_id as i64;
                }
                self.inner.fact_storage.load_fact(fact)?;
            }

            // WAL write + optional auto-checkpoint
            Minigraf::maybe_wal_write_and_checkpoint(
                &self.inner.fact_storage,
                &mut self.guard,
                &self.inner.options,
            )?;
        }

        self.committed = true;
        set_write_tx_active(false);
        Ok(())
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
    fn build_query_view(&self) -> Result<FactStorage> {
        // The shared FactStorage already has all committed facts (Arc-based clone).
        // We just need to add the pending (buffered) facts on top.
        let view = self.inner.fact_storage.clone();

        for fact in &self.pending_facts {
            view.load_fact(fact.clone())?;
        }

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
            err.to_string().contains("WriteTransaction is already in progress"),
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
        assert!(result.is_ok(), "begin_write must succeed after commit clears the flag");
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
        };
        let db = Minigraf::open_with_options(&path, opts).unwrap();
        assert_eq!(db.inner.options.wal_checkpoint_threshold, 5);
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
}
