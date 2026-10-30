#!/usr/bin/env python3
"""Make independent, disposable size probes; never edit the baseline checkout.

Usage: python3 prototypes.py BASE WORK OUTPUT [CANDIDATE ...]
Requires a baseline Cargo.lock. Each patch is measured against BASE, not the
preceding candidate. WORK and OUTPUT must be new directories.
"""
import difflib
import json
from pathlib import Path
import shutil
import subprocess
import sys

base, work, out = map(lambda p: Path(p).resolve(), sys.argv[1:4])
work.mkdir()
out.mkdir()
shutil.copytree(base, work / "source", ignore=shutil.ignore_patterns("target", ".git"))
source = work / "source"


def replace(path, old, new):
    return path, old, new


candidates = {
    "free-sort": [replace("src/storage/page.rs",
        "free.sort_unstable_by(|a, b| b.cmp(a));",
        "free.sort_unstable();\n        free.reverse();")],
    "encoded-set-sort": [replace("src/storage/keys.rs",
        "encoded.sort();", "encoded.sort_unstable();")],
    "value-order-sort": [replace("src/query/datalog/aggregate.rs",
        "xs.sort_by(cmp);\n            ys.sort_by(cmp);",
        "xs.sort_unstable_by(cmp);\n            ys.sort_unstable_by(cmp);")],
    "string-sorts": [
        replace("src/query/engine/plan.rs", "own.sort();", "own.sort_unstable();"),
        replace("src/query/datalog/prepared.rs", "result.sort();", "result.sort_unstable();")],
    "import-sort": [replace("src/query/engine/plan.rs",
        "imports.sort_by_key(|&(_, s)| s);", "imports.sort_unstable_by_key(|&(_, s)| s);")],
    "chrono-serde": [replace("Cargo.toml",
        'features = ["serde", "std"]', 'features = ["std"]')],
    "reader-erasure": [
        replace("src/storage/reader.rs", "pub struct OnDiskReader<B: StorageBackend + 'static>", "pub struct OnDiskReader"),
        replace("src/storage/reader.rs", "backend: MutexStorageBackend<B>,", "backend: Box<dyn StorageBackend>,"),
        replace("src/storage/reader.rs", "impl<B: StorageBackend + 'static> OnDiskReader<B>", "impl OnDiskReader"),
        replace("src/storage/reader.rs", "pub fn new(backend: Arc<Mutex<B>>,", "pub fn new<B: StorageBackend + 'static>(backend: Arc<Mutex<B>>,"),
        replace("src/storage/reader.rs", "backend: MutexStorageBackend(backend),", "backend: Box::new(MutexStorageBackend(backend)),"),
        *[replace("src/storage/reader.rs", f"impl<B: StorageBackend + 'static> {trait} for OnDiskReader<B>", f"impl {trait} for OnDiskReader") for trait in ("KeySource", "LogSource", "CommittedReader")],
        replace("src/storage/reader.rs", "&self.backend", "&*self.backend")],
}

results = []
for name, edits in candidates.items():
    if len(sys.argv) > 4 and name not in sys.argv[4:]:
        continue
    originals = {p: (base / p).read_text() for p, _, _ in edits}
    modified = dict(originals)
    for p, old, new in edits:
        assert old in modified[p], (name, p, old)
        modified[p] = modified[p].replace(old, new)
    patch = ""
    for p in originals:
        (source / p).write_text(modified[p])
        patch += "".join(difflib.unified_diff(originals[p].splitlines(True),
            modified[p].splitlines(True), fromfile="a/" + p, tofile="b/" + p, n=0))
    (out / (name + ".patch")).write_text(patch)
    with (out / (name + ".log")).open("w") as log:
        # Removing a dependency feature changes lockfile dependency edges.
        # Offline resolution retains pinned package versions; retain that lock
        # for inspection, then restore the baseline for the next candidate.
        lock_flag = "--offline" if name == "chrono-serde" else "--locked"
        run = subprocess.run(["cargo", "build", "--release", lock_flag, "--bin", "minigraf"], cwd=source, stdout=log, stderr=subprocess.STDOUT)
    if name == "chrono-serde":
        shutil.copyfile(source / "Cargo.lock", out / "chrono-serde-lock.toml")
        shutil.copyfile(base / "Cargo.lock", source / "Cargo.lock")
    row = {"candidate": name, "exit": run.returncode}
    if run.returncode == 0:
        binary = source / "target/release/minigraf"
        row["bytes"] = binary.stat().st_size
        shutil.copyfile(binary, out / (name + ".bin"))
    results.append(row)
    (out / "sizes.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(row), flush=True)
    for p, content in originals.items():
        (source / p).write_text(content)
