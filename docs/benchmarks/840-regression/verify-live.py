#!/usr/bin/env python3
"""Offline verification of actual #840 notification delivery evidence."""
import json
from pathlib import Path
import re
from urllib.parse import parse_qs, urlparse

if not __debug__:
    raise RuntimeError('Evidence verification requires Python assertions; do not use -O')

ROOT = Path(__file__).resolve().parent
BASE = ROOT / 'live-evidence'
SESSION = 'audit-840-20261105'


def read(path):
    return json.loads(path.read_text())


def main():
    workflow = (BASE / 'sources/production-bench.yml').read_text()
    bodies = [re.sub(r'^            ', '', body, flags=re.M)
              for body in re.findall(r'          script: \|\n((?:            .*\n|\n)+)', workflow)]
    assert len(bodies) == 2
    for name, body in zip(('native', 'binding'), bodies):
        expected = 'module.exports = async function ({github, context, core}) {\n' + body + '\n};\n'
        assert (BASE / 'sources' / (name + '.cjs')).read_text() == expected
    runs = read(BASE / 'notifications/runs.json')
    plan = read(BASE / 'sources/cleanup-plan.json')
    reports = {r['uuid']: r for r in plan['reports']}
    assert len(reports) == 34
    for report in reports.values():
        assert report['branch']['name'] in (SESSION, SESSION + '-other')
        assert report['branch']['head']['start_point'] is None
        assert report['testbed']['name'] in (SESSION + '-native', SESSION + '-binding')
        for iteration in report['results']:
            for row in iteration:
                assert row['benchmark']['name'].startswith('audit/840/20261105/')
                for measure in row['measures']:
                    model = measure['threshold']['model']
                    assert (model['test'], model['upper_boundary'], model['max_sample_size']) == ('t_test', .999, 30)
    summary = []
    delivered = []
    by_stage = {r['stage']: r['run'] for r in runs}
    for entry in runs:
        run, stage = entry['run'], entry['stage']
        folder = BASE / 'notifications' / str(run)
        metadata = read(folder / 'run.json')
        assert metadata['head_sha'] == '437d81e3566c5bb052981d65304e938873ca5eed'
        assert metadata['event'] == 'workflow_dispatch'
        assert metadata['conclusion'] == ('failure' if stage == 'upload-failure' else 'success')
        jobs = [j for page in read(folder / 'jobs.json') for j in page['jobs']]
        assert len(jobs) == 2
        artifacts = sorted((folder / 'artifacts').glob('audit-840-*'))
        assert len(artifacts) == 2
        for artifact in artifacts:
            kind = artifact.name.rsplit('-', 1)[-1]
            suite = 'audit_840_native' if kind == 'native' else 'binding_audit_840'
            if stage == 'other-suite':
                suite += '_other'
            job = next(j for j in jobs if j['name'] == 'bench / ' + suite)
            if stage == 'seed':
                assert len(list(artifact.glob('seed-*.txt'))) == 10
                continue
            receipt = read(artifact / 'notification-receipt.json')
            audit = read(artifact / 'bench-report.json')
            output = (artifact / 'bencher_out.txt').read_text()
            expected_writes = 1 if stage in ('repeat', 'upload-failure') else 0
            assert len(receipt['writes']) == expected_writes
            assert next(s for s in job['steps'] if s['name'] == 'Report regression')['conclusion'] == 'success'
            if stage == 'upload-failure':
                assert 'Status: 404 Not Found' in output
                assert '00000000-0000-0000-0000-000000000840' in output
                assert audit['uploadStatus'] == 'error' and audit['errors'] and not audit['uploaded']
                assert next(s for s in job['steps'] if s['name'] == 'Verify reporting and summarize')['conclusion'] == 'failure'
            else:
                assert audit['uploadStatus'] == 'alerts' and not audit['errors']
                assert set(audit['selected']) == set(audit['executed']) == set(audit['uploaded'])
                urls = re.findall(r'https://bencher.dev/console/projects/minigraf/perf\?\S+', output)
                report_ids = {parse_qs(urlparse(url).query)['report'][0] for url in urls}
                assert len(report_ids) == 1
                report = reports[report_ids.pop()]
                assert report['testbed']['name'] == SESSION + '-' + kind
                assert report['branch']['name'] == SESSION + ('-other' if stage == 'other-branch' else '')
                assert report['counts']['alerts']['total'] == 1
                for measure in [m for rows in report['results'] for row in rows for m in row['measures']]:
                    assert measure['boundary']['baseline'] is not None
                    assert measure['boundary']['upper_limit'] is not None
            if stage == 'repeat':
                previous = next(r for r in receipt['records'] if r['name'] == 'listJobsForWorkflowRun')
                assert previous['args']['run_id'] == by_stage['first'], 'Must skip the intervening other branch'
            if stage == 'other-suite':
                assert not any(r['name'] == 'downloadJobLogsForWorkflowRun' for r in receipt['records'])
            if stage == 'lookup-error':
                assert any(r['name'] == 'downloadJobLogsForWorkflowRun' and r.get('status') == 404 for r in receipt['records'])
                assert any('Could not read' in w for w in receipt['warnings'])
            for write in receipt['writes']:
                issue = 857 if kind == 'native' else 856
                assert f'/issues/{issue}' in write['url']
                independent = read(BASE / f'issue-{issue}.json')
                assert independent['title'] == write['title']
                if stage == 'repeat':
                    assert independent['body'] == write['body']
                else:
                    comments = [c for page in read(BASE / f'issue-{issue}-comments.json') for c in page]
                    assert any(c['html_url'] == write['url'] and c['body'] == write['body'] for c in comments)
                delivered.append(write['url'])
            summary.append(dict(stage=stage, kind=kind, run=run, writes=expected_writes,
                                upload_status=audit['uploadStatus'], audit_errors=audit['errors']))
    assert len(delivered) == 4
    cleanup = read(BASE / 'cleanup/audit-840-cleanup/cleanup-receipts.json')
    assert read(BASE / 'cleanup/run.json')['conclusion'] == 'success'
    assert len(cleanup) == 46
    planned = {kind + '/' + row['uuid'] for kind, rows in plan.items() for row in rows}
    assert {r['resource'] for r in cleanup} == planned
    assert all(r['delete_status'] in (200, 202, 204) and r['verification_status'] == 404 for r in cleanup)
    independent = read(BASE / 'independent-cleanup.json')
    assert {r['url'].split('/minigraf/')[1] for r in independent} == planned
    assert all(r['status'] == 404 for r in independent)
    for issue in (856, 857):
        assert read(BASE / f'issue-{issue}-closed.json')['state'] == 'closed'
    branch_cleanup = read(BASE / 'branch-cleanup.json')
    assert branch_cleanup['ref'] == 'refs/heads/audit/840-live-checks'
    assert branch_cleanup['readback_exit_code'] != 0 and 'HTTP 404' in branch_cleanup['readback_error']
    models = read(BASE / 'production-thresholds-after-cleanup.json')
    assert len(models) == 5
    for model in models:
        assert model['branch']['name'] == 'v3'
        assert (model['model']['test'], model['model']['upper_boundary'], model['model']['max_sample_size']) == ('t_test', .999, 30)
    result = dict(scenarios=summary, deliveries=delivered, synthetic_reports=len(reports),
                  cleanup_resources=len(cleanup), test_issues_closed=[856, 857],
                  limitation='Previous workflow lookup redirects main to audit/840-live-checks; GitHub delivery, not human mailbox receipt.')
    (ROOT / 'live-summary.json').write_text(json.dumps(result, indent=2) + '\n')
    print(f'Verified {len(summary)} live job scenarios, {len(delivered)} independently read-back deliveries, and {len(reports)} synthetic API reports')


if __name__ == '__main__':
    main()
