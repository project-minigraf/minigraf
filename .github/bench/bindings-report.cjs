// Binding jobs use Bencher JSON (nanoseconds), with separate foreign testbeds.
const fs = require('node:fs');
const { audit } = require('./report.cjs');
function expected(binding) {
  if (!['python', 'browser', 'node', 'c'].includes(binding)) throw new Error('unknown binding');
  const suffixes = ['prepared_execute/1k', 'cursor_batch_1/1k', 'cursor_batch_256/1k', 'set_roundtrip/2', 'predicate/1k', 'aggregate/1k'];
  if (binding === 'browser') suffixes.push('indexeddb_write/1k', 'indexeddb_reopen/1k');
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
function main(command) {
  const read = p => fs.existsSync(p) ? fs.readFileSync(p, 'utf8') : '';
  const binding = process.env.BINDING;
  const ids = expected(binding);
  if (command === 'validate') {
    validate(binding, JSON.parse(read('binding/binding-results.json')));
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
    binding, coreSha: process.env.BENCH_SHA, bindingSha: read('binding-sha.txt').trim(),
    workflowSha: process.env.WORKFLOW_SHA, ref: process.env.BENCH_REF,
    testbed: `binding-${binding}-ubuntu24-v1`, selected: ids, ...result,
  };
  fs.writeFileSync('binding-report.json', JSON.stringify(report, null, 2) + '\n');
  const summary = `### Binding benchmark: ${binding}\n\nCore: ${report.coreSha}; binding: ${report.bindingSha}; workflow: ${report.workflowSha}\n\nTestbed: ${report.testbed}; executed ${result.executed.length}/${ids.length}; uploaded ${result.uploaded.length}/${ids.length}; alerts ${result.alerts.length}; upload ${result.uploadStatus}\n\n${result.errors.join('\n')}\n`;
  console.log(summary);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
  if (result.errors.length) process.exitCode = 1;
}
module.exports = { expected, validate };
if (require.main === module) {
  try { main(process.argv[2]); } catch (e) { console.error(e.message); process.exitCode = 1; }
}
