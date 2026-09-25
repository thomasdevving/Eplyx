// Phase G1: the governance section of an analysis page.
//
// Bindings are engine output (docs/examples/phase-g1-squads-binding/, written
// by the real verifier over a simulated Squads world). What must hold: a match
// is never shown without its slot; an old match is not left green; a
// rewritten buffer leads with the committed sentence; a check about another
// change is refused; and the page decodes nothing itself.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const store = new Map();
globalThis.sessionStorage = {
  getItem: key => (store.has(key) ? store.get(key) : null),
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: key => store.delete(key),
};
globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
globalThis.EPLYX_API_URL = 'http://api.test';

const { governanceView, GovernanceSection, GovernanceRows, deliveryOf, STALE_BUFFER, FRESH_SECONDS } = await import('./src/governance.js');
const { attachReport } = await import('./src/report.js');
const { changeLine, ChangeIdentityRows } = await import('./src/change.js');

const root = new URL('../', import.meta.url);
const load = path => JSON.parse(readFileSync(new URL(path, root), 'utf8'));
const G1 = 'docs/examples/phase-g1-squads-binding/';
const bound = load(`${G1}bound-change-spec.json`);
const analysed = load(`${G1}analysed-change-spec.json`);
const B = {
  matched: load(`${G1}binding-matched.json`),
  stale: load(`${G1}binding-stale-artifact.json`),
  authority: load(`${G1}binding-authority-mismatch.json`),
  unsupported: load(`${G1}binding-unsupported.json`),
  cancelled: load(`${G1}binding-cancelled.json`),
};
const NOW = 1_790_000_000;
const delivery = deliveryOf(bound);
const at = (binding, secondsAgo = 60) => ({ checked_at_unix_seconds: NOW - secondsAgo, binding });
const view = (binding, secondsAgo) =>
  governanceView({ delivery, check: binding ? at(binding, secondsAgo) : null, changeSpecId: bound.change_spec_id, now: NOW });
const SAFE = /\b(safe|unsafe|approved by eplyx|can never|will never)\b/i;

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

console.log('governance view');

await check('an unbound change has no governance section at all', () => {
  assert.equal(deliveryOf(analysed), null);
  assert.equal(governanceView({ delivery: deliveryOf(analysed), check: at(B.matched) }), null);
  assert.equal(GovernanceSection(null), '');
});

