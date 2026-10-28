"""Summarize 532-profile.sh outputs; run this on the bench host."""
import collections
import json
import pathlib
import re
import sys

run_dir = pathlib.Path(sys.argv[1])
revision = 'd0ad89c1d6df734afd5d4669ca9139c8f55d7702'
summary = {'revision': revision, 'event': 'cycles:u',
           'call_graph': 'dwarf,16384', 'inline_expansion': True,
           'discarded_captures': {}, 'profiles': {}}
discarded = run_dir / 'discarded-recursive'
if discarded.exists():
    record = (discarded / 'recursive_ancestors-record.log').read_text()
    header = (discarded / 'recursive_ancestors-report.txt').read_text()
    summary['discarded_captures']['recursive_ancestors'] = {
        'frequency_hz': int(re.search(r'sample_freq\s*}\s*=\s*(\d+)', header).group(1)),
        'samples': int(re.search(r'\((\d+) samples\)', record).group(1)),
        'lost_samples': int(re.search(r'# Total Lost Samples:\s*(\d+)', header).group(1)),
        'lost_chunks': int(re.search(r'lost (\d+) chunks', record).group(1)),
        'reason': 'recorder reported data loss',
    }

def blocks(path):
    block = []
    with path.open() as stream:
        for line in stream:
            if line.strip():
                block.append(line.rstrip())
            elif block:
                yield block
                block = []
    if block:
        yield block

for name, repeats, rows in [('attribute_scan', 1, 166666),
                            ('join_3_selective', 200, 10),
                            ('join_3_scan', 1, 1667),
                            ('recursive_ancestors', 40, 6)]:
    children = [json.loads(line) for line in (run_dir / f'{name}-children.jsonl').read_text().splitlines()]
    assert len(children) == repeats
    assert all(c['name'] == name and c['rows'] == rows and c['runs'] >= 5
               and 'error' not in c and 'capped_ms' not in c for c in children)
    samples = collections.Counter()
    periods = collections.Counter()
    symbol_samples = collections.Counter()
    symbol_periods = collections.Counter()
    query_symbols = collections.Counter()
    query_symbol_periods = collections.Counter()
    diagnostics = collections.Counter()
    total = 0
    for block in blocks(run_dir / f'{name}-stacks.txt'):
        header = block[0]
        match = re.search(r'(\d+)\s+cycles:u:', header)
        if not match:
            raise ValueError(f'Unrecognized sample header: {header}')
        period = int(match.group(1))
        # rustc v0 symbols may render a crate disambiguator after the crate name.
        stack = re.sub(r'\[[0-9a-f]{16}\]', '', '\n'.join(block))
        is_query = any(symbol in stack for symbol in (
            'perf_envelope::drain', 'minigraf::query::engine',
            'net_asserted_facts', 'minigraf::graph::storage::RunWalk',
            'minigraf::graph::storage::flush_run'))
        category = ('fixture_setup' if 'fixture_shapes' in stack else
                    'query' if is_query else 'other')
        if category == 'query' and 'perf_envelope::drain' not in stack:
            diagnostics['query_without_drain_frame'] += 1
        if 'perf_envelope::drain' in stack:
            diagnostics['with_drain_frame'] += 1
        if 'ffffffffffffffff [unknown]' in stack:
            diagnostics['ending_at_unknown_outer_frame'] += 1
        samples[category] += 1
        periods[category] += period
        total += 1
        for symbol in ['net_asserted_facts', 'flush_run', 'RunWalk',
                       'filter_facts_for_query', 'RecursiveEvaluator', 'StratifiedEvaluator',
                       'minigraf::query::engine::scan', 'live_keys', 'live_walk']:
            if symbol in stack:
                symbol_samples[symbol] += 1
                symbol_periods[symbol] += period
                if category == 'query':
                    query_symbols[symbol] += 1
                    query_symbol_periods[symbol] += period
        if category == 'query' and ('[unknown]' in '\n'.join(block[:2])):
            symbol_periods['unresolved_query_leaf'] += period
    assert total >= 1000 and samples['query'] >= 1000, (name, total, samples)
    assert query_symbols['minigraf::query::engine::scan'] > 0
    report_lines = (run_dir / f'{name}-report.txt').read_text().splitlines()
    frequency = re.search(r'sample_freq\s*}\s*=\s*(\d+)', '\n'.join(report_lines))
    assert frequency is not None
    lost_lines = [line.strip() for line in report_lines if line.startswith('# Total Lost Samples:')]
    lost = re.search(r'# Total Lost Samples:\s*(\d+)', '\n'.join(report_lines))
    assert lost is not None and int(lost.group(1)) == 0, (name, lost_lines)
    summary['profiles'][name] = {
        'children': repeats, 'rows_per_query': rows,
        'frequency_hz': int(frequency.group(1)),
        'timed_queries': sum(c['runs'] for c in children), 'samples': total,
        'lost_samples': int(lost.group(1)),
        'sample_counts': dict(samples),
        'weighted_cycle_percent': {k: 100 * v / sum(periods.values()) for k, v in periods.items()},
        'stack_sample_counts': {s: symbol_samples[s] for s in [
            'net_asserted_facts', 'flush_run', 'RunWalk', 'filter_facts_for_query',
            'RecursiveEvaluator', 'StratifiedEvaluator',
            'minigraf::query::engine::scan', 'live_keys', 'live_walk']},
        'query_stack_sample_counts': dict(query_symbols),
        'unwind_diagnostics': dict(diagnostics),
        'net_asserted_facts_percent_of_identified_query_cycles':
            100 * query_symbol_periods['net_asserted_facts'] / periods['query'],
        'unresolved_leaf_percent_of_identified_query_cycles':
            100 * symbol_periods['unresolved_query_leaf'] / periods['query'],
        'lost_sample_header': lost_lines,
    }
print(json.dumps(summary, indent=2))
