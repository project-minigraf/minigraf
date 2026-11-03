#!/usr/bin/env bash
# Called from the core checkout. Never changes checked-in binding dependencies.
set -euo pipefail
mode=$1
binding=$2
core_dir=$PWD
export CARGO_TARGET_DIR="$PWD/binding/target/p16"
export CARGO_BUILD_JOBS=2
cd binding
if [[ "$mode" = build ]]; then
  mkdir -p .cargo
  # Benchmark the exact measured core checkout, not the binding's release pin.
  cat > .cargo/config.toml <<EOF
[patch."https://github.com/project-minigraf/minigraf"]
minigraf = { path = "$core_dir" }
EOF
  cargo update -p minigraf
  cargo metadata --format-version 1 > target-metadata.json
  python3 - "$core_dir" <<'PY'
import json, pathlib, sys
packages = json.load(open('target-metadata.json'))['packages']
core = [p for p in packages if p['name'] == 'minigraf']
assert len(core) == 1 and core[0]['source'] is None
assert pathlib.Path(core[0]['manifest_path']).parent == pathlib.Path(sys.argv[1])
PY
  case "$binding" in
    python)
      python3 -m venv target/bench-venv
      source target/bench-venv/bin/activate
      pip install 'maturin>=1.5,<2'
      cargo build --release --bin uniffi-bindgen
      maturin develop --release
      ;;
    browser)
      cargo install wasm-pack --locked --version 0.13.1
      wasm-pack build --target web --out-dir pkg -- --features browser
      npm install --no-save --package-lock=false playwright@1.55.0
      npx playwright install --with-deps chromium
      ;;
    node)
      cargo build --release --lib
      cp "$CARGO_TARGET_DIR/release/libminigraf_node.so" minigraf.linux-x64-gnu.node
      ;;
    c)
      cargo build --release --lib
      cc -O2 -std=c11 -Wall -Wextra -Werror -Iinclude bench/nightly.c \
        "$CARGO_TARGET_DIR/release/libminigraf.a" -lpthread -ldl -lm -o "$CARGO_TARGET_DIR/nightly"
      ;;
    *) exit 1 ;;
  esac
else
  # --test is forwarded verbatim and never writes timing data.
  shift 2
  case "$binding" in
    python) target/bench-venv/bin/python bench/nightly.py "$@" ;;
    browser|node) node bench/nightly.mjs "$@" ;;
    c) "$CARGO_TARGET_DIR/nightly" "$@" ;;
    *) exit 1 ;;
  esac
fi
