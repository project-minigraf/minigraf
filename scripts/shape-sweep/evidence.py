"""Strict reconciliation: incomplete matrices cannot become successful reports."""
import argparse
import collections
import json
import math
import statistics
from pathlib import Path


def validate(cases, rows, sessions, samples):
    ids = [c["id"] for c in cases]
    if len(set(ids)) != len(ids):
        raise ValueError("duplicate case")
    wanted = {(i, r, s) for i in ids for r in range(sessions) for s in range(samples)}
    seen = set()
    counts = collections.Counter()
    for row in rows:
        key = (row["case"], row["session"], row["sample"])
        if key not in wanted or key in seen:
            raise ValueError(f"unknown/duplicate sample: {key}")
        seen.add(key)
        disposition = row["disposition"]
        if disposition == "measured":
            ns = row.get("elapsed_ns")
            if (row.get("verified") is not True or row.get("exit_status") != 0
                    or not isinstance(ns, (int, float)) or not math.isfinite(ns) or ns <= 0
                    or not row.get("digest") or row["digest"] != row.get("expected")
                    or row.get("rows") != row["digest"].get("rows")):
                raise ValueError(f"invalid measured result: {key}")
        elif disposition in {"timeout", "resource_limit", "interference", "error", "deferred"}:
            if "elapsed_ns" in row or not row.get("reason"):
                raise ValueError(f"failure represented as latency or missing reason: {key}")
        else:
            raise ValueError(f"unknown disposition: {disposition}")
        counts[disposition] += 1
    if seen != wanted:
        raise ValueError(f"missing {len(wanted-seen)} samples")
    return {"cases": len(cases), "expected_samples": len(wanted), "measured": 0, **counts}


def summarize(cases, rows, sessions, samples):
    reconciliation = validate(cases, rows, sessions, samples)
    summaries = []
    for c in cases:
        group = [r for r in rows if r["case"] == c["id"] and r["disposition"] == "measured"]
        per_session = []
        for session in range(sessions):
            values = [r["elapsed_ns"] / 1e6 for r in group if r["session"] == session]
            if values:
                per_session.append({"session": session, "samples": len(values),
                                    "median_ms": statistics.median(values),
                                    "min_ms": min(values), "max_ms": max(values)})
        peaks = [r["rss_after"]["peak_kib"] for r in group if r.get("rss_after", {}).get("peak_kib") is not None]
        summaries.append({"case": c["id"], "sessions": per_session,
                          "peak_rss_kib": max(peaks) if peaks else None,
                          "rows": group[0]["rows"] if group else None})
    return {"reconciliation": reconciliation, "cases": summaries}


if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("directory", type=Path)
    p.add_argument("--sessions", type=int, default=3)
    p.add_argument("--samples", type=int, default=5)
    a = p.parse_args()
    cases = json.loads((a.directory / "cases.json").read_text())
    rows = [json.loads(s) for s in (a.directory / "samples.jsonl").read_text().splitlines()]
    print(json.dumps(summarize(cases, rows, a.sessions, a.samples), indent=2))
