#!/usr/bin/env python3
"""Analyze #797 campaigns: per-block log mean-latency ratios and bounds.

    analyze.py RAW.jsonl[.gz] [RAW.jsonl[.gz] ...] > summary.json

For each campaign, pair and workload: the block ratio is the candidate's
mean ns/op over the base's (for `cc_*`, wall time per operation: inverse
throughput). Ratios are compared as logs; the bound is the one-sided 95%
Student t upper bound on the mean log ratio, exponentiated. Every block must
have the same result digest in both processes (the harness checks every
timed operation's row count): a mismatch is reported and fails the workload. p50/p99, allocations and RSS are medians
over blocks and do not gate.
"""
import gzip
import json
import math
import statistics
import sys


def t_quantile(p, df):
    """Inverse Student t CDF by bisection on its numeric integral."""
    def cdf(x):
        # Regularized incomplete beta through a continued fraction.
        t = df / (df + x * x)
        ib = _betai(df / 2, 0.5, t)
        return 1 - 0.5 * ib if x > 0 else 0.5 * ib
    lo, hi = 0.0, 50.0
    for _ in range(200):
        mid = (lo + hi) / 2
        if cdf(mid) < p:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2


def _betai(a, b, x):
    if x <= 0:
        return 0.0
    if x >= 1:
        return 1.0
    lbeta = math.lgamma(a + b) - math.lgamma(a) - math.lgamma(b)
    front = math.exp(lbeta + a * math.log(x) + b * math.log(1 - x))
    if x < (a + 1) / (a + b + 2):
        return front * _betacf(a, b, x) / a
    return 1 - front * _betacf(b, a, 1 - x) / b


def _betacf(a, b, x):
    tiny = 1e-300
    c, d = 1.0, 1 - (a + b) * x / (a + 1)
    d = 1 / (d if abs(d) > tiny else tiny)
    h = d
    for m in range(1, 300):
        m2 = 2 * m
        aa = m * (b - m) * x / ((a + m2 - 1) * (a + m2))
        d = 1 + aa * d
        d = 1 / (d if abs(d) > tiny else tiny)
        c = 1 + aa / c
        c = c if abs(c) > tiny else tiny
        h *= d * c
        aa = -(a + m) * (a + b + m) * x / ((a + m2) * (a + m2 + 1))
        d = 1 + aa * d
        d = 1 / (d if abs(d) > tiny else tiny)
        c = 1 + aa / c
        c = c if abs(c) > tiny else tiny
        delta = d * c
        h *= delta
        if abs(delta - 1) < 1e-12:
            break
    return h


def metric(r):
    return r.get("inverse_throughput_ns", r["mean_ns"])


def main(paths):
    groups = {}
    for path in paths:
        for line in (gzip.open(path, "rt") if path.endswith(".gz") else open(path)):
            rec = json.loads(line)
            for rb, rc in zip(rec["base_reports"], rec["cand_reports"]):
                assert rb["name"] == rc["name"]
                key = (rec["campaign"], rec["pair"], rb["name"])
                groups.setdefault(key, []).append((rec, rb, rc))
    out = []
    for (campaign, pair, name), rows in sorted(groups.items()):
        logs = [math.log(metric(rc) / metric(rb)) for _, rb, rc in rows]
        n = len(logs)
        mean = statistics.fmean(logs)
        sd = statistics.stdev(logs) if n > 1 else float("inf")
        t = t_quantile(0.95, n - 1) if n > 1 else float("inf")
        half = t * sd / math.sqrt(n)
        # Complete results are validated once per process outside timing and
        # hashed; each timed operation checks its row count in the harness.
        mismatches = sum(1 for _, rb, rc in rows if rb["digest"] != rc["digest"])
        med = lambda rs, k: statistics.median(r[k] for r in rs)
        bs = [rb for _, rb, _ in rows]
        cs = [rc for _, _, rc in rows]
        extra = {}
        if "resident_pages_after_evict" in bs[0]:
            # The harness sums residency over a process's observations;
            # report it per observation (mean within a process).
            per = [r["resident_pages_after_evict"] / r["ops"] for r in bs + cs]
            extra["resident_pages_per_observation_median"] = statistics.median(per)
            extra["resident_pages_per_observation_max"] = max(per)
            extra["file_pages"] = bs[0]["file_pages"]
        out.append({
            "campaign": campaign, "pair": pair, "workload": name, "blocks": n,
            "ratio_geomean": math.exp(mean),
            "upper95": math.exp(mean + half), "lower95": math.exp(mean - half),
            "log_sd": sd,
            "digest_or_row_mismatches": mismatches,
            "base_mean_ns_median": med(bs, "mean_ns"), "cand_mean_ns_median": med(cs, "mean_ns"),
            "base_metric_median": statistics.median(metric(r) for r in bs),
            "cand_metric_median": statistics.median(metric(r) for r in cs),
            "base_p50_ns": med(bs, "p50_ns"), "cand_p50_ns": med(cs, "p50_ns"),
            "base_p99_ns": med(bs, "p99_ns"), "cand_p99_ns": med(cs, "p99_ns"),
            "base_allocs_per_op": med(bs, "allocs_per_op"), "cand_allocs_per_op": med(cs, "allocs_per_op"),
            "base_rss_kb": med(bs, "rss_kb"), "cand_rss_kb": med(cs, "rss_kb"),
            "ops_per_block_min": min(r["ops"] for r in bs + cs),
            **extra,
        })
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main(sys.argv[1:])
