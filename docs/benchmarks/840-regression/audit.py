#!/usr/bin/env python3
"""Verify the archived #840 evidence offline and regenerate the ID inventory."""
import collections
import json
import math
from pathlib import Path
import re
from urllib.parse import parse_qs, urlparse

ROOT = Path(__file__).resolve().parent
RUNS = (41642356471, 41711848743, 41801278030, 41908407297)


def read(path):
    return json.loads(path.read_text())


def exact(expected, actual):
    assert expected and len(expected) == len(set(expected))
    assert len(actual) == len(set(actual))
    assert set(expected) == set(actual), (set(expected) - set(actual), set(actual) - set(expected))


def numeric(value):
    return isinstance(value, (int, float)) and math.isfinite(value)


def main():
    summaries = []
    inventories = {}
    for run in RUNS:
        base = ROOT / 'evidence' / str(run)
        metadata = read(base / 'run.json')
        jobs = [job for page in read(base / 'jobs.json') for job in page['jobs']]
        artifacts = [a for page in read(base / 'artifacts.json') for a in page['artifacts']]
        native = sorted(base.glob('bench-evidence-*/bench-report.json'))
        bindings = sorted(base.glob('binding-evidence-*/binding-report.json'))
        reports = native + bindings
        expected_artifacts = {p.parent.name for p in reports} | {'bench-enumeration'}
        assert expected_artifacts == {a['name'] for a in artifacts}
        assert all(j['conclusion'] == 'success' for j in jobs)
        entries = {}
        suites = []
        for p in reports:
            report = read(p)
            folder = p.parent
            binding = 'binding' in report
            ids = report['selected']
            assert not report['errors'] and report['uploadStatus'] == 'success'
            assert not report['alerts']
            if binding:
                raw = (folder / 'binding-execution.txt').read_text()
                executed = re.findall(r'^ok: (.+)$', raw, re.M)
                metrics = read(folder / 'binding/binding-results.json')
                exact(ids, list(metrics))
                assert all(numeric(m['latency']['value']) and m['latency']['value'] > 0 for m in metrics.values())
                assert (folder / 'binding-sha.txt').read_text().strip() == report['bindingSha']
                output = (folder / 'binding-bencher.txt').read_text()
                sha, suite = report['coreSha'], 'binding_' + report['binding']
                assert report['historyBranch'] == 'v3'
            else:
                selection = read(folder / 'bench-selection.json')
                assert all(report[k] == v for k, v in selection.items())
                listed = re.findall(r'^(.+): benchmark\s*$', (folder / 'bench-list.txt').read_text(), re.M)
                exact(ids, [x for x in listed if re.search(report['filter'], x)])
                logs = list(folder.glob('bench_*.txt'))
                assert len(logs) == 1
                executed = re.findall(r'^Benchmarking (.+): Analyzing\s*$', logs[0].read_text(), re.M)
                output = (folder / 'bencher_out.txt').read_text()
                sha, suite = report['sha'], report['suite']
            uploaded = re.findall(r'^- (.+) \(Latency\): https://bencher.dev/console/projects/[^/\s]+/perf\?', output, re.M)
            for actual in (executed, uploaded, report['executed'], report['uploaded']):
                exact(ids, actual)
            assert report['workflowSha'] == metadata['head_sha']
            assert metadata['display_title'] == f"Nightly Benchmarks ({report['ref']})"
            urls = re.findall(r'https://bencher.dev/console/projects/minigraf/perf\?\S+', output)
            report_ids = {parse_qs(urlparse(u).query)['report'][0] for u in urls}
            assert len(report_ids) == 1
            report_id = report_ids.pop()
            api = read(base / 'api' / f'{report_id}.json')
            assert api['branch']['name'] == report['ref']
            assert api['branch']['head']['version']['hash'] == sha
            assert api['testbed']['name'] == (report['testbed'] if binding else 'ubuntu-latest')
            api_ids = []
            for iteration in api['results']:
                for benchmark in iteration:
                    name = benchmark['benchmark']['name']
                    assert name not in entries, f'Overlapping suites: {name}'
                    assert len(benchmark['measures']) == 1
                    measure = benchmark['measures'][0]
                    assert measure['measure']['slug'] == 'latency'
                    model = measure['threshold']['model']
                    assert (model['test'], model['upper_boundary'], model['max_sample_size']) == ('t_test', .999, 30)
                    boundary = measure['boundary']
                    usable = numeric(boundary['baseline']) and numeric(boundary['upper_limit'])
                    entries[name] = dict(suite=suite, binding=binding, testbed=api['testbed']['name'],
                        report=report_id, head=api['branch']['head']['uuid'], sha=sha,
                        metric=measure['metric']['value'], baseline=boundary['baseline'],
                        upper_limit=boundary['upper_limit'], usable=usable)
                    api_ids.append(name)
            exact(ids, api_ids)
            suites.append(dict(suite=suite, count=len(ids), sha=sha,
                binding_sha=report.get('bindingSha'), head=api['branch']['head']['uuid']))
        notifications = [s['conclusion'] for j in jobs for s in j['steps'] if s['name'] == 'Report regression']
        assert set(notifications) == {'skipped'}
        summary = dict(run=run, event=metadata['event'], created=metadata['created_at'],
            workflow_sha=metadata['head_sha'], native_suites=len(native), binding_suites=len(bindings),
            native_ids=sum(not e['binding'] for e in entries.values()),
            binding_ids=sum(e['binding'] for e in entries.values()),
            usable_native=sum(e['usable'] and not e['binding'] for e in entries.values()),
            usable_binding=sum(e['usable'] and e['binding'] for e in entries.values()),
            notification_steps=collections.Counter(notifications), suites=suites)
        summaries.append(summary)
        inventories[str(run)] = entries
    original = inventories[str(RUNS[0])]
    latest = inventories[str(RUNS[-1])]
    exact(list(original), list(latest))
    missing = {k: v for k, v in original.items() if not v['usable']}
    assert len(missing) == 270
    assert {k for k, v in latest.items() if not v['usable']} == set(missing)
    active = ROOT / 'evidence' / 'active-head'
    head = read(active / 'branch.json')['head']['uuid']
    queries = read(active / 'queries.json')
    requested = []
    active_entries = {}
    latest_reports = {v['report'] for v in latest.values()}
    for index, query in enumerate(queries):
        params = parse_qs(urlparse(query).query)
        requested += params['benchmarks'][0].split(',')
        assert params['heads'] == [head] and params['start_time'] == ['0']
        assert len(params['benchmarks'][0].split(',')) <= 8
        response = read(active / f'{index:02d}.json')
        assert response['start_time'] == '1970-01-01T00:00:00Z'
        for series in response['results']:
            name = series['benchmark']['name']
            assert name in missing and name not in active_entries
            assert series['branch']['head']['uuid'] == head
            assert series['testbed']['name'] == latest[name]['testbed']
            # Only actual v3 report IDs from the inspected run count here.
            assert len(series['metrics']) == 1
            metric = series['metrics'][0]
            assert metric['report'] in latest_reports
            assert metric['version']['hash'] == latest[name]['sha']
            assert metric['boundary']['baseline'] is None
            active_entries[name] = dict(report=metric['report'], observations=1)
    assert len(requested) == len(set(requested)) == len(missing)
    absent = sorted(set(missing) - active_entries.keys())
    assert absent == sorted(k for k in missing if k.startswith('binding/c/'))
    active_summary = dict(head=head, requested_ids=len(requested),
        series_with_one_observation=len(active_entries), absent=absent,
        observations=active_entries)
    (ROOT / 'active-head-summary.json').write_text(json.dumps(active_summary, indent=2, sort_keys=True) + '\n')
    (ROOT / 'summary.json').write_text(json.dumps(summaries, indent=2) + '\n')
    (ROOT / 'inventory.json').write_text(json.dumps(inventories, indent=2, sort_keys=True) + '\n')
    (ROOT / 'missing-baselines.json').write_text(json.dumps({k: latest[k] for k in sorted(missing)}, indent=2) + '\n')
    for summary in summaries:
        print({k: v for k, v in summary.items() if k != 'suites'})


if __name__ == '__main__':
    main()
