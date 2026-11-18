const { test } = require('node:test');
const assert = require('node:assert/strict');
const { expected, validate } = require('./bindings-report.cjs');
const data = binding => Object.fromEntries(expected(binding).map(id => [id, { latency: { value: 100 } }]));
test('exact dedicated routes and finite positive latency only', () => {
  for (const binding of ['python', 'browser', 'node', 'c', 'jvm', 'wasi']) {
    assert.equal(validate(binding, data(binding)).length, ({ browser: 8, jvm: 14, wasi: 3 }[binding] || 6));
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
    const env = { ...process.env, BINDING: 'python', BENCH_SHA: 'core-commit', WORKFLOW_SHA: 'workflow-commit', BENCH_REF: 'v3', BINDING_HISTORY: 'binding-experiment/v3/pr', BENCHER_OUTCOME: 'success' };
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
    assert.equal(report.historyBranch, 'binding-experiment/v3/pr');
    assert.equal(report.uploaded.length, 0);
    assert.ok(report.errors.length);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});

// New lanes must keep distinct inventories, not alias the six P16 cases.
test('JVM lifecycle/storage and WASI REPL shortlists are exact', () => {
  assert.deepEqual(expected('wasi'), [
    'binding/wasi/repl_startup_empty', 'binding/wasi/repl_script/1k',
    'binding/wasi/repl_query_roundtrip/1k',
  ]);
  assert.deepEqual(expected('jvm').slice(6), [
    'prepare/point', 'register_predicate/first', 'register_aggregate/first',
    'fact_log_batch_256/1k', 'options_error/negative_cache',
    'options_readonly_open/1k', 'file_transact/1k', 'log_writer_append_finish/1k',
  ].map(id => `binding/jvm/${id}`));
});

const { parseMetrics } = require('./bindings-report.cjs');
const crypto = require('node:crypto');
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
test('duplicate JSON metric keys fail instead of being overwritten', () => {
  const id = expected('jvm')[0];
  assert.throws(() => parseMetrics(`{"${id}":{"latency":{"value":1}},"${id}":{"latency":{"value":2}}}`), /duplicate/);
  assert.throws(() => parseMetrics('{"a":{"value":1,"value":2}}'), /duplicate/);
  assert.deepEqual(parseMetrics('{"a":{"value":1},"b":{"value":2}}'), {a: {value: 1}, b: {value: 2}});
});

for (const binding of ['jvm', 'wasi']) test(`${binding} checks raw medians, provenance and artifact bytes before upload`, () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'binding-evidence-'));
  const originalCwd = process.cwd();
  try {
    // Model the default-branch workflow separately from the measured v3 tree.
    // This fixture tests validator selection; Python's own suite tests #839's
    // sample contract. It also keeps these shared Node tests runnable on main.
    const measuredHarness = path.join(dir, 'benchmarks/bindings839');
    const workflowHarness = path.join(dir, '.bench-workflow/benchmarks/bindings839');
    const workflowHelpers = path.join(dir, '.bench-workflow/.github/bench');
    fs.mkdirSync(measuredHarness, {recursive: true});
    fs.mkdirSync(workflowHarness, {recursive: true});
    fs.mkdirSync(workflowHelpers, {recursive: true});
    for (const file of ['bindings-report.cjs', 'report.cjs']) {
      fs.copyFileSync(path.join(__dirname, file), path.join(workflowHelpers, file));
    }
    fs.writeFileSync(path.join(measuredHarness, 'validate.py'), `import json, pathlib, sys
pathlib.Path('measured-validator-ran').write_text(__file__)
records = [json.loads(line) for line in pathlib.Path(sys.argv[2]).read_text().splitlines()]
if records[-1]['type'] != 'complete':
    sys.exit('missing complete marker')
`);
    for (const file of ['nightly.py', 'wasi.py']) fs.writeFileSync(path.join(measuredHarness, file), file);
    fs.writeFileSync(path.join(workflowHarness, 'validate.py'), "raise SystemExit('tampered workflow validator ran')\n");
    const { verifyEvidence } = require(path.join(workflowHelpers, 'bindings-report.cjs'));
    process.chdir(dir);
    fs.mkdirSync(path.join(dir, 'benchmark'));
    const coreSha = 'a'.repeat(40), workflowSha = 'b'.repeat(40), bindingSha = binding === 'jvm' ? 'c'.repeat(40) : null;
    const ids = expected(binding);
    const environment = binding === 'jvm' ? {
      java_vendor: 'Eclipse Adoptium', java_version: '17.0.17', vm_version: '17.0.17+10',
      jvm_arguments: ['-Xms512m', '-Xmx512m', '-XX:+UseG1GC'], warmup_memory: 100, warmup_storage: 5,
      build: {core_revision: coreSha, binding_revision: bindingSha},
    } : {
      core_revision: coreSha, wasm_sha256: sha('actual artifact'),
      runtime_version: 'wasmtime 49.0.2 (3c8a3e79a 2026-10-02)',
      runtime_sha256: 'd0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635',
      runtime_flags: ['run', '-C', 'cache=n', '-C', 'parallel-compilation=n'],
      runtime_cache: false, storage: 'in-memory; no preopened directories', warmups: 3, samples: 10,
    };
    const records = [{ type: 'environment', mode: 'benchmark', ...environment }];
    const metrics = {};
    for (const id of ids) {
      const iterations = id.endsWith('repl_query_roundtrip/1k') ? 20 : binding === 'wasi' || /(?:file_transact|log_writer_append_finish)\/1k$/.test(id) ? 1 : 20;
      for (let sample = 0; sample < 10; sample++) records.push({type: 'sample', id, sample, iterations, total_ns: (sample + 1) * iterations, ns_per_op: sample + 1});
      records.push({type: 'checked', id});
      metrics[id] = { latency: { value: 5.5 } };
    }
    records.push({type: 'complete', mode: 'benchmark', count: ids.length, ids});
    const raw = path.join(dir, 'benchmark/samples.jsonl');
    fs.writeFileSync(raw, records.map(r => JSON.stringify(r)).join('\n'));
    const artifactPath = binding === 'jvm' ? 'benchmark/libminigraf_ffi.so' : 'minigraf.wasm';
    fs.writeFileSync(path.join(dir, artifactPath), 'actual artifact');
    const harnessFiles = binding === 'jvm'
      ? Object.fromEntries(['bench/nightly.sh', 'bench/manual/build.sh', 'bench/manual/build.gradle.kts',
        'bench/manual/src/main/kotlin/io/github/project_minigraf/benchmark/Benchmark.kt',
        'bench/manual/src/main/kotlin/io/github/project_minigraf/benchmark/ResultChecks.kt'].map(p => [p, sha(p)]))
      : Object.fromEntries(['nightly.py', 'wasi.py', 'validate.py'].map(p => [p, sha(fs.readFileSync(path.join(measuredHarness, p)))]));
    if (binding === 'jvm') for (const file of Object.keys(harnessFiles)) {
      fs.mkdirSync(path.dirname(path.join(dir, file)), {recursive: true});
      fs.writeFileSync(path.join(dir, file), file);
    }
    const provenance = { coreSha, workflowSha, bindingSha, artifactPath, artifactSha256: sha('actual artifact'), harnessFiles,
      harnessSha256: sha(JSON.stringify(Object.fromEntries(Object.entries(harnessFiles).sort()))),
      runtime: binding === 'jvm'
        ? { distribution: 'temurin', vendor: 'Eclipse Adoptium', version: '17.0.17+10', flags: ['-Xms512m', '-Xmx512m', '-XX:+UseG1GC'] }
        : { version: 'wasmtime 49.0.2 (3c8a3e79a 2026-10-02)', sha256: 'd0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635', flags: ['run', '-C', 'cache=n', '-C', 'parallel-compilation=n'] },
    };
    const file = path.join(dir, 'binding-provenance.json');
    fs.writeFileSync(file, JSON.stringify(provenance));
    const identity = { coreSha, workflowSha, bindingSha };
    assert.equal(verifyEvidence(binding, metrics, dir, identity).coreSha, coreSha);
    assert.equal(fs.readFileSync('measured-validator-ran', 'utf8'), path.join(measuredHarness, 'validate.py'));
    // main has no workflow harness at all; validation still uses measured v3.
    fs.rmSync(workflowHarness, {recursive: true});
    assert.equal(verifyEvidence(binding, metrics, dir, identity).coreSha, coreSha);
    metrics[ids[0]].latency.value = 6;
    assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /median/);
    metrics[ids[0]].latency.value = 5.5;
    for (const field of ['coreSha', 'workflowSha', 'bindingSha']) {
      if (field === 'bindingSha' && binding === 'wasi') continue;
      assert.throws(() => verifyEvidence(binding, metrics, dir, {...identity, [field]: 'd'.repeat(40)}), /provenance/);
    }
    fs.writeFileSync(file, JSON.stringify({...provenance, runtime: {...provenance.runtime, flags: []}}));
    assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /runtime/);
    fs.writeFileSync(file, JSON.stringify(provenance));
    fs.writeFileSync(path.join(dir, artifactPath), 'different artifact');
    assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /artifact/);
    fs.writeFileSync(path.join(dir, artifactPath), 'actual artifact');
    const wrongEnvironment = {...records[0], ...(binding === 'jvm' ? {warmup_memory: 3} : {runtime_cache: true})};
    fs.writeFileSync(raw, [wrongEnvironment, ...records.slice(1)].map(r => JSON.stringify(r)).join('\n'));
    assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /environment/);
    fs.writeFileSync(raw, records.slice(0, -1).map(r => JSON.stringify(r)).join('\n'));
    assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /samples/);
    fs.writeFileSync(raw, records.map(r => JSON.stringify(r)).join('\n'));
    // A workflow copy cannot rescue a missing measured validator on v3.
    fs.mkdirSync(workflowHarness, {recursive: true});
    fs.writeFileSync(path.join(workflowHarness, 'validate.py'), '# accepts everything\n');
    if (binding === 'wasi') {
      fs.appendFileSync(path.join(measuredHarness, 'validate.py'), '# changed since measurement\n');
      assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /harness hash/);
    }
    fs.unlinkSync(path.join(measuredHarness, 'validate.py'));
    assert.throws(() => verifyEvidence(binding, metrics, dir, identity), /validate.py|raw samples/);
  } finally {
    process.chdir(originalCwd);
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
