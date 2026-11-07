"""Summarize the #863 qualification runs (spec §4, decision D3 as amended).

Runs land on different runner CPU models, and a runner can be ~1.4x
slower on every case at once, also within one model (#678). That is the
runner effect the existing nightly suites already have, absorbed by
Bencher's t-test over 30 runs. The verdict therefore uses each case's spread
after dividing every run by its speed factor, the geometric mean of all
timed cases in that run (user decision, 2026-11-07); raw same-model and
cross-model spreads are reported alongside.
"""
import json
import math
import sys
from pathlib import Path

COST_LIMIT_S = 600  # 10 job-minutes
SPREAD_LIMIT = 0.25

# Criterion ID -> acceptance test guarding the same candidate.
CANDIDATES = {
    'N1': (['window/nightly_candidates/skew_running_sum/100k'],
           'n1_skew_running_sum_streams_by_partition'),
    'N2': (['query/nightly_candidates/set_membership_k256/10k'],
           'n2_set_membership_k256'),
    'N3': (['query/nightly_candidates/sort_offset_desc/100k',
            'query/nightly_candidates/sort_offset_asc/100k'],
           'n3_sort_heap_holds_offset_plus_limit'),
    'N4': (['recursion/nightly_candidates/tree_ancestors_k1_pending/1110'],
           'n4_small_tree_takes_ancestor_alternative'),
    'N5': (['recursion/nightly_candidates/tree_ancestors_k16_committed/1110'],
           'n5_committed_tree_uses_bound_value_probes'),
    'N6': ([], 'n6_held_cursor_quarantine_and_reuse'),
}


def cpu_model(run_dir):
    for line in (Path(run_dir) / 'runner.txt').read_text().splitlines():
        if line.startswith('model name'):
            return line.split(':', 1)[1].strip()
    raise SystemExit(f'{run_dir}: no CPU model in runner.txt')


def load_run(run_dir):
    run_dir = Path(run_dir)
    timings = [json.loads(line) for line in
               (run_dir / 'timings.jsonl').read_text().splitlines() if line.strip()]
    seconds = {}
    for t in timings:
        seconds[t.get('id') or t['test']] = t['seconds']
    estimates = {}
    for bench in sorted((run_dir / 'estimates').glob('*-benchmark.json')):
        full_id = json.loads(bench.read_text())['full_id']
        est = bench.with_name(bench.name.replace('-benchmark', '-estimates'))
        median = json.loads(est.read_text())['median']['point_estimate']
        estimates[full_id] = median / 1e6  # ns -> ms
    return cpu_model(run_dir), seconds, estimates


def spread(values):
    return (max(values) - min(values)) / min(values)


def summarize(runs):
    if len(runs) < 3:
        raise SystemExit('need at least three runs (spec D3)')
    models = {}
    for model, _, _ in runs:
        models[model] = models.get(model, 0) + 1
    timed = [i for ids, _ in CANDIDATES.values() for i in ids]
    factors = [math.exp(sum(math.log(e[i]) for i in timed) / len(timed))
               for _, _, e in runs]
    out = {}
    for name, (ids, test) in CANDIDATES.items():
        costs = [sum(s[i] for i in ids) + s[test] for _, s, _ in runs]
        medians, normalized, same, across = {}, {}, {}, {}
        for i in ids:
            medians[i] = [e[i] for _, _, e in runs]
            normalized[i] = spread([e[i] / f for (_, _, e), f in zip(runs, factors)])
            across[i] = spread(medians[i])
            same[i] = max((spread([e[i] for m, _, e in runs if m == model])
                           for model, n in models.items() if n >= 2), default=None)
        worst = max(normalized.values(), default=0.0)
        cost = max(costs)
        reasons = []
        if cost > COST_LIMIT_S:
            reasons.append(f'cost {cost:.0f} s > {COST_LIMIT_S} s')
        if worst > SPREAD_LIMIT:
            reasons.append(f'normalized spread {worst:.1%} > {SPREAD_LIMIT:.0%}')
        out[name] = {
            'criterion_ids': ids,
            'acceptance_test': test,
            'cost_seconds_per_run': costs,
            'median_ms_per_run': medians,
            'normalized_spread': normalized,
            'same_model_spread': same,
            'cross_model_spread': across,
            'verdict': 'rejected' if reasons else 'qualified',
            'reasons': reasons,
        }
    return {'runs': [m for m, _, _ in runs], 'speed_factor_ms': factors,
            'candidates': out}


def pct(values):
    values = [v for v in values if v is not None]
    return f'{max(values):.1%}' if values else '—'


def table(summary):
    lines = ['| Candidate | Max cost (s) | Criterion medians (ms), runs 1–6 | Normalized spread | Same-model spread | Cross-model spread | Verdict |',
             '| --- | ---: | --- | ---: | ---: | ---: | --- |']
    for name, s in summary['candidates'].items():
        medians = '<br>'.join(
            f'`{i.split("/", 2)[2]}`: ' + ' / '.join(f'{v:.3f}' for v in vs)
            for i, vs in s['median_ms_per_run'].items()) or 'acceptance only'
        verdict = s['verdict'] + (f" ({'; '.join(s['reasons'])})" if s['reasons'] else '')
        lines.append(f"| {name} | {max(s['cost_seconds_per_run']):.1f} | {medians} | "
                     f"{pct(s['normalized_spread'].values())} | {pct(s['same_model_spread'].values())} | "
                     f"{pct(s['cross_model_spread'].values())} | {verdict} |")
    runs = ', '.join(f'{i + 1}: {m} (speed factor {f:.2f} ms)'
                     for i, (m, f) in enumerate(zip(summary['runs'], summary['speed_factor_ms'])))
    return '\n'.join(lines) + f'\n\nRuns: {runs}.'


def main(argv):
    if len(argv) < 3:
        raise SystemExit(__doc__)
    out_dir = Path(argv[1])
    summary = summarize([load_run(d) for d in argv[2:]])
    out_dir.mkdir(parents=True, exist_ok=True)
    (out_dir / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    print(table(summary))


if __name__ == '__main__':
    main(sys.argv)
