//! #324: kernel file locking must behave correctly on network filesystems.
//!
//! `flock` over NFS is emulated by `fcntl` on Linux since 2.6.37, which works
//! on NFSv4 but needs `lockd` on NFSv3. The failure mode that matters is a
//! lock that silently no-ops rather than erroring: the new code would believe
//! it holds an exclusive lock while a second process holds the same file.
//!
//! The NFS-mount tests below need a loopback NFS export, which needs root and
//! is provisioned by `.github/workflows/nfs-lock.yml`, not by this file. They
//! skip cleanly when the environment variables they need are unset, so plain
//! `cargo test` never touches them.
//!
//! `MINIGRAF_NFS_TEST_DIR_V3_NOLOCK` is intentionally not set by that
//! workflow (#334): the `nolock` mount option is a documented-unsupported
//! deployment (docs/ERROR_REFERENCE.md STG-027), not a case CI gates on.
//! The three tests gated on it below are kept for local reproduction against
//! a manually mounted `-o nolock` export; nightly CI always skips them.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Path to `examples/pid_ns_helper`.
///
/// `MINIGRAF_PID_NS_HELPER`, if set, is used as-is. Needed under `cargo
/// llvm-cov`, whose instrumented build puts test binaries where the
/// `examples/`-relative lookup below fails, so the helper has to live in
/// a target dir it never touches; `.github/workflows/coverage-gates.yml`
/// builds it there and sets this variable. Otherwise resolved relative to the
/// test binary at `target/debug/deps/nfs_lock_test-<hash>`, the same shape as
/// `tests/pid_namespace_test.rs::helper_binary`.
fn helper_binary() -> PathBuf {
    if let Ok(path) = std::env::var("MINIGRAF_PID_NS_HELPER") {
        return PathBuf::from(path);
    }
    let mut dir = std::env::current_exe().expect("test binary path");
    dir.pop(); // deps/
    dir.pop(); // debug/
    dir.join("examples").join("pid_ns_helper")
}

fn require_helper_built(helper: &Path) {
    assert!(
        helper.exists(),
        "helper binary not found at {}; run `cargo build --examples` first",
        helper.display()
    );
}

fn run_helper(helper: &Path, db: &Path, mode: &str) -> Output {
    Command::new(helper)
        .arg(db)
        .arg(mode)
        .output()
        .expect("run helper")
}

/// The helper must reject an unrecognized mode string rather than silently
/// falling through to a plain open. Without this, a typo'd or unimplemented
/// mode (like `open-unlocked` before it exists) is indistinguishable from
/// `open` — exactly the false-green trap this guards against.
#[test]
fn test_unrecognized_mode_fails_loudly_instead_of_falling_through() {
    let helper = helper_binary();
    require_helper_built(&helper);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bogus-mode.graph");

    let out = run_helper(&helper, &db, "not-a-real-mode");
    assert!(
        out.status.code().unwrap_or(0) != 0,
        "an unrecognized mode must not exit successfully"
    );
}

