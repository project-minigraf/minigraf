//! `AntiJoin` (#546 spec §4): `not` and `not-join`. An outer row is kept
//! when its body has no row. It filters; it never deduplicates.
//!
//! A correlated body runs once per outer row, from a [`Seed`](super::seed::Seed)
//! of the cells it imports, and stops at its first row. A hashed body runs
//! once, on the first pull; its rows' keys (their cells of the imported
//! variables, as [`JoinKey`]s) form a set the outer rows are looked up in.

use super::batch::Batch;
use super::cell::{Cell, JoinKey};
use super::meter::{HASH_ENTRY_OVERHEAD, Held, Meter, growth, keys_bytes};
use super::plan::{Body as Spec, Key};
use super::{Counters, Ctx, Operator, build};
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) struct AntiJoin {
    outer: Box<dyn Operator>,
    body: Body,
    /// A hashed body's set (#549 spec §3.2), counted as long as the set is
    /// kept: until the operator drops.
    held: Held,
    /// `rows_in` counts the outer rows received.
    counters: Counters,
}

enum Body {
    Correlated {
        root: Box<dyn Operator>,
        imports: Vec<usize>,
        seed: Vec<Cell>,
    },
    Hashed {
        root: Box<dyn Operator>,
        keys: Vec<Key>,
        /// Built on the first pull, then kept: the body does not depend on
        /// the outer row, so a rewind keeps it too.
        set: Option<Set>,
    },
}

/// The keys of a hashed body's rows, and whether it has any row.
struct Set {
    keys: HashSet<Vec<JoinKey>>,
    any: bool,
}

impl AntiJoin {
    pub(crate) fn new(outer: Box<dyn Operator>, spec: Spec, meter: &Arc<Meter>) -> Self {
        let body = match spec {
            Spec::Correlated { plan, imports } => Body::Correlated {
                root: build(plan.source, plan.steps, meter),
                imports,
                seed: Vec::new(),
            },
            Spec::Hashed { plan, keys } => Body::Hashed {
                root: build(plan.source, plan.steps, meter),
                keys,
                set: None,
            },
        };
        AntiJoin {
            outer,
            body,
            held: Held::new(meter, "AntiJoin"),
            counters: Counters::new("AntiJoin"),
        }
    }
}

/// The cell of row `i` in `slot`.
fn cell(batch: &Batch, slot: usize, i: usize) -> Option<&Cell> {
    batch.cols.get(slot).and_then(|c| c.get(i))
}

impl Body {
    /// Whether the body has a row for row `i` of `batch`.
    fn matches(
        &mut self,
        batch: &Batch,
        i: usize,
        held: &mut Held,
        ctx: &mut Ctx<'_>,
    ) -> Result<bool> {
        match self {
            Body::Correlated {
                root,
                imports,
                seed,
            } => {
                seed.clear();
                for &slot in imports.iter() {
                    // Every imported slot is filled.
                    if let Some(c) = cell(batch, slot, i) {
                        seed.push(c.clone());
                    }
                }
                root.rewind(seed);
                let hit = root.next_batch(ctx)?.is_some();
                root.close();
                Ok(hit)
            }
            Body::Hashed { root, keys, set } => {
                if set.is_none() {
                    *set = Some(collect(root.as_mut(), keys, held, ctx)?);
                }
                let Some(set) = set else {
                    return Ok(false);
                };
                if keys.is_empty() {
                    return Ok(set.any);
                }
                let mut key = Vec::with_capacity(keys.len());
                for k in keys.iter() {
                    let Some(c) = cell(batch, k.outer, i) else {
                        return Ok(false);
                    };
                    // A NaN equals nothing a body row holds.
                    let Some(j) = JoinKey::of(c, k.normalise, ctx)? else {
                        return Ok(false);
                    };
                    key.push(j);
                }
                Ok(set.keys.contains(&key))
            }
        }
    }

    fn root(&self) -> &dyn Operator {
        match self {
            Body::Correlated { root, .. } | Body::Hashed { root, .. } => root.as_ref(),
        }
    }

    fn root_mut(&mut self) -> &mut dyn Operator {
        match self {
            Body::Correlated { root, .. } | Body::Hashed { root, .. } => root.as_mut(),
        }
    }
}

/// Every row of a hashed body, as the keys of `keys`' inner slots.
fn collect(
    root: &mut dyn Operator,
    keys: &[Key],
    held: &mut Held,
    ctx: &mut Ctx<'_>,
) -> Result<Set> {
    let mut set = Set {
        keys: HashSet::new(),
        any: false,
    };
    while let Some(b) = root.next_batch(ctx)? {
        set.any = true;
        if keys.is_empty() {
            break;
        }
        'row: for i in 0..b.len {
            let mut key = Vec::with_capacity(keys.len());
            for k in keys {
                let Some(c) = cell(&b, k.inner, i) else {
                    continue 'row;
                };
                // A row with a NaN key equals no outer row.
                let Some(j) = JoinKey::of(c, k.normalise, ctx)? else {
                    continue 'row;
                };
                key.push(j);
            }
            if set.keys.contains(&key) {
                continue;
            }
            let n = keys_bytes(&key) + HASH_ENTRY_OVERHEAD;
            held.charge(n)?;
            let refused = growth(set.keys.capacity(), n);
            held.reserve(set.keys.try_reserve(1), n, refused)?;
            set.keys.insert(key);
        }
    }
    root.close();
    Ok(set)
}

impl Operator for AntiJoin {
    fn next_batch(&mut self, ctx: &mut Ctx<'_>) -> Result<Option<Batch>> {
        while let Some(mut batch) = self.outer.next_batch(ctx)? {
            self.counters.rows_in += batch.len as u64;
            let mut keep = Vec::with_capacity(batch.len);
            for i in 0..batch.len {
                keep.push(!self.body.matches(&batch, i, &mut self.held, ctx)?);
            }
            batch.retain(&keep);
            if batch.len > 0 {
                self.counters.out(&batch);
                return Ok(Some(batch));
            }
        }
        Ok(None)
    }

    fn rewind(&mut self, seed: &[Cell]) {
        self.outer.rewind(seed);
    }

    fn close(&mut self) {
        self.outer.close();
        self.body.root_mut().close();
    }

    fn counters(&self, out: &mut Vec<Counters>) {
        out.push(Counters {
            bytes_held: self.held.bytes() as u64,
            ..self.counters
        });
        self.body.root().counters(out);
        self.outer.counters(out);
    }
}
