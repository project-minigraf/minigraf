#!/usr/bin/env python3
"""Read-only collection of #840 Actions and public Bencher evidence.

Run from this directory after downloading the four runs as described in README.
No authentication is sent to Bencher. GitHub authentication stays inside gh.
"""
import concurrent.futures
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import parse_qs, urlparse, urlencode
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parent / 'evidence'
RUNS = (41642356471, 41711848743, 41801278030, 41908407297)
REPO = 'repos/project-minigraf/minigraf'


def github(endpoint, destination, paginate=False):
    args = ['gh', 'api']
    if paginate:
        args += ['--paginate', '--slurp']
    data = subprocess.check_output(args + [endpoint])
    json.loads(data)
    destination.write_bytes(data)


def public_report(item):
    run, report = item
    destination = ROOT / str(run) / 'api' / f'{report}.json'
    if destination.exists():
        json.loads(destination.read_text())
        return
    url = f'https://api.bencher.dev/v0/projects/minigraf/reports/{report}'
    with urlopen(url, timeout=60) as response:
        data = response.read()
    value = json.loads(data)
    assert value['uuid'] == report
    destination.write_bytes(data)


def main():
    requests = set()
    for run in RUNS:
        root = ROOT / str(run)
        (root / 'api').mkdir(exist_ok=True)
        github(f'{REPO}/actions/runs/{run}', root / 'run.json')
        for kind in ('jobs', 'artifacts'):
            github(f'{REPO}/actions/runs/{run}/{kind}?per_page=100',
                   root / f'{kind}.json', paginate=True)
        for log in list(root.glob('*/bencher_out.txt')) + list(root.glob('*/binding-bencher.txt')):
            urls = re.findall(r'https://bencher.dev/console/projects/minigraf/perf\?\S+', log.read_text())
            assert urls, log
            reports = {parse_qs(urlparse(url).query)['report'][0] for url in urls}
            assert len(reports) == 1, log
            requests.add((run, reports.pop()))
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(public_report, sorted(requests)))
    print(f'Archived {len(requests)} public reports and metadata for {len(RUNS)} runs')
    # Inspect the live head for every missing baseline, without counting copied
    # main records as v3 observations. Request <=8 benchmarks per API dimension.
    perf = ROOT / 'active-head'
    perf.mkdir(exist_ok=True)
    with urlopen('https://api.bencher.dev/v0/projects/minigraf/branches/v3', timeout=60) as response:
        branch_bytes = response.read()
    (perf / 'branch.json').write_bytes(branch_bytes)
    branch = json.loads(branch_bytes)
    groups = {}
    for p in (ROOT / str(RUNS[-1]) / 'api').glob('*.json'):
        report = json.loads(p.read_text())
        for iteration in report['results']:
            for result in iteration:
                for measure in result['measures']:
                    if measure.get('boundary', {}).get('baseline') is None:
                        key = (report['testbed']['uuid'], measure['measure']['uuid'])
                        groups.setdefault(key, []).append(result['benchmark']['uuid'])
    queries = []
    end_time = int(datetime.now(timezone.utc).timestamp() * 1000)
    for (testbed, measure), benchmarks in sorted(groups.items()):
        for start in range(0, len(benchmarks), 8):
            params = dict(branches=branch['uuid'], heads=branch['head']['uuid'],
                          testbeds=testbed, measures=measure,
                          benchmarks=','.join(sorted(benchmarks)[start:start + 8]),
                          start_time=0, end_time=end_time)
            queries.append('https://api.bencher.dev/v0/projects/minigraf/perf?' + urlencode(params))
    (perf / 'queries.json').write_text(json.dumps(queries, indent=2) + '\n')

    def fetch_perf(item):
        index, url = item
        with urlopen(url, timeout=60) as response:
            data = response.read()
        json.loads(data)
        (perf / f'{index:02d}.json').write_bytes(data)

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(fetch_perf, enumerate(queries)))
    print(f'Archived {len(queries)} active-head queries for {sum(map(len, groups.values()))} missing baselines')


if __name__ == '__main__':
    main()
