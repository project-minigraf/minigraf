"""Summarize the #863 qualification runs (spec §4, decision D3).

Usage: python3 -I scripts/qualify_863.py OUT_DIR RUN_DIR [RUN_DIR ...]

Each RUN_DIR is one downloaded `qualify-863-run-N` artifact (runner.txt,
timings.jsonl, estimates/). Writes OUT_DIR/summary.json and prints the
Markdown table for the evidence README. Standard library only.
"""
import json
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
    return seconds, estimates


def summarize(runs):
    if len(runs) < 3:
        raise SystemExit('need at least three runs (spec D3)')
    out = {}
    for name, (ids, test) in CANDIDATES.items():
        costs = [sum(s[i] for i in ids) + s[test] for s, _ in runs]
        spreads = {}
        medians = {}
        for i in ids:
            values = [e[i] for _, e in runs]
            medians[i] = values
            spreads[i] = (max(values) - min(values)) / min(values)
        spread = max(spreads.values(), default=0.0)
        cost = max(costs)
        reasons = []
        if cost > COST_LIMIT_S:
            reasons.append(f'cost {cost:.0f} s > {COST_LIMIT_S} s')
        if spread > SPREAD_LIMIT:
            reasons.append(f'spread {spread:.1%} > {SPREAD_LIMIT:.0%}')
        out[name] = {
            'criterion_ids': ids,
            'acceptance_test': test,
            'cost_seconds_per_run': costs,
            'median_ms_per_run': medians,
            'spread': spreads,
            'verdict': 'rejected' if reasons else 'qualified',
            'reasons': reasons,
        }
    return out


def table(summary):
    lines = ['| Candidate | Max cost (s) | Criterion medians (ms), runs 1–3 | Spread | Verdict |',
             '| --- | ---: | --- | ---: | --- |']
    for name, s in summary.items():
        medians = '<br>'.join(
            f'`{i.split("/", 2)[2]}`: ' + ' / '.join(f'{v:.3f}' for v in vs)
            for i, vs in s['median_ms_per_run'].items()) or 'acceptance only'
        spread = max(s['spread'].values(), default=None)
        spread = f'{spread:.1%}' if spread is not None else '—'
        verdict = s['verdict'] + (f" ({'; '.join(s['reasons'])})" if s['reasons'] else '')
        lines.append(f"| {name} | {max(s['cost_seconds_per_run']):.1f} | {medians} | {spread} | {verdict} |")
    return '\n'.join(lines)


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
