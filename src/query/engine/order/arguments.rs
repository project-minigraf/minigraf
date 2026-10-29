//! Argument domains are independent of the grouping order. Proof reads use
//! raw history from the same pinned reader, never a sample of visible rows.
use super::*;

pub(super) struct Arguments<'a> {
    /// None for the speculative demand pass, which must perform no I/O.
    pub(super) reader: Option<(&'a Snapshot, &'a dyn KeySource)>,
    pub(super) source: (u32, Kinds),
    pub(super) cache: Vec<(u32, Kinds)>,
}

impl Kinds {
    pub(super) fn stored(kind: ValueKind) -> Self {
        // Keep the mapping exhaustive if storage gains another value kind.
        let bit = match kind {
            ValueKind::Null => 0,
            ValueKind::Boolean => 1,
            ValueKind::Integer => 2,
            ValueKind::Float => 3,
            ValueKind::String => 4,
            ValueKind::Keyword => 5,
            ValueKind::Ref => 6,
            ValueKind::Set => 7,
        };
        Self(1 << bit)
    }

    pub(super) fn group_domain(self) -> Option<Domain> {
        Some(match self.0 {
            2 => Domain::Single(ValueKind::Boolean),
            4 => Domain::Single(ValueKind::Integer),
            8 => Domain::Single(ValueKind::Float),
            12 => Domain::Numbers,
            32 => Domain::Single(ValueKind::Keyword),
            64 => Domain::Single(ValueKind::Ref),
            _ => return None,
        })
    }
}

