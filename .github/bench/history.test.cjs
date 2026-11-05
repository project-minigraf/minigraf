// Execute the workflow's upload shell with a capture-only Bencher function.
// This checks the actual arguments without uploading or accessing credentials.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const workflow = fs.readFileSync(path.join(__dirname, '../workflows/bench.yml'), 'utf8');
const step = workflow.split('      - name: Upload to Bencher\n')[1];
assert.ok(step, 'native upload step must exist');
const shell = step.split('        run: |\n')[1].split('\n\n')[0]
  .replace(/^          /gm, '')
  .replace(/\$\{\{ secrets\.BENCHER_API_TOKEN \}\}/g, 'unused-test-placeholder');

function uploadArgs(ref) {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'minigraf-history-'));
  try {
    const result = spawnSync('bash', ['-c', 'bencher() { printf "%s\\0" "$@"; }\n' + shell], {
      cwd, encoding: 'utf8',
      env: { ...process.env, BENCH_REF: ref, BENCH_SHA: 'a'.repeat(40),
        BENCH_THRESHOLD: '0.999', BENCH_FILE: 'unused.txt' },
    });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.split('\0').filter(Boolean);
  } finally {
    fs.rmSync(cwd, { recursive: true, force: true });
  }
}

for (const ref of ['main', 'v3']) {
  test(`${ref} preserves its own history when the other branch advances`, () => {
    const args = uploadArgs(ref);
    assert.equal(args[args.indexOf('--branch') + 1], ref);
    assert.ok(!args.some(arg => arg.startsWith('--start-point')),
      'a persistent release branch must not rebase its Bencher head');
    assert.equal(args[args.indexOf('--threshold-test') + 1], 't_test');
    assert.equal(args[args.indexOf('--threshold-max-sample-size') + 1], '30');
    assert.equal(args[args.indexOf('--threshold-upper-boundary') + 1], '0.999');
  });
}

test('ad hoc branches retain their explicit main comparison history', () => {
  const args = uploadArgs('experiment/840');
  assert.equal(args[args.indexOf('--branch') + 1], 'experiment/840');
  assert.equal(args[args.indexOf('--start-point') + 1], 'main');
  assert.ok(args.includes('--start-point-clone-thresholds'));
});
