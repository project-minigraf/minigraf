//! Incremental delivery of query results (#432).
//!
//! [`crate::Minigraf::query`] and [`crate::PreparedQuery::query`] return a
//! [`Cursor`], which hands the answer back in [`Batch`]es of rows. The answer
//! is fixed when the cursor opens: writes that commit while it is open do not
//! change it.
//!
//! Queries the streaming engine runs (#473) produce rows as they are pulled,
//! one internal batch at a time: closing or dropping the cursor stops the
//! work. Other queries compute the whole answer when the cursor opens, so
//! `max_results` and `max_derived_facts` apply exactly as they do to
//! [`crate::Minigraf::execute`]. Either way a cursor is owned (it borrows
//! nothing from the database handle), and [`Cursor::next_batch`] can fail.

use crate::error::{ErrorCode, MinigrafError, err_coded};
use crate::graph::storage::Snapshot;
use crate::graph::types::Value;
use crate::query::datalog::executor::{Answer, QueryResult};
use crate::query::engine::Stream;

/// A forward-only cursor over the rows of one query.
///
/// Pull rows with [`Cursor::next_batch`] or iterate over them; each item is one
/// row aligned with [`Cursor::vars`]. Stop early with [`Cursor::close`] or by
/// dropping the cursor.
///
/// ```
/// # use minigraf::Minigraf;
/// let db = Minigraf::in_memory().unwrap();
/// db.execute(r#"(transact [[:alice :person/name "Alice"] [:bob :person/name "Bob"]])"#)
///     .unwrap();
///
/// let mut cursor = db
///     .query("(query [:find ?name :where [?e :person/name ?name]])")
///     .unwrap();
/// assert_eq!(cursor.vars(), ["?name".to_string()].as_slice());
/// while let Some(batch) = cursor.next_batch(1000).unwrap() {
///     for row in batch.rows() {
///         println!("{:?}", row[0]);
///     }
/// }
/// ```
#[derive(Debug)]
pub struct Cursor {
    vars: Vec<String>,
    /// Rows ready to return: the whole answer, or the rest of the last batch
    /// pulled from `stream`.
    rows: std::vec::IntoIter<Vec<Value>>,
    /// The engine's operators, while they have rows to give (#544).
    stream: Option<Box<Stream>>,
    /// An error met after some rows of a batch were gathered; returned by the
    /// next call.
    failed: Option<anyhow::Error>,
    /// The state the answer is read from. It pins the committed generation,
    /// so no checkpoint reuses its pages while the cursor is open (#543).
    _snapshot: Snapshot,
}

impl Cursor {
    /// A cursor over `answer`, read from `snapshot`. A result other than rows
    /// is a write, which the callers reject before executing.
    pub(crate) fn open(answer: Answer, snapshot: Snapshot) -> anyhow::Result<Self> {
        let (vars, rows, stream) = match answer {
            Answer::Result(QueryResult::QueryResults { vars, results }) => (vars, results, None),
            Answer::Result(_) => return Err(err_coded!(ErrorCode::Api002)),
            Answer::Stream(stream) => (stream.vars().to_vec(), Vec::new(), Some(stream)),
        };
        Ok(Cursor {
            vars,
            rows: rows.into_iter(),
            stream,
            failed: None,
            _snapshot: snapshot,
        })
    }

    /// Pull the next batch of rows from the stream into `rows`. False once
    /// the stream has no more.
    fn refill(&mut self) -> anyhow::Result<bool> {
        let Some(stream) = &mut self.stream else {
            return Ok(false);
        };
        match stream.next_rows() {
            Ok(Some(rows)) => {
                self.rows = rows.into_iter();
                Ok(true)
            }
            Ok(None) => {
                self.stream = None;
                Ok(false)
            }
            Err(e) => {
                self.stream = None;
                Err(e)
            }
        }
    }

    /// The engine stream, while it has rows to give.
    #[cfg(test)]
    pub(crate) fn stream_mut(&mut self) -> Option<&mut Stream> {
        self.stream.as_deref_mut()
    }

    /// The engine's operator counters (spec §6.3); empty once the stream is
    /// done or for an answer computed at open.
    #[cfg(test)]
    pub(crate) fn counters(&self) -> Vec<crate::query::engine::Counters> {
        self.stream
            .as_ref()
            .map(|s| s.counters())
            .unwrap_or_default()
    }

