#!/usr/bin/env python3
"""Verify real-run uploads and active numeric comparisons for every intended ID."""
from datetime import datetime
import json
import math
from pathlib import Path
import re
import tarfile
from urllib.parse import parse_qs, urlparse

if not __debug__:
    raise RuntimeError('Evidence verification requires Python assertions; do not use -O')

ROOT = Path(__file__).resolve().parent
BASE = ROOT / 'completion-evidence'
HEAD = '01a1a84c-e947-745c-98e7-018e0c2245a3'


def read(path):
    return json.loads(path.read_text())


def exact(expected, actual):
    assert expected and len(expected) == len(set(expected))
    assert len(actual) == len(set(actual)) and set(actual) == set(expected)


def finite(value):
    return type(value) in (int, float) and math.isfinite(value)


def model(measure):
    m = measure['threshold']['model']
    assert (m['test'], m['upper_boundary'], m['max_sample_size']) == ('t_test', .999, 30)


def main():
    original = read(ROOT / 'inventory.json')
    expected = original['41908407297']
    plan = read(BASE / 'runs.json')
    assert [p['role'] for p in plan] == ['full-v3', 'full-v3', 'c-probe', 'main-probe', 'v3-probe']
    assert len({p['run'] for p in plan}) == 5
    provenance = {v['report']: dict(run=41908407297, sha=v['sha']) for v in expected.values() if v['head'] == HEAD}
    observations = {}
    summaries = []
    previous_end = None
    for item in plan:
        folder = BASE / str(item['run'])
        metadata = read(folder / 'run.json')
        assert metadata['event'] == 'workflow_dispatch' and metadata['conclusion'] == 'success'
        jobs = [j for page in read(folder / 'jobs.json') for j in page['jobs']]
        assert all(j['conclusion'] in ('success', 'skipped') for j in jobs)
        paths = sorted(folder.glob('artifacts/*/bench-report.json')) + sorted(folder.glob('artifacts/*/binding-report.json'))
        union = []
        report_ids = []
        alerted = []
        report_starts, report_ends = [], []
        sha_set = set()
        branch = 'main' if item['role'] == 'main-probe' else 'v3'
        for path in paths:
            report = read(path)
            binding = 'binding' in report
            selected = report['selected']
            alerted.extend(report['alerts'])
            assert report['ref'] == branch
            assert not report['errors'] and report['uploadStatus'] in ('success', 'alerts')
            assert report['workflowSha'] == metadata['head_sha']
            if binding:
                assert report['historyBranch'] == branch
                assert report['bindingSha'] == (path.parent / 'binding-sha.txt').read_text().strip()
                executed = re.findall(r'^ok: (.+)$', (path.parent / 'binding-execution.txt').read_text(), re.M)
                metrics = read(path.parent / 'binding/binding-results.json')
                exact(selected, list(metrics))
                assert all(finite(m['latency']['value']) and m['latency']['value'] > 0 for m in metrics.values())
                output = (path.parent / 'binding-bencher.txt').read_text()
                sha = report['coreSha']
                testbed = report['testbed']
            else:
                selection = read(path.parent / 'bench-selection.json')
                assert all(report[k] == v for k, v in selection.items())
                listed = re.findall(r'^(.+): benchmark\s*$', (path.parent / 'bench-list.txt').read_text(), re.M)
                exact(selected, [name for name in listed if re.search(report['filter'], name)])
                logs = list(path.parent.glob('bench_*.txt'))
                assert len(logs) == 1
                executed = re.findall(r'^Benchmarking (.+): Analyzing\s*$', logs[0].read_text(), re.M)
                output = (path.parent / 'bencher_out.txt').read_text()
                sha = report['sha']
                testbed = 'ubuntu-latest'
            assert all(expected[name]['testbed'] == testbed for name in selected)
            sha_set.add(sha)
            uploaded = re.findall(r'^- (.+) \(Latency\): https://bencher.dev/console/projects/[^/\s]+/perf\?', output, re.M)
            for actual in (executed, uploaded, report['executed'], report['uploaded']):
                exact(selected, actual)
            urls = re.findall(r'https://bencher.dev/console/projects/minigraf/perf\?\S+', output)
            ids = {parse_qs(urlparse(url).query)['report'][0] for url in urls}
            assert len(ids) == 1
            report_id = ids.pop()
            report_ids.append(report_id)
            api = read(folder / 'api' / (report_id + '.json'))
            report_starts.append(datetime.fromisoformat(api['start_time']))
            report_ends.append(datetime.fromisoformat(api['end_time']))
            assert api['branch']['name'] == branch
            assert api['branch']['head']['version']['hash'] == sha
            assert api['testbed']['name'] == testbed
            if branch == 'v3':
                assert api['branch']['head']['uuid'] == HEAD
                provenance[report_id] = dict(run=item['run'], sha=sha)
            else:
                assert sha != 'e7923eb710b1e1d8aff517721643c0b0b1f2cb7a'
                assert api['branch']['head']['version']['number'] > 84
            api_ids = []
            for iteration in api['results']:
                for row in iteration:
                    name = row['benchmark']['name']
                    assert len(row['measures']) == 1
                    measure = row['measures'][0]
                    assert measure['measure']['slug'] == 'latency'
                    model(measure)
                    api_ids.append(name)
                    assert finite(measure['metric']['value']) and measure['metric']['value'] > 0
                    if binding:
                        assert measure['metric']['value'] == metrics[name]['latency']['value']
                    observations[(report_id, name)] = measure
            exact(selected, api_ids)
            union.extend(selected)
        assert len(report_ids) == len(set(report_ids))
        assert len(sha_set) == 1
        if item['role'] == 'full-v3':
            assert len(paths) == 30
            exact(list(expected), union)
        elif item['role'] == 'c-probe':
            exact([name for name in expected if name.startswith('binding/c/')], union)
        elif item['role'] == 'v3-probe':
            exact([name for name in expected if name.startswith('udf/')], union)
        else:
            exact([name for name in original['41801278030'] if name.startswith('udf/')], union)
        if item['run'] == 41965724402:
            assert sorted(alerted) == [
                'concurrent_file/serialized_writers/100k_16t',
                'concurrent_file/serialized_writers/100k_2t',
                'concurrent_file/serialized_writers/100k_4t']
            job = next(j for j in jobs if j['name'] == 'bench / concurrent_file/serialized_writers')
            assert any(step['name'] == 'Report regression' and step['conclusion'] == 'success' for step in job['steps'])
            log = (folder / f'notification-job-{job["id"]}.txt').read_text()
            assert re.search(r'^2026-11-05T\S+ Alerts in previous run: none$', log, re.M)
            assert re.search(r'^2026-11-05T\S+ .*Bencher flagged 3 benchmark\(s\).*Not reporting\.$', log, re.M)
        if previous_end is not None:
            assert previous_end < min(report_starts), 'Fresh reports must follow the preceding run'
        previous_end = max(report_ends)
        summaries.append(dict(**item, sha=sha_set.pop(), workflow_sha=metadata['head_sha'],
                              first_report=min(report_starts).isoformat(), last_report=previous_end.isoformat(),
                              selected=len(union), reports=report_ids, alerts=sorted(alerted)))
    active = BASE / 'active-head'
    branch = read(active / 'branch.json')
    assert branch['head']['uuid'] == HEAD
    prior_ids = {v['report'] for v in expected.values() if v['head'] == HEAD}
    with tarfile.open(ROOT / 'evidence.tar.gz') as archive:
        prior = [json.load(archive.extractfile(f'evidence/41908407297/api/{report}.json'))
                 for report in prior_ids]
    assert all(r['branch']['name'] == 'v3' and r['branch']['head']['uuid'] == HEAD for r in prior)
    earliest = min(r['start_time'] for r in prior)
    start = int(datetime.fromisoformat(earliest).timestamp() * 1000)
    assert read(active / 'window.json') == dict(head_created=branch['head']['created'],
        earliest_actual_v3_report_start=earliest, start_time_ms=start)
    inventory = {}
    requested = []
    for index, query in enumerate(read(active / 'queries.json')):
        params = parse_qs(urlparse(query).query)
        ids = params['benchmarks'][0].split(',')
        assert len(ids) <= 8 and params['heads'] == [HEAD]
        assert params['branches'] == [branch['uuid']] and params['start_time'] == [str(start)]
        requested.extend(ids)
        data = read(active / f'{index:02d}.json')
        for series in data['results']:
            name = series['benchmark']['name']
            assert name in expected and name not in inventory
            assert series['branch']['name'] == 'v3' and series['branch']['head']['uuid'] == HEAD
            assert series['testbed']['name'] == expected[name]['testbed']
            real = [m for m in series['metrics'] if m['report'] in provenance]
            assert len(real) == len(series['metrics']), 'Unattributed history cannot count as independent v3 runs'
            independent = {provenance[m['report']]['run'] for m in real}
            assert len(independent) >= 3, name
            for metric in real:
                assert metric['version']['hash'] == provenance[metric['report']]['sha']
                model(metric)
            latest = max(real, key=lambda m: m['start_time'])
            assert (latest['report'], name) in observations
            model(latest)
            boundary = latest['boundary']
            assert finite(boundary['baseline']) and finite(boundary['upper_limit']), name
            stored = observations[(latest['report'], name)]
            assert latest['metric']['value'] == stored['metric']['value']
            assert boundary == stored['boundary']
            inventory[name] = dict(testbed=series['testbed']['name'], independent_runs=sorted(independent),
                report=latest['report'], sha=latest['version']['hash'], value=latest['metric']['value'],
                baseline=boundary['baseline'], upper_limit=boundary['upper_limit'])
    exact(list(expected), list(inventory))
    assert len(requested) == len(set(requested)) == 453
    for filename in ('before-c-probe.json', 'after-c-probe.json', 'before-main-probe.json',
                     'after-main-probe.json', 'before-v3-probe.json', 'after-v3-probe.json'):
        assert read(BASE / 'head-probes' / filename)['head']['uuid'] == HEAD
    cleanup = read(BASE / 'branch-cleanup.json')
    assert cleanup['ref'] == 'refs/heads/audit/840-history-probe'
    assert cleanup['last_sha'] == summaries[-1]['workflow_sha']
    assert cleanup['readback_exit_code'] != 0 and 'HTTP 404' in cleanup['readback_error']
    summary = dict(runs=summaries, native_numeric=427, binding_numeric=26, missing=0, head=HEAD,
                   temporary_probe_ref_deleted=True,
                   model=dict(test='t_test', upper_probability=.999, max_sample_size=30),
                   minimum_independent_v3_runs=min(len(v['independent_runs']) for v in inventory.values()))
    (ROOT / 'completion-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    (ROOT / 'comparison-inventory.json').write_text(json.dumps(inventory, indent=2, sort_keys=True) + '\n')
    print('Verified all 453 numeric comparisons, at least three independent v3 runs per ID, and unchanged v3 head after main advanced')


if __name__ == '__main__':
    main()
