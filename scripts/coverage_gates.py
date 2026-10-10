#!/usr/bin/env python3
"""Check per-file coverage floors from coverage-thresholds.toml (#396).

Usage: coverage_gates.py <thresholds.toml> <llvm-cov summary.json>

The JSON is `cargo llvm-cov report --branch --json --summary-only`. Every file
named in the thresholds must be in the report, with branch data when it has a
branch floor; a missing file or missing branch data fails the gate instead of
being skipped, so an empty or partial report can never pass.
"""

import json
import sys
import tomllib


def main() -> int:
    thresholds_path, report_path = sys.argv[1], sys.argv[2]
    with open(thresholds_path, "rb") as f:
        floors = tomllib.load(f)["files"]
    with open(report_path) as f:
        report = json.load(f)

    by_path = {}
    for entry in report["data"][0]["files"]:
        name = entry["filename"].replace("\\", "/")
        for path in floors:
            if name.endswith("/" + path):
                by_path[path] = entry["summary"]

    failures = 0
    print(f"{'file':<40} {'metric':<9} {'covered':>8} {'floor':>6}")
    for path, floor in floors.items():
        summary = by_path.get(path)
        if summary is None:
            print(f"{path:<40} FAIL: not in the coverage report")
            failures += 1
            continue
        for metric in ("lines", "branches"):
            if metric not in floor:
                continue
            counts = summary.get(metric, {})
            if not counts.get("count"):
                print(f"{path:<40} {metric:<9} FAIL: no {metric} data")
                failures += 1
                continue
            pct = counts["percent"]
            ok = pct >= floor[metric]
            failures += not ok
            mark = "" if ok else "  FAIL"
            print(f"{path:<40} {metric:<9} {pct:>7.2f}% {floor[metric]:>5}%{mark}")

    if failures:
        print(f"\n{failures} coverage gate(s) failed. Add tests, or list a path "
              "left uncovered, with its reason, in the thresholds file; lower a "
              "floor only with the reason in the PR.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
