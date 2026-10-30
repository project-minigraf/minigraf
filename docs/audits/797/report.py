#!/usr/bin/env python3
"""Markdown tables of the #797 evidence, from analyze.py's summary.

    report.py SUMMARY.json > tables.md

Prints, per category, every workload's B:H and B:P bound against its gate,
and the B:B control of each primary campaign. No pooled score.
"""
import json
import sys

summary = json.load(open(sys.argv[1]))
PRIMARY = {"pt_ea", "pt_av", "pt_asof"}
# Workloads with no result to hash: construction and checkpoint time only.
NO_RESULT = {"con_open", "con_direct", "con_checkpoint", "wasi_committed_con_direct"}


def gate(row):
    if row["campaign"].startswith("primary"):
        return 1.02 if row["workload"] in PRIMARY else None
    return 1.05


def validation(row):
    """How the workload's results were checked against B."""
    w = row["workload"]
    if w in NO_RESULT:
        return "none (no result)"
    if w.startswith("cc_"):
        return "row counts only"
    if w.startswith("cold_"):
        return "digest of a warm run; cold reads row counts only"
    return "digest of complete results; timed reads row counts"


def ns(x):
    return f"{x:,.1f}"


def verdict(row):
    if row["digest_or_row_mismatches"]:
        return "FAIL (results differ)"
    g = gate(row)
    if row["pair"] == "B:B":
        ok = row["lower95"] >= 0.98 and row["upper95"] <= 1.02
        return "resolves ±2%" if ok else "INCONCLUSIVE"
    if g is None:
        return ""
    within = row["upper95"] <= g
    if row["workload"].startswith("cold_"):
        # acceptance-plan.json: residency after eviction must be 0.
        return ("bound ≤ 1.05" if within else "FAIL") + "; category outstanding: residency ≠ 0"
    if row["workload"].startswith("cc_"):
        return ("bound ≤ 1.05" if within else "FAIL") + "; not fully accepted: results not digested"
    return "pass" if within else "FAIL"


def allocs(row):
    if row["campaign"] == "browser":
        return "n/a"
    return f"{row['base_allocs_per_op']:.1f}→{row['cand_allocs_per_op']:.1f}"


def rss(row):
    if row["campaign"] in ("browser", "wasi"):
        return "n/a"
    return f"{row['base_rss_kb']:,.0f}→{row['cand_rss_kb']:,.0f}"


by_campaign = {}
for r in summary:
    by_campaign.setdefault(r["campaign"], []).append(r)

print("Means, deltas and percentiles are medians over blocks, in ns. Δ is the candidate's median "
      "block mean minus the base's (for `cc_*`, wall ns per operation). Allocations and RSS are "
      "n/a where the harness cannot measure them: browser (both) and WASI (RSS).")
for campaign in sorted(by_campaign, key=lambda c: (not c.startswith("primary"), c)):
    rows = sorted(by_campaign[campaign], key=lambda r: (r["workload"], r["pair"]))
    print(f"\n### {campaign}\n")
    print("Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Validation | Base mean | Cand mean | Δ | "
          "Base p50/p99 | Cand p50/p99 | Allocs/op B→C | RSS KiB B→C")
    print("---|---|---:|---:|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:")
    for r in rows:
        g = gate(r)
        extra = ""
        if "resident_pages_per_observation_max" in r:
            extra = (f" (pages resident after eviction, per observation: median "
                     f"{r['resident_pages_per_observation_median']:.1f}, worst process "
                     f"{r['resident_pages_per_observation_max']:.1f}, of {r['file_pages']})")
        print(
            f"{r['workload']}{extra} | {r['pair']} | {r['blocks']} | {r['ratio_geomean']:.4f} | "
            f"{r['upper95']:.4f} | {g if g and r['pair'] != 'B:B' else ('±2% control' if r['pair'] == 'B:B' else '')} | "
            f"{verdict(r)} | {validation(r)} | {ns(r['base_metric_median'])} | {ns(r['cand_metric_median'])} | "
            f"{r['cand_metric_median'] - r['base_metric_median']:+,.1f} | "
            f"{r['base_p50_ns']:,.0f}/{r['base_p99_ns']:,.0f} | {r['cand_p50_ns']:,.0f}/{r['cand_p99_ns']:,.0f} | "
            f"{allocs(r)} | {rss(r)}"
        )
