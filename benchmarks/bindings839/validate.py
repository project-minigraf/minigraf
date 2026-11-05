"""Validate a complete #839 timing JSONL; a passing smoke is not timing evidence."""
import collections
import json
import math
import pathlib
import sys

if not __debug__:
    raise RuntimeError('do not disable validation with Python -O')

COMMON = {
    'prepared_execute/1k', 'cursor_batch_1/1k', 'cursor_batch_256/1k',
    'set_roundtrip/2', 'predicate/1k', 'aggregate/1k', 'prepare/point',
    'register_predicate/first', 'register_aggregate/first', 'fact_log_batch_256/1k',
    'options_error/negative_cache', 'options_readonly_open/1k',
    'file_transact/1k', 'log_writer_append_finish/1k',
}
WASI = {'repl_startup_empty', 'repl_script/1k', 'repl_query_roundtrip/1k'}


def validate(records, runtime):
    assert runtime in ('jvm', 'android', 'swift-ios', 'wasi'), 'unknown runtime'
    expected = {f'binding/{runtime}/{name}' for name in (WASI if runtime == 'wasi' else COMMON)}
    assert all(r.get('type') in ('environment', 'sample', 'checked', 'complete') for r in records)
    environments = [r for r in records if r['type'] == 'environment']
    complete = [r for r in records if r['type'] == 'complete']
    checked = [r['id'] for r in records if r['type'] == 'checked']
    assert len(environments) == len(complete) == 1, 'missing or duplicate run metadata'
    assert environments[0]['mode'] == complete[0]['mode'] == 'benchmark', 'not a timing run'
    assert complete[0]['count'] == len(expected)
    assert len(complete[0]['ids']) == len(expected) and set(complete[0]['ids']) == expected
    assert len(checked) == len(expected) and set(checked) == expected, 'incomplete checks'
    groups = collections.defaultdict(list)
    for record in records:
        if record['type'] == 'sample':
            groups[record['id']].append(record)
    assert set(groups) == expected, 'missing or unexpected sample IDs'
    for name, samples in groups.items():
        assert len(samples) == 10 and {s['sample'] for s in samples} == set(range(10))
        if runtime == 'wasi':
            iterations = 20 if name.endswith('repl_query_roundtrip/1k') else 1
        else:
            iterations = 1 if name.endswith(('file_transact/1k', 'log_writer_append_finish/1k')) else 20
        for sample in samples:
            assert sample['iterations'] == iterations
            assert type(sample['total_ns']) is int and sample['total_ns'] > 0
            assert math.isfinite(sample['ns_per_op']) and sample['ns_per_op'] > 0
            assert math.isclose(sample['ns_per_op'], sample['total_ns'] / iterations, rel_tol=1e-12)
    return len(expected), sum(map(len, groups.values()))


if __name__ == '__main__':
    runtime, filename = sys.argv[1:]
    records = [json.loads(line) for line in pathlib.Path(filename).read_text(encoding='utf-8').splitlines()]
    cases, samples = validate(records, runtime)
    print(f'Validated {cases} cases and {samples} timing samples for {runtime}')