await check('a bound change that was never checked says so, and is not green', () => {
  const model = view(null);
  assert.equal(model.state, 'unchecked');
  assert.equal(model.tone, 'neutral');
  assert.match(GovernanceSection(model), /Squads #42/);
  assert.doesNotMatch(GovernanceSection(model), /Matches analysed change/);
});

await check('a fresh match shows the proposal, the slot, the commitment and the status', () => {
  const model = view(B.matched, 60);
  assert.equal(model.state, 'matched');
  assert.equal(model.tone, 'ok');
  const html = GovernanceSection(model);
  assert.match(html, /Matches analysed change/);
  assert.match(html, /✓/);
  assert.match(html, new RegExp(`slot ${B.matched.observation.slot}`));
  assert.match(html, /finalized/);
  assert.match(html, /Proposal status/);
  assert.match(html, /Active/);
  assert.match(html, /re-verify immediately before approving or executing/);
  assert.doesNotMatch(html, SAFE);
});

await check('a separate G2 attestation reports deployment, supersession and mismatch', () => {
  const base = { change_spec_id: bound.change_spec_id, binding_id: B.matched.binding_id,
    execution: { signature: '3Nvd3MPxvNKDj6h5YVuaUd5xgjfJeByPJw1HdRu8QwdrEpSt7csgYaQ2qrXNifxwbGs7p9gGMt5Vq1JJRywgG2wV', slot: 7000 } };
  const render = outcome => governanceView({ delivery, check: at(B.matched), attestation: { ...base, outcome }, changeSpecId: bound.change_spec_id, now: NOW });
  const matched = render('deployed_match');
  assert.equal(matched.tone, 'ok');
  assert.match(GovernanceSection(matched), /Analysed candidate was deployed/);
  assert.match(GovernanceSection(matched), /Executed at slot 7000/);
  assert.equal(render('superseded').tone, 'dated');
  assert.match(GovernanceSection(render('superseded')), /since been upgraded again/);
  const mismatch = render('deployed_mismatch');
  assert.equal(mismatch.tone, 'alert');
  assert.match(GovernanceSection(mismatch), /role="alert"/);
  assert.match(GovernanceSection(mismatch), /does not match the candidate/);
});

await check('an old match is dated, not green, and still names its slot', () => {
  const model = view(B.matched, FRESH_SECONDS + 3 * 3600);
  assert.equal(model.state, 'matched_dated');
  assert.equal(model.tone, 'dated');
  const html = GovernanceSection(model);
  assert.doesNotMatch(html, /✓/);
  assert.match(html, new RegExp(`slot ${B.matched.observation.slot}`));
  assert.match(html, /hours ago/);
  // A check with no recorded time is never treated as fresh.
  const undated = governanceView({ delivery, check: { binding: B.matched }, changeSpecId: bound.change_spec_id, now: NOW });
  assert.equal(undated.tone, 'dated');
});

await check('a rewritten buffer leads with the committed sentence', () => {
  const model = view(B.stale, 30);
  assert.equal(model.tone, 'alert');
  assert.equal(model.title, STALE_BUFFER);
  const html = GovernanceSection(model);
  assert.match(html, /role="alert"/);
  assert.ok(html.includes("The proposal&#039;s buffer no longer matches the candidate Eplyx analysed."));
  assert.match(html, /Buffer holds/);
  assert.doesNotMatch(html, /✓/);
});

await check('an authority outside the vault is not bound, and says which', () => {
  const model = view(B.authority);
  assert.equal(model.state, 'authority_mismatch');
  assert.equal(model.tone, 'alert');
  assert.match(model.note, /authority is .* not the Squads vault/);
});

await check('an unsupported proposal is neutral and explains the shape', () => {
  const model = view(B.unsupported);
  assert.equal(model.state, 'unsupported_proposal');
  assert.equal(model.tone, 'neutral');
  assert.match(model.note, /instructions/);
});

await check('status is shown as observed: a cancelled proposal still matches, marked final', () => {
  const model = view(B.cancelled);
  assert.equal(model.state, 'matched');
  assert.match(GovernanceSection(model), /Cancelled \(final\)/);
});

await check('a recorded check about another change is refused', () => {
  const model = governanceView({ delivery, check: at(B.matched), changeSpecId: 'f'.repeat(64), now: NOW });
  assert.equal(model.state, 'identity_mismatch');
  assert.equal(model.tone, 'alert');
  // Asked about this change but bound elsewhere (the message differs) is
  // still about this change, and is shown as the failure it is.
  assert.notEqual(B.unsupported.bound_change_spec_id, bound.change_spec_id);
  assert.equal(view(B.unsupported).state, 'unsupported_proposal');
});

await check('evidence the server could not verify is an alert, never a result', () => {
  const model = governanceView({ delivery, error: 'governance evidence abc does not verify', now: NOW });
  assert.equal(model.state, 'evidence_error');
  assert.match(GovernanceSection(model), /could not be verified/);
});

await check('technical rows carry the binding, decoder provenance and message identity', () => {
  const rows = GovernanceRows(view(B.matched));
  for (const expected of [B.matched.binding_id, B.matched.decoder.source_revision, delivery.message_sha256, delivery.vault, 'SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf']) {
    assert.ok(rows.includes(expected), `missing ${expected}`);
  }
});

await check('the change identity rows and history line name the proposal', () => {
  const rows = ChangeIdentityRows({ change: { change_spec_id: bound.change_spec_id, kind: 'program_upgrade', target_program_id: bound.change.target.program_id }, spec: bound });
  assert.match(rows, /Squads V4 · transaction #42/);
  assert.ok(rows.includes(delivery.message_sha256));
  assert.match(changeLine({ change: { change_spec_id: bound.change_spec_id, kind: 'program_upgrade', target_program_id: 'x', delivery } }), /via Squads #42/);
  assert.doesNotMatch(changeLine({ change: { change_spec_id: analysed.change_spec_id, kind: 'program_upgrade', target_program_id: 'x' } }), /Squads/);
});

// --- the real page, through its real fetches -------------------------------
//
// The report body is a committed P3 report with its `change` replaced by the
// bound spec's binding: a composed input, because the page only cross-checks
// change identities, and that is what this renders.
async function page(governance) {
  store.clear();
  store.set('eplyx-operator-token', 'operator-token');
  const report = load('docs/examples/phase-p3-impact-view/drift-semantic-baseline.json');
  const change = { change_spec_id: bound.change_spec_id, kind: 'program_upgrade', target_program_id: bound.change.target.program_id, candidate_sha256: bound.change.candidate.sha256, delivery };
  report.change = change;
  report.candidate = { ...report.candidate, sha256: bound.change.candidate.sha256, len: bound.change.candidate.len };
  const run = {
    run_id: 'run_G1', project_id: 'proj_G1', status: 'failed', exit_code: 1, report_available: true,
    bundle_sha256: report.bundle.sha256, candidate_sha256: bound.change.candidate.sha256,
    change: { ...change, candidate_len: bound.change.candidate.len, origin: 'submitted', label: null },
  };
  const requests = [];
  globalThis.fetch = async url => {
    const path = String(url).replace('http://api.test', '');
    requests.push(path);
    const answer = (value, status = 200) => ({ ok: status < 400, status, json: async () => value });
    if (path === '/v1/runs/run_G1') return answer(run);
    if (path.endsWith('/report.json')) return answer(report);
    if (path.endsWith('/change_spec.json')) return answer(bound);
    if (path.startsWith('/v1/projects/proj_G1/governance/changes/')) return governance(path);
    if (path === '/v1/projects/proj_G1') return answer({ project: { name: 'Governed project' } });
    throw new Error(`unexpected request ${path}`);
  };
  let html = '';
  const stop = attachReport('run_G1', markup => { html = markup; });
  await new Promise(resolve => setTimeout(resolve, 40));
  stop();
  return { html, requests };
}

await check('the page fetches the bound change’s checks and renders the newest one', async () => {
  const fresh = Math.floor(Date.now() / 1000) - 30;
  const { html, requests } = await page(() => ({ ok: true, status: 200, json: async () => ({ checks: [{ checked_at_unix_seconds: fresh, binding: B.stale }, { checked_at_unix_seconds: fresh - 60, binding: B.matched }] }) }));
  assert.ok(requests.includes(`/v1/projects/proj_G1/governance/changes/${bound.change_spec_id}`));
  assert.ok(html.includes('Governance proposal'));
  assert.ok(html.includes("The proposal&#039;s buffer no longer matches the candidate Eplyx analysed."));
  assert.ok(html.includes('Governance binding'));
  assert.ok(html.includes(B.stale.binding_id));
});

await check('a tampered stored binding reaches the page as an error, not a match', async () => {
  const { html } = await page(() => ({ ok: false, status: 500, json: async () => ({ error: 'governance evidence 00ff does not verify' }) }));
  assert.ok(html.includes('Stored governance evidence could not be verified.'));
  assert.ok(!html.includes('Matches analysed change'));
});

if (failures) {
  console.log(`\n${failures} governance view check(s) failed`);
  process.exit(1);
}
console.log('governance view verified');
