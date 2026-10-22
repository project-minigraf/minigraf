//! Large transactions (#550, spec `2026-10-21-streaming-transactions-design.md`
//! §4): a transaction with more facts than one chunk writes them, a chunk at a
//! time, into copy-on-write working trees, and commits as a checkpoint.
//!
//! The working trees start as generation `g`'s and become generation `g + 1`'s
//! at commit. Every page they gain comes from an allocator that hands out only
//! ids `g` does not reference, so generation `g` and every reader of it are
//! untouched until the commit point, the meta page write, and after a rollback.
//! The caller holds the database's write lock throughout.

use super::PersistentFactStorage;
use crate::error::{ErrorCode, bail_coded, err_coded};
use crate::graph::storage::{check_one_window_per_triple, check_valid_windows};
use crate::graph::types::{Fact, TxId};
use crate::storage::StorageBackend;
use crate::storage::btree::{LeafCursor, cow_update};
use crate::storage::cache::PageCache;
use crate::storage::dict::{DictReader, Encoded, Encoder};
use crate::storage::keys::{self, Index, KeyFact};
use crate::storage::meta::{MetaPage, slot_page};
use crate::storage::node::Entry;
use crate::storage::page::PageAllocator;
use anyhow::Result;
use std::collections::HashSet;

/// A large transaction in progress: generation `g`'s meta, the working state
/// that becomes `g + 1`'s, and the pages it has written.
pub(crate) struct LargeTx {
    /// Generation `g`'s meta, active while the transaction runs.
    base: MetaPage,
    /// The working roots, counters and fact count, as generation `g + 1`.
    work: MetaPage,
    /// Hands out only ids `g` does not reference, and records them.
    alloc: PageAllocator,
    /// Pages `g` references that the working trees replaced.
    freed: Vec<u64>,
    tx_count: u64,
    tx_id: TxId,
}

/// How a chunk failed.
pub(crate) enum ChunkError {
    /// The statement is rejected (`API-011`, `API-019`, `WAL-003`): roll back,
    /// the handle stays usable.
    Statement(anyhow::Error),
    /// A write or read failed after pages may have been written: the handle
    /// is failed (`STG-046`); roll back.
    Failure(anyhow::Error),
}

impl LargeTx {
    /// The transaction time every fact is stamped with, fixed when the
    /// transaction became large.
    pub(crate) fn tx_id(&self) -> TxId {
        self.tx_id
    }
}

