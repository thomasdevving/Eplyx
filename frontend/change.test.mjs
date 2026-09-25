// Functional render tests for how a proposed change is presented.
//
// Phase P1 made the change a run analyses its product identity. What must hold
// on the page: the ordinary view names the change briefly and in product
// language; the technical view carries every identifying field in full; the
// page never computes a change identity of its own; and when the run record,
// the stored spec and the report disagree, no verdict is shown at all.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';

const store = new Map();
globalThis.sessionStorage = {
  getItem: key => (store.has(key) ? store.get(key) : null),
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: key => store.delete(key),
};
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.EPLYX_API_URL = 'http://api.test';

const { ChangeCard, ChangeIdentityRows, changeLine, resolveChange } = await import('./src/change.js');
const { AnalysePage, ProposedChangePreview, acceptedMismatch, sha256Hex } = await import('./src/analyse.js');
const { attachReport } = await import('./src/report.js');

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

const PROGRAM = 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy';
const CANDIDATE = 'ec2dfef0a7f7c5c1d3f3e7aa62b0bb0d9a6f9ed3e0c5a4f2b8b1c7d6e5f4a3b2';
const CHANGE_ID = 'b5a894cdbec6251f73b4224a294579fe1af9e316232949fa92da852468900bf3';
const OTHER_ID = 'a'.repeat(64);
const PROGRAMDATA = '7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP';
const AUTHORITY = 'Ad21qwCb3C98M6UNqjGsZgR48549Spp7W1UWETV29cZ9';
const REPLACED = 'c'.repeat(64);

const change = (overrides = {}) => ({
  change_spec_id: CHANGE_ID,
  kind: 'program_upgrade',
  target_program_id: PROGRAM,
  candidate_sha256: CANDIDATE,
  candidate_len: 32240,
  label: null,
  origin: 'derived_from_candidate',
  ...overrides,
});
const reportChange = (id = CHANGE_ID) => ({
  change_spec_id: id,
  kind: 'program_upgrade',
  target_program_id: PROGRAM,
  candidate_sha256: CANDIDATE,
});
const spec = (overrides = {}) => ({
  schema_version: 1,
  change_spec_id: CHANGE_ID,
  change: {
    kind: 'program_upgrade',
    target: { program_id: PROGRAM, programdata_address: PROGRAMDATA },
    candidate: { sha256: CANDIDATE, len: 32240 },
    replaces: { sha256: REPLACED, len: 31808 },
    expected_upgrade_authority: AUTHORITY,
  },
  activation: { slot: 400000000, unix_timestamp: 1790000000 },
  metadata: { label: 'Release 2.1', source: 'release tooling' },
  ...overrides,
});
const report = (id = CHANGE_ID) => ({
  change: reportChange(id),
  candidate: { sha256: CANDIDATE, len: 32240 },
  bundle: { program_id: PROGRAM, record_count: 1, limitations: [] },
  summary: { passed: true, exit_code: 0, unexpected: 0, expected: 0 },
  findings: [], undeclarable: [], unmatched: [], coverage: [],
});

/** Visible text: markup, attributes and their values removed. */
const visible = html => html.replace(/<[^>]*>/g, ' ');
/** The ordinary view: everything except the technical details layer. */
const ordinary = html => visible(html.replace(/<section[^>]*data-technical[\s\S]*?<\/section>/g, ''));

/** Follow one run to the stubbed server's answer and return the page. */
async function follow(run, { report: body = null, spec: stored = null, project = null } = {}) {
  store.clear();
  store.set('eplyx-operator-token', 'operator-token');
  globalThis.fetch = async url => {
    const path = String(url).replace('http://api.test', '');
    const answer = value => value
      ? { ok: true, status: 200, json: async () => value }
      : { ok: false, status: 404, json: async () => ({ error: 'no' }) };
    if (path === `/v1/runs/${run.run_id}`) return answer(run);
    if (path.endsWith('/report.json')) return answer(body);
    if (path.endsWith('/change_spec.json')) return answer(stored);
    if (path.startsWith('/v1/projects/')) return answer(project);
    throw new Error(`unexpected request ${path}`);
  };
  let html = '';
  const stop = attachReport(run.run_id, markup => { html = markup; });
  await new Promise(resolve => setTimeout(resolve, 40));
  stop();
  return html;
}

