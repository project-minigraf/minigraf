"""Reject incomplete sweeps, then report median/min/max; standard library only."""
import argparse
import json
import math
import statistics
from pathlib import Path


def summarize(fixtures, rows, samples, browser, test):
    operations = {"migrate_v7", "reopen_v8", "churn_unpinned", "churn_pinned"}
    if not browser:
        operations.add("read_only_v7")
    expected = {(f["file"], op, i) for f in fixtures for op in operations for i in range(samples)}
    hashes = {f["file"]: f["sha256"] for f in fixtures}
    assert len(hashes) == len(fixtures) and fixtures and samples > 0
    actual = set()
    groups = {}
    for row in rows:
        key = row["fixture"], row["operation"], row["sample"]
        assert key in expected and key not in actual, f"unexpected/duplicate sample: {key}"
        assert row["sha256"] == hashes[row["fixture"]], "fixture changed"
        assert row["correctness"] == "passed", "correctness failed"
        ns = row["ns"]
        if test:
            assert ns is None, "test mode must not report timings"
        else:
            assert type(ns) in (int, float) and math.isfinite(ns) and ns > 0, "invalid timing"
            groups.setdefault(key[:2], []).append(ns / 1e6)
        actual.add(key)
    assert actual == expected, f"missing {len(expected - actual)} samples"
    return [{"fixture": fixture, "operation": operation, "samples": len(values),
             "median_ms": statistics.median(values), "min_ms": min(values), "max_ms": max(values)}
            for (fixture, operation), values in sorted(groups.items())]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("raw", type=Path)
    parser.add_argument("--samples", type=int, required=True)
    parser.add_argument("--browser", action="store_true")
    parser.add_argument("--max-facts", type=int)
    parser.add_argument("--test", action="store_true")
    args = parser.parse_args()
    text = args.raw.read_text()
    rows = json.loads(text)["results"] if args.browser else [json.loads(line) for line in text.splitlines()]
    fixtures = json.loads(args.manifest.read_text())
    if args.max_facts is not None:
        assert args.browser and args.max_facts > 0, "fact cap is browser-only"
        fixtures = [f for f in fixtures if f["facts"] <= args.max_facts]
        result = json.loads(text)
        assert result["maxFacts"] == args.max_facts, "selection mismatch"
        assert result["selectedFixtures"] == [f["file"] for f in fixtures], "selection mismatch"
    print(json.dumps(summarize(fixtures, rows, args.samples,
                               args.browser, args.test), indent=2))
