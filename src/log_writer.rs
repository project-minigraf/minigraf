//! Build a new database from explicit fact records (#431).
//!
//! [`LogWriter`] is the load step of an offline migration: it writes
//! [`FactRecord`]s, such as those read by [`crate::Minigraf::fact_log`] from a
//! read-only source, into a new `.graph` file, keeping each record's
//! `tx_count`, `tx_id`, `asserted` flag and valid-time window. The finished
//! file is an ordinary database, as if the same transactions had been written
//! normally and checkpointed.
//!
//! The file is built beside the target, at `<path>.partial`, and renamed into
//! place by [`LogWriter::finish`]. A crash or an abandoned build never leaves a
//! database at `path` that holds only part of the log.

use crate::db::OpenOptions;
use crate::error::{ErrorCode, MinigrafError, bail_coded, err_coded};
use crate::fact_log::FactRecord;
use crate::graph::types::{EntityId, Fact};
use crate::storage::backend::file::{FileBackend, LockMode};
use crate::storage::dir_sync::sync_parent_dir;
use crate::storage::index::encode_value;
use crate::storage::persistent_facts::PersistentFactStorage;
use anyhow::Result;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Pending facts after which the next transaction boundary commits a batch.
const BATCH_FACTS: usize = 1 << 16;

/// An `(entity, attribute, encoded value)` triple.
type TripleKey = (EntityId, String, Vec<u8>);

/// Writes explicit fact records into a new database file.
///
/// Records must come in transaction order: each one either opens a new
/// transaction, with a `tx_count` above every earlier one, or joins the open
/// transaction, with its `tx_count` and `tx_id`. Gaps between transactions are
/// allowed and stay gaps, so `:as-of N` on a missing transaction shows the
/// state before it. Nothing beyond the file's structure is checked: the caller
/// owns the meaning of the records.
///
/// ```no_run
/// use minigraf::{FactFilter, LogWriter, Minigraf, OpenOptions};
///
/// # fn main() -> Result<(), minigraf::MinigrafError> {
/// let src = Minigraf::open_with_options("old.graph", OpenOptions::new().read_only(true))?;
/// let mut out = LogWriter::create("new.graph", OpenOptions::new())?;
/// for rec in src.fact_log(&FactFilter::new())? {
///     let rec = rec?;
///     if !rec.attribute.starts_with(":secret/") {
///         out.append(&rec)?;
///     }
/// }
/// out.advance_tx_count(src.current_tx_count())?;
/// out.finish()?;
/// # Ok(())
/// # }
/// ```
pub struct LogWriter {
    /// `None` only after `finish` or `drop` took it.
    pfs: Option<PersistentFactStorage<FileBackend>>,
    path: PathBuf,
    partial: PathBuf,
    /// The highest `tx_count` appended or advanced to.
    tx_count: u64,
    /// The open transaction's `(tx_count, tx_id)`, if a record can join it.
    open_tx: Option<(u64, u64)>,
    /// What the open transaction writes per `(e, a, v)`: its valid-time
    /// window, or `None` for a retraction.
    written: HashMap<TripleKey, Option<(i64, i64)>>,
    /// Facts loaded since the last batch commit.
    pending: usize,
}

impl LogWriter {
    /// Start building a new database at `path`.
    ///
    /// `opts.page_cache_size` and `opts.allow_unlocked` apply; the WAL options
    /// do not, since the writer has no WAL.
    ///
    /// # Errors
    ///
    /// - `STG-043` if `path` or its WAL (`<path>.wal`) already exists.
    /// - `STG-025`/`STG-026` if another writer is building the same `path`.
    /// - `API-014` if `opts.read_only` is set.
    /// - An I/O error creating `<path>.partial`.
    pub fn create(path: impl AsRef<Path>, opts: OpenOptions) -> Result<Self, MinigrafError> {
        Self::create_inner(path.as_ref(), opts).map_err(MinigrafError::from)
    }

    fn create_inner(path: &Path, opts: OpenOptions) -> Result<Self> {
        if opts.read_only {
            bail_coded!(ErrorCode::Api014, "LogWriter::create");
        }
        let wal = with_suffix(path, ".wal");
        for p in [path, wal.as_path()] {
            if p.exists() {
                bail_coded!(ErrorCode::Stg043, p.display());
            }
        }
        let partial = with_suffix(path, ".partial");
        let mut backend =
            FileBackend::open_with(&partial, opts.allow_unlocked, LockMode::Exclusive)?;
        // A `.partial` from an earlier build that never finished: start over.
        backend.truncate()?;
        let pfs = PersistentFactStorage::open(backend, opts.page_cache_size, None)?;
        Ok(LogWriter {
            pfs: Some(pfs),
            path: path.to_path_buf(),
            partial,
            tx_count: 0,
            open_tx: None,
            written: HashMap::new(),
            pending: 0,
        })
    }

