#!/usr/bin/env python3
"""Compare a thin-LTO and a full-LTO `dist` CLI on fixed REPL workloads (#796).

Usage: 796-dist-lto.py THIN_EXE FULL_EXE OUTPUT_JSON [ROUNDS]

Each round runs every workload once per binary, alternating which goes first.
Both binaries must print the same output (transaction timestamps aside).
Times include process start, ingestion and printing; this is a CLI check,
not a query benchmark. Runs on Linux, macOS and Windows with no extra tools.
"""
import json
import os
import platform
import re
import statistics
import subprocess
import sys
import tempfile
import time

thin, full, output = sys.argv[1:4]
rounds = int(sys.argv[4]) if len(sys.argv) > 4 else 10


def count():
    # The #772 audit workload: 2,000 facts, 400 counts.
    ingest = "(transact [" + " ".join(f"[:e{i} :v {i % 20}]" for i in range(2000)) + "])\n"
    return ingest + "(query [:find (count ?e) :where [?e :v ?v]])\n" * 400


def recursive():
    # Transitive closure of a 300-node chain, 10 times.
    edges = " ".join(f"[:n{i} :next :n{i + 1}]" for i in range(300))
    return (f"(transact [{edges}])\n"
            "(rule [(reach ?x ?y) [?x :next ?y]])\n"
            "(rule [(reach ?x ?y) [?x :next ?z] (reach ?z ?y)])\n"
            + "(query [:find (count ?y) :where (reach :n0 ?y)])\n" * 10)


def aggregate():
    # 5,000 people in 20 departments: a join and grouped sums, 50 times.
    people = " ".join(f"[:p{i} :dept :d{i % 20}] [:p{i} :salary {1000 + i % 97}]"
                      for i in range(5000))
    depts = " ".join(f'[:d{i} :name "dept {i}"]' for i in range(20))
    return (f"(transact [{depts}])\n(transact [{people}])\n"
            + "(query [:find ?n (sum ?s) :where [?p :dept ?d] [?d :name ?n] [?p :salary ?s]])\n" * 50)


def file_backed():
    # 20 transactions of 500 facts into a .graph file (WAL, close-time save).
    txs = "".join("(transact [" + " ".join(f"[:f{t}-{i} :v {i}]" for i in range(500)) + "])\n"
                  for t in range(20))
    return txs + "(query [:find (count ?e) :where [?e :v ?v]])\n"


WORKLOADS = {"count": count(), "recursive": recursive(),
             "aggregate": aggregate(), "file": file_backed()}
TX = re.compile(rb"\(tx: \d+\)")


def run(binary, name, script):
    args = [binary]
    tmp = None
    if name == "file":
        tmp = tempfile.mkdtemp(prefix="minigraf-796-")
        args += ["--file", os.path.join(tmp, "db.graph")]
    start = time.perf_counter()
    r = subprocess.run(args, input=script.encode(), stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, check=True)
    elapsed = time.perf_counter() - start
    if tmp:
        for f in os.listdir(tmp):
            os.remove(os.path.join(tmp, f))
        os.rmdir(tmp)
    if r.stderr:
        sys.exit(f"{binary} {name}: stderr: {r.stderr[:200]!r}")
    return elapsed, TX.sub(b"(tx: _)", r.stdout)


rows = []
for i in range(rounds):
    for name, script in WORKLOADS.items():
        order = [("thin", thin), ("full", full)] if i % 2 == 0 else [("full", full), ("thin", thin)]
        row = {"round": i, "workload": name, "first": order[0][0]}
        outs = {}
        for label, binary in order:
            row[f"{label}_seconds"], outs[label] = run(binary, name, script)
        if outs["thin"] != outs["full"]:
            sys.exit(f"{name}: thin and full LTO outputs differ")
        if name == "count" and outs["full"].splitlines().count(b"2000") != 400:
            sys.exit("count: expected 400 counts of 2000")
        rows.append(row)
        print(row, flush=True)

summary = {}
for name in WORKLOADS:
    ws = [r for r in rows if r["workload"] == name]
    deltas = [r["full_seconds"] / r["thin_seconds"] - 1 for r in ws]
    summary[name] = {
        "thin_median_seconds": statistics.median(r["thin_seconds"] for r in ws),
        "full_median_seconds": statistics.median(r["full_seconds"] for r in ws),
        "median_delta": statistics.median(deltas),
        "full_faster_rounds": sum(d < 0 for d in deltas),
        "rounds": len(ws),
    }
result = {"platform": platform.platform(), "machine": platform.machine(),
          "thin_bytes": os.path.getsize(thin), "full_bytes": os.path.getsize(full),
          "summary": summary, "rows": rows}
with open(output, "w") as f:
    f.write(json.dumps(result, indent=2) + "\n")
print(json.dumps(summary, indent=2))
