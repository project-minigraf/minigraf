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
use crate::storage::btree::{self, LeafCursor, cow_update};
use crate::storage::cache::PageCache;
use crate::storage::dict::{DictReader, Encoded, Encoder};
use crate::storage::keys::{self, Index, KeyFact};
use crate::storage::meta::{MetaPage, slot_page};
use crate::storage::node::Entry;
use crate::storage::page::PageAllocator;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;

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
    /// The pending store's facts written as the first chunk (plan decision
    /// 27): the number of them the commit publishes.
    pending_len: usize,
    /// Whether a statement of the transaction is in the working trees: until
    /// then every key of this `tx_count` is the current statement's (spec
    /// §5.2 fast path).
    written: bool,
    /// The running statement's base, in a `WriteTransaction` (plan decision 33).
    stmt: Option<Base>,
}

/// A statement's base (spec §5.1, plan decision 28): what a failed statement
/// returns to. The ids handed out since are the allocator's
/// [`handed_since`](PageAllocator::handed_since) set.
struct Base {
    /// The working roots, counters and fact count when the statement started.
    work: MetaPage,
    /// `freed`'s length then: pages of `g` the statement replaced are
    /// referenced again by the restored roots (plan decision 31).
    freed_len: usize,
    /// Pages the base references that the statement replaced.
    held: Vec<u64>,
    /// Root of the claims tree: the triples whose base keys the statement
    /// deleted (plan decision 30).
    claims: u64,
    /// Whether the statement wrote a fact (a statement with none, like a
    /// rejected one, leaves the transaction as it was).
    wrote: bool,
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

    /// Whether a statement wrote facts into the working trees. A transaction
    /// with none commits nothing and takes no `tx_count`, as a small one.
    pub(crate) fn wrote(&self) -> bool {
        self.written
    }
}

