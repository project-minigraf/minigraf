#!/usr/bin/env bash
# Run on the designated benchmark host. No fixture generation is timed.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE="$(cd "$HERE/../../.." && pwd)"
OUTPUT="${1:?usage: build-fixtures.sh output-directory}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
mkdir "$SCRATCH/src"
cp "$HERE/fixture.rs" "$SCRATCH/src/main.rs"
python3 - "$CORE" "$SCRATCH/Cargo.toml" <<'PY'
import json, sys
from pathlib import Path
Path(sys.argv[2]).write_text('[package]\nname = "opfs-fixtures"\nversion = "0.0.0"\nedition = "2024"\n[dependencies]\nminigraf = { path = ' + json.dumps(sys.argv[1]) + ' }\n[workspace]\n')
PY
cargo run --release --manifest-path "$SCRATCH/Cargo.toml" -- "$OUTPUT"
