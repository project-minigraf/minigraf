// Run workflow shell/notification bodies locally; no network or GitHub writes.
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const workflow = fs.readFileSync(path.join(__dirname, '../workflows/bench.yml'), 'utf8');
const bindingJob = workflow.split('\n  bindings:\n')[1];
function body(name, kind = 'run') {
  const step = bindingJob.split(`      - name: ${name}\n`)[1];
  assert.ok(step, `missing step ${name}`);
  return step.split(`          ${kind}: |\n`)[1]?.split(/\n      - /)[0].replace(/^            /gm, '').trimEnd()
    || step.split(`        ${kind}: |\n`)[1].split(/\n      - /)[0].replace(/^          /gm, '').trimEnd();
}
function shell(name, env, setup = () => {}, prefix = '') {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'binding-workflow-'));
  try {
    setup(cwd);
    const result = spawnSync('bash', ['-e', '-c', prefix + body(name).replace(/\$\{\{ secrets\.BENCHER_API_TOKEN \}\}/g, 'test-token')], {
      cwd, encoding: 'utf8', env: {...process.env, GITHUB_OUTPUT: path.join(cwd, 'out'), GITHUB_ENV: path.join(cwd, 'env'), GITHUB_STEP_SUMMARY: path.join(cwd, 'summary'), ...env},
    });
    const read = file => fs.existsSync(path.join(cwd, file)) ? fs.readFileSync(path.join(cwd, file), 'utf8') : '';
    return {...result, output: read('out'), vars: read('env'), summary: read('summary')};
  } finally { fs.rmSync(cwd, {recursive: true, force: true}); }
}
test('new rows use the actual binding suite job names', () => {
  assert.match(bindingJob, /name: bench \/ binding_\$\{\{ matrix.binding \}\}/);
  assert.match(bindingJob, /binding: jvm, repo: minigraf-java/);
  assert.match(bindingJob, /binding: wasi/);
  assert.match(workflow, /jvm-ref:/);
});
test('main skips with a summary; a missing v3 harness fails', () => {
  const main = shell('Check v3 binding suite availability', {BENCH_REF: 'main', BINDING: 'wasi'});
  assert.equal(main.status, 0, main.stderr);
  assert.match(main.summary, /Skipping/);
  assert.doesNotMatch(main.output, /enabled=true/);
  const missing = shell('Check v3 binding suite availability', {BENCH_REF: 'v3', BINDING: 'wasi'});
  assert.notEqual(missing.status, 0);
});
for (const binding of ['jvm', 'wasi']) test(`${binding} uploads only to its own testbed without inherited history`, () => {
  for (const history of ['v3', 'binding-experiment/experiment/v3']) {
    const result = shell('Upload foreign runtime metrics', {BINDING: binding, BINDING_HISTORY: history, BENCH_SHA: 'a'.repeat(40), BENCH_THRESHOLD: '0.999'}, undefined,
      'bencher() { printf "%s\\0" "$@"; }\n');
    assert.equal(result.status, 0, result.stderr);
    const args = result.stdout.split('\0');
    assert.equal(args[args.indexOf('--testbed') + 1], `binding-${binding}-ubuntu24-v1`);
    assert.equal(args[args.indexOf('--branch') + 1], history);
    assert.equal(args[args.indexOf('--threshold-max-sample-size') + 1], '30');
    assert.equal(args[args.indexOf('--threshold-upper-boundary') + 1], '0.999');
    assert.ok(!args.some(a => a.startsWith('--start-point')));
  }
});

const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
for (const binding of ['jvm', 'wasi']) test(`${binding} repeated exact IDs, immediate upload failure and failed lookup`, async () => {
  const suite = `binding_${binding}`, id = `binding/${binding}/example`;
  const alert = name => `- ${name} (Latency): https://bencher.dev/console/projects/minigraf/alerts/123`;
  const original = {...process.env};
  try {
    Object.assign(process.env, {SUITE: suite, BENCH_REF: 'v3', BENCH_SHA: 'a'.repeat(40)});
    for (const scenario of ['repeat', 'different-id', 'upload-failure', 'lookup-failure', 'existing-issue', 'experiment-prior']) {
      const calls = [], warnings = [];
      const github = {rest: {actions: {
        listWorkflowRuns: async () => ({data: {workflow_runs: [{id: 1, display_title: 'Nightly Benchmarks (main)'}, {id: 2, display_title: 'Nightly Benchmarks (v3)', html_url: 'prior'}]}}),
        listJobsForWorkflowRun: async args => { assert.equal(args.run_id, 2); return {data: {jobs: [{id: 3, name: `bench / ${suite}`}, {id: 4, name: 'bench / binding_python'}]}}; },
        downloadJobLogsForWorkflowRun: async args => {
          assert.equal(args.job_id, 3);
          if (scenario === 'lookup-failure') throw new Error('unavailable logs');
          return {data: `History branch: ${scenario === 'experiment-prior' ? 'binding-experiment/v3/java-only' : 'v3'}; testbed: binding-${binding}-ubuntu24-v1;\n` + alert(scenario === 'different-id' ? `${id}/other` : id)};
        },
      }, issues: {
        listForRepo: async () => ({data: scenario === 'existing-issue' ? [{number: 8, title: `Benchmark regression: ${suite} (v3)`}] : []}),
        create: async args => { calls.push(args); return {data: {number: 9}}; },
        createComment: async args => { calls.push(args); },
      }}};
      const summary = {addHeading() {return this;}, addRaw() {return this;}, addList() {return this;}, addBreak() {return this;}, async write() {}};
      const mockRequire = name => { assert.equal(name, 'fs'); return {readFileSync: () => scenario === 'upload-failure' ? 'Unauthorized' : alert(id)}; };
      await new AsyncFunction('require', 'github', 'context', 'core', 'console', body('Report regression', 'script'))(
        mockRequire, github, {repo: {owner: 'test', repo: 'test'}, runId: 10, serverUrl: 'https://example.com'}, {warning: text => warnings.push(text), summary}, {log() {}});
      assert.equal(calls.length, ['repeat', 'upload-failure', 'existing-issue'].includes(scenario) ? 1 : 0);
      if (scenario === 'lookup-failure') assert.ok(warnings.some(w => w.includes('Could not read')));
      if (scenario === 'upload-failure') assert.match(calls[0].body, /API or upload error/);
      if (scenario === 'repeat') assert.match(calls[0].body, /two runs in a row/);
      if (scenario === 'existing-issue') assert.equal(calls[0].issue_number, 8);
    }
  } finally {
    for (const key of ['SUITE', 'BENCH_REF', 'BENCH_SHA']) {
      if (original[key] === undefined) delete process.env[key]; else process.env[key] = original[key];
    }
  }
});