impl<B: StorageBackend + 'static> PersistentFactStorage<B> {
    /// Make the current transaction large (spec §4.1 steps 2–4): raise the
    /// cache's generation bound to `g + 1`, reserve the next `tx_count`, build
    /// the allocator, and write the pending store's facts into the working
    /// trees (plan decision 27). The caller has created the WAL guard (step 1).
    ///
    /// The `tx_count` is the next one; it is allocated at the commit point,
    /// so a transaction that rolls back takes none (plan decision 14). An
    /// error writing the pending facts fails the handle (`STG-046`).
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
        let mut tx = LargeTx {
            base,
            work: MetaPage {
                generation: next,
                ..base
            },
            alloc,
            freed: Vec::new(),
            tx_count,
            tx_id,
            pending_len: 0,
            written: false,
            stmt: None,
        };
        // No fact joins the pending store while the write lock is held, so
        // these are the facts the commit publishes. Their own tx_counts and
        // statement rules: only encoded and inserted.
        let mut pending = self.storage.get_pending_facts();
        tx.pending_len = pending.len();
        if let Err(ChunkError::Statement(e) | ChunkError::Failure(e)) =
            self.apply_chunk(&mut tx, &mut pending, false)
        {
            self.failed = true;
            self.rollback_large(tx);
            return Err(e);
        }
        Ok(tx)
    }

    /// Write a `WriteTransaction`'s staged statements, reduced to the last
    /// statement of each triple, as one chunk with no base (plan decision 26).
    /// They passed every statement check, so any error fails the handle.
    pub(crate) fn write_staged_chunk(
        &mut self,
        tx: &mut LargeTx,
        facts: &mut [Fact],
    ) -> Result<()> {
        match self.write_large_chunk(tx, facts) {
            Ok(()) => {
                tx.written = true;
                Ok(())
            }
            Err(ChunkError::Statement(e) | ChunkError::Failure(e)) => {
                self.failed = true;
                Err(e)
            }
        }
    }

    /// Start a statement of a large `WriteTransaction` (spec §5.1): record
    /// its base.
    pub(crate) fn begin_statement(&self, tx: &mut LargeTx) {
        tx.alloc.begin_base();
        tx.stmt = Some(Base {
            work: tx.work,
            freed_len: tx.freed.len(),
            held: Vec::new(),
            claims: 0,
            wrote: false,
        });
    }

    /// End the running statement (spec §5.3). On success its base is dropped,
    /// and the pages the base held and the claims tree are released. On
    /// failure the working trees, counters and `freed` return to the base,
    /// every page the statement wrote is released, and the held ones stay in
    /// use. A failure here fails the handle.
    pub(crate) fn end_statement(&mut self, tx: &mut LargeTx, ok: bool) -> Result<()> {
        let Some(base) = tx.stmt.take() else {
            return Ok(());
        };
        let since = tx.alloc.end_base();
        if ok {
            let claims = {
                let backend = self.lock()?;
                btree::tree_pages(base.claims, &*backend, &self.page_cache)
            };
            let claims = claims.inspect_err(|_| self.failed = true)?;
            for id in claims.into_iter().chain(base.held) {
                tx.alloc.release(id);
            }
            tx.written |= base.wrote;
        } else {
            tx.work = base.work;
            tx.freed.truncate(base.freed_len);
            for id in since {
                tx.alloc.release(id);
            }
        }
        Ok(())
    }

    /// A reader of the working trees, for a query inside the transaction
    /// (spec §7): every committed fact, the pending store's, and the finished
    /// statements'. Its own dictionary cache, so the committed reader's never
    /// learns an id that is not committed. Unpinned: nothing frees a working
    /// page while the transaction holds the write lock.
    pub(crate) fn large_reader(&self, tx: &LargeTx) -> Arc<dyn crate::storage::CommittedReader> {
        Arc::new(crate::storage::reader::OnDiskReader::new(
            self.backend.clone(),
            self.page_cache.clone(),
            &MetaPage {
                last_checkpointed_tx_count: tx.tx_count,
                ..tx.work
            },
        ))
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

    /// Commit the transaction as a checkpoint (spec §4.3): the free list is
    /// written, and the meta page of `g + 1` is the commit point; the pending
    /// store's facts joined the working trees in [`Self::begin_large`]. The
    /// caller deletes the WAL next.
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
        if tx.stmt.is_some() {
            bail_coded!(ErrorCode::Int049, "commit inside a statement");
        }
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
            tx.pending_len,
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
    /// working trees. With `own`, they are this transaction's (spec §5.2): a
    /// fact the statement already wrote is kept once, another window of one
    /// of its triples is `API-011`, and an earlier statement's keys of a
    /// triple it writes are deleted.
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
        let mut deletes: [Vec<Vec<u8>>; 4] = Default::default();
        let mut claims = Vec::new();
        if own && let Some(stmt) = tx.stmt.as_mut() {
            stmt.wrote |= !facts.is_empty();
        }
        if own {
            // Without a base, or before any statement is in the working
            // trees, every key of this tx_count is the statement's.
            let base = match &tx.stmt {
                Some(b) if tx.written => Some((b.work.eavt_root, b.claims)),
                _ => None,
            };
            let r = resolve(&tx.work, base, &index[0], tx.tx_count, &*backend, cache)?;
            drop_repeats(&mut index, &r.repeats).map_err(failure)?;
            deletes = index_deletes(r.deletes).map_err(failure)?;
            claims = r.claims;
        }
        let added = u64::try_from(index[0].len())
            .map_err(|_| ChunkError::Failure(err_coded!(ErrorCode::Stg024)))?;
        let deleted = u64::try_from(deletes[0].len())
            .map_err(|_| ChunkError::Failure(err_coded!(ErrorCode::Stg024)))?;

        let mut roots = tx.work.tree_roots();
        let deletes = deletes.into_iter().chain([Vec::new()]);
        for ((root, entries), dels) in roots
            .iter_mut()
            .zip(index.into_iter().chain([dict]))
            .zip(deletes)
        {
            let mut replaced = Vec::new();
            *root = cow_update(
                *root,
                entries,
                dels,
                &mut *backend,
                cache,
                &mut tx.alloc,
                &mut replaced,
            )
            .map_err(failure)?;
            tx.replaced(replaced);
        }
        if !claims.is_empty()
            && let Some(stmt) = tx.stmt.as_mut()
        {
            let mut replaced = Vec::new();
            let entries = claims.into_iter().map(|t| (t, Vec::new())).collect();
            stmt.claims = cow_update(
                stmt.claims,
                entries,
                Vec::new(),
                &mut *backend,
                cache,
                &mut tx.alloc,
                &mut replaced,
            )
            .map_err(failure)?;
            tx.replaced(replaced);
        }
        let [eavt_root, aevt_root, avet_root, vaet_root, dict_root] = roots;
        let fact_count = tx
            .work
            .fact_count
            .checked_add(added)
            .and_then(|n| n.checked_sub(deleted))
            .ok_or_else(|| {
                ChunkError::Failure(err_coded!(ErrorCode::Int048, "fact_count overflow"))
            })?;
        tx.work = MetaPage {
            eavt_root,
            aevt_root,
            avet_root,
            vaet_root,
            dict_root,
            next_eid,
            next_iid,
            fact_count,
            ..tx.work
        };
        Ok(())
    }
}