// --- 12. the summary card ---------------------------------------------
await check('a program upgrade renders as a concise proposed change', () => {
  const html = ChangeCard({ change: change({ label: 'v2.1 release candidate' }), targetName: 'Example Pool' });
  assert.match(html, /Proposed change/);
  assert.match(html, /Program upgrade/);
  assert.match(html, /v2\.1 release candidate/);
  assert.match(html, /Example Pool/);
  assert.match(visible(html), /SPoo1K…kuHy/, 'target is not shown briefly');
  assert.match(visible(html), new RegExp(CANDIDATE.slice(0, 8)));
  assert.match(visible(html), new RegExp(CHANGE_ID.slice(0, 8)));
  // Hashes are short in the main view; full values live in technical details.
  for (const full of [CANDIDATE, CHANGE_ID, PROGRAM]) {
    assert.ok(!visible(html).includes(full), `full ${full.slice(0, 8)}… dominates the card`);
  }
  assert.match(html, /32\.2 KB|31\.5 KB/);
});

await check('activation is shown as a proposal, and analysis as counterfactual', () => {
  const html = ChangeCard({ change: change(), spec: spec() });
  assert.match(html, /Proposed effective/);
  assert.match(html, /slot 400000000/);
  assert.match(html, /counterfactual/);
  const plain = ChangeCard({ change: change() });
  assert.doesNotMatch(plain, /Proposed effective/, 'invented an activation');
});

// --- 13. the technical view -------------------------------------------
await check('the technical view exposes the complete identity', () => {
  const resolution = resolveChange({ change: change() }, report(), spec());
  const html = ChangeIdentityRows({ change: resolution.change, spec: spec(), resolution });
  for (const value of [CHANGE_ID, 'program_upgrade', PROGRAM, CANDIDATE, '32240 bytes', PROGRAMDATA, REPLACED, '31808', AUTHORITY, '400000000', '1790000000', 'v1', 'release tooling']) {
    assert.ok(html.includes(value), `missing ${value}`);
  }
  assert.match(html, /Derived from the uploaded candidate/);
  assert.match(html, /name the same change/);
});

await check('fields a spec does not state are shown as not stated, not invented', () => {
  const html = ChangeIdentityRows({ change: change() });
  assert.doesNotMatch(html, /ProgramData/);
  assert.doesNotMatch(html, /Upgrade authority/);
  assert.doesNotMatch(html, /Activation/);
});

// --- the report page ----------------------------------------------------
await check('a completed run names its change from the report', async () => {
  const html = await follow(
    { run_id: 'r1', project_id: 'proj_A', status: 'passed', exit_code: 0, report_available: true, candidate_sha256: CANDIDATE, change: change() },
    { report: report(), spec: spec(), project: { project: { name: 'Example Pool' } } },
  );
  assert.match(html, /Proposed change/);
  assert.match(html, /Program upgrade/);
  assert.match(html, /Example Pool/);
  assert.ok(html.includes(CHANGE_ID), 'full change ID missing from the technical view');
  assert.match(html, /name the same change/);
  assert.match(html, /Passed/);
});

await check('a run whose records disagree shows no verdict', async () => {
  const html = await follow(
    { run_id: 'r2', status: 'passed', exit_code: 0, report_available: true, candidate_sha256: CANDIDATE, change: change() },
    { report: report(OTHER_ID), spec: spec() },
  );
  assert.match(html, /does not agree with itself/);
  assert.ok(html.includes(OTHER_ID));
  assert.doesNotMatch(html, /No unexpected economic changes/);
  assert.doesNotMatch(html, /<span>Passed<\/span>/, 'a verdict was shown for a mismatched identity');
});

await check('a stored spec that names another change is also a mismatch', () => {
  const resolution = resolveChange({ change: change() }, report(), spec({ change_spec_id: OTHER_ID }));
  assert.equal(resolution.conflicts.length, 1);
  assert.equal(resolution.verified, false);
});

await check('a queued run already names the change it will analyse', async () => {
  const html = await follow(
    { run_id: 'r3', status: 'queued', exit_code: null, report_available: false, candidate_sha256: CANDIDATE, change: change() },
    { spec: spec() },
  );
  assert.match(html, /execution slot/);
  assert.match(html, /Program upgrade/);
  assert.ok(html.includes(CHANGE_ID));
  assert.doesNotMatch(html, /Exit code/);
});