test('experimental binding and core refs are isolated, including WASI without a repo', () => {
  for (const [binding, ref, bindingRef, history] of [
    ['jvm', 'v3', 'v3', 'v3'],
    ['jvm', 'v3', 'java-only', 'binding-experiment/v3/java-only'],
    ['jvm', 'candidate', 'v3', 'binding-experiment/candidate/v3'],
    ['wasi', 'candidate', 'v3', 'binding-experiment/candidate/v3'],
  ]) {
    const result = shell('Pin binding source', {BINDING: binding, BENCH_REF: ref, BINDING_REF: bindingRef}, undefined,
      'git() { printf "%s\\n" binding-sha; }\n');
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.vars, new RegExp(`BINDING_HISTORY=${history}\\n`));
  }
});

test('JVM manual builder owns the patch and wrong Cargo resolutions fail', () => {
  const helper = path.join(__dirname, 'bindings.sh');
  for (const resolution of ['correct', 'release-pin', 'other-checkout', 'duplicate']) {
    const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'binding-build-'));
    try {
      fs.mkdirSync(path.join(cwd, 'binding/bench/manual'), {recursive: true});
      fs.mkdirSync(path.join(cwd, 'bin'));
      fs.writeFileSync(path.join(cwd, 'binding/bench/nightly.sh'), '# contract placeholder\n');
      fs.writeFileSync(path.join(cwd, 'binding/bench/manual/build.sh'), `set -euo pipefail
        test ! -e .cargo/config.toml
        test "$CARGO_BUILD_JOBS" = 4
        test "$CARGO_TARGET_DIR" = "$PWD/target"
        test "$CORE_REVISION" = "$BENCH_SHA"
        mkdir .cargo
        echo 'owned by manual build' > .cargo/config.toml
      `);
      fs.writeFileSync(path.join(cwd, 'bin/git'), '#!/bin/sh\nprintf "%s\\n" "$BENCH_SHA"\n', {mode: 0o755});
      const pkg = {name: 'minigraf', source: resolution === 'release-pin' ? 'git+release' : null,
        manifest_path: path.join(resolution === 'other-checkout' ? '/wrong' : cwd, 'Cargo.toml')};
      const metadata = {packages: resolution === 'duplicate' ? [pkg, pkg] : [pkg]};
      fs.writeFileSync(path.join(cwd, 'metadata.json'), JSON.stringify(metadata));
      fs.writeFileSync(path.join(cwd, 'bin/cargo'), '#!/bin/sh\ncat ../metadata.json\n', {mode: 0o755});
      const result = spawnSync('bash', [helper, 'build', 'jvm'], {cwd, encoding: 'utf8',
        env: {...process.env, PATH: `${cwd}/bin:${process.env.PATH}`, BENCH_SHA: 'a'.repeat(40)}});
      assert.equal(result.status === 0, resolution === 'correct', result.stderr);
    } finally { fs.rmSync(cwd, {recursive: true, force: true}); }
  }
});

test('JVM run refuses stale output and smoke that emits metrics', () => {
  for (const scenario of ['stale', 'smoke-metrics', 'failed-exit']) {
    const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'binding-run-'));
    try {
      fs.mkdirSync(path.join(cwd, 'binding/bench'), {recursive: true});
      fs.writeFileSync(path.join(cwd, 'binding/bench/nightly.sh'), scenario === 'failed-exit' ? 'exit 7\n' : 'echo "{}" > binding-results.json\n');
      if (scenario === 'stale') fs.writeFileSync(path.join(cwd, 'binding/binding-results.json'), '{}');
      const args = [path.join(__dirname, 'bindings.sh'), 'run', 'jvm'];
      if (scenario === 'smoke-metrics') args.push('--test');
      const result = spawnSync('bash', args, {cwd, encoding: 'utf8'});
      assert.notEqual(result.status, 0, scenario);
    } finally { fs.rmSync(cwd, {recursive: true, force: true}); }
  }
});
