#!/usr/bin/env bash
# Shipped-artifact sizes of one variant (#797), the #772 matrix.sh method.
# Run from the variant's source root, with the shared Cargo.lock in place.
# Usage: bash sizes.sh OUTPUT_DIR WASM_BINDGEN_EXE
set -euo pipefail
out=${1:?output directory required}
bindgen=${2:?wasm-bindgen executable required}
mkdir -p "$out"
out=$(realpath "$out")
unset RUSTFLAGS CARGO_BUILD_RUSTFLAGS
rustc -Vv > "$out/compiler.txt"
cargo -V >> "$out/compiler.txt"
"$bindgen" --version >> "$out/compiler.txt"
git rev-parse HEAD > "$out/commit.txt"
sha256sum Cargo.lock > "$out/lock.sha256"
printf 'case\tbytes\tsha256\n' > "$out/sizes.tsv"
record() {
    printf '%s\t%s\t%s\n' "$1" "$(stat -c %s "$2")" "$(sha256sum "$2" | cut -d ' ' -f 1)" >> "$out/sizes.tsv"
}
measure() {
    local name=$1 artifact=$2
    shift 2
    cargo build --locked "$@" > "$out/$name.log" 2>&1
    cp "$artifact" "$out/$name.bin"
    record "$name" "$out/$name.bin"
}
# The binary-size gate's artifact: `cargo build --release`, stripped by the profile.
measure native target/release/minigraf --release --bin minigraf
measure wasi target/wasm32-wasip1/release/minigraf.wasm --release --target wasm32-wasip1 --bin minigraf
measure browser-raw target/wasm32-unknown-unknown/release/minigraf.wasm --release --target wasm32-unknown-unknown --features browser --lib
rm -rf "$out/pkg"
"$bindgen" --target web --out-name minigraf --out-dir "$out/pkg" "$out/browser-raw.bin"
record browser-packaged-wasm "$out/pkg/minigraf_bg.wasm"
record browser-packaged-js "$out/pkg/minigraf.js"
measure consumer-size target/release/examples/perf_envelope --release --example perf_envelope
measure consumer-speed target/envelope/examples/perf_envelope --profile envelope --example perf_envelope
strip --strip-all -o "$out/consumer-speed-stripped.bin" "$out/consumer-speed.bin"
record consumer-speed-stripped "$out/consumer-speed-stripped.bin"
cat "$out/sizes.tsv"