// --- 11. legacy runs ----------------------------------------------------
await check('a legacy run is labelled legacy and is given no invented identity', async () => {
  const legacy = { run_id: 'r4', status: 'failed', exit_code: 1, report_available: true, candidate_sha256: CANDIDATE, change: null };
  const bare = report();
  delete bare.change;
  const html = await follow(legacy, { report: bare });
  assert.match(html, /Legacy run/);
  assert.doesNotMatch(html, /Change ID<\/dt>/, 'a change ID was shown for a run that has none');
  assert.ok(!html.includes(CHANGE_ID), 'an identity was invented');
  assert.match(html, /id="technical"/, 'the legacy report itself was not shown');
  assert.match(html, /Economic impact/);

  // One recorded after C1 but before P1 carries the engine's own change in
  // its report; that is shown, and is said to come from the report.
  const html2 = await follow(legacy, { report: report() });
  assert.ok(html2.includes(CHANGE_ID));
  assert.match(html2, /comes from its report/);
});

await check('history lines name the change, and legacy runs as legacy', () => {
  const line = changeLine({ change: change({ label: null }) });
  assert.match(line, /Program upgrade → /);
  assert.match(line, new RegExp(`change ${CHANGE_ID.slice(0, 8)}`));
  // P3: a history row names the change, not the candidate hash database.
  assert.doesNotMatch(line, new RegExp(CANDIDATE.slice(0, 8)));
  assert.match(changeLine({ change: change({ label: 'Release 2.1' }) }), /Release 2\.1 → /);
  const legacy = changeLine({ change: null, candidate_sha256: CANDIDATE });
  assert.match(legacy, /Legacy run/);
  assert.doesNotMatch(legacy, /change [0-9a-f]{8}/);
});

// --- 14. the default flow asks for nothing technical --------------------
await check('the analyse page requires no ChangeSpec knowledge', () => {
  const html = AnalysePage();
  assert.match(html, /name="candidate"/);
  for (const field of ['change_spec', 'schema', 'sha256', 'programdata', 'authority', 'change_spec_id', 'activation']) {
    assert.doesNotMatch(html, new RegExp(`name="${field}`, 'i'), `the form asks for ${field}`);
  }
  assert.doesNotMatch(html, /<textarea/, 'a raw document field');
  assert.doesNotMatch(html, /ChangeSpec|JSON/, 'the form speaks the internal contract');
  // Only the one real kind is offered.
  assert.match(html, /program upgrade/i);
  for (const unsupported of [/lifecycle/i, /parameter change/i, /governance/i, /any on-chain change/i]) {
    assert.doesNotMatch(html, unsupported, `advertises ${unsupported}`);
  }
});

await check('the confirmation shows target and fingerprint before submitting', () => {
  const html = ProposedChangePreview({
    project: { name: 'Example Pool', program_id: PROGRAM },
    candidate: { name: 'program.so', size: 32240, sha256: CANDIDATE },
  });
  assert.match(html, /Program upgrade/);
  assert.match(html, /Example Pool/);
  assert.match(visible(html), new RegExp(CANDIDATE.slice(0, 8)));
  assert.match(html, /program\.so/);
  assert.equal(ProposedChangePreview({}), '', 'a preview appeared before any file was chosen');
});

await check('the browser fingerprint is the server’s fingerprint', async () => {
  const bytes = new TextEncoder().encode('candidate elf');
  const hex = await sha256Hex(new Blob([bytes]));
  assert.equal(hex, createHash('sha256').update(bytes).digest('hex'));
});

await check('an accepted change that is not the prepared one is not followed', () => {
  const prepared = { project: { program_id: PROGRAM }, candidate: { sha256: CANDIDATE } };
  assert.equal(acceptedMismatch({ change: change() }, prepared), null);
  assert.match(acceptedMismatch({ change: change({ candidate_sha256: OTHER_ID }) }, prepared), /candidate/);
  assert.match(acceptedMismatch({ change: change({ target_program_id: PROGRAMDATA }) }, prepared), /change to/);
  assert.match(acceptedMismatch({}, prepared), /named no change/);
});

// --- 18. main-view language ---------------------------------------------
await check('the ordinary report view does not speak internal type names', async () => {
  const html = await follow(
    { run_id: 'r5', status: 'passed', exit_code: 0, report_available: true, candidate_sha256: CANDIDATE, change: change() },
    { report: report(), spec: spec() },
  );
  for (const internal of ['ReplayObservationV2', 'CheckpointedExecutionV1', 'contract 3', 'SemanticBinding', 'ChangeSpec']) {
    assert.ok(!ordinary(html).includes(internal), `the page says ${internal}`);
  }
});

console.log(failures === 0 ? '\nfrontend change identity verified' : `\n${failures} change identity failures`);
process.exit(failures === 0 ? 0 : 1);