    /// The highest `tx_count` appended or passed to
    /// [`advance_tx_count`](Self::advance_tx_count); 0 before either.
    pub fn tx_count(&self) -> u64 {
        self.tx_count
    }

    /// Append one record.
    ///
    /// A record with `tx_count` above [`tx_count`](Self::tx_count) opens a
    /// new transaction. A record with the open transaction's `tx_count` joins
    /// it and must carry its `tx_id`. An exact repeat of a record already in
    /// the open transaction is dropped, as a normal write drops a repeated
    /// fact. When enough facts are pending, opening a transaction first
    /// commits the earlier ones to the file.
    ///
    /// # Errors
    ///
    /// A rejected record changes nothing, and the writer stays usable:
    /// - `API-015` if `tx_count` is 0, below the writer's, or names a
    ///   transaction that is already closed.
    /// - `API-016` if it joins the open transaction with a different `tx_id`.
    /// - `API-019` if it asserts a valid-time window that ends at or before
    ///   it starts.
    /// - `API-011` if it asserts an `(entity, attribute, value)` the open
    ///   transaction already asserts with another valid-time window.
    /// - `API-020` if it asserts an `(entity, attribute, value)` the open
    ///   transaction retracts, or retracts one it asserts.
    /// - `WAL-003` if the attribute, a keyword or a string value is longer
    ///   than the format stores.
    ///
    /// An I/O error from committing a batch is also returned; drop the writer
    /// after one.
    pub fn append(&mut self, rec: &FactRecord) -> Result<(), MinigrafError> {
        self.append_inner(rec).map_err(MinigrafError::from)
    }

    fn append_inner(&mut self, rec: &FactRecord) -> Result<()> {
        let joins = match self.open_tx {
            Some((tx, tx_id)) if tx == rec.tx_count => {
                if tx_id != rec.tx_id {
                    bail_coded!(ErrorCode::Api016, tx, tx_id, rec.tx_id);
                }
                true
            }
            _ if rec.tx_count > self.tx_count => false,
            _ => bail_coded!(ErrorCode::Api015, rec.tx_count, self.tx_count),
        };
        let fact = Fact {
            entity: rec.entity,
            attribute: rec.attribute.clone(),
            value: rec.value.clone(),
            tx_id: rec.tx_id,
            tx_count: rec.tx_count,
            valid_from: rec.valid_from,
            valid_to: rec.valid_to,
            asserted: rec.asserted,
        };
        crate::wal::check_fact_size(&fact)?;
        // The rules of a normal write hold for every file built here (#477):
        // a source carrying older records that break them must be repaired
        // before it is copied.
        if fact.asserted && fact.valid_to <= fact.valid_from {
            bail_coded!(ErrorCode::Api019, fact.attribute);
        }
        let key = (
            fact.entity,
            fact.attribute.clone(),
            encode_value(&fact.value),
        );
        let this = fact.asserted.then_some((fact.valid_from, fact.valid_to));
        if joins && let Some(prev) = self.written.get(&key) {
            match (prev, this) {
                (Some(a), Some(b)) if *a != b => bail_coded!(ErrorCode::Api011, fact.attribute),
                (Some(_), None) | (None, Some(_)) => {
                    bail_coded!(ErrorCode::Api020, fact.attribute)
                }
                _ => {}
            }
        }

        if !joins {
            self.close_tx();
            if self.pending >= BATCH_FACTS {
                self.commit_batch()?;
            }
            self.tx_count = rec.tx_count;
            self.open_tx = Some((rec.tx_count, rec.tx_id));
        }
        self.written.insert(key, this);
        let pfs = self.pfs_mut()?;
        if pfs.storage().load_fact(fact)? {
            pfs.mark_dirty();
            self.pending = self.pending.saturating_add(1);
        }
        Ok(())
    }

