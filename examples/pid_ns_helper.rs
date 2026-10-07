//! Helper for `tests/pid_namespace_test.rs`. Opens a database and either dies
//! holding the lock, or reports whether the open succeeded.
//!
//! An example rather than a test so it is a standalone binary the test can run
//! under `unshare`.
//!
//! # Why the release marker is an argument, not an environment variable
//!
//! It was an environment variable first, and that silently did nothing on CI.
//! GitHub's runners restrict unprivileged user namespaces, so the test falls
//! back to `sudo unshare ...`, and sudo defaults to `env_reset`: everything
//! outside a small allowlist is stripped before the command runs. The holder
//! therefore saw no marker, took the "nothing to wait for" path, and aborted
//! the instant it had the lock -- so the second container found the file free
//! and opened it, and the test reported that a live holder had not been
//! excluded, when in truth there had been no live holder to exclude.
//! Arguments cross that boundary; environment variables do not.
//!
//! # Why this kills itself
//!
//! The obvious design is for the test to spawn a holder and kill it. That does
//! not survive contact with CI. GitHub's runners restrict unprivileged user
//! namespaces, so the test falls back to `sudo unshare ...`, and Ubuntu's sudo
//! defaults to `use_pty`, which runs the command in a NEW SESSION. Signalling
//! the process we spawned then reaches only `sudo` itself — SIGKILL runs no
//! handler, so nothing is forwarded — and killing the process group misses the
//! holder too, because it is no longer in that group. The holder survives,
//! still holding the flock, and the #317 regression test fails identically
//! whether or not the bug is present.
//!
//! So the holder ends its own life instead. `abort()` is the right instrument:
//! it terminates without unwinding and without running any `Drop`, which is
//! precisely what a SIGKILLed container leaves behind — no checkpoint, no lock
//! release by the library, nothing but whatever the kernel does on its own.
//! That is the exact state #317 is about.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let mode = &args[2];
    // Optional: a path this process waits for before dying. An argument rather
    // than an env var so it survives `sudo` -- see the note above.
    let release_marker = args.get(3).map(std::path::PathBuf::from);

    // Validate the mode up front rather than falling through to a plain open
    // for anything unrecognized: a typo'd or not-yet-implemented mode must
    // fail loudly, not silently behave like "open".
    if !["hold", "open", "open-unlocked"].contains(&mode.as_str()) {
        eprintln!("unknown mode: {mode}");
        std::process::exit(2);
    }

    let open_result = if mode == "open-unlocked" {
        minigraf::OpenOptions::default()
            .allow_unlocked(true)
            .path(path)
            .open()
    } else {
        minigraf::Minigraf::open(path)
    };

    match open_result {
        Ok(db) => {
            db.execute(r#"(transact [[:a :name "A"]])"#).ok();
            println!("OPEN_OK");

            if mode == "hold" {
                // Hold, then die WITHOUT running Drop.
                //
                // If the test names a release marker, wait for it to appear.
                // A fixed sleep cannot work here: the test must spawn a second
                // container through `sudo unshare` and let it attempt an open,
                // and how long that takes varies wildly between a laptop and a
                // loaded CI runner. A fixed hold that is comfortable locally
                // expires mid-attempt on a slow runner, the second container
                // then succeeds, and the test fails claiming a live holder was
                // not excluded. Letting the parent say when it is done removes
                // the guesswork entirely.
                //
                // The deadline is a backstop, not the mechanism: if the parent
                // dies without writing the marker, this process must not sit on
                // the lock indefinitely.
                if let Some(marker) = release_marker {
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                    while !marker.exists() && std::time::Instant::now() < deadline {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
                // Flush stdout first: abort() will not.
                use std::io::Write;
                let _ = std::io::stdout().flush();
                std::process::abort();
            }
        }
        Err(e) => {
            println!("OPEN_ERR {e}");
            std::process::exit(3);
        }
    }
}
