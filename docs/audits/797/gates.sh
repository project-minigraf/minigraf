#!/usr/bin/env bash
# Correctness gates of one variant (#797, AC 8-11), on the bench machine.
# Run from the variant's source root with the shared Cargo.lock in place.
# Usage: bash gates.sh OUTPUT_DIR TOOLS_DIR
# Every step runs; each one's exit status is recorded in status.tsv.
set -uo pipefail
out=${1:?output directory required}
tools=${2:?tools directory required}
mkdir -p "$out"
out=$(realpath "$out")
tools=$(realpath "$tools")
# The fault matrix and file tests write under TMPDIR: keep it on disk (ext4).
export TMPDIR="$out/tmp"
mkdir -p "$TMPDIR"
export CARGO_TERM_COLOR=never
git rev-parse HEAD > "$out/commit.txt"
sha256sum Cargo.lock > "$out/lock.sha256"
rustc -Vv > "$out/compiler.txt"
printf 'step\tstatus\n' > "$out/status.tsv"
step() {
    local name=$1
    shift
    "$@" > "$out/$name.log" 2>&1
    printf '%s\t%s\n' "$name" "$?" >> "$out/status.tsv"
}
step test-all-features cargo test --locked --all-features
step test-hooks cargo test --locked --features test-hooks
step clippy-ci cargo clippy --locked --all-features -- -D warnings
step clippy-all-targets cargo clippy --locked --all-targets -- -D warnings
step fmt cargo fmt --check
step msrv cargo +1.89.0 check --all-features --locked
step clippy-wasm-browser cargo clippy --locked --target wasm32-unknown-unknown --features browser -- -D warnings
step clippy-wasi cargo clippy --locked --target wasm32-wasip1 -- -D warnings
export PATH="$tools/wasmtime-v49.0.2-x86_64-linux:$tools/wasm-pack-v0.13.1-x86_64-unknown-linux-musl:$PATH"
step wasi-tests env CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime run --dir ." cargo test --locked --target wasm32-wasip1 --lib --tests
step browser-firefox wasm-pack test --headless --firefox --features browser
# #760 deterministic storage suite, in its own cfg build (PR settings).
shuttle() {
    local target=target-shuttle
    RUSTFLAGS="--cfg minigraf_shuttle" CARGO_TARGET_DIR=$target cargo test --locked --lib graph::storage::deterministic:: -- --list > "$out/shuttle-list.txt" 2>&1 || return 1
    test "$(grep -c '^graph::storage::deterministic::.*: test$' "$out/shuttle-list.txt")" -eq 3 || return 1
    RUSTFLAGS="--cfg minigraf_shuttle" CARGO_TARGET_DIR=$target MINIGRAF_DST_SEED=760 MINIGRAF_DST_ITERATIONS=1000 \
        cargo test --locked --lib graph::storage::deterministic:: -- --nocapture --test-threads=1 || return 1
}
step shuttle shuttle
# Coverage gates (#396), CI's nightly and cargo-llvm-cov pins.
coverage() {
    cargo build --examples --target-dir "$out/lock-examples" || return 1
    MINIGRAF_PID_NS_HELPER="$out/lock-examples/debug/examples/pid_ns_helper" \
        cargo +nightly-2026-10-01 llvm-cov --branch --no-report || return 1
    cargo +nightly-2026-10-01 llvm-cov report --branch --json --summary-only --output-path "$out/coverage.json" || return 1
    python3 scripts/coverage_gates.py coverage-thresholds.toml "$out/coverage.json"
}
step coverage coverage
rm -rf "$TMPDIR" "$out/lock-examples"
cat "$out/status.tsv"
