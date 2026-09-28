import assert from 'node:assert/strict';
import {
  parseProjectCapabilities,
  fetchProjectCapabilities,
  capabilityFor,
  canSubmitCapability,
  capabilityHTML,
  capabilityLoadingHTML,
  capabilityFailureHTML,
  projectCapabilitiesHTML,
} from './src/capabilities.js';

const missingBundle = {
  code: 'active_bundle_missing',
  message: 'No active analysis bundle is selected.',
  action: 'Upload and activate a compatible bundle.',
};
const migrationCandidate = {
  code: 'migration_candidate_not_configured',
  message: 'No migration candidate is registered.',
  action: 'Ask the operator to register the migration candidate.',
};
const capability = (kind, status = 'ready', missing = []) => ({
  kind,
  status,
  supported: status !== 'unsupported',
  can_submit: status === 'ready',
  missing,
});
const payload = {
  schema_version: 1,
  project_id: 'project/readiness',
  analyses: [
    capability('program_upgrade', 'not_ready', [missingBundle]),
    capability('token_migration'),
    capability('lifecycle_change'),
    capability('current_observation'),
    capability('current_path'),
    capability('current_candidate', 'not_ready', [migrationCandidate]),
    capability('current_preflight'),
    capability('current_stress', 'unsupported', [{
      code: 'analysis_kind_unsupported',
      message: 'Bounded stress is unavailable in this deployment.',
      action: 'Use a deployment that supports bounded stress.',
    }]),
  ],
};

// Loading and failed discovery never create a runnable state.
assert.match(capabilityLoadingHTML(), /Checking project analysis availability/);
assert.equal(canSubmitCapability(null), false);
assert.match(capabilityFailureHTML(), /controls remain unavailable/);

const parsed = parseProjectCapabilities(payload, 'project/readiness');
assert.equal(canSubmitCapability(capabilityFor(parsed, 'program_upgrade')), false);
assert.equal(canSubmitCapability(capabilityFor(parsed, 'token_migration')), true,
  'prepared migration must remain independent from upgrade readiness');
assert.equal(canSubmitCapability(capabilityFor(parsed, 'lifecycle_change')), true);
assert.equal(canSubmitCapability(capabilityFor(parsed, 'current_observation')), true);
assert.equal(canSubmitCapability(capabilityFor(parsed, 'current_candidate')), false,
  'current-state subkinds must remain independently gated');
assert.equal(canSubmitCapability(capabilityFor(parsed, 'current_stress')), false);

// Overview and Technical modes are two CSS views of one server response.
const upgrade = capabilityHTML(capabilityFor(parsed, 'program_upgrade'));
assert.match(upgrade, /No active analysis bundle is selected/);
assert.match(upgrade, /Upload and activate a compatible bundle/);
assert.match(upgrade, /class="tech-only">active_bundle_missing/);
assert.match(upgrade, /status <code>not_ready<\/code>/);
assert.match(upgrade, /can_submit <code>false<\/code>/);

const unsupported = capabilityHTML(capabilityFor(parsed, 'current_stress'));
assert.match(unsupported, /Unsupported/);
assert.match(unsupported, /Bounded stress is unavailable/);
assert.match(unsupported, /Use a deployment that supports bounded stress/);
assert.match(unsupported, /analysis_kind_unsupported/);

const groups = projectCapabilitiesHTML(parsed);
for (const kind of payload.analyses.map(item => item.kind)) {
  assert.match(groups, new RegExp(`data-capability-kind="${kind}"`));
}
assert.match(groups, /Prepared token migration/);
assert.match(groups, /Current-state analysis/);

let requested = '';
const fetched = await fetchProjectCapabilities('project/readiness', async path => {
  requested = path;
  return payload;
});
assert.equal(requested, '/v1/projects/project%2Freadiness/capabilities');
assert.equal(fetched, payload);
await assert.rejects(
  fetchProjectCapabilities('project/readiness', async () => { throw new Error('network failed'); }),
  /network failed/,
);
assert.throws(
  () => parseProjectCapabilities({ ...payload, project_id: 'another-project' }, 'project/readiness'),
  /invalid project capability response/,
);

console.log('Project capability loading, failure, per-kind gates, reasons and shared Overview/Technical truth verified.');