    /// The variable names of the `:find` clause, in order. Every row has one
    /// value per variable.
    pub fn vars(&self) -> &[String] {
        &self.vars
    }

    /// The next batch of at most `max_rows` rows, or `None` once every row has
    /// been returned. A batch is never empty; `max_rows == 0` is treated as 1.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the database fails while rows are
    /// produced (an I/O error, or a page that fails its checks). Rows
    /// gathered before the failure are returned first, and the error by the
    /// next call; after it the cursor returns `None`.
    pub fn next_batch(&mut self, max_rows: usize) -> Result<Option<Batch>, MinigrafError> {
        if let Some(e) = self.failed.take() {
            return Err(e.into());
        }
        let max = max_rows.max(1);
        let mut rows: Vec<Vec<Value>> = Vec::new();
        loop {
            rows.extend(self.rows.by_ref().take(max - rows.len()));
            if rows.len() == max {
                break;
            }
            match self.refill() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) if rows.is_empty() => return Err(e.into()),
                Err(e) => {
                    self.failed = Some(e);
                    break;
                }
            }
        }
        if rows.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Batch { rows }))
        }
    }

    /// Stop reading: no further rows are read. Equivalent to dropping the
    /// cursor.
    pub fn close(mut self) {
        if let Some(stream) = &mut self.stream {
            stream.close();
        }
    }
}

