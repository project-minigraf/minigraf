//! Test-only fault plan for the file and WAL I/O paths (#390).
//!
//! Every write and sync that reaches the disk through `FileBackend`, the WAL
//! writer and `sync_parent_dir` asks [`on`] first. With a plan armed on the
//! calling thread, the `at`-th call of the plan's kind (writes or syncs,
//! counted from 0) fails the way [`Fault`] says. A [`Fault::Enospc`] plan
//! keeps failing every later write too, like a full disk. With no plan
//! armed, [`on`] returns [`Action::Proceed`] and the call runs as usual.
//!
//! The plan is thread-local, so tests running in parallel do not see each
//! other's faults; a `Minigraf` call runs its I/O on the calling thread.

use std::cell::RefCell;
use std::io;

/// Where an I/O call happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// `FileBackend::write_page`.
    PageWrite,
    /// `FileBackend::sync`.
    PageSync,
    /// A write to the WAL: its header or an entry.
    WalWrite,
    /// Cutting a torn tail off the WAL (`set_len`).
    WalTruncate,
    /// An fsync of the WAL.
    WalSync,
    /// Removing the WAL after a checkpoint.
    WalRemove,
    /// `sync_parent_dir`.
    DirSync,
}

impl Site {
    fn is_write(self) -> bool {
        matches!(
            self,
            Site::PageWrite | Site::WalWrite | Site::WalTruncate | Site::WalRemove
        )
    }
}

/// How the chosen call fails.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// The write fails with EIO and writes nothing.
    Eio,
    /// The write stores its first `n` bytes, then fails with EIO: a short
    /// write, or a torn page.
    Torn(usize),
    /// The write fails with ENOSPC and writes nothing, and so does every
    /// later write (removing a file still works).
    Enospc,
    /// The sync fails with EIO; what was written stays in the file.
    SyncEio,
    /// The sync fails with EIO and the writes since that file's last good
    /// sync are lost, as when the kernel drops dirty pages after a failed
    /// fsync (PostgreSQL's "fsyncgate").
    SyncLost,
}

impl Fault {
    fn targets(self, site: Site) -> bool {
        match self {
            Fault::Eio | Fault::Torn(_) => site.is_write(),
            Fault::Enospc => site.is_write() && site != Site::WalRemove,
            Fault::SyncEio | Fault::SyncLost => !site.is_write(),
        }
    }
}

/// What the call site must do.
#[derive(Debug)]
pub(crate) enum Action {
    /// Run the call as usual.
    Proceed,
    /// Fail with this error, writing nothing.
    Fail(io::Error),
    /// Write the first `n` bytes, then fail with EIO.
    Tear(usize),
    /// Throw away the writes since the last good sync, then fail with EIO.
    Lose,
}

struct Plan {
    fault: Fault,
    at: u64,
    seen: u64,
    tripped: Option<Site>,
}

thread_local! {
    static PLAN: RefCell<Option<Plan>> = const { RefCell::new(None) };
}

/// Arm `fault` at the `at`-th call it targets on this thread.
pub(crate) fn arm(fault: Fault, at: u64) {
    PLAN.with(|p| {
        *p.borrow_mut() = Some(Plan {
            fault,
            at,
            seen: 0,
            tripped: None,
        })
    });
}

/// Remove the plan. Returns the site where it fired, if it did.
pub(crate) fn disarm() -> Option<Site> {
    PLAN.with(|p| p.borrow_mut().take().and_then(|plan| plan.tripped))
}

/// Whether a plan is armed on this thread. Call sites keep what they need
/// to undo unsynced writes ([`Fault::SyncLost`]) only then.
pub(crate) fn armed() -> bool {
    PLAN.with(|p| p.borrow().is_some())
}

/// How many calls the armed plan has targeted so far.
pub(crate) fn count() -> u64 {
    PLAN.with(|p| p.borrow().as_ref().map_or(0, |plan| plan.seen))
}

/// An EIO for a failed call.
pub(crate) fn eio() -> io::Error {
    io::Error::other("fault injection: EIO")
}

/// Ask the plan what the call at `site` must do.
pub(crate) fn on(site: Site) -> Action {
    PLAN.with(|p| {
        let mut p = p.borrow_mut();
        let Some(plan) = p.as_mut() else {
            return Action::Proceed;
        };
        if !plan.fault.targets(site) {
            return Action::Proceed;
        }
        if plan.fault == Fault::Enospc && plan.tripped.is_some() {
            return Action::Fail(io::Error::from(io::ErrorKind::StorageFull));
        }
        let n = plan.seen;
        plan.seen = n.saturating_add(1);
        if n != plan.at {
            return Action::Proceed;
        }
        plan.tripped = Some(site);
        match plan.fault {
            Fault::Eio | Fault::SyncEio => Action::Fail(eio()),
            Fault::Torn(bytes) => Action::Tear(bytes),
            Fault::Enospc => Action::Fail(io::Error::from(io::ErrorKind::StorageFull)),
            Fault::SyncLost => Action::Lose,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_plan_proceeds() {
        disarm();
        assert!(matches!(on(Site::PageWrite), Action::Proceed));
        assert!(!armed());
    }

    #[test]
    fn write_fault_counts_writes_only() {
        arm(Fault::Eio, 1);
        assert!(matches!(on(Site::PageWrite), Action::Proceed));
        assert!(matches!(on(Site::PageSync), Action::Proceed));
        assert!(matches!(on(Site::WalWrite), Action::Fail(_)));
        assert!(matches!(on(Site::WalWrite), Action::Proceed));
        assert_eq!(disarm(), Some(Site::WalWrite));
    }

    #[test]
    fn sync_fault_counts_syncs_only() {
        arm(Fault::SyncLost, 1);
        assert!(matches!(on(Site::DirSync), Action::Proceed));
        assert!(matches!(on(Site::PageWrite), Action::Proceed));
        assert!(matches!(on(Site::PageSync), Action::Lose));
        assert_eq!(disarm(), Some(Site::PageSync));
    }

    #[test]
    fn enospc_keeps_failing_writes_but_not_removes() {
        arm(Fault::Enospc, 0);
        assert!(matches!(on(Site::WalRemove), Action::Proceed));
        assert!(matches!(on(Site::PageWrite), Action::Fail(_)));
        assert!(matches!(on(Site::WalWrite), Action::Fail(_)));
        assert!(matches!(on(Site::PageSync), Action::Proceed));
        assert!(matches!(on(Site::WalRemove), Action::Proceed));
        assert_eq!(disarm(), Some(Site::PageWrite));
    }

    #[test]
    fn plan_that_never_fires_reports_none() {
        arm(Fault::Torn(10), 5);
        assert!(matches!(on(Site::PageWrite), Action::Proceed));
        assert_eq!(disarm(), None);
    }
}
