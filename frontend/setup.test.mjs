// Render tests for the guided first check and the operator queue view.
import assert from 'node:assert/strict';
import { parseProjectSetup, fetchProjectSetup, projectSetupHTML, setupHeadline } from './src/setup.js';
import { parseOps, opsHTML, duration } from './src/ops.js';

let failures = 0;
async function check(name, fn) {
  try {
    await fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n       ${error.message.split('\n')[0]}`);
  }
}

const step = (id, status, extra = {}) => ({
  id, title: id.replace(/_/g, ' '), required: status !== 'optional', status,
  detail: `${id} detail`, actions: [], ...extra,
});
const setup = {
  schema_version: 1,
  project_id: 'proj_1',
  kind: 'program_upgrade',
  ready_for_first_check: false,
  first_check_complete: false,
  next_step: 'bundle_active',
  steps: [
    step('project_enabled', 'done'),
    step('bundle_registered', 'done', { evidence: { record_count: 10 } }),
    step('bundle_active', 'todo', { actions: [{ label: 'Activate the reviewed bundle', actor: 'operator', command: 'eplyx-server admin activate-bundle --project proj_1 --bundle <b>' }] }),
    step('ci_token', 'todo', { actions: [{ label: 'Issue a token', actor: 'workspace_member' }] }),
    step('expectations', 'optional'),
    step('first_check', 'blocked'),
  ],
  repository: { variables: { EPLYX_PROJECT_ID: 'proj_1' } },
};

await check('a valid setup payload parses unchanged', () => {
  assert.equal(parseProjectSetup(structuredClone(setup), 'proj_1').next_step, 'bundle_active');
});

await check('a setup for another project, an unknown status or a dangling next step is refused', () => {
  assert.throws(() => parseProjectSetup(setup, 'proj_2'));
  const unknown = structuredClone(setup);
  unknown.steps[0].status = 'safe';
  assert.throws(() => parseProjectSetup(unknown));
  const dangling = structuredClone(setup);
  dangling.next_step = 'nowhere';
  assert.throws(() => parseProjectSetup(dangling));
  const actor = structuredClone(setup);
  actor.steps[2].actions[0].actor = 'anyone';
  assert.throws(() => parseProjectSetup(actor));
});

await check('fetch uses the caller request function and the encoded project path', async () => {
  let asked = '';
  await fetchProjectSetup('proj_1', async path => { asked = path; return setup; });
  assert.equal(asked, '/v1/projects/proj_1/setup');
});

await check('the checklist marks the next step and escapes server text', () => {
  const hostile = structuredClone(setup);
  hostile.steps[2].detail = '<img src=x onerror=alert(1)>';
  const html = projectSetupHTML(parseProjectSetup(hostile));
  assert.match(html, /data-setup-step="bundle_active" data-setup-status="todo"/);
  assert.match(html, /setup-step--todo is-next/);
  assert.match(html, /eplyx-server admin activate-bundle --project proj_1 --bundle &lt;b&gt;/);
  assert.match(html, /Service operator/);
  assert.match(html, /Workspace member/);
  assert.ok(!html.includes('<img'));
  assert.match(html, /not required/);
});

await check('the headline follows the server flags, never a browser-derived readiness', () => {
  assert.equal(setupHeadline(setup), 'Next: bundle active.');
  assert.match(setupHeadline({ ...setup, ready_for_first_check: true }), /in place/);
  assert.match(setupHeadline({ ...setup, first_check_complete: true }), /reached a verdict/);
});

const ops = {
  schema_version: 1,
  generated_at_unix_seconds: 1000,
  window_seconds: 86400,
  workers: { max_concurrent_runs: 2, busy: 1 },
  queue: { queued: 3, running: 1, oldest_queued_age_seconds: 125, longest_running_seconds: 40 },
  outcomes: { created: 9, passed: 4, failed: 2, completed: 0, execution_error: 1, queued: 1, running: 1 },
  wait: { count: 7, p50_seconds: 3, p90_seconds: 61, max_seconds: 400 },
  execution: { count: 0, p50_seconds: null, p90_seconds: null, max_seconds: null },
  retries: { runs_retried: 2, interrupted_attempts: 4, finalized_on_recovery: 0, exhausted: 1 },
  recent_worker_failures: [{ run_id: 'run_9', project_id: 'proj_1', attempts: 3, detail: '<b>interrupted</b>', created_at_unix_seconds: 1, completed_at_unix_seconds: 2 }],
  recoveries: [{ at_unix_seconds: 1, swept_temporary_artifacts: 0, requeued: ['run_a'], finalized: [], failed: ['run_9'] }],
};

await check('the ops payload parses and malformed counts are refused', () => {
  assert.equal(parseOps(structuredClone(ops)).queue.queued, 3);
  const bad = structuredClone(ops);
  bad.queue.queued = -1;
  assert.throws(() => parseOps(bad));
  const missing = structuredClone(ops);
  delete missing.retries;
  assert.throws(() => parseOps(missing));
});

await check('absent durations render as a dash, never as zero', () => {
  assert.equal(duration(null), '—');
  assert.equal(duration(0), '0s');
  assert.equal(duration(125), '2m 5s');
  assert.equal(duration(3700), '1h 1m');
  const html = opsHTML(parseOps(ops));
  assert.match(html, /— \/ —/);
  assert.match(html, /oldest 2m 5s/);
});

await check('worker failures are listed as infrastructure outcomes and escaped', () => {
  const html = opsHTML(parseOps(ops));
  assert.match(html, /run_9/);
  assert.match(html, /&lt;b&gt;interrupted&lt;\/b&gt;/);
  assert.match(html, /says nothing about the candidate/);
  assert.match(html, /1 re-enqueued, 0 finalized from a written report, 1 given up/);
  assert.match(html, /data-execution-errors="1"/);
});

if (failures) {
  console.log(`\n${failures} setup/ops rendering check(s) failed`);
  process.exit(1);
}
console.log('\nfrontend setup and ops rendering verified');