    /// Close the open transaction and raise the writer's `tx_count` to
    /// `tx_count`, with no facts: the transactions in between, or a trailing
    /// run of purged ones, stay empty. The finished file's
    /// [`current_tx_count`](crate::Minigraf::current_tx_count) is the writer's,
    /// so after a purge of the last transactions the next write still takes the
    /// source's next number.
    ///
    /// # Errors
    ///
    /// `API-015` if `tx_count` is below the writer's. Nothing changes.
    pub fn advance_tx_count(&mut self, tx_count: u64) -> Result<(), MinigrafError> {
        if tx_count < self.tx_count {
            return Err(MinigrafError::from(err_coded!(
                ErrorCode::Api015,
                tx_count,
                self.tx_count
            )));
        }
        self.close_tx();
        self.tx_count = tx_count;
        Ok(())
    }

    /// Commit every pending record, close the file and rename it to `path`.
    ///
    /// The result is an ordinary database with no WAL: its indexes hold every
    /// record and its transaction counter is [`tx_count`](Self::tx_count).
    ///
    /// # Errors
    ///
    /// - `STG-043` if a file appeared at `path` during the build. It is left
    ///   untouched, and the build is discarded.
    /// - An I/O error from the commit, the rename or the directory sync.
    pub fn finish(mut self) -> Result<(), MinigrafError> {
        self.finish_inner().map_err(MinigrafError::from)
    }

    fn finish_inner(&mut self) -> Result<()> {
        self.close_tx();
        let mut pfs = self
            .pfs
            .take()
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "log writer already finished"))?;
        pfs.storage().restore_tx_counter_from(self.tx_count);
        // Also with no records: the meta must carry the counter.
        pfs.force_dirty();
        let saved = pfs.save();
        // Release the file (and its lock) before the rename.
        pfs.discard();
        drop(pfs);
        saved?;
        if self.path.exists() {
            bail_coded!(ErrorCode::Stg043, self.path.display());
        }
        std::fs::rename(&self.partial, &self.path)?;
        sync_parent_dir(&self.path)?;
        Ok(())
    }

    fn close_tx(&mut self) {
        self.open_tx = None;
        self.written.clear();
    }

    /// Commit the pending records as the next generation of the file.
    fn commit_batch(&mut self) -> Result<()> {
        let tx_count = self.tx_count;
        let pfs = self.pfs_mut()?;
        pfs.storage().restore_tx_counter_from(tx_count);
        pfs.save()?;
        self.pending = 0;
        Ok(())
    }

    fn pfs_mut(&mut self) -> Result<&mut PersistentFactStorage<FileBackend>> {
        self.pfs
            .as_mut()
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "log writer already finished"))
    }
}

impl Drop for LogWriter {
    /// An unfinished build (or one whose `finish` failed) is deleted.
    fn drop(&mut self) {
        if let Some(mut pfs) = self.pfs.take() {
            pfs.discard();
            drop(pfs);
        }
        if self.partial.exists() {
            let _ = std::fs::remove_file(&self.partial);
        }
    }
}

/// `path` with `suffix` appended to its file name.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::Value;

    fn rec(tx_count: u64, i: u64) -> FactRecord {
        FactRecord {
            entity: uuid::Uuid::from_u128(u128::from(i) + 1),
            attribute: ":n/v".to_string(),
            value: Value::Integer(i64::try_from(i).unwrap()),
            tx_count,
            tx_id: 1_000 + tx_count,
            valid_from: 0,
            valid_to: i64::MAX,
            asserted: true,
        }
    }

    fn generation(w: &LogWriter) -> u64 {
        w.pfs.as_ref().unwrap().generation()
    }

    #[test]
    fn a_batch_commits_only_when_a_transaction_opens_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = LogWriter::create(dir.path().join("b.graph"), OpenOptions::new()).unwrap();
        let n = u64::try_from(BATCH_FACTS).unwrap();
        for i in 0..n + 10 {
            w.append(&rec(1, i)).unwrap();
        }
        // One transaction is never split, however large.
        assert_eq!(generation(&w), 1);
        assert_eq!(w.pending, BATCH_FACTS + 10);
        w.append(&rec(2, 0)).unwrap();
        assert_eq!(generation(&w), 2);
        assert_eq!(w.pending, 1);
        w.finish().unwrap();
    }

    #[test]
    fn truncate_empties_a_file_backend() {
        use crate::storage::{PAGE_SIZE, StorageBackend};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let mut b = FileBackend::open_with(&path, false, LockMode::Exclusive).unwrap();
        b.write_page(2, &vec![1u8; PAGE_SIZE]).unwrap();
        assert_eq!(b.page_count().unwrap(), 3);
        b.truncate().unwrap();
        assert_eq!(b.page_count().unwrap(), 0);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }
}
