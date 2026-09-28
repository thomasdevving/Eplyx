import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  LEGACY_TOKEN_PROGRAM, TOKEN_2022_PROGRAM, U64_MAX,
  canEnterPreparedMigration, isPublicKey, canonicalU64, utcInputToUnix,
  validateMigrationDraft, buildMigrationChangeSpec, proposalDocument,
  submitPreparedMigration, describeMigrationSubmissionError,
} from './cloud/migration-model.js';

const source = '6u4knmGQzqoVmVaeztpQDiL8mx2u29wjbEjDCyrV2GYq';
const destination = '9WDiiWiBjvUQe2eWyir6n9N54F39SjeLKBeBDuGsBTQS';
const mechanism = 'Akajkkga5U8d6xkVQUMXjnbdSuMtGAHBLbsr9dUvQGsj';
const file = (contents, name) => new File([contents], name, { type: 'application/octet-stream' });
const files = {
  candidate: file('candidate SBF bytes', 'migration.so'),
  state_input: file('{"schema_version":1}', 'state.json'),
  state_artifact: file('{"schemaVersion":1}', 'fixture.json'),
};
const draft = () => ({
  source_mint: source,
  source_token_program: LEGACY_TOKEN_PROGRAM,
  source_decimals: '6',
  destination_mint: destination,
  destination_token_program: TOKEN_2022_PROGRAM,
  destination_decimals: '9',
  numerator: '18446744073709551615',
  denominator: '3',
  rounding: 'floor',
  fee_kind: 'source_bps',
  fee_bps: '25',
  minimum_output_raw: '1',
  reserve_raw: '18446744073709551615',
  activation_utc: '2026-10-01T12:30:00',
  deadline_utc: '2026-11-01T12:30:00',
  mechanism_program_id: mechanism,
  state_input: files.state_input,
  state_artifact: files.state_artifact,
});
const candidate = { sha256: 'a'.repeat(64), len: files.candidate.size };

assert.match(readFileSync(new URL('./dashboard/dashboard.js', import.meta.url), 'utf8'), /\/analyse\\\/migration/);
assert.match(readFileSync(new URL('../server/src/cloud/web.rs', import.meta.url), 'utf8'), /frontend!\("cloud\/migration\.js"\)/);

// The server-authored token_migration capability is the only entry gate.
assert.equal(canEnterPreparedMigration({ kind: 'token_migration', can_submit: true }), true);
assert.equal(canEnterPreparedMigration({ kind: 'token_migration', can_submit: false }), false);
assert.equal(canEnterPreparedMigration({ kind: 'program_upgrade', can_submit: true }), false);

assert.equal(isPublicKey(source), true);
assert.equal(isPublicKey('not-a-solana-key'), false);
assert.equal(canonicalU64(String(U64_MAX)), String(U64_MAX));
assert.equal(canonicalU64('9007199254740993'), '9007199254740993');
assert.equal(canonicalU64('01'), null);
assert.equal(canonicalU64('18446744073709551616'), null);
assert.equal(utcInputToUnix('2026-10-01T12:30:00'), '1790857800');

const missing = draft();
missing.source_mint = '';
assert.match(validateMigrationDraft(missing, candidate).source_mint, /required/);
const invalidKey = draft();
invalidKey.destination_mint = 'abc';
assert.match(validateMigrationDraft(invalidKey, candidate).destination_mint, /public key/);
const zeroRatio = draft();
zeroRatio.numerator = '0';
assert.match(validateMigrationDraft(zeroRatio, candidate).numerator, /positive/);
const reversed = draft();
reversed.deadline_utc = reversed.activation_utc;
assert.match(validateMigrationDraft(reversed, candidate).deadline_utc, /follow/);

const prepared = buildMigrationChangeSpec(draft(), candidate);
assert.equal(prepared.change.conversion.numerator, '18446744073709551615');
assert.equal(prepared.change.destination_funding.reserve.funded_raw, '18446744073709551615');
assert.equal(prepared.change.conversion.fee.bps, 25);
assert.equal(prepared.change.conversion.ratio_basis, 'ui');
assert.deepEqual(prepared.change.eligibility, {
  amount_policy: 'full_balance',
  minimum_source_balance_raw: '1',
  holder_authorization: ['owner'],
  owner_authority_classes: ['wallet', 'multisig'],
  excluded_accounts: [],
});
assert.equal(prepared.activation.unix_timestamp, 1790857800);
assert.equal(prepared.change.deadline.value, '1793536200');

// Overview and Technical presentation consume this one object. Its exact JSON
// document is also the multipart part, so a mode switch cannot create a second
// proposal state and the preview retains the submitted large values.
const technicalPreview = proposalDocument(prepared);
assert.match(technicalPreview, /"numerator": "18446744073709551615"/);
assert.match(technicalPreview, /"funded_raw": "18446744073709551615"/);
assert.equal(proposalDocument(prepared), technicalPreview);

let endpoint;
let submitted;
const result = await submitPreparedMigration({
  projectId: 'proj_browser',
  spec: prepared,
  files,
  request: async (path, options) => {
    endpoint = path;
    assert.equal(options.method, 'POST');
    submitted = JSON.parse(await options.body.get('change_spec').text());
    assert.equal(await options.body.get('candidate').text(), await files.candidate.text());
    assert.equal(await options.body.get('state_input').text(), await files.state_input.text());
    assert.equal(await options.body.get('state_artifact').text(), await files.state_artifact.text());
    assert.deepEqual(JSON.parse(await options.body.get('analysis_options').text()), { policy: 'block-only' });
    return {
      run_id: 'run_guided',
      change: {
        kind: 'token_migration',
        source_mint: source,
        destination_mint: destination,
        candidate_sha256: candidate.sha256,
        candidate_len: String(candidate.len),
      },
    };
  },
});
assert.equal(endpoint, '/v1/projects/proj_browser/checks');
assert.deepEqual(submitted, prepared);
assert.equal(result.route, '/p/proj_browser/runs/run_guided');

await assert.rejects(
  submitPreparedMigration({
    projectId: 'proj_browser', spec: prepared, files,
    request: async () => ({ run_id: 'run_other', change: { kind: 'token_migration', source_mint: source, destination_mint: destination, candidate_sha256: 'b'.repeat(64), candidate_len: String(candidate.len) } }),
  }),
  error => error.kind === 'accepted_identity' && /run_other/.test(error.message),
);

// A failed POST does not mutate the proposal model; the UI likewise never
// resets its form. Server input rejection and stale readiness remain distinct.
const retained = draft();
const before = { ...retained };
await assert.rejects(submitPreparedMigration({
  projectId: 'proj_browser', spec: prepared, files,
  request: async () => { throw Object.assign(new Error('analytical inputs do not validate together'), { status: 400 }); },
}));
assert.deepEqual(retained, before);
assert.deepEqual(
  describeMigrationSubmissionError(Object.assign(new Error('analytical inputs do not validate together'), { status: 400 })),
  { kind: 'server_input', message: 'Eplyx rejected the prepared inputs: analytical inputs do not validate together' },
);
assert.deepEqual(
  describeMigrationSubmissionError(Object.assign(new Error('this project is disabled and accepts no checks'), { status: 409 })),
  { kind: 'readiness', message: 'Project availability changed before submission: this project is disabled and accepts no checks' },
);

console.log('Guided prepared migration: capability gate, structural guidance, exact values, preview, hosted submission, navigation and retained failure state verified.');