/// The `open-unlocked` mode must work on an ordinary filesystem, independent
/// of any NFS provisioning. This is the part of #324 this sandbox can
/// actually verify: that `allow_unlocked` opts in correctly when asked, on a
/// mount where locking works fine. The NFS tests below cover the fail-closed
/// side, which needs a mount where locking does NOT work.
#[test]
fn test_open_unlocked_mode_succeeds_on_an_ordinary_filesystem() {
    let helper = helper_binary();
    require_helper_built(&helper);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("plain.graph");

    let out = run_helper(&helper, &db, "open-unlocked");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("OPEN_OK"),
        "open-unlocked must succeed on a filesystem that can lock.\n\
         stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Directory holding a loopback NFSv4 export, provisioned by
/// `.github/workflows/nfs-lock.yml`. `None` outside that workflow.
fn nfsv4_dir() -> Option<PathBuf> {
    std::env::var("MINIGRAF_NFS_TEST_DIR_V4")
        .ok()
        .map(PathBuf::from)
}

/// Directory holding a loopback NFSv3 export mounted with `nolock` (no
/// `lockd`), provisioned by the same workflow. `None` outside it.
fn nfsv3_nolock_dir() -> Option<PathBuf> {
    std::env::var("MINIGRAF_NFS_TEST_DIR_V3_NOLOCK")
        .ok()
        .map(PathBuf::from)
}

/// #343: the first lock ever taken on a freshly mounted loopback NFSv4
/// export can stall for tens of seconds (observed ~57s locally, in line with
/// the mount's default 60s RPC timeout) before the kernel even attempts the
/// lock -- almost certainly one-time NFSv4 lock-state/callback-channel setup
/// between this host acting as both client and server. That stall lands on
/// whichever process asks first, so it hits the holder spawned below and the
/// second opener at nearly the same moment, erasing the head start the
/// holder would otherwise have (it creates the database and calls
/// `try_lock` on the very next line) and turning "holder always wins" into a
/// coin flip. Paying that cost here, before the timed race, keeps it out of
/// the assertion below. Confirmed locally: back-to-back lock cycles on one
/// mount pay the stall only once; every cycle after is exclusive and
/// effectively instant.
fn warm_up_locking(mount: &Path, helper: &Path) {
    let db = mount.join("warmup.graph");
    let out = run_helper(helper, &db, "open");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("OPEN_OK"),
        "[warm-up] open failed on {}; cannot proceed to the timed race.\nstderr: {}",
        mount.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// While a holder is alive on the mount, a second open must never report
/// success. This is the core #324 assertion: genuinely exclusive, or
/// refused — a silent no-op (both processes believing they hold the lock) is
/// the one outcome that must never happen.
fn assert_lock_excludes_second_holder(mount: &Path, label: &str) {
    let helper = helper_binary();
    require_helper_built(&helper);
    warm_up_locking(mount, &helper);

    let db = mount.join(format!("{label}.graph"));
    let release = mount.join(format!("{label}-release"));

    let mut holder = Command::new(&helper)
        .arg(&db)
        .arg("hold")
        .arg(&release)
        .spawn()
        .expect("spawn holder");

    // Wait for the holder to actually open the database before racing the
    // second opener against it.
    let mut opened = false;
    for _ in 0..200 {
        if db.exists() {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(opened, "[{label}] holder never created the database file");

    let second = run_helper(&helper, &db, "open");
    let second_out = String::from_utf8_lossy(&second.stdout);

    std::fs::write(&release, b"").expect("write release marker");
    let _ = holder.wait();

    assert!(
        !second_out.contains("OPEN_OK"),
        "[{label}] a second open must not succeed while a holder is alive on \
         this mount; a silent no-op here means the mount let two processes \
         both believe they hold the lock.\nsecond open stdout: {second_out}"
    );
}

#[test]
fn test_lock_is_exclusive_over_nfsv4() {
    let Some(mount) = nfsv4_dir() else {
        eprintln!("SKIP: MINIGRAF_NFS_TEST_DIR_V4 not set");
        return;
    };
    assert_lock_excludes_second_holder(&mount, "nfsv4");
}

#[test]
fn test_lock_is_exclusive_or_refused_over_nfsv3_without_lockd() {
    let Some(mount) = nfsv3_nolock_dir() else {
        eprintln!("SKIP: MINIGRAF_NFS_TEST_DIR_V3_NOLOCK not set");
        return;
    };
    assert_lock_excludes_second_holder(&mount, "nfsv3-nolock");
}

/// `open-unlocked` must succeed regardless of what the mount's locking
/// support turns out to be — it is documented as accepting that risk
/// unconditionally, so this holds on the `nolock` mount too.
#[test]
fn test_open_unlocked_mode_succeeds_on_the_unsupported_nfsv3_mount() {
    let Some(mount) = nfsv3_nolock_dir() else {
        eprintln!("SKIP: MINIGRAF_NFS_TEST_DIR_V3_NOLOCK not set");
        return;
    };
    let helper = helper_binary();
    require_helper_built(&helper);

    let db = mount.join("open-unlocked-nolock.graph");
    let unlocked_open = run_helper(&helper, &db, "open-unlocked");
    let unlocked_out = String::from_utf8_lossy(&unlocked_open.stdout);
    assert!(
        unlocked_out.contains("OPEN_OK"),
        "open-unlocked must proceed on a mount that cannot lock.\n\
         stdout: {unlocked_out}\nstderr: {}",
        String::from_utf8_lossy(&unlocked_open.stderr)
    );
}

/// #334: `-o nolock` is a known, permanent gap in lock detection, not a bug
/// to fix here. This test documents the actual (surprising) behavior rather
/// than the behavior a naive reading of `allow_unlocked` would predict.
///
/// A `nolock` mount tells the Linux NFS client to serve `flock`/OFD locks
/// out of its own local lock table instead of going over NLM to the server.
/// `File::try_lock` sees a normal local success and returns `Ok(())` —
/// indistinguishable, from inside `classify()` in `src/storage/backend/
/// file.rs`, from a mount where locking genuinely works. So the default
/// (`allow_unlocked = false`) open does NOT fail closed here, even though
/// this mount cannot actually coordinate locks across separate client
/// hosts. `allow_unlocked` is never consulted because the code never
/// learns the mount can't really lock; see docs/ERROR_REFERENCE.md
/// STG-027 for the operator-facing writeup of why this mode is
/// unsupported rather than detected and rejected.
#[test]
fn test_default_open_is_not_detected_as_unsupported_on_the_nolock_mount() {
    let Some(mount) = nfsv3_nolock_dir() else {
        eprintln!("SKIP: MINIGRAF_NFS_TEST_DIR_V3_NOLOCK not set");
        return;
    };
    let helper = helper_binary();
    require_helper_built(&helper);

    let db = mount.join("default-open-nolock.graph");
    let default_open = run_helper(&helper, &db, "open");
    let default_out = String::from_utf8_lossy(&default_open.stdout);
    assert!(
        default_out.contains("OPEN_OK"),
        "default open is expected to succeed on a `nolock` mount — this is \
         the known, undetectable gap tracked by #334, not a regression.\n\
         stdout: {default_out}"
    );
}