impl<B: StorageBackend + 'static> PersistentFactStorage<B> {
    /// Make the current transaction large (spec §4.1 steps 2–4): raise the
    /// cache's generation bound to `g + 1`, reserve the next `tx_count`, and
    /// build the allocator. The caller has created the WAL guard (step 1).
    ///
    /// The `tx_count` is the next one; it is allocated at the commit point,
    /// so a transaction that rolls back takes none (plan decision 14).
    pub(crate) fn begin_large(&mut self, tx_id: TxId) -> Result<LargeTx> {
        self.refuse_if_failed()?;
        let base = self.meta;
        let next = base.next_generation()?;
        let tx_count = self
            .storage
            .current_tx_count()
            .checked_add(1)
            .ok_or_else(|| err_coded!(ErrorCode::Int048, "tx_count overflow"))?;
        let alloc = PageAllocator::from_chain(
            base.freelist_head,
            base.freelist_count,
            base.page_count,
            next,
        )
        .with_quarantine(self.storage.pages_in_force())
        .tracking();
        self.page_cache.set_generation_bound(next);
        Ok(LargeTx {
            base,
            work: MetaPage {
                generation: next,
                ..base
            },
            alloc,
            freed: Vec::new(),
            tx_count,
            tx_id,
        })
    }

    /// Write one chunk of the statement (spec §4.2): check it, stamp it with
    /// the transaction's `tx_count` and `tx_id`, encode it against the
    /// working dictionary and apply it to the working trees.
    ///
    /// A [`ChunkError::Failure`] has already marked the handle failed.
    pub(crate) fn write_large_chunk(
        &mut self,
        tx: &mut LargeTx,
        facts: &mut [Fact],
    ) -> std::result::Result<(), ChunkError> {
        // Checks first, before this chunk writes anything: a rejected
        // statement is not a failure.
        check_chunk(facts, tx.tx_id).map_err(ChunkError::Statement)?;
        for f in facts.iter_mut() {
            f.tx_id = tx.tx_id;
            f.tx_count = tx.tx_count;
            if f.asserted && f.valid_from == crate::db::VALID_FROM_USE_TX_TIME {
                f.valid_from = tx.tx_id.cast_signed();
            }
        }
        let result = self.apply_chunk(tx, facts, true);
        if let Err(ChunkError::Failure(_)) = &result {
            self.failed = true;
        }
        #[cfg(test)]
        if result.is_ok() {
            tests::after_chunk();
        }
        result
    }

    /// Commit the transaction as a checkpoint (spec §4.3): the pending store's
    /// facts join the working trees, the free list is written, and the meta
    /// page of `g + 1` is the commit point. The caller deletes the WAL next.
    ///
    /// On an error the handle is failed (`STG-046`) and the in-memory state is
    /// rolled back; the file holds `g`, or `g + 1` complete if the meta page
    /// reached it.
    pub(crate) fn commit_large(&mut self, mut tx: LargeTx) -> Result<()> {
        self.failed = true;
        match self.commit_large_inner(&mut tx) {
            Ok(()) => {
                self.failed = false;
                Ok(())
            }
            Err(e) => {
                // Dirty, so a checkpoint is refused (STG-046) rather than
                // deleting the WAL guard without a meta.
                self.rollback_large(tx);
                Err(e)
            }
        }
    }

    fn commit_large_inner(&mut self, tx: &mut LargeTx) -> Result<()> {
        let mut pending = self.storage.get_pending_facts();
        let pending_len = pending.len();
        // Their own tx_counts and statement rules: only encoded and inserted.
        match self.apply_chunk(tx, &mut pending, false) {
            Ok(()) => {}
            Err(ChunkError::Statement(e) | ChunkError::Failure(e)) => return Err(e),
        }
        drop(pending);

        let mut backend = self.lock()?;
        let (freelist_head, freelist_count, _) =
            tx.alloc
                .finish_free_list(tx.freed.clone(), &mut *backend, &self.page_cache)?;
        backend.sync()?;
        let new_meta = MetaPage {
            page_count: tx.alloc.next_append(),
            last_checkpointed_tx_count: tx.tx_count,
            freelist_head,
            freelist_count,
            ..tx.work
        };
        super::check_written(&*backend, &new_meta)?;
        backend.write_page(slot_page(new_meta.generation), &new_meta.encode())?;
        backend.sync()?;
        drop(backend);

        // Committed. No other writer runs under the write lock, so the
        // reserved tx_count is the next one.
        let tx_count = self.storage.allocate_tx_count();
        if tx_count != tx.tx_count {
            bail_coded!(
                ErrorCode::Int049,
                format!("tx_count {tx_count} allocated, {} reserved", tx.tx_count)
            );
        }
        self.dirty = false;
        let reader = self.activate(new_meta);
        self.storage.publish(
            reader,
            new_meta.generation,
            pending_len,
            std::mem::take(&mut tx.freed),
        );
        Ok(())
    }

    /// Roll back (spec §4.4): generation `g` stays active and untouched. The
    /// pages written are free in `g` or past its `page_count`. The handle is
    /// marked dirty, so its next checkpoint commits a meta before it deletes
    /// the WAL guard (§4.5).
    pub(crate) fn rollback_large(&mut self, tx: LargeTx) {
        self.undo_large(&tx);
        self.dirty = true;
    }

    /// Set the cache's generation bound back to `g` and drop the pages the
    /// transaction wrote from the cache.
    fn undo_large(&self, tx: &LargeTx) {
        self.page_cache.set_generation_bound(tx.base.generation);
        if let Some(handed) = tx.alloc.handed_ids() {
            self.page_cache.evict(handed);
        }
    }

    /// Encode `facts` against the working dictionary and apply them to the
    /// working trees. With `own`, they are this transaction's: a fact an
    /// earlier chunk wrote is kept once, and another window of one of its
    /// triples is `API-011`.
    fn apply_chunk(
        &mut self,
        tx: &mut LargeTx,
        facts: &mut [Fact],
        own: bool,
    ) -> std::result::Result<(), ChunkError> {
        let failure = ChunkError::Failure;
        let mut backend = self.lock().map_err(failure)?;
        let cache = &*self.page_cache;
        let encoded =
            encode(facts, &tx.work, &mut tx.alloc, &mut *backend, cache).map_err(failure)?;
        let Encoded {
            mut index,
            dict,
            next_eid,
            next_iid,
        } = encoded;
        if own {
            let repeats = repeats(&tx.work, &index[0], tx.tx_count, &*backend, cache)?;
            drop_repeats(&mut index, &repeats).map_err(failure)?;
        }
        let added = u64::try_from(index[0].len())
            .map_err(|_| ChunkError::Failure(err_coded!(ErrorCode::Stg024)))?;

        let mut roots = tx.work.tree_roots();
        for (root, entries) in roots.iter_mut().zip(index.into_iter().chain([dict])) {
            let mut replaced = Vec::new();
            *root = cow_update(
                *root,
                entries,
                Vec::new(),
                &mut *backend,
                cache,
                &mut tx.alloc,
                &mut replaced,
            )
            .map_err(failure)?;
            // A page this transaction wrote is referenced by nothing now; a
            // page of `g` is freed at commit (spec §4.2 step 3; no statement
            // base in a one-statement transaction, plan decision 13).
            for id in replaced {
                if tx.alloc.handed(id) {
                    tx.alloc.release(id);
                } else {
                    tx.freed.push(id);
                }
            }
        }
        let [eavt_root, aevt_root, avet_root, vaet_root, dict_root] = roots;
        tx.work = MetaPage {
            eavt_root,
            aevt_root,
            avet_root,
            vaet_root,
            dict_root,
            next_eid,
            next_iid,
            fact_count: tx.work.fact_count.checked_add(added).ok_or_else(|| {
                ChunkError::Failure(err_coded!(ErrorCode::Int048, "fact_count overflow"))
            })?,
            ..tx.work
        };
        Ok(())
    }
}

