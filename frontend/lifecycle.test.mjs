import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  canEnterPreparedLifecycle, validateLifecycleDraft, buildLifecycleDocuments,
  digestLifecycleFile, submitPreparedLifecycle, lifecycleSubmissionError,
} from './cloud/lifecycle-model.js';

const asset = '6u4knmGQzqoVmVaeztpQDiL8mx2u29wjbEjDCyrV2GYq';
const successor = '9WDiiWiBjvUQe2eWyir6n9N54F39SjeLKBeBDuGsBTQS';
const snapshot = new File(['{\n  "schema_version": 2,\n  "sentinel": "unchanged"\n}\n'], 'snapshot.json', { type: 'application/json' });
const draft = () => ({
  scenario_id: 'browser-lifecycle-2031',
  reference: 'proposal:2031-transition',
  description: 'Evaluate a declared successor transition at the stated policy boundary.',
  asset_mint: asset,
  effective_at: '2031-01-01T00:00:00',
  captured_at: '2030-06-15T09:30:00',
  has_successor: true,
  successor_mint: successor,
  successor_description: 'Declared successor asset; no conversion mechanism asserted.',
  has_deadline: true,
  deadline_at: '2031-02-01T00:00:00',
  post_deadline_status: 'Expired',
  snapshot,
});

const dashboardSource = readFileSync(new URL('./dashboard/dashboard.js', import.meta.url), 'utf8');
const lifecycleSource = readFileSync(new URL('./cloud/lifecycle.js', import.meta.url), 'utf8');
const analysisSource = readFileSync(new URL('./cloud/analysis.js', import.meta.url), 'utf8');
assert.match(dashboardSource, /\/analyse\\\/lifecycle/);
assert.match(readFileSync(new URL('../server/src/cloud/web.rs', import.meta.url), 'utf8'), /frontend!\("cloud\/lifecycle\.js"\)/);
assert.match(analysisSource, /Lifecycle change/);
assert.match(analysisSource, /capabilityFor\(capabilities,'lifecycle_change'\)/);
assert.match(analysisSource, /\/analyse\/lifecycle/);

// The server-authored lifecycle_change capability is the only entry gate.
assert.equal(canEnterPreparedLifecycle({ kind: 'lifecycle_change', can_submit: true }), true);
assert.equal(canEnterPreparedLifecycle({ kind: 'lifecycle_change', can_submit: false }), false);
assert.equal(canEnterPreparedLifecycle({ kind: 'token_migration', can_submit: true }), false);

const missing = draft();
missing.description = '';
missing.reference = '';
missing.snapshot = null;
const missingErrors = validateLifecycleDraft(missing);
assert.match(missingErrors.description, /Describe/);
assert.match(missingErrors.reference, /source or proposal reference/);
assert.match(missingErrors.snapshot, /snapshot/);

const invalidKey = draft();
invalidKey.asset_mint = 'not-a-key';
assert.match(validateLifecycleDraft(invalidKey).asset_mint, /32-byte base58/);
const invalidTime = draft();
invalidTime.effective_at = '2031-02-31T00:00:00';
assert.match(validateLifecycleDraft(invalidTime).effective_at, /valid whole-second/);
const reversedDeadline = draft();
reversedDeadline.deadline_at = reversedDeadline.effective_at;
assert.match(validateLifecycleDraft(reversedDeadline).deadline_at, /after the effective/);
const unsupportedStatus = draft();
unsupportedStatus.post_deadline_status = 'MadeUp';
assert.match(validateLifecycleDraft(unsupportedStatus).post_deadline_status, /supported/);
const sameSuccessor = draft();
sameSuccessor.successor_mint = asset;
assert.match(validateLifecycleDraft(sameSuccessor).successor_mint, /differ/);

const prepared = buildLifecycleDocuments(draft());
assert.deepEqual(prepared.scenario.policy, {
  asset_mint: asset,
  effective_at: '2031-01-01T00:00:00Z',
  before: 'Active',
  after: 'TransitionRequired',
  deadline: { at: '2031-02-01T00:00:00Z', after: 'Expired' },
  successor: { mint: successor, description: 'Declared successor asset; no conversion mechanism asserted.' },
});
assert.deepEqual(prepared.scenario.sources[0].supports, [
  '/policy/effective_at', '/policy/before', '/policy/after', '/policy/deadline', '/policy/successor',
]);
assert.equal(prepared.scenario.sources[0].kind, 'ScenarioAssumption');
assert.equal(prepared.scenario.sources[0].artifact, null);
assert.equal(prepared.changeSpec.activation.unix_timestamp, 1924992000);
assert.equal(prepared.changeSpec.change.deadline.unix_timestamp, '1927670400');
assert.deepEqual(prepared.changeSpec.change.eligibility, { kind: 'unknown' });
assert.equal('ratio' in prepared.changeSpec.change, false);
assert.deepEqual(prepared.analysisOptions, {
  before: '2030-12-31T23:59:59Z',
  at: '2031-01-01T00:00:00Z',
});
const minimalDraft = draft();
minimalDraft.has_successor = false;
minimalDraft.has_deadline = false;
const minimal = buildLifecycleDocuments(minimalDraft);
assert.equal('destination' in minimal.changeSpec.change, false);
assert.equal('deadline' in minimal.changeSpec.change, false);
assert.equal(minimal.scenario.policy.successor, null);
assert.equal(minimal.scenario.policy.deadline, null);
assert.deepEqual(minimal.scenario.sources[0].supports, [
  '/policy/effective_at', '/policy/before', '/policy/after',
]);

