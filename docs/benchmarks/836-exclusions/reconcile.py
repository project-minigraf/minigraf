#!/usr/bin/env python3
"""Reproduce #836's inventory from a real Criterion listing and pinned workflow.

Usage: python3 reconcile.py BENCH_LIST WORKFLOW OUTPUT_DIRECTORY
Standard library only. Does not build, execute, or time database queries.
"""

import csv
import hashlib
import itertools
import json
import math
from pathlib import Path
import re
import sys


def require(condition, message):
    if not condition:
        raise ValueError(message)


def expand(prefix, *axes):
    return {"/".join((prefix, *parts)) for parts in itertools.product(*axes)}


def row_shape(identifier, registered=True):
    _, size, shape, function, frame = identifier.split("/")
    n = int(size)
    partitions = 1 if shape == "one" else 100
    m = n // partitions
    preceding = {
        "default": "", "zero": 0, "two": 2,
        "sqrt": math.isqrt(m), "half": m // 2, "prefix": m,
    }[frame]
    # Values are distinct ascending integers: default peer frames are prefixes.
    width = m if preceding == "" else min(m, preceding + 1)
    steps = ""
    slots = ""
    reason = "R1"
    cost = "linear aggregate pass after query join/sort; n output rows"
    if function in ("min", "max") and preceding != "":
        slots = width
        cost = "amortized linear aggregate pass; bounded deque; n output rows"
        reason = "R2"
    elif function == "frame_bench_sum":
        if preceding == "" or preceding >= m - 1:
            steps = n
            cost = "running UDF: n step calls and n finalizations"
            reason = "R4"
        else:
            steps = partitions * (width * (width + 1) // 2 + (m - width) * width)
            cost = "replayed UDF: listed step calls plus n factories/finalizations"
            reason = "R3"
    if not registered:
        reason = "R5"
    return {
        "id": identifier, "registered": str(registered).lower(),
        "disposition": "MANUAL_DEFERRED" if registered else "EXCLUDED_UNREGISTERED",
        "reason": reason, "rows": n, "facts": 2 * n,
        "partitions": partitions, "partition_rows": m,
        "function": function, "frame": frame, "preceding": preceding,
        "max_frame_rows": width, "udf_step_calls_per_query": steps,
        "deque_reserved_slots_per_partition": slots,
        "cost_model": cost, "measured_here": "false",
    }


def reconcile(listing_path, workflow_path, output):
    listing = listing_path.read_text()
    workflow = workflow_path.read_text()
    ids = re.findall(r"^(.+): benchmark\s*$", listing, re.M)
    require(len(ids) == len(set(ids)), "duplicate Criterion IDs")
    registered = set(ids)
    filters = re.findall(r'^\s+filter: "(.+)"\s*$', workflow, re.M)
    require(len(filters) == 26, "expected the 26 pinned native routes")
    routes = [{i for i in ids if re.search(f, i)} for f in filters]
    require(all(routes), "empty native route")
    selected = set().union(*routes)
    require(sum(map(len, routes)) == len(selected), "overlapping native routes")

    rows = expand("window_rows", ["10000", "100000"], ["one", "hundred"],
                  ["sum", "count", "min", "max", "avg"],
                  ["default", "zero", "two", "sqrt", "half", "prefix"])
    rows |= expand("window_rows", ["10000"], ["one", "hundred"],
                   ["frame_bench_sum"], ["zero", "two", "half"])
    rows |= expand("window_rows", ["100000"], ["one", "hundred"],
                   ["frame_bench_sum"], ["prefix"])
    nightly_rows = expand("window_rows", ["10000", "100000"], ["one", "hundred"],
                           ["sum"], ["zero", "two", "prefix"])
    nightly_rows |= expand("window_rows", ["100000"], ["hundred"],
                            ["min", "max"], ["two", "prefix"])
    nightly_rows |= {"window_rows/10000/hundred/frame_bench_sum/two",
                     "window_rows/100000/hundred/frame_bench_sum/prefix"}
    require(rows == {i for i in registered if i.startswith("window_rows/")},
            "registered row frames drifted from #836")
    require(rows & selected == nightly_rows, "row-frame shortlist drifted from #836")
    families = {
        "window_rows": rows - nightly_rows,
        "history": expand("point_query_chain_depth",
                          ["churned_attr", "sibling_attr", "attr_scan", "churned_attr_as_of"], ["500"]),
        "retract": expand("retract", ["single_fact", "batch_100"], ["1k", "100k", "1m"]),
        "lookup": expand("btree_lookup", ["entity_point", "attribute_scan"], ["1k", "10k", "100k"]),
        "simd": set().union(*(expand(f"simd_{kind}", ["scalar", "simd"],
                                    ["100", "1k", "10k", "100k", "1m"])
                              for kind in ["temporal", "as_of", "aggregate"])),
    }
    expected = set().union(*families.values())
    omitted = registered - selected
    require(omitted == expected,
            f"unselected mismatch: unexpected={sorted(omitted - expected)}, missing={sorted(expected - omitted)}")
    require((len(registered), len(selected), len(omitted)) == (583, 427, 156),
            "total registration/selection counts drifted")
    unregistered = expand("window_rows", ["100000"], ["one", "hundred"],
                          ["frame_bench_sum"], ["half"])
    require(not unregistered & registered, "100k UDF half-frame unexpectedly registered")

    dispositions = [row_shape(i) for i in sorted(families["window_rows"])]
    for family, reason in [("history", "E1"), ("retract", "E2"), ("lookup", "E3"), ("simd", "E4")]:
        dispositions.extend({"id": i, "registered": "true", "disposition": "EXCLUDED",
                             "reason": reason, "measured_here": "false"}
                            for i in sorted(families[family]))
    fields = list(dispositions[0])
    output.mkdir(parents=True, exist_ok=True)
    for filename, records in [("dispositions.csv", dispositions),
                              ("unregistered.csv", [row_shape(i, False) for i in sorted(unregistered)])]:
        with (output / filename).open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=fields, lineterminator="\n")
            writer.writeheader()
            writer.writerows(records)
    summary = {
        "listing_sha256": hashlib.sha256(listing_path.read_bytes()).hexdigest(),
        "workflow_sha256": hashlib.sha256(workflow_path.read_bytes()).hexdigest(),
        "registered": len(registered), "selected": len(selected), "unselected": len(omitted),
        "unselected_families": {k: len(v) for k, v in families.items()},
        "unregistered_half_frame_ids": sorted(unregistered),
        "selected_ids": sorted(selected),
    }
    (output / "reconciliation.json").write_text(json.dumps(summary, indent=2) + "\n")
    print("583 registered = 427 selected + 156 unselected (110/4/6/6/30); 2 unregistered half-frame shapes")


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    reconcile(*(Path(arg) for arg in sys.argv[1:]))
