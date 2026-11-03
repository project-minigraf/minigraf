const { test } = require('node:test');
const assert = require('node:assert/strict');
const { expected, validate } = require('./bindings-report.cjs');
const data = binding => Object.fromEntries(expected(binding).map(id => [id, { latency: { value: 100 } }]));
test('exact dedicated routes and finite positive latency only', () => {
  for (const binding of ['python', 'browser', 'node', 'c']) {
    assert.equal(validate(binding, data(binding)).length, binding === 'browser' ? 8 : 6);
    const missing = data(binding); delete missing[expected(binding)[0]];
    assert.throws(() => validate(binding, missing), /IDs/);
    const extra = data(binding); extra['native/query'] = { latency: { value: 1 } };
    assert.throws(() => validate(binding, extra), /IDs/);
    for (const value of [0, -1, null, Infinity, '12']) {
      const bad = data(binding); bad[expected(binding)[0]].latency.value = value;
      assert.throws(() => validate(binding, bad), /latency/);
    }
  }
  assert.throws(() => expected('unknown'), /binding/);
});

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
test('CLI verifies execution and records provenance even when upload is partial', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'p16-report-'));
  try {
    fs.mkdirSync(path.join(dir, 'binding'));
    fs.writeFileSync(path.join(dir, 'binding/binding-results.json'), JSON.stringify(data('python')));
    fs.writeFileSync(path.join(dir, 'binding-execution.txt'), expected('python').map(id => `ok: ${id}`).join('\n'));
    fs.writeFileSync(path.join(dir, 'binding-sha.txt'), 'binding-commit\n');
    const env = { ...process.env, BINDING: 'python', BENCH_SHA: 'core-commit', WORKFLOW_SHA: 'workflow-commit', BENCH_REF: 'v3', BENCHER_OUTCOME: 'success' };
    const run = command => spawnSync(process.execPath, [path.join(__dirname, 'bindings-report.cjs'), command], { cwd: dir, env, encoding: 'utf8' });
    assert.equal(run('validate').status, 0);
    fs.appendFileSync(path.join(dir, 'binding-execution.txt'), '\nok: binding/python/aggregate/1k');
    assert.notEqual(run('validate').status, 0);
    assert.notEqual(run('finish').status, 0);
    const report = JSON.parse(fs.readFileSync(path.join(dir, 'binding-report.json')));
    assert.equal(report.coreSha, 'core-commit');
    assert.equal(report.bindingSha, 'binding-commit');
    assert.equal(report.workflowSha, 'workflow-commit');
    assert.equal(report.testbed, 'binding-python-ubuntu24-v1');
    assert.equal(report.uploaded.length, 0);
    assert.ok(report.errors.length);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