// Overview and Technical presentation read these same immutable documents.
// A presentation-mode change has no alternate proposal model to rebuild.
assert.equal(prepared.changeSpecDocument, `${JSON.stringify(prepared.changeSpec, null, 2)}\n`);
assert.equal(prepared.scenarioDocument, `${JSON.stringify(prepared.scenario, null, 2)}\n`);
assert.match(prepared.changeSpecDocument, /"unix_timestamp": "1927670400"/);
assert.match(lifecycleSource, /Declared/);
assert.match(lifecycleSource, /Observed \/ prepared/);
assert.match(lifecycleSource, /Derived by Eplyx/);
assert.doesNotMatch(lifecycleSource, /\.reset\s*\(/);
assert.doesNotMatch(readFileSync(new URL('./cloud/lifecycle-model.js', import.meta.url), 'utf8'), /impact_classification|economic_meaning_changed|mobility|RequiresTransition/);

const snapshotHash = await digestLifecycleFile(snapshot);
assert.match(snapshotHash, /^[a-f0-9]{64}$/);
let endpoint;
let submitted;
const result = await submitPreparedLifecycle({
  projectId: 'proj_lifecycle',
  draft: draft(),
  request: async (path, options) => {
    endpoint = path;
    assert.equal(options.method, 'POST');
    submitted = {
      changeSpec: JSON.parse(await options.body.get('change_spec').text()),
      scenario: JSON.parse(await options.body.get('scenario').text()),
      analysisOptions: JSON.parse(await options.body.get('analysis_options').text()),
    };
    assert.equal(await options.body.get('snapshot').text(), await snapshot.text());
    return {
      run_id: 'run_lifecycle_guided',
      status_url: '/v1/runs/run_lifecycle_guided',
      change: {
        change_spec_id: 'a'.repeat(64), label: 'browser-lifecycle-2031',
        kind: 'lifecycle_change', asset_mint: asset, destination_mint: successor,
      },
    };
  },
});
assert.equal(endpoint, '/v1/projects/proj_lifecycle/checks');
assert.deepEqual(submitted.changeSpec, prepared.changeSpec);
assert.deepEqual(submitted.scenario, prepared.scenario);
assert.deepEqual(submitted.analysisOptions, prepared.analysisOptions);
assert.equal(result.route, '/p/proj_lifecycle/runs/run_lifecycle_guided');

await assert.rejects(
  submitPreparedLifecycle({
    projectId: 'proj_lifecycle', draft: draft(),
    request: async () => ({
      run_id: 'run_other', status_url: '/v1/runs/run_other',
      change: {
        change_spec_id: 'b'.repeat(64), label: 'browser-lifecycle-2031',
        kind: 'lifecycle_change', asset_mint: asset, destination_mint: null,
      },
    }),
  }),
  error => error.kind === 'accepted_identity' && /run_other/.test(error.message),
);

// Failed submission retains every entered term and the selected File object.
const retained = draft();
const before = { ...retained };
await assert.rejects(submitPreparedLifecycle({
  projectId: 'proj_lifecycle', draft: retained,
  request: async () => { throw Object.assign(new Error('scenario and snapshot asset differ'), { status: 400 }); },
}));
assert.deepEqual(retained, before);
assert.deepEqual(
  lifecycleSubmissionError(Object.assign(new Error('scenario and snapshot asset differ'), { status: 400 })),
  { kind: 'validation', message: 'Eplyx rejected the prepared inputs: scenario and snapshot asset differ' },
);
assert.deepEqual(
  lifecycleSubmissionError(Object.assign(new Error('this project is disabled and accepts no checks'), { status: 409 })),
  { kind: 'stale', message: 'Project readiness changed before submission: this project is disabled and accepts no checks' },
);

console.log('Guided prepared lifecycle: capability gate, structural guidance, exact documents, snapshot preservation, hosted submission, navigation and retained failure state verified.');
