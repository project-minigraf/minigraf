//! Incremental delivery of query results (#432).
//!
//! [`crate::Minigraf::query`] and [`crate::PreparedQuery::query`] return a
//! [`Cursor`], which hands the answer back in [`Batch`]es of rows. The answer
//! is fixed when the cursor opens: writes that commit while it is open do not
//! change it.
//!
//! The engine behind the cursor currently computes the whole answer when the
//! cursor opens, so `max_results` and `max_derived_facts` apply exactly as they
//! do to [`crate::Minigraf::execute`]. The API is shaped for a streaming engine
//! that produces rows as they are pulled, which is why a cursor is owned (it
//! borrows nothing from the database handle) and why
//! [`Cursor::next_batch`] can fail.

use crate::error::{ErrorCode, MinigrafError, err_coded};
use crate::graph::types::Value;
use crate::query::datalog::executor::QueryResult;

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
    rows: std::vec::IntoIter<Vec<Value>>,
}

impl Cursor {
    /// Wrap the answer of a query. Any other result is a write, which the
    /// callers reject before executing.
    pub(crate) fn from_result(result: QueryResult) -> anyhow::Result<Self> {
        match result {
            QueryResult::QueryResults { vars, results } => Ok(Cursor {
                vars,
                rows: results.into_iter(),
            }),
            _ => Err(err_coded!(ErrorCode::Api002)),
        }
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
    /// Reserved for failures while rows are produced. The current engine
    /// computes the answer when the cursor opens and never fails here.
    pub fn next_batch(&mut self, max_rows: usize) -> Result<Option<Batch>, MinigrafError> {
        let rows: Vec<Vec<Value>> = self.rows.by_ref().take(max_rows.max(1)).collect();
        if rows.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Batch { rows }))
        }
    }

    /// Stop reading. Equivalent to dropping the cursor.
    pub fn close(self) {}
}

impl Iterator for Cursor {
    type Item = Result<Vec<Value>, MinigrafError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows.next().map(Ok)
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
