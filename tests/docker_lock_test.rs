//! #324: kernel file locking, tier 3 — two containers sharing a Docker
//! volume, matching the deployment shape in the #317 report literally.
//!
//! Mechanically the same path as `tests/pid_namespace_test.rs` (tier 1): a
//! holder dies without running `Drop`, and a replacement sharing the same
//! storage must still be able to open the database. Docker gives it a real
//! `SIGKILL` and a genuinely separate PID namespace per container, rather
//! than `unshare` simulating one.
//!
//! Needs a working Docker daemon, which this sandbox does not have and which
//! `.github/workflows/nfs-lock.yml` provisions on its runner. Skips cleanly
//! when `docker` is unavailable, so plain `cargo test` never touches it.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const IMAGE: &str = "ubuntu:24.04";

/// Whether a Docker daemon is actually reachable, not just whether the CLI
/// exists. `docker info` fails fast (and without hanging) when there is no
/// daemon to talk to.
fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Path to `examples/pid_ns_helper`.
///
/// `MINIGRAF_PID_NS_HELPER`, if set, is used as-is. Needed under `cargo
/// llvm-cov`, whose instrumented build puts test binaries where the
/// `examples/`-relative lookup below fails, so the helper has to live in
/// a target dir it never touches; `.github/workflows/coverage-gates.yml`
/// builds it there and sets this variable. Otherwise resolved relative to the
/// test binary, the same shape as `tests/pid_namespace_test.rs::helper_binary`.
fn helper_binary() -> PathBuf {
    if let Ok(path) = std::env::var("MINIGRAF_PID_NS_HELPER") {
        return PathBuf::from(path);
    }
    let mut dir = std::env::current_exe().expect("test binary path");
    dir.pop(); // deps/
    dir.pop(); // debug/ (or release/)
    dir.join("examples").join("pid_ns_helper")
}

fn require_helper_built(helper: &Path) {
    assert!(
        helper.exists(),
        "helper binary not found at {}; run `cargo build --examples` first",
        helper.display()
    );
}

/// A name unlikely to collide with a concurrently running test or a prior
/// run's leftovers. Rust runs `#[test]` functions in parallel by default, and
/// this file has two tests that each create a container and a volume.
fn unique_name(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    format!("minigraf-{label}-{}-{nanos}", std::process::id())
}

/// Removes the container and volume on drop, so a failed assertion mid-test
/// does not leak Docker resources on the runner.
struct DockerCleanup {
    container: String,
    volume: String,
}

impl Drop for DockerCleanup {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.container])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("docker")
            .args(["volume", "rm", "-f", &self.volume])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Starts a container that shares `volume` and the helper binary, running
/// detached so the caller can poll it and kill it while it is alive.
fn start_holder(container: &str, volume: &str, helper: &Path) {
    let status = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            container,
            "-v",
            &format!("{volume}:/data"),
            "-v",
            &format!("{}:/helper:ro", helper.display()),
            IMAGE,
            "/helper",
            "/data/shared.graph",
            "hold",
            "/data/release",
        ])
        .status()
        .expect("start holder container");
    assert!(status.success(), "docker run -d for the holder failed");
}

/// Blocks until `docker logs` on the holder shows it opened the database, or
/// panics after a generous timeout.
fn wait_for_holder_to_open(container: &str) {
    for _ in 0..200 {
        let logs = Command::new("docker")
            .args(["logs", container])
            .output()
            .expect("docker logs");
        if String::from_utf8_lossy(&logs.stdout).contains("OPEN_OK") {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("holder container never reported opening the database");
}

/// Runs a fresh, short-lived container against the shared volume in `open`
/// mode and returns its output.
fn run_opener(volume: &str, helper: &Path) -> Output {
    Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{volume}:/data"),
            "-v",
            &format!("{}:/helper:ro", helper.display()),
            IMAGE,
            "/helper",
            "/data/shared.graph",
            "open",
        ])
        .output()
        .expect("run opener container")
}

#[test]
fn test_lock_survives_holder_container_being_killed() {
    if !docker_available() {
        eprintln!("SKIP: no reachable Docker daemon");
        return;
    }
    let helper = helper_binary();
    require_helper_built(&helper);

    let container = unique_name("holder");
    let volume = unique_name("vol");
    Command::new("docker")
        .args(["volume", "create", &volume])
        .status()
        .expect("create volume");
    let _cleanup = DockerCleanup {
        container: container.clone(),
        volume: volume.clone(),
    };

    start_holder(&container, &volume, &helper);
    wait_for_holder_to_open(&container);

    // Kill it the way a Kubernetes pod eviction does: no graceful shutdown,
    // no Drop.
    let status = Command::new("docker")
        .args(["kill", &container])
        .status()
        .expect("docker kill");
    assert!(status.success(), "docker kill failed");
    let _ = Command::new("docker").args(["wait", &container]).output();

    let b = run_opener(&volume, &helper);
    let out = String::from_utf8_lossy(&b.stdout);
    assert!(
        out.contains("OPEN_OK"),
        "a replacement container sharing the volume must be able to open the \
         database after its predecessor was killed; this is #317's shape \
         with real containers.\nstdout: {out}\nstderr: {}",
        String::from_utf8_lossy(&b.stderr)
    );
}

#[test]
fn test_two_live_containers_are_still_refused() {
    if !docker_available() {
        eprintln!("SKIP: no reachable Docker daemon");
        return;
    }
    let helper = helper_binary();
    require_helper_built(&helper);

    let container = unique_name("holder2");
    let volume = unique_name("vol2");
    Command::new("docker")
        .args(["volume", "create", &volume])
        .status()
        .expect("create volume");
    let _cleanup = DockerCleanup {
        container: container.clone(),
        volume: volume.clone(),
    };

    start_holder(&container, &volume, &helper);
    wait_for_holder_to_open(&container);

    // A is still alive. A second, independent container sharing the volume
    // must be refused -- the guarantee PR #318's start-time comparison would
    // have broken by admitting two live writers.
    let b = run_opener(&volume, &helper);
    let out = String::from_utf8_lossy(&b.stdout);

    // Release A now that B's attempt is recorded, then let it exit.
    let _ = Command::new("docker")
        .args(["exec", &container, "/bin/touch", "/data/release"])
        .status();
    let _ = Command::new("docker").args(["wait", &container]).output();

    assert!(
        !out.contains("OPEN_OK"),
        "a second live container sharing the volume must be refused.\n\
         stdout: {out}\nstderr: {}",
        String::from_utf8_lossy(&b.stderr)
    );
}
