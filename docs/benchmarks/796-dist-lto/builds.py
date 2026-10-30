#!/usr/bin/env python3
"""Time clean thin- and full-LTO `dist` builds, two rounds, alternating (#796).

Usage: builds.py OUTPUT_JSON  (from the repository root; copies each binary
to bins/thin-minigraf and bins/full-minigraf for runtime.py)
"""
import json, os, platform, shutil, subprocess, sys, time
exe = "minigraf.exe" if os.name == "nt" else "minigraf"
rows = []
for rnd, order in enumerate([("thin", "full"), ("full", "thin")]):
    for label in order:
        tdir = f"tgt-{label}-{rnd}"
        env = dict(os.environ)
        if label == "thin":
            env["CARGO_PROFILE_DIST_LTO"] = "thin"
        start = time.perf_counter()
        subprocess.run(["cargo", "build", "--locked", "--profile", "dist", "--target-dir", tdir], env=env, check=True)
        secs = time.perf_counter() - start
        size = os.path.getsize(os.path.join(tdir, "dist", exe))
        rows.append({"round": rnd, "lto": label, "build_seconds": secs, "bytes": size})
        print(rows[-1], flush=True)
        os.makedirs("bins", exist_ok=True)
        shutil.copy(os.path.join(tdir, "dist", exe), os.path.join("bins", f"{label}-{exe}"))
        shutil.rmtree(tdir)
json.dump({"platform": platform.platform(), "machine": platform.machine(), "builds": rows},
          open(sys.argv[1], "w"), indent=2)
