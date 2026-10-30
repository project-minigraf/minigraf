#!/usr/bin/env bash
# Run in an isolated source snapshot with the audit Cargo.lock, on the bench.
# Usage: bash matrix.sh /absolute/output/directory
set -euo pipefail
out=${1:?output directory required}
mkdir -p "$out"
out=$(realpath "$out")
rustc -Vv > "$out/compiler.txt"
cargo -V >> "$out/compiler.txt"
sha256sum Cargo.lock > "$out/lock.sha256"
cp Cargo.toml "$out/Cargo.toml"
cargo tree --locked -e features > "$out/dependencies.txt"
printf 'case\tbytes\tsha256\n' > "$out/sizes.tsv"
measure() {
    local name=$1 artifact=$2
    shift 2
    cargo build --locked "$@" > "$out/$name.log" 2>&1
    printf '%s\t%s\t%s\n' "$name" "$(stat -c %s "$artifact")" "$(sha256sum "$artifact" | cut -d ' ' -f 1)" >> "$out/sizes.tsv"
    cp "$artifact" "$out/$name.bin"
}
measure native target/release/minigraf --release --bin minigraf
measure native-wasm-feature target/release/minigraf --release --bin minigraf --features wasm
measure native-browser-feature target/release/minigraf --release --bin minigraf --features browser
measure dist target/dist/minigraf --profile dist --bin minigraf
measure wasi target/wasm32-wasip1/release/minigraf.wasm --release --target wasm32-wasip1 --bin minigraf
measure browser target/wasm32-unknown-unknown/release/minigraf.wasm --release --target wasm32-unknown-unknown --features browser --lib
measure consumer-size target/release/examples/perf_envelope --release --example perf_envelope
measure consumer-speed target/envelope/examples/perf_envelope --profile envelope --example perf_envelope
# Keep the profile's original artifact above, but compare shipped-code sizes
# without letting its retained symbols dominate the opt-level-3 measurement.
strip --strip-all -o "$out/consumer-speed-stripped.bin" "$out/consumer-speed.bin"
printf 'consumer-speed-stripped\t%s\t%s\n' "$(stat -c %s "$out/consumer-speed-stripped.bin")" "$(sha256sum "$out/consumer-speed-stripped.bin" | cut -d ' ' -f 1)" >> "$out/sizes.tsv"
cat "$out/sizes.tsv"