/// A chunk's statement checks, before it writes anything (plan decision 18):
/// valid windows at the fixed `tx_id` (`API-019`), one window per triple
/// within the chunk (`API-011`), and the sizes a checkpoint can store
/// (`WAL-003`, as the WAL entry checked them).
fn check_chunk(facts: &[Fact], tx_id: TxId) -> Result<()> {
    check_valid_windows(facts, tx_id)?;
    check_one_window_per_triple(facts)?;
    for f in facts {
        crate::wal::check_fact_size(f)?;
    }
    Ok(())
}

/// Assign ids for `facts` against the working dictionary of `work` (with no
/// `SharedDictCache`: the committed reader's cache must never learn an id
/// that is not committed), write their new long values, and build every
/// tree's entries.
fn encode(
    facts: &[Fact],
    work: &MetaPage,
    alloc: &mut PageAllocator,
    backend: &mut dyn StorageBackend,
    cache: &PageCache,
) -> Result<Encoded> {
    let mut encoder = Encoder::new(work.next_eid, work.next_iid);
    {
        let mut dict = DictReader::new(work.dict_root, &*backend, cache);
        for f in facts {
            encoder.stage(f, &mut dict)?;
        }
    }
    encoder.finish(alloc, backend, cache)
}

/// The positions in `eavt` (a chunk's sorted EAVT entries, all of `tx_count`)
/// of facts an earlier chunk of the transaction already wrote to the working
/// trees of `work` (#550 spec §5.2, fast path). Another window of an asserted
/// triple there is `API-011`, as within one statement today.
fn repeats(
    work: &MetaPage,
    eavt: &[Entry],
    tx_count: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> std::result::Result<Vec<usize>, ChunkError> {
    let failure = ChunkError::Failure;
    let mut out = Vec::new();
    if work.eavt_root == 0 {
        return Ok(out);
    }
    let mut cursor: Option<LeafCursor<'_>> = None;
    // The working keys of the current triple and this tx_count: one run per
    // triple, read once even when the chunk has several records of it.
    let mut start: Vec<u8> = Vec::new();
    let mut run: Vec<Vec<u8>> = Vec::new();
    for (i, (key, _)) in eavt.iter().enumerate() {
        let triple = keys::triple_len(Index::Eavt, key).map_err(failure)?;
        let mut s = key.get(..triple).unwrap_or_default().to_vec();
        keys::put_tx_desc(&mut s, tx_count);
        if s != start {
            start = s;
            run.clear();
            let cursor = match &mut cursor {
                Some(c) => {
                    c.seek(&start).map_err(failure)?;
                    c
                }
                None => cursor.insert(
                    LeafCursor::new(work.eavt_root, Some(&start), backend, cache)
                        .map_err(failure)?,
                ),
            };
            while let Some((k, _)) = cursor.peek_ref().map_err(failure)? {
                if !k.starts_with(&start) {
                    break;
                }
                run.push(k.clone());
                cursor.next_ref().map_err(failure)?;
            }
        }
        let asserted = key.last() == Some(&1);
        if run.iter().any(|k| k == key) {
            out.push(i);
        } else if asserted && run.iter().any(|k| k.last() == Some(&1)) {
            let attribute = KeyFact::decode(Index::Eavt, key)
                .and_then(|kf| DictReader::new(work.dict_root, backend, cache).name_of(kf.a))
                .map_err(failure)?;
            return Err(ChunkError::Statement(err_coded!(
                ErrorCode::Api011,
                attribute
            )));
        }
    }
    Ok(out)
}

/// Remove the facts at `positions` of the EAVT entries from all four index
/// trees' entries.
fn drop_repeats(index: &mut [Vec<Entry>; 4], positions: &[usize]) -> Result<()> {
    if positions.is_empty() {
        return Ok(());
    }
    let mut gone: [HashSet<Vec<u8>>; 4] = Default::default();
    for &i in positions {
        let key = index[0]
            .get(i)
            .ok_or_else(|| err_coded!(ErrorCode::Int049, "repeat out of range"))?;
        let kf = KeyFact::decode(Index::Eavt, &key.0)?;
        for (set, idx) in gone.iter_mut().zip(Index::ALL) {
            if let Some(k) = kf.key(idx) {
                set.insert(k);
            }
        }
    }
    for (entries, set) in index.iter_mut().zip(&gone) {
        entries.retain(|(k, _)| !set.contains(k));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::{Minigraf, OpenOptions};
    use crate::fact_log::FactFilter;
    use crate::query::datalog::parser::statement::set_tx_chunk_facts;
    use crate::storage::meta::{MetaPage, SlotState};
    use std::cell::RefCell;
    use std::path::Path;

    thread_local! {
        static AFTER_CHUNK: RefCell<Option<Box<dyn FnMut()>>> = RefCell::new(None);
    }

    /// Run the hook a test set, after each chunk a large transaction writes.
    pub(super) fn after_chunk() {
        let hook = AFTER_CHUNK.with(|h| h.borrow_mut().take());
        if let Some(mut f) = hook {
            f();
            AFTER_CHUNK.with(|h| *h.borrow_mut() = Some(f));
        }
    }

    /// One fact version, comparable across databases: a default `valid_from`
    /// (the transaction time) is `None`.
    type Rec = (String, String, String, u64, Option<i64>, i64, bool);

    fn records(db: &Minigraf) -> Vec<Rec> {
        let mut v: Vec<Rec> = db
            .fact_log(&FactFilter::new())
            .unwrap()
            .map(|r| {
                let r = r.unwrap();
                let default_from = r.valid_from == i64::try_from(r.tx_id).unwrap();
                (
                    r.entity.to_string(),
                    r.attribute,
                    format!("{:?}", r.value),
                    r.tx_count,
                    (!default_from).then_some(r.valid_from),
                    r.valid_to,
                    r.asserted,
                )
            })
            .collect();
        v.sort();
        v
    }

    fn wal_of(path: &Path) -> std::path::PathBuf {
        let mut wal = path.as_os_str().to_owned();
        wal.push(".wal");
        wal.into()
    }

    /// The highest generation of a valid meta slot in the file at `path`.
    fn file_generation(path: &Path) -> u64 {
        let bytes = std::fs::read(path).unwrap();
        bytes
            .chunks(crate::storage::PAGE_SIZE)
            .take(2)
            .filter_map(|p| match MetaPage::decode(p) {
                SlotState::Valid(m) => Some(m.generation),
                _ => None,
            })
            .max()
            .unwrap()
    }

    fn code(r: Result<crate::QueryResult, crate::MinigrafError>) -> String {
        r.expect_err("rejected").code().to_string()
    }

    const LONG: &str =
        "llllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllll";

    /// Statements covering what a chunk boundary can split: repeats,
    /// several values and windows of one attribute, long values shared
    /// between facts, refs, keywords, transaction-level windows, retractions
    /// of committed and unknown facts, and one-fact statements whose WAL
    /// entries are pending when a large statement commits.
    fn workload() -> Vec<String> {
        vec![
            format!(
                r#"(transact [[:a :n 1] [:a :n 2] [:a :n 1] [:b :ref :a] [:b :s "{LONG}"]
                   [:c :s "{LONG}"] [:a :n 3 {{:valid-from "2020-01-01T00:00:00Z"
                   :valid-to "2021-01-01T00:00:00Z"}}] [:d :k :kw] [:d :k :kw]])"#
            ),
            "(transact [[:h :n 1]])".into(),
            "(retract [[:a :n 1] [:b :ref :a] [:zz :n 9]])".into(),
            r#"(transact {:valid-from "2019-01-01T00:00:00Z"} [[:e :n 1] [:e :n 2] [:e :n 1] [:f :n 5]])"#.into(),
            "(transact [[:h :n 2]])".into(),
            "(transact [[:a :n 4] [:a :n 4] [:a :n 4]])".into(),
            format!(
                r#"(transact [[:g :s "{LONG}x"] [:g :s "{LONG}x"] [:a :n 1] [:g :n 1] [:g :n 2]])"#
            ),
        ]
    }

    #[test]
    fn large_statements_store_what_small_ones_do() {
        let dir = tempfile::tempdir().unwrap();
        let small_path = dir.path().join("small.graph");
        let small = Minigraf::open(&small_path).unwrap();
        for stmt in workload() {
            small.execute(&stmt).unwrap();
        }
        small.checkpoint().unwrap();
        let want = records(&small);
        assert!(want.len() > 10);

        for chunk in 1..=4 {
            let path = dir.path().join(format!("large{chunk}.graph"));
            let db = Minigraf::open(&path).unwrap();
            {
                let _chunk = set_tx_chunk_facts(Some(chunk));
                for stmt in workload() {
                    db.execute(&stmt).unwrap();
                }
            }
            assert!(records(&db) == want, "records at chunk {chunk}");
            assert_eq!(db.current_tx_count(), small.current_tx_count());
            assert!(db.verify().unwrap().is_ok(), "verify at chunk {chunk}");
            assert!(
                !wal_of(&path).exists(),
                "the last large statement committed the WAL's entries"
            );
            drop(db);
            let db = Minigraf::open(&path).unwrap();
            assert!(
                records(&db) == want,
                "records after reopen at chunk {chunk}"
            );
            assert!(db.verify().unwrap().is_ok());
        }
    }

    #[test]
    fn a_large_statement_returns_its_tx_id_and_answers_as_of() {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("t.graph")).unwrap();
        db.execute("(transact [[:a :n 1]])").unwrap();
        let _chunk = set_tx_chunk_facts(Some(2));
        let before = crate::graph::types::tx_id_now();
        let crate::QueryResult::Transacted(tx_id) = db
            .execute("(transact [[:a :n 2] [:a :n 3] [:b :n 4]])")
            .unwrap()
        else {
            panic!("not a transact result");
        };
        assert!(tx_id >= before);
        let as_of = |n: u64| {
            let q = format!("(query [:find ?v :as-of {n} :where [:a :n ?v]])");
            match db.execute(&q).unwrap() {
                crate::QueryResult::QueryResults { results, .. } => results.len(),
                _ => panic!("not query results"),
            }
        };
        assert_eq!(as_of(1), 1);
        assert_eq!(as_of(2), 3);
        let log: Vec<_> = db
            .fact_log(&FactFilter::new())
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            log.iter()
                .filter(|r| r.tx_count == 2)
                .all(|r| r.tx_id == tx_id && r.valid_from == i64::try_from(tx_id).unwrap())
        );
    }

    #[test]
    fn a_rejected_large_statement_writes_nothing_and_takes_no_tx_count() {
        let w1 = r#"{:valid-from "2020-01-01T00:00:00Z"}"#;
        let w2 = r#"{:valid-from "2021-01-01T00:00:00Z"}"#;
        let over = format!(
            r#""{}""#,
            "x".repeat(crate::storage::keys::MAX_VALUE_BYTES + 1)
        );
        let cases = [
            // Another window of a triple an earlier chunk wrote.
            (
                format!("(transact [[:a :n 1 {w1}] [:b :n 1] [:c :n 1] [:a :n 1 {w2}]])"),
                "API-011",
            ),
            // A value a checkpoint cannot store, in a later chunk.
            (
                format!("(transact [[:a :n 1] [:b :n 1] [:c :n 1] [:d :s {over}]])"),
                "WAL-003",
            ),
        ];
        for (stmt, want) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("t.graph");
            let db = Minigraf::open(&path).unwrap();
            db.execute(&workload()[0]).unwrap();
            db.checkpoint().unwrap();
            db.execute("(transact [[:h :n 1]])").unwrap();
            let before = records(&db);
            let generation = file_generation(&path);

            let small = code(db.execute(&stmt));
            assert_eq!(small, want, "small path");
            let message = db.execute(&stmt).unwrap_err().to_string();
            // The small path takes a tx_count for WAL-003 (the WAL entry
            // checks sizes after the count is allocated); the large one none.
            let tx_count = db.current_tx_count();
            let large = {
                let _chunk = set_tx_chunk_facts(Some(2));
                db.execute(&stmt).unwrap_err()
            };
            assert_eq!(large.code(), want, "large path");
            assert_eq!(large.to_string(), message, "same message");
            assert_eq!(db.current_tx_count(), tx_count, "no tx_count taken");
            assert!(records(&db) == before);
            assert!(db.verify().unwrap().is_ok());
            assert_eq!(file_generation(&path), generation, "no meta written");
            assert!(wal_of(&path).exists(), "the WAL stays");

            // The transaction stays usable, and the next checkpoint commits
            // a meta before it deletes the WAL.
            db.execute("(transact [[:i :n 1]])").unwrap();
            db.checkpoint().unwrap();
            assert_eq!(file_generation(&path), generation + 1);
            assert!(!wal_of(&path).exists());
            let after = records(&db);
            drop(db);
            let db = Minigraf::open(&path).unwrap();
            assert!(records(&db) == after);
            assert!(db.verify().unwrap().is_ok());
        }
    }

    #[test]
    fn a_rollback_alone_makes_the_next_checkpoint_write_a_meta() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        let _chunk = set_tx_chunk_facts(Some(1));
        let stmt = r#"(transact [[:a :n 1 {:valid-from "2020-01-01T00:00:00Z"}]
                        [:a :n 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#;
        assert_eq!(code(db.execute(stmt)), "API-011");
        assert_eq!(file_generation(&path), 1);
        assert!(wal_of(&path).exists(), "the WAL guard");
        db.checkpoint().unwrap();
        assert_eq!(file_generation(&path), 2, "a meta before the WAL goes");
        assert!(!wal_of(&path).exists());
    }

    #[test]
    fn a_wal_at_open_makes_the_first_checkpoint_write_a_meta() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        db.execute("(transact [[:a :n 1]])").unwrap();
        db.checkpoint().unwrap();
        db.kill();
        // A WAL every entry of which is checkpointed (none here): only its
        // header, as a large transaction's guard leaves it.
        let _ =
            crate::wal::WalWriter::open_or_create_at(&wal_of(&path), crate::db::SyncMode::Full, 2)
                .unwrap();
        let db = Minigraf::open(&path).unwrap();
        db.checkpoint().unwrap();
        assert_eq!(file_generation(&path), 3);
        assert!(!wal_of(&path).exists());
    }

    /// Spec §4.5: a large transaction that stops after writing pages of
    /// `g + 1` on a new file (one valid meta slot) reopens at `g` because its
    /// WAL guard says so; without the guard the file would be refused.
    #[test]
    fn the_wal_guard_lets_a_new_file_reopen_after_a_crash_in_a_large_transaction() {
        use crate::storage::fault::{self, Fault, Site};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        let _chunk = set_tx_chunk_facts(Some(2));
        let stmt = "(transact [[:a :n 1] [:b :n 2] [:c :n 3] [:d :n 4] [:e :n 5]])";
        // The meta page is the last page write; fail it.
        let writes = {
            let probe_dir = tempfile::tempdir().unwrap();
            let probe = Minigraf::open(probe_dir.path().join("p.graph")).unwrap();
            fault::arm(Fault::Eio, u64::MAX);
            probe.execute(stmt).unwrap();
            let n = fault::count();
            fault::disarm();
            probe.kill();
            n
        };
        // Writes: the WAL header, pages, the meta, the WAL removal.
        fault::arm(Fault::Eio, writes - 2);
        db.execute(stmt).expect_err("the meta write fails");
        assert_eq!(fault::disarm(), Some(Site::PageWrite));
        db.kill();

        let copy = dir.path().join("copy.graph");
        std::fs::copy(&path, &copy).unwrap();
        let db = Minigraf::open(&path).expect("the WAL guard says generation 1");
        assert!(records(&db).is_empty());
        assert!(db.verify().unwrap().is_ok());
        db.execute(stmt).unwrap();
        assert_eq!(records(&db).len(), 5);
        drop(db);
        let err = Minigraf::open(&copy).err().expect("no guard");
        assert_eq!(err.code(), "STG-033");
    }

    #[test]
    fn readers_see_their_snapshot_through_a_large_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let db = OpenOptions::new()
            .path(dir.path().join("t.graph"))
            .open()
            .unwrap();
        db.execute(&workload()[0]).unwrap();
        db.checkpoint().unwrap();
        db.execute("(transact [[:h :n 1]])").unwrap();
        let before = records(&db);
        let query = "(query [:find ?e ?a ?v :where [?e ?a ?v]])";
        let rows = |c: crate::Cursor| {
            let mut v: Vec<String> = c.map(|r| format!("{:?}", r.unwrap())).collect();
            v.sort();
            v
        };
        let want_rows = rows(db.query(query).unwrap());

        let cursor_before = db.query(query).unwrap();
        let log_before = db.fact_log(&FactFilter::new()).unwrap();
        let during: std::rc::Rc<RefCell<Vec<(crate::Cursor, crate::FactLog)>>> =
            std::rc::Rc::default();
        {
            let (db, during) = (db.clone(), during.clone());
            AFTER_CHUNK.with(|h| {
                *h.borrow_mut() = Some(Box::new(move || {
                    let c = db.query(query).unwrap();
                    let l = db.fact_log(&FactFilter::new()).unwrap();
                    during.borrow_mut().push((c, l));
                }))
            });
        }
        let big: String = (0..40)
            .map(|i| format!("[:x{i} :n {i}] [:a :n {i}]"))
            .collect();
        {
            let _chunk = set_tx_chunk_facts(Some(8));
            db.execute(&format!("(transact [{big}])")).unwrap();
        }
        AFTER_CHUNK.with(|h| h.borrow_mut().take());
        // Later commits must not reuse a page the readers may still read.
        for i in 0..5 {
            db.execute(&format!("(transact [[:a :n {i}] [:y{i} :n {i}]])"))
                .unwrap();
            db.checkpoint().unwrap();
        }
        let during = std::mem::take(&mut *during.borrow_mut());
        assert_eq!(during.len(), 10, "one pair per chunk");
        let log_rows = |l: crate::FactLog| {
            let mut v: Vec<String> = l
                .map(|r| {
                    let r = r.unwrap();
                    format!("{} {} {:?} {}", r.entity, r.attribute, r.value, r.tx_count)
                })
                .collect();
            v.sort();
            v
        };
        let want_log: Vec<String> = {
            let mut v: Vec<String> = before
                .iter()
                .map(|r| format!("{} {} {} {}", r.0, r.1, r.2, r.3))
                .collect();
            v.sort();
            v
        };
        assert!(rows(cursor_before) == want_rows, "cursor opened before");
        assert!(log_rows(log_before) == want_log, "log opened before");
        for (c, l) in during {
            assert!(rows(c) == want_rows, "cursor opened during");
            assert!(log_rows(l) == want_log, "log opened during");
        }
        let after = db.query(query).unwrap();
        assert!(rows(after).len() > want_rows.len(), "cursor opened after");
        assert!(db.verify().unwrap().is_ok());
    }
}
