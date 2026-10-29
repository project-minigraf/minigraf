"""Compare two perf_envelope binaries on existing #394 fixtures.

Usage: python3 774-compare.py BEFORE AFTER FIXTURES OUTPUT.jsonl [point]
Build both binaries with the eight #774 shapes (indexes 24..31). Run on an
otherwise idle host with NUMA nodes 1 and 3 on separate sockets. Each pair
runs concurrently; the next round swaps sockets. No build/test may overlap.
"""

import concurrent.futures
import json
import pathlib
import subprocess
import sys

before, after, fixtures, output = map(pathlib.Path, sys.argv[1:5])
assert sys.argv[5:] in ([], ["point"])
point_check = sys.argv[5:] == ["point"]


def measure(binary, fixture, index, node):
    command = [
        "numactl", f"--cpunodebind={node}", f"--membind={node}",
        str(binary), "__query", str(fixture), str(index), "3", "60",
    ]
    result = json.loads(subprocess.check_output(command, text=True))
    assert "error" not in result and "capped_ms" not in result, result
    assert result["runs"] >= 5, result
    return result


with output.open("x") as stream, concurrent.futures.ThreadPoolExecutor(2) as pool:
    for scale in ["100k", "1m"]:
        fixture = fixtures / f"fixture-v1-{scale}.graph"
        for round_index in range(20 if point_check else 4):
            nodes = (1, 3) if round_index % 2 == 0 else (3, 1)
            indexes = [0] if point_check else [*range(24, 32), 0, 2, 4, 6]
            if round_index % 2:
                indexes.reverse()
            for index in indexes:
                a = pool.submit(measure, before, fixture, index, nodes[0])
                b = pool.submit(measure, after, fixture, index, nodes[1])
                a, b = a.result(), b.result()
                assert a["name"] == b["name"] and a["rows"] == b["rows"], (a, b)
                record = {
                    "scale": scale, "round": round_index,
                    "before_node": nodes[0], "after_node": nodes[1],
                    "before": a, "after": b,
                }
                stream.write(json.dumps(record) + "\n")
                stream.flush()
                print(scale, round_index, a["name"], a["p50_ms"], b["p50_ms"], flush=True)
