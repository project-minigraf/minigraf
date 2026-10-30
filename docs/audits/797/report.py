#!/usr/bin/env python3
"""Markdown tables of the #797 evidence, from analyze.py's summary.

    report.py SUMMARY.json ACCEPTANCE-PLAN.json > tables.md

Prints, per category, every workload's B:H and B:P bound against its gate,
and the B:B control of each primary campaign. No pooled score.
"""
import json
import sys

summary = json.load(open(sys.argv[1]))
PRIMARY = {"pt_ea", "pt_av", "pt_asof"}


def gate(row):
    if row["campaign"].startswith("primary"):
        return 1.02 if row["workload"] in PRIMARY else None
    return 1.05


def us(ns):
    return f"{ns / 1000:,.1f}" if ns < 10_000_000 else f"{ns / 1_000_000:,.1f} ms"


def verdict(row):
    if row["digest_or_row_mismatches"]:
        return "FAIL (results differ)"
    g = gate(row)
    if row["pair"] == "B:B":
        ok = row["lower95"] >= 0.98 and row["upper95"] <= 1.02
        return "resolves ±2%" if ok else "INCONCLUSIVE"
    if g is None:
        return ""
    return "pass" if row["upper95"] <= g else "FAIL"


by_campaign = {}
for r in summary:
    by_campaign.setdefault(r["campaign"], []).append(r)

for campaign in sorted(by_campaign, key=lambda c: (not c.startswith("primary"), c)):
    rows = sorted(by_campaign[campaign], key=lambda r: (r["workload"], r["pair"]))
    print(f"\n### {campaign}\n")
    print("Workload | Pair | Blocks | Ratio | 95% upper | Gate | Verdict | Base mean µs | Cand mean µs | "
          "Base p50/p99 µs | Cand p50/p99 µs | Allocs/op B→C | RSS KiB B→C")
    print("---|---|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|---:")
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
            f"{verdict(r)} | {us(r['base_metric_median'])} | {us(r['cand_metric_median'])} | "
            f"{us(r['base_p50_ns'])}/{us(r['base_p99_ns'])} | {us(r['cand_p50_ns'])}/{us(r['cand_p99_ns'])} | "
            f"{r['base_allocs_per_op']:.1f}→{r['cand_allocs_per_op']:.1f} | "
            f"{r['base_rss_kb']:,.0f}→{r['cand_rss_kb']:,.0f}"
        )
