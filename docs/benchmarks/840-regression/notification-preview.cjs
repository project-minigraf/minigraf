// Offline policy checks and message preview. This does NOT prove live delivery.
// Runs the two unmodified github-script bodies from the archived workflow.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const root = __dirname;
const workflow = fs.readFileSync(path.join(root, 'evidence/workflows/e7923eb71-bench.yml'), 'utf8');
const bodies = [...workflow.matchAll(/          script: \|\n((?:            .*\n|\n)+)/g)]
  .map(m => m[1].replace(/^            /gm, ''));
assert.equal(bodies.length, 2);
const alert = id => `- ${id} (Latency): https://bencher.dev/console/projects/minigraf/alerts/preview\n`;
const records = [];

async function check(kind, scenario) {
  const suite = kind === 0 ? 'audit_840_native' : 'binding_audit_840';
  const ref = 'audit-840-preview';
  const warnings = [], posts = [], calls = [];
  process.env.SUITE = suite;
  process.env.BENCH_REF = ref;
  process.env.BENCH_SHA = 'a'.repeat(40);
  const current = scenario === 'failed-upload' ? 'controlled upload failure' : alert('audit/840/example');
  const previous = scenario === 'different-id' ? alert('audit/840/other') : alert('audit/840/example');
  const summary = { addHeading() { return this; }, addRaw() { return this; },
    addList() { return this; }, addBreak() { return this; }, async write() {} };
  const github = { rest: { actions: {
    async listWorkflowRuns(args) {
      calls.push({ action: 'listWorkflowRuns', args });
      if (scenario === 'lookup-error') throw new Error('controlled previous-log lookup failure');
      return { data: { workflow_runs: [
        { id: 1, display_title: 'Nightly Benchmarks (different-branch)', html_url: 'https://example.invalid/wrong-branch' },
        ...scenario === 'different-branch' ? [] : [{ id: 2, display_title: `Nightly Benchmarks (${ref})`, html_url: 'https://example.invalid/prior-run' }],
      ] } };
    },
    async listJobsForWorkflowRun(args) {
      calls.push({ action: 'listJobsForWorkflowRun', args });
      assert.equal(args.run_id, 2);
      return { data: { jobs: [{ id: 3, name: `bench / ${scenario === 'different-suite' ? 'wrong-suite' : suite}` }] } };
    },
    async downloadJobLogsForWorkflowRun(args) {
      calls.push({ action: 'downloadJobLogsForWorkflowRun', args });
      return { data: previous };
    },
  }, issues: {
    async listForRepo() { return { data: [] }; },
    async create(args) { posts.push(args); return { data: { number: 999999 } }; },
  } } };
  const requirePreview = name => {
    assert.equal(name, 'fs');
    return { readFileSync() { return current; } };
  };
  await new AsyncFunction('require', 'github', 'context', 'core', 'console', bodies[kind])(
    requirePreview, github,
    { repo: { owner: 'project-minigraf', repo: 'minigraf' }, runId: 4, serverUrl: 'https://example.invalid' },
    { warning(message) { warnings.push(message); }, summary }, { log() {} });
  assert.equal(posts.length, ['failed-upload', 'repeated'].includes(scenario) ? 1 : 0);
  if (scenario === 'failed-upload') assert.equal(calls.length, 0);
  if (scenario === 'lookup-error') assert.ok(warnings.some(w => w.includes('Could not read')));
  records.push({ kind: kind === 0 ? 'native' : 'binding', scenario, posts, warnings, calls });
}

(async () => {
  for (const kind of [0, 1]) {
    for (const scenario of ['failed-upload', 'repeated', 'different-id', 'different-branch', 'different-suite', 'lookup-error']) {
      await check(kind, scenario);
    }
  }
  fs.writeFileSync(path.join(root, 'notification-preview.json'), JSON.stringify(records, null, 2) + '\n');
  console.log('12 offline policy checks passed; no network requests or notifications sent');
})().catch(error => { console.error(error); process.exitCode = 1; });
