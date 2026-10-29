#!/usr/bin/env python3
"""Run the existing envelope query probes on opposite sockets, then swap.

Usage: compare.py BASE_EXE CANDIDATE_EXE FIXTURE OUTPUT_JSON [ROUNDS] [INDICES]
INDICES is a comma-separated subset of the envelope's query-shape indices.
The fixture and adjacent .json metadata must already exist. Run only when
builds/tests have finished. This measures warm read queries, not fsync latency.
"""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess
import sys

base, candidate, fixture, output = sys.argv[1:5]
round_count = int(sys.argv[5]) if len(sys.argv) > 5 else 4
# Representative point, scans, joins, not/or, rules, temporal, grouped/window
# and long-history reads from the pinned perf_envelope source.
indices = [0, 1, 2, 3, 5, 7, 8, 9, 10, 12, 13, 14, 19, 20, 21, 22, 23]
if len(sys.argv) > 6:
    indices = [int(i) for i in sys.argv[6].split(",")]


def run(binary, node, index):
    cmd = ["numactl", f"--cpunodebind={node}", f"--membind={node}",
           binary, "__query", fixture, str(index), "2", "30"]
    return json.loads(subprocess.check_output(cmd, text=True))


rows = []
with ThreadPoolExecutor(max_workers=2) as pool:
    for round_no in range(round_count):
        base_node, candidate_node = (1, 3) if round_no % 2 == 0 else (3, 1)
        for index in indices:
            a = pool.submit(run, base, base_node, index)
            b = pool.submit(run, candidate, candidate_node, index)
            a, b = a.result(), b.result()
            assert not any(k in r for r in (a, b) for k in ("error", "capped_ms")), (a, b)
            assert a["name"] == b["name"] and a["rows"] == b["rows"], (a, b)
            row = {"round": round_no, "base_node": base_node, "candidate_node": candidate_node,
                   "base": a, "candidate": b}
            rows.append(row)
            Path(output).write_text(json.dumps(rows, indent=2) + "\n")
            print(round_no, a["name"], a["p50_ms"], b["p50_ms"], flush=True)
