// Workflow-only checks for the pinned Criterion and Bencher console formats.
const fs = require('node:fs');

const matches = (text, regex) => [...text.matchAll(regex)].map(match => match[1]);
const executions = text => matches(text, /^Benchmarking (.+): Analyzing\s*$/gm);
const uploads = text => matches(text, /^- (.+) \(Latency\): https:\/\/bencher\.dev\/console\/projects\/[^/\s]+\/perf\?/gm);
const parseAlerts = text => new Set(matches(text, /- (\S+) \(Latency\): https:\/\/bencher\.dev\/console\/projects\/[^/\s]+\/alerts\//g));

function selection(text, filter) {
  const ids = matches(text, /^(.+): benchmark\s*$/gm);
  if (!ids.length) throw new Error(`Required route is empty: ${filter}`);
  if (new Set(ids).size !== ids.length) throw new Error('Enumeration contains duplicate IDs');
  const regex = new RegExp(filter);
  if (ids.some(id => !regex.test(id))) throw new Error('Enumerated ID does not match filter');
  return ids;
}

function compare(expected, actual, label) {
  const wanted = new Set(expected);
  const found = new Set(actual);
  const errors = [];
  if (found.size !== actual.length) errors.push(`${label}: duplicate IDs`);
  const missing = expected.filter(id => !found.has(id));
  const extra = actual.filter(id => !wanted.has(id));
  if (missing.length) errors.push(`${label}: missing ${missing.join(', ')}`);
  if (extra.length) errors.push(`${label}: unexpected ${extra.join(', ')}`);
  return errors;
}

function audit(expected, measured, output, outcome) {
  const executed = executions(measured);
  const uploaded = uploads(output);
  const alerts = [...parseAlerts(output)];
  const errors = [
    ...compare(expected, executed, 'Execution'),
    ...compare(expected, uploaded, 'Upload'),
  ];
  let uploadStatus = 'success';
  if (outcome === 'failure' && alerts.length) uploadStatus = 'alerts';
  else if (outcome !== 'success') {
    uploadStatus = 'error';
    errors.push(`Bencher outcome: ${outcome || 'not run'} (no alerts)`);
  }
  if (!expected.length) errors.push('No verified selection');
  return { executed, uploaded, alerts, uploadStatus, errors };
}

function main(command) {
  const read = path => fs.existsSync(path) ? fs.readFileSync(path, 'utf8') : '';
  const manifestPath = 'bench-selection.json';
  if (command === 'select') {
    const selected = selection(read('bench-list.txt'), process.env.BENCH_FILTER);
    const report = {
      suite: process.env.BENCH_SUITE, ref: process.env.BENCH_REF,
      sha: process.env.BENCH_SHA, workflowSha: process.env.WORKFLOW_SHA,
      filter: process.env.BENCH_FILTER, threshold: process.env.BENCH_THRESHOLD,
      selected,
    };
    fs.writeFileSync(manifestPath, JSON.stringify(report, null, 2) + '\n');
    console.log(`Selected ${selected.length} IDs at ${report.sha}`);
    return;
  }
  const selectionReport = JSON.parse(read(manifestPath) || '{}');
  if (command === 'execution') {
    const errors = compare(selectionReport.selected || [], executions(read(process.env.BENCH_FILE)), 'Execution');
    if (!selectionReport.selected?.length) errors.push('No verified selection');
    if (errors.length) throw new Error(errors.join('\n'));
    return;
  }
  if (command !== 'finish') throw new Error(`Unknown command: ${command}`);
  const report = {
    ...selectionReport,
    sha: process.env.BENCH_SHA, ref: process.env.BENCH_REF,
    suite: process.env.BENCH_SUITE, filter: process.env.BENCH_FILTER,
    workflowSha: process.env.WORKFLOW_SHA, threshold: process.env.BENCH_THRESHOLD,
    ...audit(selectionReport.selected || [], read(process.env.BENCH_FILE), read('bencher_out.txt'), process.env.BENCHER_OUTCOME),
  };
  fs.writeFileSync('bench-report.json', JSON.stringify(report, null, 2) + '\n');
  const summary = [
    `### Benchmark coverage: ${report.suite}`,
    `Measured ref: \`${report.ref}\`; checkout: \`${report.sha}\``,
    `Workflow revision: \`${report.workflowSha}\``,
    `Filter: \`${report.filter}\`; shared latency t-test boundary: ${report.threshold}`,
    `Selected: ${report.selected?.length || 0}; analyzed: ${report.executed.length}; uploaded: ${report.uploaded.length}; alerts: ${report.alerts.length}`,
    `Bencher: ${report.uploadStatus}; coverage: ${report.errors.length ? 'FAILED' : 'verified'}`,
    ...report.errors.map(error => `- ${error}`), '',
  ].join('\n\n');
  console.log(summary);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
  if (report.errors.length) process.exitCode = 1;
}

module.exports = { selection, executions, uploads, audit, parseAlerts };
if (require.main === module) {
  try { main(process.argv[2]); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
