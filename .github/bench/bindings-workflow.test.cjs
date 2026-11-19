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
test('main skips; either v3 lane fails when its measured validator is absent', () => {
  for (const binding of ['jvm', 'wasi']) {
    const main = shell('Check v3 binding suite availability', {BENCH_REF: 'main', BINDING: binding});
    assert.equal(main.status, 0, main.stderr);
    assert.match(main.summary, /Skipping/);
    assert.doesNotMatch(main.output, /enabled=true/);
    for (const missing of [true, false]) {
      const result = shell('Check v3 binding suite availability', {BENCH_REF: 'v3', BINDING: binding}, cwd => {
        for (const file of ['benches/minigraf_bench/prepared.rs', 'benchmarks/bindings839/nightly.py',
          'benchmarks/bindings839/wasi.py', ...(missing ? [] : ['benchmarks/bindings839/validate.py'])]) {
          fs.mkdirSync(path.dirname(path.join(cwd, file)), {recursive: true});
          fs.writeFileSync(path.join(cwd, file), 'fixture');
        }
      });
      assert.equal(result.status === 0, !missing, result.stderr);
      assert.equal(result.output.includes('enabled=true'), !missing);
    }
  }
});
test('workflow checkout needs only shared helpers, never the measured Python harness', () => {
  const checkout = bindingJob.split('ref: ${{ github.workflow_sha }}')[1].split('persist-credentials: false')[0];
  assert.match(checkout, /\/\.github\/bench\//);
  assert.doesNotMatch(checkout, /bindings839/);
});
test('reporting CI skips absent Python tests on main and runs them on v3', () => {
  const reporting = fs.readFileSync(path.join(__dirname, '../workflows/bench-report.yml'), 'utf8');
  assert.equal((reporting.match(/'benchmarks\/bindings839\/\*\*'/g) || []).length, 2, 'keep both path triggers');
  const step = reporting.split('      - ').find(s => s.includes('python3 -I -m unittest'));
  assert.match(step, /if:.*hashFiles\('benchmarks\/bindings839\/test_\*\.py'\) != ''/);
  const expression = step.match(/if: \$\{\{ (.+) \}\}/)[1];
  for (const [hash, shouldRun] of [['', false], ['test-files-hash', true]]) {
    const hashFiles = pattern => {
      assert.equal(pattern, 'benchmarks/bindings839/test_*.py');
      return hash;
    };
    assert.equal(new Function('hashFiles', `return ${expression}`)(hashFiles), shouldRun);
  }
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
        test "\${LD_LIBRARY_PATH+x}" != x
        test ! -e ../Cargo.lock
        echo resolved > Cargo.lock
        mkdir .cargo
        echo 'owned by manual build' > .cargo/config.toml
      `);
      fs.writeFileSync(path.join(cwd, 'bin/git'), '#!/bin/sh\nprintf "%s\\n" "$BENCH_SHA"\n', {mode: 0o755});
      const pkg = {name: 'minigraf', source: resolution === 'release-pin' ? 'git+release' : null,
        manifest_path: path.join(resolution === 'other-checkout' ? '/wrong' : cwd, 'Cargo.toml')};
      const metadata = {packages: resolution === 'duplicate' ? [pkg, pkg] : [pkg]};
      fs.writeFileSync(path.join(cwd, 'metadata.json'), JSON.stringify(metadata));
      fs.writeFileSync(path.join(cwd, 'bin/cargo'), '#!/bin/sh\ntest -f Cargo.lock || exit 9\ncat ../metadata.json\n', {mode: 0o755});
      const result = spawnSync('bash', [helper, 'build', 'jvm'], {cwd, encoding: 'utf8',
        env: {...process.env, PATH: `${cwd}/bin:${process.env.PATH}`, LD_LIBRARY_PATH: '/hosted/python/lib', BENCH_SHA: 'a'.repeat(40)}});
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

for (const binding of ['jvm', 'wasi']) test(`${binding} finds the most recent production history within a bounded search`, async () => {
  const original = {...process.env};
  const suite = `binding_${binding}`, id = `binding/${binding}/example`;
  const alert = `- ${id} (Latency): https://bencher.dev/console/projects/minigraf/alerts/123`;
  const production = `History branch: v3; testbed: binding-${binding}-ubuntu24-v1;`;
  try {
    Object.assign(process.env, {SUITE: suite, BENCH_REF: 'v3', BENCH_SHA: 'a'.repeat(40)});
    for (const scenario of ['repeat-after-experiment', 'clean-production', 'wrong-testbed', 'no-suite', 'lookup-failure', 'exhausted', 'failed-before-summary', 'failed-experiment', 'failed-production-alert']) {
      const calls = [], warnings = [], visited = [];
      const github = {rest: {actions: {
        listWorkflowRuns: async args => {
          assert.equal(args.status, 'completed');
          assert.equal(args.per_page, 20);
          return {data: {workflow_runs: [
            {id: 100, display_title: 'Nightly Benchmarks (v3)'}, // current
            {id: 99, display_title: 'Nightly Benchmarks (main)'},
            ...Array.from({length: 18}, (_, i) => ({id: 98 - i, display_title: 'Nightly Benchmarks (v3)', html_url: `prior-${98 - i}`})),
          ]}};
        },
        listJobsForWorkflowRun: async args => {
          visited.push(args.run_id);
          return {data: {jobs: scenario === 'no-suite' && args.run_id === 98 ? [] : [{id: args.run_id, name: `bench / ${suite}`, status: 'completed', conclusion: scenario.startsWith('failed-') ? 'failure' : 'success'}]}};
        },
        downloadJobLogsForWorkflowRun: async ({job_id}) => {
          if (scenario === 'lookup-failure' && job_id === 97) throw new Error('unavailable logs');
          if (job_id === 98 && scenario === 'failed-before-summary') return {data: `build failed\n${alert}`};
          if (job_id === 98 && scenario === 'failed-production-alert') return {data: `${production}\n${alert}`};
          if (job_id === 98 || scenario === 'exhausted') {
            const history = scenario === 'wrong-testbed' ? 'History branch: v3; testbed: binding-python-ubuntu24-v1;' :
              `History branch: binding-experiment/v3/java-only; testbed: binding-${binding}-ubuntu24-v1;`;
            return {data: `${history}\n${alert}`};
          }
          return {data: `${production}\n${scenario === 'clean-production' && job_id === 97 ? '' : alert}`};
        },
      }, issues: {
        listForRepo: async () => ({data: []}),
        create: async args => { calls.push(args); return {data: {number: 1}}; },
      }}};
      const summary = {addHeading() {return this;}, addRaw() {return this;}, addList() {return this;}, addBreak() {return this;}, async write() {}};
      await new AsyncFunction('require', 'github', 'context', 'core', 'console', body('Report regression', 'script'))(
        () => ({readFileSync: () => alert}), github,
        {repo: {owner: 'test', repo: 'test'}, runId: 100, serverUrl: 'https://example.com'},
        {warning: message => warnings.push(message), summary}, {log() {}});
      const reports = ['repeat-after-experiment', 'wrong-testbed', 'no-suite', 'failed-experiment', 'failed-production-alert'].includes(scenario);
      assert.equal(calls.length, reports ? 1 : 0, scenario);
      if (reports) assert.match(calls[0].body, new RegExp(`Previous run:\\*\\* prior-${scenario === 'failed-production-alert' ? 98 : 97}`));
      assert.deepEqual(visited, scenario === 'exhausted' ? Array.from({length: 18}, (_, i) => 98 - i) : ['failed-before-summary', 'failed-production-alert'].includes(scenario) ? [98] : [98, 97]);
      if (scenario === 'lookup-failure') assert.ok(warnings.some(w => w.includes('Could not read')));
      if (scenario === 'exhausted') assert.ok(warnings.some(w => w.includes('No matching production history')));
    }
  } finally {
    for (const key of ['SUITE', 'BENCH_REF', 'BENCH_SHA']) {
      if (original[key] === undefined) delete process.env[key]; else process.env[key] = original[key];
    }
  }
});

function executable(cwd, name, source) {
  fs.mkdirSync(path.join(cwd, 'bin'), {recursive: true});
  fs.writeFileSync(path.join(cwd, 'bin', name), `#!/bin/bash\nset -euo pipefail\n${source}\n`, {mode: 0o755});
}

test('WASI builds in a fresh core checkout and retains the generated lockfile', () => {
  const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'binding-wasi-build-'));
  try {
    fs.mkdirSync(path.join(cwd, 'benchmarks/bindings839'), {recursive: true});
    for (const name of ['nightly.py', 'wasi.py', 'validate.py']) fs.writeFileSync(path.join(cwd, 'benchmarks/bindings839', name), 'fixture');
    executable(cwd, 'git', 'echo "$BENCH_SHA"');
    // Model Cargo's lockfile rule; allow either explicit generation or an
    // unlocked initial build, but never --locked without a resolved lockfile.
    executable(cwd, 'cargo', `
      if [[ " $* " = *' --locked '* && ! -f Cargo.lock ]]; then
        echo 'cannot create the lock file Cargo.lock because --locked was passed' >&2
        exit 101
      fi
      [[ -f Cargo.lock ]] || echo 'resolved dependency fixture' > Cargo.lock
      if [[ "$1" = build ]]; then
        [[ " $* " = *' --release '* && " $* " = *' --target wasm32-wasip1 '* && " $* " = *' --bin minigraf '* ]]
        mkdir -p "$CARGO_TARGET_DIR/wasm32-wasip1/release"
        echo wasm > "$CARGO_TARGET_DIR/wasm32-wasip1/release/minigraf.wasm"
      else
        [[ "$1" = generate-lockfile ]]
      fi`);
    executable(cwd, 'curl', ':');
    executable(cwd, 'tar', ':');
    // No real runtime downloaded: retain the production checksum assertion.
    executable(cwd, 'sha256sum', `[[ "$*" = '--check --strict' ]]
      read -r digest filename
      [[ "$digest" = d0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635 ]]
      [[ "$filename" = binding/wasmtime-v49.0.2-x86_64-linux/wasmtime ]]`);
    assert.ok(!fs.existsSync(path.join(cwd, 'Cargo.lock')));
    const result = spawnSync('bash', [path.join(__dirname, 'bindings.sh'), 'build', 'wasi'], {
      cwd, encoding: 'utf8', env: {...process.env, PATH: `${cwd}/bin:${process.env.PATH}`, BENCH_SHA: 'a'.repeat(40)},
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(fs.readFileSync(path.join(cwd, 'binding/Cargo.lock'), 'utf8'), fs.readFileSync(path.join(cwd, 'Cargo.lock'), 'utf8'));
    assert.equal(fs.readFileSync(path.join(cwd, 'binding/minigraf.wasm'), 'utf8'), 'wasm\n');
    assert.match(bindingJob, /            binding\/Cargo.lock\n/);
  } finally { fs.rmSync(cwd, {recursive: true, force: true}); }
});

for (const [step, smoke] of [['Verify every binding case (untimed)', true], ['Measure binding shortlist', false]]) {
  test(`JVM ${step} removes hosted LD_LIBRARY_PATH and records the timed launch environment`, () => {
    const result = shell(step, {BINDING: 'jvm', LD_LIBRARY_PATH: '/hosted/python/lib'}, cwd => {
      const helpers = path.join(cwd, '.bench-workflow/.github/bench');
      fs.mkdirSync(helpers, {recursive: true});
      fs.copyFileSync(path.join(__dirname, 'bindings.sh'), path.join(helpers, 'bindings.sh'));
      fs.writeFileSync(path.join(helpers, 'bindings-report.cjs'), `
        const assert = require('node:assert/strict');
        const p = require(process.cwd() + '/binding/binding-provenance.json');
        assert.deepEqual(p.launchEnvironment, {LD_LIBRARY_PATH: null});
      `);
      fs.mkdirSync(path.join(cwd, 'binding/bench'), {recursive: true});
      fs.writeFileSync(path.join(cwd, 'binding/bench/nightly.sh'), `set -euo pipefail
        if [[ "\${LD_LIBRARY_PATH+x}" = x ]]; then
          echo 'nightly: LD_LIBRARY_PATH must be unset for a nightly run' >&2
          exit 1
        fi
        if [[ "\${1:-}" = --test ]]; then
          echo smoke
        else
          echo '{}' > binding-provenance.json
          echo benchmark
        fi
      `);
    });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, smoke ? /smoke/ : /benchmark/);
  });
}
