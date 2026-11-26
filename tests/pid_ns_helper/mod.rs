//! Locates the compiled `examples/pid_ns_helper` for the lock tests that run
//! it in another process (`docker_lock_test`, `nfs_lock_test`,
//! `pid_namespace_test`). A directory module, so Cargo does not build it as a
//! test target of its own.

use std::path::{Path, PathBuf};

/// `MINIGRAF_PID_NS_HELPER`, if set, is used as-is: `cargo llvm-cov` builds
/// into a target dir the lookup below cannot reach, so
/// `.github/workflows/coverage-gates.yml` builds the helper elsewhere and sets
/// it.
///
/// Otherwise the nearest ancestor of the test binary with an
/// `examples/pid_ns_helper`. The test binary is at
/// `target/debug/deps/<test>-<hash>` up to Rust 1.99 and at
/// `target/debug/build/minigraf/<hash>/out/<test>-<hash>` from 1.100, while
/// Cargo still copies the example to `target/debug/examples/`.
pub fn helper_binary() -> PathBuf {
    if let Ok(path) = std::env::var("MINIGRAF_PID_NS_HELPER") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().expect("test binary path");
    let relative = Path::new("examples").join("pid_ns_helper");
    exe.ancestors()
        .skip(1)
        .map(|dir| dir.join(&relative))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| exe.with_file_name(&relative))
}

/// Fails loudly rather than skipping: a missing helper is a build problem
/// (`cargo build --examples` not run, or a Cargo layout change), and a skip
/// here would report green while asserting nothing.
pub fn require_helper_built(helper: &Path) {
    assert!(
        helper.exists(),
        "helper binary not found at {}; run `cargo build --examples` first",
        helper.display()
    );
}
