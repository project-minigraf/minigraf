// Binding jobs use Bencher JSON (nanoseconds), with separate foreign testbeds.
const fs = require('node:fs');
const { audit } = require('./report.cjs');
function expected(binding) {
  if (!['python', 'browser', 'node', 'c', 'jvm', 'wasi'].includes(binding)) throw new Error('unknown binding');
  if (binding === 'wasi') return ['repl_startup_empty', 'repl_script/1k', 'repl_query_roundtrip/1k'].map(s => `binding/wasi/${s}`);
  const suffixes = ['prepared_execute/1k', 'cursor_batch_1/1k', 'cursor_batch_256/1k', 'set_roundtrip/2', 'predicate/1k', 'aggregate/1k'];
  if (binding === 'browser') suffixes.push('indexeddb_write/1k', 'indexeddb_reopen/1k');
  if (binding === 'jvm') suffixes.push('prepare/point', 'register_predicate/first', 'register_aggregate/first',
    'fact_log_batch_256/1k', 'options_error/negative_cache', 'options_readonly_open/1k',
    'file_transact/1k', 'log_writer_append_finish/1k');
  return suffixes.map(s => `binding/${binding}/${s}`);
}
function validate(binding, metrics) {
  const ids = expected(binding);
  if (JSON.stringify(Object.keys(metrics).sort()) !== JSON.stringify([...ids].sort())) throw new Error('binding IDs do not match shortlist');
  for (const id of ids) {
    const value = metrics[id]?.latency?.value;
    if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) throw new Error(`invalid latency: ${id}`);
  }
  return ids;
}
// JSON.parse alone silently overwrites duplicate keys. Parse syntax first, then
// inspect object keys (including escaped spellings) in the valid token stream.
function parseMetrics(text) {
  const result = JSON.parse(text);
  const tokens = text.match(/"(?:\\.|[^"\\])*"|[{}\[\]:,]|[^\s{}\[\]:,]+/g) || [];
  const objects = [];
  for (let i = 0; i < tokens.length; i++) {
    const token = tokens[i];
    if (token === '{') objects.push(new Set());
    else if (token === '}') objects.pop();
    else if (token.startsWith('"') && tokens[i + 1] === ':') {
      const key = JSON.parse(token), keys = objects.at(-1);
      if (keys.has(key)) throw new Error(`duplicate JSON key: ${key}`);
      keys.add(key);
    }
  }
  return result;
}

function verifyEvidence(binding, metrics, directory, identity) {
  const path = require('node:path');
  const { spawnSync } = require('node:child_process');
  const sha = bytes => require('node:crypto').createHash('sha256').update(bytes).digest('hex');
  const read = name => fs.readFileSync(path.join(directory, name), 'utf8');
  const provenance = parseMetrics(read('binding-provenance.json'));
  for (const field of ['coreSha', 'workflowSha', 'bindingSha']) {
    if (provenance[field] !== identity[field] ||
        (!(binding === 'wasi' && field === 'bindingSha') && !/^[a-f0-9]{40}$/.test(provenance[field]))) {
      throw new Error(`wrong provenance: ${field}`);
    }
  }
  const artifact = binding === 'jvm' ? 'benchmark/libminigraf_ffi.so' : 'minigraf.wasm';
  if (provenance.artifactPath !== artifact || sha(fs.readFileSync(path.join(directory, artifact))) !== provenance.artifactSha256) {
    throw new Error('wrong executed artifact');
  }
  const harness = provenance.harnessFiles;
  const required = binding === 'jvm' ? ['bench/nightly.sh', 'bench/manual/build.sh', 'bench/manual/build.gradle.kts',
    'bench/manual/src/main/kotlin/io/github/project_minigraf/benchmark/Benchmark.kt',
    'bench/manual/src/main/kotlin/io/github/project_minigraf/benchmark/ResultChecks.kt'] : ['nightly.py', 'wasi.py', 'validate.py'];
  if (!harness || required.some(file => !harness[file])) throw new Error('missing harness hashes');
  // Python validation for both lanes belongs to the measured core, even when
  // these reporting helpers come from main (which has no bindings839 harness).
  // WASI provenance must hash the same validator that we execute below.
  const measuredHarness = path.resolve('benchmarks/bindings839');
  const harnessRoot = binding === 'jvm' ? directory : measuredHarness;
  for (const [file, digest] of Object.entries(harness)) {
    if (path.isAbsolute(file) || file.split('/').includes('..') ||
        sha(fs.readFileSync(path.join(harnessRoot, file))) !== digest) throw new Error('wrong harness hash');
  }
  if (sha(JSON.stringify(Object.fromEntries(Object.entries(harness).sort()))) !== provenance.harnessSha256) {
    throw new Error('wrong combined harness hash');
  }
  const runtime = provenance.runtime;
  if (binding === 'jvm') {
    if (provenance.launchEnvironment?.LD_LIBRARY_PATH !== null) throw new Error('wrong JVM launch environment');
    if (runtime?.distribution !== 'temurin' || runtime.vendor !== 'Eclipse Adoptium' || runtime.version !== '17.0.17+10' ||
        JSON.stringify(runtime.flags) !== JSON.stringify(['-Xms512m', '-Xmx512m', '-XX:+UseG1GC'])) throw new Error('wrong JVM runtime');
  } else if (runtime?.version !== 'wasmtime 49.0.2 (3c8a3e79a 2026-10-02)' ||
      runtime.sha256 !== 'd0a014e0d5b0cf48dd3549c38e2e6ecd78ff09fb3e3f10d751b87b72bdfd8635' ||
      JSON.stringify(runtime.flags) !== JSON.stringify(['run', '-C', 'cache=n', '-C', 'parallel-compilation=n'])) {
    throw new Error('wrong WASI runtime');
  }
  if (binding === 'wasi' && (provenance.cargoLockPath !== 'Cargo.lock' ||
      sha(fs.readFileSync(path.join(directory, 'Cargo.lock'))) !== provenance.cargoLockSha256)) {
    throw new Error('wrong WASI lockfile hash');
  }
  const rawPath = path.join(directory, 'benchmark/samples.jsonl');
  // Keep #839's validator authoritative for sample IDs/counts/iterations and
  // the complete/check markers, including rejection of smoke streams.
  const validation = spawnSync('python3', ['-I', path.join(measuredHarness, 'validate.py'), binding, rawPath], { encoding: 'utf8' });
  if (validation.status !== 0) throw new Error(`invalid raw samples: ${validation.stderr || validation.error}`);
  const records = read('benchmark/samples.jsonl').trim().split('\n').map(parseMetrics);
  const environment = records.find(r => r.type === 'environment');
  if (binding === 'jvm') {
    if (environment.java_vendor !== runtime.vendor || environment.java_version !== '17.0.17' ||
        environment.vm_version !== runtime.version || JSON.stringify(environment.jvm_arguments) !== JSON.stringify(runtime.flags) ||
        environment.warmup_memory !== 100 || environment.warmup_storage !== 5 ||
        environment.build?.core_revision !== identity.coreSha || environment.build?.binding_revision !== identity.bindingSha) {
      throw new Error('wrong JVM sample environment');
    }
  } else if (environment.core_revision !== identity.coreSha || environment.wasm_sha256 !== provenance.artifactSha256 ||
      environment.runtime_version !== runtime.version || environment.runtime_sha256 !== runtime.sha256 ||
      JSON.stringify(environment.runtime_flags) !== JSON.stringify(runtime.flags) || environment.runtime_cache !== false ||
      environment.storage !== 'in-memory; no preopened directories' || environment.warmups !== 3 || environment.samples !== 10) {
    throw new Error('wrong WASI sample environment');
  }
  for (const id of expected(binding)) {
    const means = records.filter(r => r.type === 'sample' && r.id === id).map(r => r.total_ns / r.iterations).sort((a, b) => a - b);
    const median = (means[4] + means[5]) / 2;
    if (Math.abs(metrics[id].latency.value - median) > Math.abs(median) * 1e-12) throw new Error(`wrong raw-sample median: ${id}`);
  }
  return provenance;
}

