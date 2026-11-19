#!/usr/bin/env bash
# Called from the core checkout. Never changes checked-in binding dependencies.
set -euo pipefail
mode=$1
binding=$2
core_dir=$PWD
export CARGO_TARGET_DIR="$PWD/binding/target/p16"
export CARGO_BUILD_JOBS=4
[[ "$mode" = build || "$mode" = run ]]
if [[ "$binding" = wasi ]]; then
  if [[ "$mode" = build ]]; then
    test "$(git rev-parse HEAD)" = "$BENCH_SHA"
    test -f benchmarks/bindings839/nightly.py
    test -f benchmarks/bindings839/wasi.py
    test -f benchmarks/bindings839/validate.py
    # No binding repo. Build the CLI from this once-resolved core checkout.
    export CARGO_TARGET_DIR="$core_dir/target"
    # Core does not track Cargo.lock. Resolve once, then keep the build locked
    # and retain those exact dependencies with the measured artifact.
    cargo generate-lockfile
    cargo build --locked --release --target wasm32-wasip1 --bin minigraf
    mkdir binding
    cp Cargo.lock binding/Cargo.lock
    cp target/wasm32-wasip1/release/minigraf.wasm binding/minigraf.wasm
    curl -sSfL 'https://github.com/bytecodealliance/wasmtime/releases/download/v49.0.2/wasmtime-v49.0.2-x86_64-linux.tar.xz' \
      -o binding/wasmtime.tar.xz
    tar -xf binding/wasmtime.tar.xz -C binding
    echo 'd0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635  binding/wasmtime-v49.0.2-x86_64-linux/wasmtime' | sha256sum --check --strict
  else
    shift 2
    python3 -I benchmarks/bindings839/nightly.py \
      --runtime binding/wasmtime-v49.0.2-x86_64-linux/wasmtime \
      --wasm binding/minigraf.wasm --core-dir "$core_dir" --output binding "$@"
  fi
  exit
fi
if [[ "$binding" = jvm ]]; then
  # setup-python exports this on hosted runners. The adapter intentionally
  # rejects native search overrides; clear it for build, smoke and measurement.
  unset LD_LIBRARY_PATH
  # manual/build.sh and Gradle use target/release explicitly. It alone owns
  # the patch, so do not create .cargo/config.toml before calling it.
  export CARGO_TARGET_DIR="$core_dir/binding/target"
  cd binding
  test -f bench/nightly.sh
  if [[ "$mode" = build ]]; then
    test "$(git -C "$core_dir" rev-parse HEAD)" = "$BENCH_SHA"
    CORE_REVISION="$BENCH_SHA" BINDING_REVISION="$(git rev-parse HEAD)" \
      bash bench/manual/build.sh "$core_dir"
    # The manual builder already resolved binding/Cargo.lock via cargo update.
    # This is the binding workspace, not the lockfile-free core workspace.
    cargo metadata --format-version 1 --locked > target-metadata.json
    python3 -I - "$core_dir" <<'PY_METADATA'
import json, pathlib, sys
packages = json.load(open('target-metadata.json'))['packages']
core = [p for p in packages if p['name'] == 'minigraf']
assert len(core) == 1 and core[0]['source'] is None
assert pathlib.Path(core[0]['manifest_path']).parent.resolve() == pathlib.Path(sys.argv[1]).resolve()
PY_METADATA
  else
    shift 2
    # Contract in docs/BINDING_BENCHMARKS.md; failures must propagate unchanged.
    test ! -e binding-results.json
    test ! -e binding-provenance.json
    if [[ "${1:-}" = --test ]]; then
      test ! -e smoke
    else
      test ! -e benchmark
    fi
    bash bench/nightly.sh "$@"
    if [[ "${1:-}" = --test ]]; then
      test ! -e binding-results.json
      test ! -e binding-provenance.json
    else
      # Record the helper's actual launch environment alongside adapter evidence.
      python3 -I - <<'PY_ENVIRONMENT'
import json, os, pathlib
assert 'LD_LIBRARY_PATH' not in os.environ
path = pathlib.Path('binding-provenance.json')
provenance = json.loads(path.read_text())
provenance['launchEnvironment'] = {'LD_LIBRARY_PATH': None}
path.write_text(json.dumps(provenance, indent=2, allow_nan=False) + '\n')
PY_ENVIRONMENT
    fi
  fi
  exit
fi
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
