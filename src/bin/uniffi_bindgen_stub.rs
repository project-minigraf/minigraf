// Stub that lets `cargo run --bin uniffi-bindgen` resolve from the workspace root.
//
// maturin (bindings = "uniffi") calls `cargo run --bin uniffi-bindgen generate ...`
// without --package, so cargo searches the workspace default members (root package
// only). This stub is compiled from the root package and execs the real binary,
// which is pre-built by build.sh before maturin is invoked:
//
//   cargo build --bin uniffi-bindgen --manifest-path minigraf-ffi/Cargo.toml
//
// The real binary lives at target/debug/uniffi-bindgen[.exe] relative to the
// workspace root. Using exec-style dispatch keeps this a no-op passthrough with
// no extra dependencies and no nested-cargo issues.

use std::path::PathBuf;
use std::process;

fn main() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let mut real_bin = manifest_dir
        .join("target")
        .join("debug")
        .join("uniffi-bindgen");
    if cfg!(windows) {
        real_bin.set_extension("exe");
    }

    let status = process::Command::new(&real_bin)
        .args(std::env::args_os().skip(1))
        .status()
        .unwrap_or_else(|e| {
            eprintln!(
                "uniffi-bindgen stub: failed to exec {}: {}",
                real_bin.display(),
                e
            );
            process::exit(1);
        });

    process::exit(status.code().unwrap_or(1));
}
