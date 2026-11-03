const { test } = require('node:test');
const assert = require('node:assert/strict');
const { selection, executions, uploads, audit, parseAlerts } = require('./report.cjs');

const ids = ['window/rank/1k', 'window/rank/10k'];
const listing = ids.map(id => `${id}: benchmark`).join('\n');
const measured = ids.map(id => `Benchmarking ${id}: Analyzing`).join('\n');
const results = ids.map(id => `- ${id} (Latency): https://bencher.dev/console/projects/minigraf/perf?x=1`).join('\n');
const alert = '- window/rank/1k (Latency): https://bencher.dev/console/projects/minigraf/alerts/123';

test('enumeration ignores cargo chatter and validates the route', () => {
  assert.deepEqual(selection(`Compiling minigraf\n${listing}\n`, '^window/'), ids);
});
test('shared enumeration selects only the matrix filter', () => {
  assert.deepEqual(selection(`${listing}\nquery/point_entity/1k: benchmark`, '^window/'), ids);
});
test('required empty routes fail', () => {
  assert.throws(() => selection('Finished bench profile', '^typo/'), /empty/);
});
test('wrong or duplicate enumerated IDs fail', () => {
  assert.throws(() => selection(listing, '^query/'), /empty/);
  assert.throws(() => selection(`${listing}\n${listing}`, '^window/'), /duplicate/);
});
test('execution includes only completed analysis lines', () => {
  assert.deepEqual(executions(`${measured}\nBenchmarking incomplete\n`), ids);
});
test('uploads exclude alert links', () => {
  assert.deepEqual(uploads(`${results}\n${alert}`), ids);
  assert.deepEqual([...parseAlerts(`${results}\n${alert}`)], [ids[0]]);
});
test('successful upload verifies all selected and executed IDs', () => {
  assert.deepEqual(audit(ids, measured, results, 'success').errors, []);
});
test('an alert exit with a complete upload is preserved for repeat reporting', () => {
  const report = audit(ids, measured, `${results}\n${alert}`, 'failure');
  assert.deepEqual(report.errors, []);
  assert.equal(report.uploadStatus, 'alerts');
});
test('API failures without alerts fail the job', () => {
  const report = audit(ids, measured, 'Unauthorized', 'failure');
  assert.equal(report.uploadStatus, 'error');
  assert.ok(report.errors.length > 0);
});
test('partial uploads with alerts still fail the job', () => {
  assert.ok(audit(ids, measured, `${results.split('\n')[0]}\n${alert}`, 'failure').errors.length > 0);
});
test('green empty or unexpected uploads fail', () => {
  assert.ok(audit(ids, measured, '', 'success').errors.length > 0);
  assert.ok(audit(ids, measured, `${results}\n- extra (Latency): https://bencher.dev/console/projects/minigraf/perf?x=1`, 'success').errors.length > 0);
});
test('missing or duplicate analysis fails', () => {
  assert.ok(audit(ids, measured.split('\n')[0], results, 'success').errors.length > 0);
  assert.ok(audit(ids, `${measured}\n${measured}`, results, 'success').errors.length > 0);
});
test('skipped upload never counts as verified', () => {
  assert.ok(audit(ids, measured, results, 'skipped').errors.length > 0);
});

test('duplicate upload results fail', () => {
  assert.ok(audit(ids, measured, `${results}\n${results}`, 'success').errors.length > 0);
});

test('CLI retains provenance, IDs and summary on success and early failure', () => {
  const fs = require('node:fs');
  const os = require('node:os');
  const path = require('node:path');
  const { spawnSync } = require('node:child_process');
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'minigraf-bench-report-'));
  try {
    const summary = path.join(cwd, 'summary.md');
    const env = {
      ...process.env, BENCH_SUITE: 'window', BENCH_REF: 'v3',
      BENCH_SHA: 'a'.repeat(40), WORKFLOW_SHA: 'b'.repeat(40),
      BENCH_FILTER: '^window/', BENCH_THRESHOLD: '0.999',
      BENCH_FILE: 'bench.txt', BENCHER_OUTCOME: 'success', GITHUB_STEP_SUMMARY: summary,
    };
    const run = command => spawnSync(process.execPath, [path.join(__dirname, 'report.cjs'), command], { cwd, env, encoding: 'utf8' });
    fs.writeFileSync(path.join(cwd, 'bench-list.txt'), listing);
    assert.equal(run('select').status, 0);
    fs.writeFileSync(path.join(cwd, 'bench.txt'), measured);
    assert.equal(run('execution').status, 0);
    fs.writeFileSync(path.join(cwd, 'bencher_out.txt'), results);
    assert.equal(run('finish').status, 0);
    const report = JSON.parse(fs.readFileSync(path.join(cwd, 'bench-report.json')));
    assert.equal(report.sha, env.BENCH_SHA);
    assert.equal(report.workflowSha, env.WORKFLOW_SHA);
    assert.deepEqual(report.selected, ids);
    assert.deepEqual(report.uploaded, ids);
    assert.match(fs.readFileSync(summary, 'utf8'), /Selected: 2; analyzed: 2; uploaded: 2/);
    fs.rmSync(path.join(cwd, 'bench-selection.json'));
    fs.rmSync(path.join(cwd, 'bench.txt'));
    fs.rmSync(path.join(cwd, 'bencher_out.txt'));
    env.BENCHER_OUTCOME = 'skipped';
    assert.equal(run('finish').status, 1);
    assert.match(fs.readFileSync(summary, 'utf8'), /coverage: FAILED/);
    assert.equal(JSON.parse(fs.readFileSync(path.join(cwd, 'bench-report.json'))).sha, env.BENCH_SHA);
  } finally { fs.rmSync(cwd, { recursive: true, force: true }); }
});