impl Iterator for Cursor {
    type Item = Result<Vec<Value>, MinigrafError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(e) = self.failed.take() {
            return Some(Err(e.into()));
        }
        loop {
            if let Some(row) = self.rows.next() {
                return Some(Ok(row));
            }
            match self.refill() {
                Ok(true) => {}
                Ok(false) => return None,
                Err(e) => return Some(Err(e.into())),
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

/// One batch of rows from a [`Cursor`]. Each row is aligned with
/// [`Cursor::vars`].
#[derive(Debug, Clone, PartialEq)]
pub struct Batch {
    rows: Vec<Vec<Value>>,
}

impl Batch {
    /// The number of rows in this batch.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether this batch has no rows. Batches returned by
    /// [`Cursor::next_batch`] are never empty.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows of this batch.
    pub fn rows(&self) -> &[Vec<Value>] {
        &self.rows
    }

    /// Take the rows of this batch.
    pub fn into_rows(self) -> Vec<Vec<Value>> {
        self.rows
    }
}

impl IntoIterator for Batch {
    type Item = Vec<Value>;
    type IntoIter = std::vec::IntoIter<Vec<Value>>;

    fn into_iter(self) -> Self::IntoIter {
        self.rows.into_iter()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::Minigraf;
    use crate::query::engine::{BATCH_ROWS, Counters};

    const AGES: &str = "(query [:find ?e ?a :where [?e :person/age ?a]])";
    const N: usize = 5000;

    fn people(db: &Minigraf) {
        let facts: String = (0..N).map(|i| format!("[:p{i} :person/age {i}]")).collect();
        db.execute(&format!("(transact [{facts}])")).unwrap();
    }

    fn scan(counters: &[Counters]) -> Counters {
        *counters.last().expect("a scan")
    }

    /// Every row of `cursor`, `n` at a time.
    fn drain(cursor: &mut Cursor, n: usize) -> Vec<Vec<Value>> {
        let mut rows = Vec::new();
        while let Some(batch) = cursor.next_batch(n).unwrap() {
            rows.extend(batch.into_rows());
        }
        rows
    }

    fn sorted(mut rows: Vec<Vec<Value>>) -> Vec<Vec<u8>> {
        let mut out: Vec<Vec<u8>> = rows
            .drain(..)
            .map(|r| {
                r.iter()
                    .flat_map(crate::storage::index::encode_value)
                    .collect()
            })
            .collect();
        out.sort();
        out
    }

    /// One pull reads one internal batch; closing reads nothing more.
    fn first_batch_then_close(db: &Minigraf) {
        let mut cursor = db.query(AGES).unwrap();
        assert_eq!(cursor.next_batch(7).unwrap().expect("a batch").len(), 7);
        let counters = cursor.counters();
        let first = scan(&counters);
        assert!(first.rows_in > 0, "the scan read keys");
        assert!(
            first.rows_in <= 2 * BATCH_ROWS as u64,
            "one pull reads one internal batch, not the attribute"
        );
        assert_eq!(first.batches, 1);
        assert_eq!(counters[0].op, "Project");
        assert_eq!(counters[0].rows_out, first.rows_out);

        // The rest of that batch comes from the cursor's buffer.
        let rest = first.rows_out as usize - 7;
        assert_eq!(
            cursor.next_batch(rest).unwrap().expect("a batch").len(),
            rest
        );
        assert_eq!(cursor.counters(), counters, "no pull for buffered rows");

        let mut stream = cursor.stream.take().expect("an engine stream");
        stream.close();
        assert!(stream.next_rows().unwrap().is_none(), "closed");
        assert_eq!(stream.counters(), counters, "no rows read after close");
    }

    #[test]
    fn in_memory_cursor_reads_one_batch_at_a_time() {
        let db = Minigraf::in_memory().unwrap();
        people(&db);
        first_batch_then_close(&db);
    }

    #[test]
    fn file_cursor_reads_one_batch_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("lazy.graph")).unwrap();
        people(&db);
        db.checkpoint().unwrap();
        first_batch_then_close(&db);
    }

    #[test]
    fn filters_count_their_rows() {
        let db = Minigraf::in_memory().unwrap();
        people(&db);
        let mut cursor = db
            .query("(query [:find ?a :where [?e :person/age ?a] [(< ?a 10)]])")
            .unwrap();
        assert_eq!(drain(&mut cursor, 3).len(), 10);
        let mut cursor = db
            .query("(query [:find ?a :where [?e :person/age ?a] [(< ?a 10)]])")
            .unwrap();
        cursor.next_batch(1).unwrap().expect("a batch");
        let counters = cursor.counters();
        let ops: Vec<&str> = counters.iter().map(|c| c.op).collect();
        assert_eq!(ops, ["Project", "Filter", "Scan"]);
        assert_eq!(counters[1].rows_in, counters[2].rows_out);
        assert_eq!(counters[1].rows_out, counters[0].rows_in);
    }

    /// Spec §10: a lazy cursor's answer is unchanged by writes and by
    /// checkpoints that free the pages it has yet to read.
    #[test]
    fn lazy_cursor_keeps_its_answer_across_page_freeing_checkpoints() {
        let dir = tempfile::tempdir().unwrap();
        let db = Minigraf::open(dir.path().join("pinned.graph")).unwrap();
        people(&db);
        db.checkpoint().unwrap();
        let expected = sorted(drain(&mut db.query(AGES).unwrap(), 1000));
        assert_eq!(expected.len(), N);

        let mut cursor = db.query(AGES).unwrap();
        let mut rows = cursor.next_batch(7).unwrap().expect("a batch").into_rows();
        assert!(
            scan(&cursor.counters()).rows_in < N as u64,
            "the cursor has keys left to read"
        );
        // Rewrite every leaf of the attribute, several times over.
        for round in 0..4 {
            let facts: String = (0..N)
                .map(|i| format!("[:p{i} :person/age {}]", i + (round + 1) * N))
                .collect();
            db.execute(&format!("(transact [{facts}])")).unwrap();
            db.checkpoint().unwrap();
        }
        db.execute("(transact [[:late :person/age -1]])").unwrap();
        assert!(
            db.verify().unwrap().is_ok(),
            "verify clean with the cursor open"
        );
        rows.extend(drain(&mut cursor, 500));
        assert_eq!(sorted(rows), expected, "the answer at open");
    }

    #[test]
    fn in_memory_lazy_cursor_ignores_later_writes() {
        let db = Minigraf::in_memory().unwrap();
        people(&db);
        let expected = sorted(drain(&mut db.query(AGES).unwrap(), 1000));
        let mut cursor = db.query(AGES).unwrap();
        let mut rows = cursor.next_batch(7).unwrap().expect("a batch").into_rows();
        db.execute("(retract [[:p0 :person/age 0] [:p4999 :person/age 4999]])")
            .unwrap();
        db.execute("(transact [[:late :person/age -1]])").unwrap();
        rows.extend(cursor.by_ref().map(Result::unwrap));
        assert_eq!(sorted(rows), expected, "the answer at open");
    }
}