impl Arguments<'_> {
    fn scan(&mut self, spec: &ScanSpec, slot: usize) -> Result<Option<Kinds>> {
        if spec.derived.is_some() {
            return Ok(None);
        }
        if let Some(&(meta, _)) = spec.meta.iter().find(|&&(_, s)| s == slot) {
            return Ok(Some(if meta == Meta::ValidAt {
                Kinds(Kinds::INTEGER.0 | Kinds::NULL.0)
            } else {
                Kinds::INTEGER
            }));
        }
        let (Term::Const(attribute), Term::Bind(value)) = (&spec.a, &spec.v) else {
            return Ok(None);
        };
        if *value != slot {
            return Ok(None);
        }
        let Some((snapshot, keys)) = self.reader else {
            // A syntactically provable origin is enough for the demand pass;
            // the real pass must replace this with the raw stored domain.
            return Ok(Some(Kinds::INTEGER));
        };
        if snapshot.has_pending(attribute, None)? {
            return Ok(None);
        }
        let Some(attribute) = keys.iid_of(attribute)? else {
            return Ok(Some(Kinds(0)));
        };
        if self.source.0 == attribute {
            return Ok(Some(self.source.1));
        }
        if let Some(&(_, kinds)) = self.cache.iter().find(|&&(a, _)| a == attribute) {
            return Ok(Some(kinds));
        }
        let kinds = value_kinds(keys, attribute)?;
        self.cache.push((attribute, kinds));
        Ok(Some(kinds))
    }

    fn kind(
        &mut self,
        source: &Source,
        steps: &[Step],
        mut slot: usize,
        proof: &Ordering,
    ) -> Result<Option<Kinds>> {
        if let Some(kind) = proof.constant_kind(slot) {
            return Ok(Some(kind));
        }
        if proof.ordered_slot(slot) {
            return Ok(Some(self.source.1));
        }
        let mut scan = match source {
            Source::Scan(scan) => scan,
            Source::Seed => return Ok(None),
        };
        // Trace only unchanged columns. A new entity-probe value may use a
        // whole-attribute superset; a replacement or dynamic expression may not.
        for step in steps.iter().rev() {
            match step {
                Step::Bind(_, _, out, mode) if *out == slot && *mode != Out::Check => {
                    return Ok(None);
                }
                Step::Pseudo {
                    col,
                    value: Term::Bind(out),
                } if *out == slot => slot = *col,
                Step::Join(join) => {
                    if join.replace.iter().any(|&(_, out)| out == slot) {
                        return Ok(None);
                    }
                    if let Some(at) = slot.checked_sub(join.outer_width) {
                        if let Some(&inner) = join.new.get(at)
                            && matches!(join.kind, JoinKind::Probe(_))
                        {
                            scan = &join.inner;
                            slot = inner;
                            break;
                        }
                        return Ok(None);
                    }
                }
                Step::Union(_)
                | Step::CollectSet(_)
                | Step::Aggregate { .. }
                | Step::Window { .. }
                | Step::Sort { .. } => return Ok(None),
                _ => {}
            }
        }
        self.scan(scan, slot)
    }

    pub(super) fn accepts(
        &mut self,
        aggs: &[(String, usize)],
        source: &Source,
        steps: &[Step],
        proof: &Ordering,
        functions: &FunctionRegistry,
    ) -> Result<bool> {
        for (func, slot) in aggs {
            let Some(desc) = functions.get(func) else {
                return Ok(false);
            };
            let allowed = match desc.impl_ {
                AggImpl::Builtin(
                    Builtin::Count | Builtin::CountDistinct | Builtin::Avg | Builtin::Sum,
                ) => continue,
                AggImpl::Builtin(Builtin::Min | Builtin::Max) => {
                    Kinds::INTEGER.0 | Kinds::FLOAT.0 | Kinds::BOOLEAN.0 | Kinds::NULL.0
                }
                AggImpl::Builtin(Builtin::SumDistinct) => Kinds::INTEGER.0 | Kinds::NULL.0,
                _ => return Ok(false),
            };
            if !self
                .kind(source, steps, *slot, proof)?
                .is_some_and(|k| k.within(allowed))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::engine::plan::{JoinSpec, Valid};

    fn scan() -> ScanSpec {
        ScanSpec {
            index: None,
            e: Term::Bind(0),
            a: Term::Const(":arg".into()),
            v: Term::Bind(1),
            meta: vec![(Meta::TxCount, 2), (Meta::ValidAt, 3)],
            valid: Valid::Any,
            width: 4,
            derived: None,
        }
    }

    fn join(kind: JoinKind, replace: Vec<(usize, usize)>) -> Step {
        Step::Join(JoinSpec {
            inner: scan(),
            kind,
            keys: vec![],
            new: vec![1, 2, 3],
            replace,
            outer_width: 4,
        })
    }

    #[test]
    fn argument_origin_rejects_replacements_and_unproved_join_orientations() {
        let spec = scan();
        let proof = Ordering::source(&spec, Domain::Single(ValueKind::Integer));
        let source = Source::Scan(spec);
        let mut args = Arguments {
            reader: None,
            source: (1, Kinds::INTEGER),
            cache: vec![],
        };
        for kind in [
            JoinKind::ProbeValue {
                key: 0,
                build_inner: true,
            },
            JoinKind::ProbeValue {
                key: 0,
                build_inner: false,
            },
            JoinKind::Hash { build_inner: true },
            JoinKind::Hash { build_inner: false },
        ] {
            assert!(
                args.kind(&source, &[join(kind, vec![])], 4, &proof)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            args.kind(
                &source,
                &[join(JoinKind::Probe(0), vec![(0, 2)])],
                2,
                &proof
            )
            .unwrap()
            .is_none(),
            "a replaced integer metadata slot cannot reuse its original domain"
        );
        let mut dynamic = join(JoinKind::Probe(0), vec![]);
        if let Step::Join(j) = &mut dynamic {
            j.inner.a = Term::Any;
        }
        assert!(args.kind(&source, &[dynamic], 4, &proof).unwrap().is_none());
        let mut derived = join(JoinKind::Probe(0), vec![]);
        if let Step::Join(j) = &mut derived {
            j.inner.derived = Some(crate::query::engine::relation::Derived {
                rels: vec![0],
                range: crate::query::engine::relation::Range::Full,
            });
        }
        assert!(args.kind(&source, &[derived], 4, &proof).unwrap().is_none());
    }

    #[test]
    fn argument_origin_traces_metadata_and_checks_but_not_dynamic_bindings() {
        let spec = scan();
        let proof = Ordering::source(&spec, Domain::Single(ValueKind::Integer));
        let source = Source::Scan(spec);
        let mut args = Arguments {
            reader: None,
            source: (1, Kinds::INTEGER),
            cache: vec![],
        };
        for (slot, expected) in [(5, Kinds::INTEGER.0), (6, Kinds::INTEGER.0 | Kinds::NULL.0)] {
            let steps = [
                join(JoinKind::Probe(0), vec![]),
                Step::Pseudo {
                    col: slot,
                    value: Term::Bind(7),
                },
            ];
            assert_eq!(
                args.kind(&source, &steps, 7, &proof).unwrap().unwrap().0,
                expected,
                "new-slot mapping resolves the correct inner metadata column"
            );
        }
        for mode in [Out::New, Out::Replace, Out::Check] {
            let steps = [
                join(JoinKind::Probe(0), vec![]),
                Step::Bind(
                    Expr::Var("?dynamic".into()),
                    vec![("?dynamic".into(), 0)],
                    4,
                    mode,
                ),
            ];
            assert_eq!(
                args.kind(&source, &steps, 4, &proof).unwrap().is_some(),
                mode == Out::Check
            );
        }
    }
}
