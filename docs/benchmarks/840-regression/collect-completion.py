#!/usr/bin/env python3
"""Collect approved #840 real-run evidence. No public API writes."""
import concurrent.futures
from datetime import datetime
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
from urllib.parse import parse_qs, urlparse, urlencode
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parent
OUT = ROOT / 'completion-evidence'
REPO = 'project-minigraf/minigraf'


def gh(args):
    return subprocess.check_output(['gh'] + args)


def get(url, path):
    if not path.exists():
        with urlopen(url, timeout=60) as response:
            data = response.read()
        json.loads(data)
        path.write_bytes(data)
    return json.loads(path.read_text())


def collect_run(item):
    run = item['run']
    folder = OUT / str(run)
    folder.mkdir(exist_ok=True)
    if not (folder / 'downloaded').exists():
        metadata = gh(['api', f'repos/{REPO}/actions/runs/{run}'])
        assert json.loads(metadata)['status'] == 'completed'
        (folder / 'run.json').write_bytes(metadata)
        for kind in ('jobs', 'artifacts'):
            (folder / f'{kind}.json').write_bytes(gh(['api', '--paginate', '--slurp', f'repos/{REPO}/actions/runs/{run}/{kind}?per_page=100']))
        subprocess.run(['gh', 'run', 'download', str(run), '--repo', REPO,
                        '--pattern', 'bench-evidence-*', '--pattern', 'binding-evidence-*',
                        '--pattern', 'bench-enumeration', '--dir', str(folder / 'artifacts')], check=True)
        (folder / 'downloaded').write_text('Complete artifact download\n')
    metadata = json.loads((folder / 'run.json').read_text())
    source = folder / 'workflow.yml'
    if not source.exists():
        source.write_bytes(gh(['api', '--header', 'Accept: application/vnd.github.raw+json',
            f'repos/{REPO}/contents/.github/workflows/bench.yml?ref={metadata["head_sha"]}']))
    (folder / 'api').mkdir(exist_ok=True)
    for log in list(folder.glob('artifacts/*/bencher_out.txt')) + list(folder.glob('artifacts/*/binding-bencher.txt')):
        urls = re.findall(r'https://bencher.dev/console/projects/minigraf/perf\?\S+', log.read_text())
        reports = {parse_qs(urlparse(url).query)['report'][0] for url in urls}
        assert len(reports) == 1, log
        report = reports.pop()
        get(f'https://api.bencher.dev/v0/projects/minigraf/reports/{report}', folder / 'api' / f'{report}.json')
    print('Collected', item, flush=True)


def main():
    OUT.mkdir(exist_ok=True)
    ledger = Path(sys.argv[1])
    plan = json.loads(ledger.read_text())
    (OUT / 'runs.json').write_text(json.dumps(plan, indent=2) + '\n')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
        list(pool.map(collect_run, plan))
    active = OUT / 'active-head'
    active.mkdir(exist_ok=True)
    branch = get('https://api.bencher.dev/v0/projects/minigraf/branches/v3', active / 'branch.json')
    assert branch['head']['uuid'] == '01a1a84c-e947-745c-98e7-018e0c2245a3', 'v3 head was unexpectedly replaced'
    # The head was created after inherited main data. Bound the request to
    # actual observations since then; count independence by real report/run IDs.
    start = int(datetime.fromisoformat(branch['head']['created'].replace('Z', '+00:00')).timestamp() * 1000)
    groups = {}
    for item in plan:
        if item['role'] != 'full-v3':
            continue
        for file in (OUT / str(item['run']) / 'api').glob('*.json'):
            report = json.loads(file.read_text())
            assert report['branch']['name'] == 'v3'
            for iteration in report['results']:
                for result in iteration:
                    for measure in result['measures']:
                        key = (report['testbed']['uuid'], measure['measure']['uuid'])
                        groups.setdefault(key, set()).add(result['benchmark']['uuid'])
    queries = []
    for (testbed, measure), ids in sorted(groups.items()):
        ids = sorted(ids)
        for offset in range(0, len(ids), 8):
            query = dict(branches=branch['uuid'], heads=branch['head']['uuid'], testbeds=testbed,
                         measures=measure, benchmarks=','.join(ids[offset:offset + 8]), start_time=start)
            queries.append('https://api.bencher.dev/v0/projects/minigraf/perf?' + urlencode(query))
    (active / 'queries.json').write_text(json.dumps(queries, indent=2) + '\n')
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        list(pool.map(lambda pair: get(pair[1], active / f'{pair[0]:02d}.json'), enumerate(queries)))
    probes = OUT / 'head-probes'
    probes.mkdir(exist_ok=True)
    for role in ('c-probe', 'main-probe', 'v3-probe'):
        for stage in ('before', 'after'):
            filename = f'{stage}-{role}.json'
            shutil.copyfile(ledger.parent / filename, probes / filename)
    orchestration = OUT / 'orchestration'
    orchestration.mkdir(exist_ok=True)
    for filename in ('accumulate.py', 'probes.py'):
        shutil.copyfile(ledger.parent / filename, orchestration / filename)
    print('Collected active-head comparisons for', sum(map(len, groups.values())), 'IDs')


if __name__ == '__main__':
    main()
