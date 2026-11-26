//! #317: a `.graph` file must stay openable after its holder dies in another
//! PID namespace.
//!
//! Kubernetes is where this was reported, but the bug has nothing to do with
//! Kubernetes: every container's main process is PID 1 in its own namespace, so
//! a lock keyed on a PID cannot tell a crashed holder from a live one. `unshare`
//! reproduces it in a few seconds with no container runtime.
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Stdio};

mod pid_ns_helper;
use pid_ns_helper::{helper_binary, require_helper_built};

/// The `unshare` invocation that puts a command at PID 1 in a fresh namespace,
/// or `None` if this environment cannot create one.
///
/// Ubuntu 24.04 restricts unprivileged user namespaces via
/// `kernel.apparmor_restrict_unprivileged_userns`, so fall back to `sudo` where
/// it is available passwordlessly (GitHub runners). If neither works the test
/// skips rather than fails: not every environment permits namespaces.
fn unshare_prefix() -> Option<Vec<String>> {
    let unprivileged: Vec<String> = [
        "unshare",
        "--user",
        "--map-root-user",
        "--pid",
        "--fork",
        "--mount-proc",
        "--kill-child",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let privileged: Vec<String> = [
        "sudo",
        "-n",
        "unshare",
        "--pid",
        "--fork",
        "--mount-proc",
        "--kill-child",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    for candidate in [unprivileged, privileged] {
        let ok = Command::new(&candidate[0])
            .args(&candidate[1..])
            .args(["sh", "-c", "test \"$$\" = 1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(candidate);
        }
    }
    None
}

/// Runs the helper binary at PID 1 in a fresh PID namespace.
fn spawn_in_namespace(
    prefix: &[String],
    helper: &Path,
    db: &Path,
    mode: &str,
    release_marker: Option<&Path>,
) -> std::process::Child {
    let mut cmd = Command::new(&prefix[0]);
    cmd.args(&prefix[1..])
        .arg(helper)
        .arg(db)
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // With no marker the holder aborts the moment it has the lock. With one,
    // it holds until this process creates that file. See the module comment on
    // `examples/pid_ns_helper.rs` for why the holder ends its own life rather
    // than being killed from here, why the hold cannot be a fixed sleep, and
    // why this is an argument rather than an environment variable -- sudo
    // strips the environment, and an env var here silently did nothing on CI.
    if let Some(marker) = release_marker {
        cmd.arg(marker);
    }

    cmd.spawn().expect("spawn in namespace")
}

#[test]
fn test_lock_survives_holder_death_in_another_pid_namespace() {
    let Some(prefix) = unshare_prefix() else {
        eprintln!("SKIP: this environment cannot create PID namespaces");
        return;
    };
    let helper = helper_binary();
    require_helper_built(&helper);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("shared.graph");

    // Container A: opens the database and holds it. Wrapped so a panic
    // unwinding past the explicit `kill`/`wait` below (e.g. the `assert!`
    // right after this, on a namespace flake or a slow runner) still kills
    // and reaps it instead of orphaning a namespaced holder sitting on the
    // flock for its 300-second sleep.
    // Container A opens the database and then dies WITHOUT running Drop --
    // no checkpoint, no library-side lock release, exactly what a SIGKILLed
    // container leaves behind. The holder aborts itself rather than being
    // killed from here; see the module comment on examples/pid_ns_helper.rs
    // for why killing across the privilege boundary cannot be made reliable.
    let a = spawn_in_namespace(&prefix, &helper, &db, "hold", None)
        .wait_with_output()
        .expect("container A ran");

    // A must actually have taken the lock before dying, or this proves
    // nothing. It printed OPEN_OK before aborting, and it did not exit
    // cleanly.
    let a_out = String::from_utf8_lossy(&a.stdout);
    assert!(
        a_out.contains("OPEN_OK"),
        "container A never opened the database.\n\
         prefix used: {prefix:?}\n\
         container A stdout: {a_out}\n\
         container A stderr: {}",
        String::from_utf8_lossy(&a.stderr)
    );
    assert!(
        a.status.code().unwrap_or(1) != 0,
        "container A must die by abort, not exit cleanly"
    );

    let b = spawn_in_namespace(&prefix, &helper, &db, "open", None)
        .wait_with_output()
        .expect("container B ran");

    let out = String::from_utf8_lossy(&b.stdout);
    let err = String::from_utf8_lossy(&b.stderr);
    assert!(
        out.contains("OPEN_OK"),
        "a replacement container must be able to open the database after its \
         predecessor was killed; this is #317.\n\
         prefix used: {prefix:?}\n\
         container B stdout: {out}\n\
         container B stderr: {err}"
    );
}

#[test]
fn test_two_live_namespaced_holders_are_still_refused() {
    let Some(prefix) = unshare_prefix() else {
        eprintln!("SKIP: this environment cannot create PID namespaces");
        return;
    };
    let helper = helper_binary();
    require_helper_built(&helper);

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("contended.graph");

    // panicking (B fails to spawn/run) without leaking a namespaced holder.
    // A holds until this process releases it, however long B takes to start
    // and be refused. The helper caps the wait as a backstop, so a panic here
    // cannot strand a namespaced process on the flock.
    let release = dir.path().join("release-a");
    let a = spawn_in_namespace(&prefix, &helper, &db, "hold", Some(&release));
    let mut held = false;
    for _ in 0..80 {
        if db.exists() && std::fs::metadata(&db).map(|m| m.len() > 0).unwrap_or(false) {
            held = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(held, "container A never opened the database");

    // A is still alive. B must be refused — this is the guarantee PR #318's
    // start-time comparison would have broken, admitting two live writers.
    let b = spawn_in_namespace(&prefix, &helper, &db, "open", None)
        .wait_with_output()
        .expect("container B ran");
    let out = String::from_utf8_lossy(&b.stdout);
    let err = String::from_utf8_lossy(&b.stderr);

    // Release A now that B's attempt is recorded, then reap it.
    std::fs::write(&release, b"").expect("write release marker");
    let mut a = a;
    let _ = a.wait();

    assert!(
        out.contains("OPEN_ERR"),
        "a second live holder in another PID namespace must still be refused.\n\
         prefix used: {prefix:?}\n\
         container B stdout: {out}\n\
         container B stderr: {err}"
    );
}