impl LargeTx {
    /// Account for the pages a `cow_update` replaced (spec §4.2 step 3). With
    /// a base: written since it, released; not written by this transaction
    /// (generation `g` references it), freed at commit; otherwise held by the
    /// base. Without one, a page this transaction wrote is referenced by
    /// nothing now (plan decision 13).
    fn replaced(&mut self, replaced: Vec<u64>) {
        for id in replaced {
            if !self.alloc.handed(id) {
                self.freed.push(id);
            } else if let Some(stmt) = self.stmt.as_mut()
                && !self.alloc.handed_since(id)
            {
                stmt.held.push(id);
            } else {
                self.alloc.release(id);
            }
        }
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

/// What a chunk's records do to the working trees (spec §5.2).
struct Resolved {
    /// Positions in the chunk's EAVT entries of facts the statement already
    /// wrote: not inserted again.
    repeats: Vec<usize>,
    /// Earlier statements' EAVT keys of triples the chunk writes, sorted.
    deletes: Vec<Vec<u8>>,
    /// Those triples (EAVT key prefixes), sorted: they join the claims.
    claims: Vec<Vec<u8>>,
}

/// Resolve the records of a chunk (`eavt`: its sorted EAVT entries, all of
/// `tx_count`) against the working trees of `work` (spec §5.2, plan decision
/// 30). For each triple, its working keys of `tx_count` are the current
/// statement's when `base` is `None` (the fast path), when the triple is in
/// the claims (`base.1`), or when the first of them is not in the base's EAVT
/// tree (`base.0`); a fact the statement wrote is kept once, and another
/// window of an asserted triple is `API-011`, as within one statement today.
/// Otherwise they are earlier statements': deleted, and the triple is claimed.
fn resolve(
    work: &MetaPage,
    base: Option<(u64, u64)>,
    eavt: &[Entry],
    tx_count: u64,
    backend: &dyn StorageBackend,
    cache: &PageCache,
) -> std::result::Result<Resolved, ChunkError> {
    let failure = ChunkError::Failure;
    let mut out = Resolved {
        repeats: Vec::new(),
        deletes: Vec::new(),
        claims: Vec::new(),
    };
    if work.eavt_root == 0 {
        return Ok(out);
    }
    let mut cursor: Option<LeafCursor<'_>> = None;
    // The working keys of the current triple and this tx_count: one run per
    // triple, read once even when the chunk has several records of it.
    let mut start: Vec<u8> = Vec::new();
    let mut run: Vec<Vec<u8>> = Vec::new();
    let mut earlier = false;
    for (i, (key, _)) in eavt.iter().enumerate() {
        let triple = keys::triple_len(Index::Eavt, key).map_err(failure)?;
        let t = key.get(..triple).unwrap_or_default();
        let mut s = t.to_vec();
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
            earlier = match (base, run.first()) {
                (Some((base_eavt, claims)), Some(first)) => {
                    !in_tree(claims, t, backend, cache).map_err(failure)?
                        && in_tree(base_eavt, first, backend, cache).map_err(failure)?
                }
                _ => false,
            };
            if earlier {
                out.deletes.append(&mut run);
                out.claims.push(t.to_vec());
            }
        }
        if earlier {
            continue;
        }
        let asserted = key.last() == Some(&1);
        if run.iter().any(|k| k == key) {
            out.repeats.push(i);
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

/// Whether the tree at `root` (0: none) holds `key`.
fn in_tree(root: u64, key: &[u8], backend: &dyn StorageBackend, cache: &PageCache) -> Result<bool> {
    Ok(root != 0 && btree::get(root, key, backend, cache)?.is_some())
}

/// The keys of the four index trees for sorted EAVT keys `eavt`, each sorted
/// and distinct.
fn index_deletes(eavt: Vec<Vec<u8>>) -> Result<[Vec<Vec<u8>>; 4]> {
    let mut out: [Vec<Vec<u8>>; 4] = Default::default();
    for key in &eavt {
        let kf = KeyFact::decode(Index::Eavt, key)?;
        for (v, idx) in out.iter_mut().zip(Index::ALL).skip(1) {
            v.extend(kf.key(idx));
        }
    }
    for v in out.iter_mut().skip(1) {
        v.sort_unstable();
        v.dedup();
    }
    out[0] = eavt;
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

    fn outcome(r: Result<crate::QueryResult, crate::MinigrafError>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => e.code().to_string(),
        }
    }

    /// The statements of a `WriteTransaction` that statement bases decide:
    /// repeats and other windows of earlier statements' facts within and
    /// across chunks, retract-then-assert and assert-then-retract, failing
    /// statements, and a repeat of the first statement.
    fn tx_workload() -> Vec<String> {
        let w = workload();
        vec![
            w[0].clone(),
            // Statement 1's `[:a :n 2]` again, then another window of it in
            // a later chunk: API-011 (plan decision 30, the claims case).
            format!(
                r#"(transact [[:a :n 2] [:b :ref :a] [:c :s "{LONG}"]
                   [:a :n 2 {{:valid-from "2020-01-01T00:00:00Z"}}]])"#
            ),
            "(retract [[:a :n 1] [:d :k :kw]])".into(),
            "(transact [[:a :n 1] [:x :n 1] [:y :n 1]])".into(),
            "(transact [[:z :n 1] [:z :n 2] [:z :n 3]])".into(),
            "(retract [[:z :n 2] [:z :n 9] [:a :n 2]])".into(),
            // Another window of an earlier statement's fact: the last wins.
            r#"(transact [[:a :n 3 {:valid-from "2022-01-01T00:00:00Z"}] [:b :s "short"] [:e :n 1]])"#.into(),
            // API-011 within one statement, in a later chunk.
            r#"(transact [[:f :n 1 {:valid-from "2020-01-01T00:00:00Z"}] [:g :n 1] [:h :n 1]
                [:f :n 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#
                .into(),
            w[0].clone(),
            w[3].clone(),
            w[5].clone(),
            w[6].clone(),
        ]
    }

    /// Run `stmts` in one `WriteTransaction` and commit; each statement's
    /// outcome.
    fn run_tx(db: &Minigraf, stmts: &[String]) -> Vec<String> {
        let mut tx = db.begin_write().unwrap();
        let out = stmts.iter().map(|s| outcome(tx.execute(s))).collect();
        tx.commit().unwrap();
        out
    }

    #[test]
    fn large_write_transactions_store_what_small_ones_do() {
        let dir = tempfile::tempdir().unwrap();
        let stmts = tx_workload();
        let (left, right) = stmts.split_at(5);
        // One transaction of every statement, then two transactions with a
        // committed and a pending statement between them.
        let script = |db: &Minigraf| -> Vec<String> {
            let mut out = run_tx(db, &stmts);
            out.extend(run_tx(db, left));
            db.execute("(transact [[:a :n 7] [:p :n 1]])").unwrap();
            db.checkpoint().unwrap();
            db.execute("(transact [[:a :n 8] [:p :n 2]])").unwrap();
            out.extend(run_tx(db, right));
            out
        };
        let small = Minigraf::open(dir.path().join("small.graph")).unwrap();
        let want_outcomes = script(&small);
        assert!(want_outcomes.iter().filter(|o| *o == "API-011").count() >= 2);
        let want = records(&small);

        for chunk in 1..=4 {
            let path = dir.path().join(format!("large{chunk}.graph"));
            let db = Minigraf::open(&path).unwrap();
            let outcomes = {
                let _chunk = set_tx_chunk_facts(Some(chunk));
                script(&db)
            };
            assert_eq!(outcomes, want_outcomes, "outcomes at chunk {chunk}");
            assert!(records(&db) == want, "records at chunk {chunk}");
            assert_eq!(db.current_tx_count(), small.current_tx_count());
            assert!(db.verify().unwrap().is_ok(), "verify at chunk {chunk}");
            drop(db);
            let db = Minigraf::open(&path).unwrap();
            assert!(
                records(&db) == want,
                "records after reopen at chunk {chunk}"
            );
            assert!(db.verify().unwrap().is_ok());
        }
    }

    const IN_TX_QUERIES: [&str; 5] = [
        "(query [:find ?e ?a ?v :where [?e ?a ?v]])",
        "(query [:find ?v :where [:a :n ?v]])",
        "(query [:find ?e ?v :any-valid-time :where [?e :n ?v]])",
        "(query [:find ?e ?v :as-of 1 :where [?e :n ?v]])",
        "(query [:find (count ?e) :where [?e :n _] (not [?e :ref _])])",
    ];

    fn query_rows(r: crate::QueryResult) -> Vec<String> {
        match r {
            crate::QueryResult::QueryResults { results, .. } => {
                let mut v: Vec<String> = results.iter().map(|r| format!("{r:?}")).collect();
                v.sort();
                v
            }
            _ => panic!("not query results"),
        }
    }

    /// [`IN_TX_QUERIES`] inside one transaction, after each statement.
    fn tx_reads(db: &Minigraf, stmts: &[String]) -> Vec<Vec<Vec<String>>> {
        let mut tx = db.begin_write().unwrap();
        let mut out = Vec::new();
        for stmt in stmts {
            let _ = tx.execute(stmt);
            out.push(
                IN_TX_QUERIES
                    .iter()
                    .map(|q| query_rows(tx.execute(q).unwrap()))
                    .collect(),
            );
        }
        tx.rollback();
        out
    }

    #[test]
    fn queries_inside_a_transaction_see_what_its_commit_would_hold() {
        let dir = tempfile::tempdir().unwrap();
        let setup = |db: &Minigraf| {
            db.execute("(transact [[:a :n 1] [:z :n 2] [:k :n 5]])")
                .unwrap();
            db.checkpoint().unwrap();
            // Pending: a retraction and an assertion the transaction changes.
            db.execute("(retract [[:k :n 5]])").unwrap();
            db.execute("(transact [[:a :n 9]])").unwrap();
        };
        let mut stmts = tx_workload();
        stmts.insert(1, "(transact [[:k :n 5]])".into());
        stmts.insert(2, "(retract [[:a :n 9]])".into());
        // After each statement: what a commit of the statements so far holds.
        let want: Vec<Vec<Vec<String>>> = (1..=stmts.len())
            .map(|n| {
                let db = Minigraf::in_memory().unwrap();
                setup(&db);
                run_tx(&db, &stmts[..n]);
                IN_TX_QUERIES
                    .iter()
                    .map(|q| query_rows(db.execute(q).unwrap()))
                    .collect()
            })
            .collect();
        for chunk in [None, Some(1), Some(2), Some(3), Some(4)] {
            let db = Minigraf::open(dir.path().join(format!("t{chunk:?}.graph"))).unwrap();
            setup(&db);
            let got = {
                let _chunk = set_tx_chunk_facts(chunk);
                tx_reads(&db, &stmts)
            };
            assert_eq!(got.len(), want.len());
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert!(g == w, "reads after statement {i} at chunk {chunk:?}");
            }
        }
    }

    /// Spec §5.3 and plan decision 31: a statement that replaces pages of
    /// `g` and then fails leaves them referenced by the restored trees, so
    /// they must not be freed at commit.
    #[test]
    fn a_failed_statement_gives_back_the_pages_of_g_it_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        let bulk: String = (0..400)
            .map(|i| format!(r#"[:e{i} :n {i}] [:e{i} :s "value {i}"]"#))
            .collect();
        db.execute(&format!("(transact [{bulk}])")).unwrap();
        db.checkpoint().unwrap();
        let w1 = r#"{:valid-from "2020-01-01T00:00:00Z"}"#;
        let w2 = r#"{:valid-from "2021-01-01T00:00:00Z"}"#;
        let _chunk = set_tx_chunk_facts(Some(2));
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:e1 :n 10] [:e2 :n 20] [:e3 :n 30]])")
            .unwrap();
        for i in [150, 250, 350] {
            let err = tx
                .execute(&format!(
                    "(transact [[:e{i} :n 1 {w1}] [:e{i} :t 1] [:e{i} :u 1] [:e{i} :n 1 {w2}]])"
                ))
                .unwrap_err();
            assert_eq!(err.code(), "API-011");
        }
        tx.execute("(transact [[:e399 :n 0] [:e398 :n 0] [:e397 :n 0]])")
            .unwrap();
        tx.commit().unwrap();
        assert!(db.verify().unwrap().is_ok(), "no page in use is free");
        for i in 0..5 {
            db.execute(&format!("(transact [[:e{} :n -1] [:f{i} :n 1]])", i * 90))
                .unwrap();
            db.checkpoint().unwrap();
        }
        assert!(db.verify().unwrap().is_ok(), "no page handed out twice");
    }

    #[test]
    fn a_rolled_back_large_write_transaction_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        db.execute(&workload()[0]).unwrap();
        db.checkpoint().unwrap();
        db.execute("(transact [[:h :n 1]])").unwrap();
        let before = records(&db);
        let generation = db.generation();
        {
            let _chunk = set_tx_chunk_facts(Some(2));
            let mut tx = db.begin_write().unwrap();
            for stmt in tx_workload() {
                let _ = tx.execute(&stmt);
            }
            drop(tx);
        }
        assert!(records(&db) == before);
        assert_eq!(db.generation(), generation, "no meta written");
        assert!(wal_of(&path).exists(), "the WAL guard stays");
        db.checkpoint().unwrap();
        assert_eq!(db.generation(), generation + 1);
        assert!(records(&db) == before);
        assert!(db.verify().unwrap().is_ok(), "every page accounted for");
        drop(db);
        let db = Minigraf::open(&path).unwrap();
        assert!(records(&db) == before);
    }

    /// Spec §4.1 step 5: a staged window that has ended when the transaction
    /// becomes large rejects the statement that makes it large, and the
    /// transaction stays small.
    #[test]
    fn a_staged_window_ended_at_the_switch_rejects_the_statement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        let _chunk = set_tx_chunk_facts(Some(2));
        let mut tx = db.begin_write().unwrap();
        // Timestamps are whole seconds: one to two seconds from now.
        let ends = (crate::graph::types::tx_id_now().cast_signed() / 1000 + 2) * 1000;
        let ends_text = crate::temporal::millis_to_timestamp_string(ends).unwrap();
        tx.execute(&format!(
            r#"(transact [[:a :n 1 {{:valid-to "{ends_text}"}}]])"#
        ))
        .unwrap();
        while crate::graph::types::tx_id_now().cast_signed() <= ends {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let err = tx.execute("(transact [[:b :n 1] [:c :n 1]])").unwrap_err();
        assert_eq!(err.code(), "API-019");
        assert!(!wal_of(&path).exists(), "nothing to undo: no WAL guard");
        // Still small and usable: a one-fact statement is staged.
        tx.execute("(transact [[:d :n 1]])").unwrap();
        assert_eq!(tx.commit().unwrap_err().code(), "API-019");
        assert!(records(&db).is_empty());
    }

    /// A large transaction whose statements all failed commits nothing and
    /// takes no `tx_count`, as a small one with nothing staged.
    #[test]
    fn a_large_write_transaction_with_no_statement_takes_no_tx_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        db.execute("(transact [[:a :n 1]])").unwrap();
        let before = records(&db);
        let _chunk = set_tx_chunk_facts(Some(2));
        let mut tx = db.begin_write().unwrap();
        let stmt = r#"(transact [[:b :n 1 {:valid-from "2020-01-01T00:00:00Z"}] [:c :n 1]
                        [:d :n 1] [:b :n 1 {:valid-from "2021-01-01T00:00:00Z"}]])"#;
        assert_eq!(tx.execute(stmt).unwrap_err().code(), "API-011");
        assert!(wal_of(&path).exists());
        tx.commit().unwrap();
        assert_eq!(db.current_tx_count(), 1);
        assert!(records(&db) == before);
        db.checkpoint().unwrap();
        assert!(db.verify().unwrap().is_ok());
    }

    #[test]
    fn a_failed_handle_is_refused_when_a_transaction_would_become_large() {
        use crate::storage::fault::{self, Fault};
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("t.graph")).unwrap();
        db.execute("(transact [[:a :n 1]])").unwrap();
        fault::arm(Fault::Eio, 0);
        db.checkpoint().expect_err("the checkpoint fails");
        fault::disarm();
        let _chunk = set_tx_chunk_facts(Some(2));
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:b :n 1]])").unwrap();
        let err = tx.execute("(transact [[:c :n 1] [:d :n 1]])").unwrap_err();
        assert_eq!(err.code(), "STG-046");
        assert_eq!(tx.commit().unwrap_err().code(), "STG-046");
    }

    #[test]
    fn a_failure_in_a_large_write_transaction_fails_its_commit() {
        use crate::storage::fault::{self, Fault};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.graph");
        let db = Minigraf::open(&path).unwrap();
        db.execute(&workload()[0]).unwrap();
        db.checkpoint().unwrap();
        let before = records(&db);
        let _chunk = set_tx_chunk_facts(Some(2));
        let mut tx = db.begin_write().unwrap();
        tx.execute("(transact [[:b :n 1] [:c :n 1] [:d :n 1]])")
            .unwrap();
        fault::arm(Fault::Eio, 0);
        tx.execute("(transact [[:e :n 1] [:f :n 1] [:g :n 1]])")
            .expect_err("a page write fails");
        fault::disarm();
        assert_eq!(tx.commit().unwrap_err().code(), "STG-046");
        db.kill();
        let db = Minigraf::open(&path).unwrap();
        assert!(records(&db) == before);
        assert!(db.verify().unwrap().is_ok());
    }

    /// `OpenOptions::transaction_chunk_facts` sets the chunk size, kept
    /// between 1 and the most facts a WAL entry holds.
    #[test]
    fn the_chunk_size_is_an_open_option() {
        use crate::query::datalog::parser::statement::{
            DEFAULT_TX_CHUNK_FACTS, MAX_TX_CHUNK_FACTS, tx_chunk_facts,
        };
        assert_eq!(OpenOptions::default().transaction_chunk_facts, 65_536);
        assert_eq!(DEFAULT_TX_CHUNK_FACTS, 65_536);
        assert_eq!(tx_chunk_facts(0), 1);
        assert_eq!(tx_chunk_facts(usize::MAX), MAX_TX_CHUNK_FACTS);
        assert_eq!(MAX_TX_CHUNK_FACTS, crate::wal::MAX_FACTS_PER_ENTRY);
        let dir = tempfile::tempdir().unwrap();
        for (chunk, large) in [(2, true), (3, false), (0, true)] {
            let path = dir.path().join(format!("t{chunk}.graph"));
            let db = OpenOptions::new()
                .transaction_chunk_facts(chunk)
                .path(&path)
                .open()
                .unwrap();
            db.execute("(transact [[:a :n 1] [:b :n 2] [:c :n 3]])")
                .unwrap();
            // A large statement commits as a checkpoint and deletes the WAL.
            assert_eq!(!wal_of(&path).exists(), large, "chunk {chunk}");
            let mut tx = db.begin_write().unwrap();
            tx.execute("(transact [[:d :n 1] [:e :n 2]])").unwrap();
            tx.execute("(transact [[:f :n 1] [:g :n 2]])").unwrap();
            tx.commit().unwrap();
            assert_eq!(records(&db).len(), 7);
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
            let generation = db.generation();

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
            assert_eq!(db.generation(), generation, "no meta written");
            assert!(wal_of(&path).exists(), "the WAL stays");

            // The transaction stays usable, and the next checkpoint commits
            // a meta before it deletes the WAL.
            db.execute("(transact [[:i :n 1]])").unwrap();
            db.checkpoint().unwrap();
            assert_eq!(db.generation(), generation + 1);
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
        assert_eq!(db.generation(), 1);
        assert!(wal_of(&path).exists(), "the WAL guard");
        db.checkpoint().unwrap();
        assert_eq!(db.generation(), 2, "a meta before the WAL goes");
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
        assert_eq!(db.generation(), 3);
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
