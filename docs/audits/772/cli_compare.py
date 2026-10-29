#!/usr/bin/env python3
"""Compare dist thin-LTO and release full-LTO on a fixed CLI workload.

Usage: cli_compare.py DIST_EXE RELEASE_EXE OUTPUT_JSON
Includes startup, ingestion and printing; not a general query benchmark.
"""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import subprocess
import sys
import time

base, candidate, output = sys.argv[1:]
ingest = "(transact [" + " ".join(f"[:e{i} :v {i % 20}]" for i in range(2000)) + "])\n"
query = "(query [:find (count ?e) :where [?e :v ?v]])\n"
commands = (ingest + query * 400).encode()


def run(binary, node):
    start = time.perf_counter()
    r = subprocess.run(["numactl", f"--cpunodebind={node}", f"--membind={node}", binary],
                       input=commands, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    elapsed = time.perf_counter() - start
    assert not r.stderr, r.stderr
    assert r.stdout.splitlines().count(b"2000") == 400
    return elapsed


rows = []
with ThreadPoolExecutor(max_workers=2) as pool:
    for i in range(8):
        a_node, b_node = (1, 3) if i % 2 == 0 else (3, 1)
        a = pool.submit(run, base, a_node)
        b = pool.submit(run, candidate, b_node)
        row = {"round": i, "base_node": a_node, "candidate_node": b_node,
               "dist_seconds": a.result(), "release_seconds": b.result()}
        rows.append(row)
        Path(output).write_text(json.dumps(rows, indent=2) + "\n")
        print(row, flush=True)