function main(command) {
  const read = p => fs.existsSync(p) ? fs.readFileSync(p, 'utf8') : '';
  const binding = process.env.BINDING;
  const ids = expected(binding);
  if (command === 'validate') {
    const metrics = parseMetrics(read('binding/binding-results.json'));
    validate(binding, metrics);
    if (['jvm', 'wasi'].includes(binding)) verifyEvidence(binding, metrics, 'binding', {
      coreSha: process.env.BENCH_SHA, workflowSha: process.env.WORKFLOW_SHA,
      bindingSha: binding === 'wasi' ? null : read('binding-sha.txt').trim(),
    });
    const executed = read('binding-execution.txt').split('\n').filter(s => s.startsWith('ok: ')).map(s => s.slice(4));
    if (executed.length !== ids.length || new Set(executed).size !== ids.length || executed.some(id => !ids.includes(id))) throw new Error('executed IDs do not match shortlist');
    return;
  }
  if (command !== 'finish') throw new Error('unknown command');
  // Reuse the tested upload/alert parser without representing these as native
  // Criterion measurements: only the list of executed IDs is adapted.
  const execution = read('binding-execution.txt').replace(/^ok: (.+)$/gm, 'Benchmarking $1: Analyzing');
  const result = audit(ids, execution, read('binding-bencher.txt'), process.env.BENCHER_OUTCOME);
  const report = {
    binding, coreSha: process.env.BENCH_SHA, bindingSha: binding === 'wasi' ? null : read('binding-sha.txt').trim(),
    workflowSha: process.env.WORKFLOW_SHA, ref: process.env.BENCH_REF, historyBranch: process.env.BINDING_HISTORY,
    suite: `binding_${binding}`, testbed: `binding-${binding}-ubuntu24-v1`, selected: ids, ...result,
    provenance: read('binding/binding-provenance.json') ? parseMetrics(read('binding/binding-provenance.json')) : null,
  };
  fs.writeFileSync('binding-report.json', JSON.stringify(report, null, 2) + '\n');
  const summary = `### Binding benchmark: ${binding}\n\nCore: ${report.coreSha}; binding: ${report.bindingSha}; workflow: ${report.workflowSha}\n\nHistory branch: ${report.historyBranch}; testbed: ${report.testbed}; executed ${result.executed.length}/${ids.length}; uploaded ${result.uploaded.length}/${ids.length}; alerts ${result.alerts.length}; upload ${result.uploadStatus}\n\n${result.errors.join('\n')}\n`;
  console.log(summary);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
  if (result.errors.length) process.exitCode = 1;
}
module.exports = { expected, validate, parseMetrics, verifyEvidence };
if (require.main === module) {
  try { main(process.argv[2]); } catch (e) { console.error(e.message); process.exitCode = 1; }
}
