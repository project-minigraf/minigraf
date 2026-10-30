#!/usr/bin/env python3
"""Build and run the #797 reader A/B on the bench machine.

    run.py setup ROOT                       build every variant's binaries, the fixture
    run.py campaign ROOT NAME SPEC.json     run one fixed-size campaign
    run.py wasi ROOT NAME SPEC.json         the same, under Wasmtime

ROOT holds the variant sources (ROOT/src/{B,H,P}, each with the shared
Cargo.lock), ROOT/bin, ROOT/fixture and ROOT/raw. A campaign SPEC fixes, before
any of its results are seen: pairs, profile, workloads, seconds per workload,
block count, placement ("paired": base and candidate at once on NUMA nodes 1
and 3, swapped every block; "sequential": one at a time on `nodes`, order
alternating). Every process's output is appended to raw/NAME.jsonl as it
arrives; nothing is retried or dropped.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

VARIANTS = ["B", "H", "P"]
HERE = Path(__file__).resolve().parent


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def sh(cmd, cwd=None, env=None, out=None):
    e = dict(os.environ)
    e.pop("RUSTFLAGS", None)
    e.update(env or {})
    with open(out, "a") if out else open(os.devnull, "w") as log:
        subprocess.run(cmd, cwd=cwd, env=e, check=True, stdout=log, stderr=subprocess.STDOUT)


def setup(root):
    root = Path(root)
    bins = root / "bin"
    bins.mkdir(parents=True, exist_ok=True)
    log = root / "build.log"
    builds = {
        # name: (cargo args, env, artifact under the variant's source)
        "envelope": (["--profile", "envelope"], {}, "target/envelope/examples/reader_ab"),
        "release": (["--release"], {}, "target/release/examples/reader_ab"),
        "ab-envelope": (["--profile", "envelope"],
                        {"RUSTFLAGS": "--cfg minigraf_ab", "CARGO_TARGET_DIR": "target-ab"},
                        "target-ab/envelope/examples/reader_ab"),
        "wasi-envelope": (["--profile", "envelope", "--target", "wasm32-wasip1"],
                          {"RUSTFLAGS": "--cfg minigraf_ab", "CARGO_TARGET_DIR": "target-ab"},
                          "target-ab/wasm32-wasip1/envelope/examples/reader_ab.wasm"),
    }
    manifest = {"rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
                "harness_sha256": sha256(HERE / "reader_ab.rs"),
                "hook_patch_sha256": sha256(HERE / "ab-hook.patch"),
                "variants": {}}
    for v in VARIANTS:
        src = root / "src" / v
        shutil.copy(HERE / "reader_ab.rs", src / "examples" / "reader_ab.rs")
        info = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=src, text=True).strip(),
                "lock_sha256": sha256(src / "Cargo.lock"),
                "reader_rs_sha256": sha256(src / "src/storage/reader.rs"),
                "binaries": {}}
        for name, (args, env, artifact) in builds.items():
            sh(["cargo", "build", "--locked", "--example", "reader_ab", *args], cwd=src, env=env, out=log)
            dst = bins / f"{v}-{name}" if not artifact.endswith(".wasm") else bins / f"{v}-{name}.wasm"
            shutil.copy(src / artifact, dst)
            info["binaries"][name] = {"file": dst.name, "sha256": sha256(dst),
                                      "cargo": " ".join(args), "env": env}
        # The browser module: an example built as a wasm-bindgen test, the
        # release profile without LTO (see ab_browser.rs).
        shutil.copy(HERE / "ab_browser.rs", src / "examples" / "ab_browser.rs")
        built = subprocess.run(
            ["cargo", "test", "--locked", "--release", "--target", "wasm32-unknown-unknown",
             "--features", "browser", "--example", "ab_browser", "--no-run", "--message-format=json"],
            cwd=src, env={**{k: x for k, x in os.environ.items() if k != "RUSTFLAGS"},
                          "CARGO_PROFILE_RELEASE_LTO": "false"},
            check=True, text=True, capture_output=True).stdout
        wasm = [m["executable"] for m in map(json.loads, built.splitlines())
                if m.get("reason") == "compiler-artifact" and m["target"]["name"] == "ab_browser"
                and m.get("executable")][-1]
        dst = bins / f"{v}-browser.wasm"
        shutil.copy(wasm, dst)
        info["binaries"]["browser"] = {"file": dst.name, "sha256": sha256(dst),
                                       "cargo": "test --release --target wasm32-unknown-unknown --features browser --example ab_browser --no-run",
                                       "env": {"CARGO_PROFILE_RELEASE_LTO": "false"}}
        manifest["variants"][v] = info
    # The fixture: B's perf_envelope build of 100K facts.
    fx = root / "fixture"
    fx.mkdir(exist_ok=True)
    graph = fx / "fixture-100k.graph"
    if not graph.exists():
        src = root / "src" / "B"
        sh(["cargo", "build", "--locked", "--profile", "envelope", "--example", "perf_envelope"], cwd=src, out=log)
        meta = subprocess.check_output([src / "target/envelope/examples/perf_envelope", "__build", graph, "100000"], text=True)
        (fx / "fixture-100k.json").write_text(meta)
    manifest["fixture"] = {"file": graph.name, "sha256": sha256(graph),
                           "meta": json.loads((fx / "fixture-100k.json").read_text())}
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest["fixture"]))


def process(root, variant, profile, workloads, secs, node_args, scratch, wasi_entities=None):
    """Run one harness process; returns its parsed reports."""
    root = Path(root)
    if profile == "browser":
        # One headless Firefox run of the prebuilt ab_browser module, from
        # the harness directory, where webdriver.json is.
        env = dict(os.environ)
        env["GECKODRIVER"] = shutil.which("geckodriver") or "geckodriver"
        cmd = [str(root / "tools" / "wasm-bindgen-test-runner"),
               str(root / "bin" / f"{variant}-browser.wasm"), "--nocapture"]
        out = subprocess.run(node_args + cmd, cwd=HERE, env=env, check=True, text=True, capture_output=True).stdout
        return [json.loads(line[line.index("AB {") + 3:]) for line in out.splitlines() if "AB {" in line]
    if wasi_entities is not None:
        wasm = root / "bin" / f"{variant}-{profile}.wasm"
        cmd = [str(root / "tools" / "wasmtime"), "run", str(wasm), "wasi", ",".join(workloads), str(secs), str(wasi_entities)]
    else:
        exe = root / "bin" / f"{variant}-{profile}"
        cmd = [str(exe), "run", str(root / "fixture" / "fixture-100k.graph"), ",".join(workloads), str(secs)]
        if scratch:
            cmd.append(str(scratch))
    out = subprocess.run(node_args + cmd, check=True, text=True, capture_output=True).stdout
    return [json.loads(line) for line in out.splitlines() if line.startswith("{")]


def numa(nodes):
    return ["numactl", f"--cpunodebind={nodes}", f"--membind={nodes}"]


def campaign(root, name, spec_path, wasi=False):
    root = Path(root)
    spec = json.loads(Path(spec_path).read_text())
    raw = root / "raw"
    raw.mkdir(exist_ok=True)
    out = raw / f"{name}.jsonl"
    assert not out.exists(), f"{out} exists: campaigns are never rerun into the same file"
    shutil.copy(spec_path, raw / f"{name}.spec.json")
    scratch_dir = root / "scratch"
    scratch_dir.mkdir(exist_ok=True)
    fixture = root / "fixture" / "fixture-100k.graph"
    needs_scratch = any(w.startswith(("con_checkpoint", "con_large_tx", "cold_")) for w in spec["workloads"])

    def scratch_for(tag):
        if not needs_scratch:
            return None
        p = scratch_dir / f"{name}-{tag}.graph"
        shutil.copy(fixture, p)
        return p

    def cleanup(p):
        if p:
            for f in (p, Path(str(p) + ".wal")):
                f.unlink(missing_ok=True)

    pairs = spec["pairs"]
    blocks = spec["blocks"]
    entities = spec.get("wasi_entities") if wasi else None
    with open(out, "a") as f, ThreadPoolExecutor(max_workers=2) as pool:
        for b in range(blocks):
            # Balanced pair order and placement.
            order = pairs[b % len(pairs):] + pairs[:b % len(pairs)]
            for pair in order:
                base, cand = pair.split(":")
                swap = b % 2 == 1
                started = time.time()
                if spec["placement"] == "paired":
                    nb, nc = ("3", "1") if swap else ("1", "3")
                    sb, sc = scratch_for(f"{b}-{pair}-base"), scratch_for(f"{b}-{pair}-cand")
                    fb = pool.submit(process, root, base, spec["profile"], spec["workloads"], spec["secs"], numa(nb), sb, entities)
                    fc = pool.submit(process, root, cand, spec["profile"], spec["workloads"], spec["secs"], numa(nc), sc, entities)
                    rb, rc = fb.result(), fc.result()
                    cleanup(sb)
                    cleanup(sc)
                    first = "both"
                else:
                    # One at a time; the node alternates every two blocks
                    # unless the spec fixes it.
                    nodes = spec.get("nodes") or ("1" if (b // 2) % 2 == 0 else "3")
                    nb = nc = nodes
                    runs = [("base", base), ("cand", cand)]
                    if swap:
                        runs.reverse()
                    res = {}
                    for role, v in runs:
                        s = scratch_for(f"{b}-{pair}-{role}")
                        res[role] = process(root, v, spec["profile"], spec["workloads"], spec["secs"], numa(nodes), s, entities)
                        cleanup(s)
                    rb, rc = res["base"], res["cand"]
                    first = runs[0][0]
                rec = {"campaign": name, "block": b, "pair": pair, "base": base, "cand": cand,
                       "base_node": nb, "cand_node": nc, "first": first,
                       "started": started, "seconds": time.time() - started,
                       "base_reports": rb, "cand_reports": rc}
                f.write(json.dumps(rec) + "\n")
                f.flush()
                print(name, b, pair, f"{rec['seconds']:.1f}s", flush=True)


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "setup":
        setup(sys.argv[2])
    elif cmd == "campaign":
        campaign(sys.argv[2], sys.argv[3], sys.argv[4])
    elif cmd == "browser":
        campaign(sys.argv[2], sys.argv[3], sys.argv[4])
    elif cmd == "wasi":
        campaign(sys.argv[2], sys.argv[3], sys.argv[4], wasi=True)
    else:
        sys.exit(f"unknown command {cmd}")
