// Phase P3: the analysis impact view, against real reports.
//
// The page is a projection of the engine's report. What must hold: execution
// and economic evaluation are separate answers; "not evaluated" is never
// dressed as safe or as adverse; a failing proposed build leads the page; an
// unexplained change never disappears; every number shown was emitted by the
// engine; and the full evidence is still there one layer down.
//
// Fixtures are engine output, not hand-written objects: see
// docs/examples/phase-p3-impact-view/README.md. The few synthetic run records
// below stand for server states that produce no report at all.

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

const {
  analysisView, runSummary, formatValue, formatBps, humanize, classifyUndeclarable,
  parseFingerprint, shortId, middle, TRANSACTION_LEVEL_CHANGES, EVALUATION_UNAVAILABLE,
} = await import('./src/analysis.js');
const { attachReport } = await import('./src/report.js');
const { runRow } = await import('./src/projects.js');
const { resolveChange } = await import('./src/change.js');

const root = new URL('../', import.meta.url);
const load = path => JSON.parse(readFileSync(new URL(path, root), 'utf8'));
const P3 = 'docs/examples/phase-p3-impact-view/';
const F = {
  driftSemantic: load(`${P3}drift-semantic-baseline.json`),
  driftReplayOnly: load(`${P3}drift-replay-only.json`),
  orcaSemantic: load(`${P3}orca-semantic-baseline.json`),
  orcaReplayOnly: load(`${P3}orca-replay-only.json`),
  reversal: load(`${P3}drift-controlled-sign-reversal.json`),
  unexplained: load(`${P3}drift-controlled-unexplained-bytes.json`),
  revert: load(`${P3}drift-controlled-revert.json`),
  legacyU14: load('docs/examples/phase-u14-drift-validation/ci.json'),
  stakeRegression: load('docs/examples/phase-u4-validation/product-controls/regression.stdout'),
  stakeBounded: load('docs/examples/phase-u4-validation/product-controls/bounded.stdout'),
  stakeStale: load('docs/examples/phase-u4-validation/product-controls/stale.stdout'),
  stakeUnevaluable: load('docs/examples/phase-u4-validation/product-controls/unevaluable.stdout'),
  stakeBaseline: load('docs/examples/phase-u4-validation/product-controls/baseline.stdout'),
};

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

/** The run record the server would hold for a report. */
function runFor(report, overrides = {}) {
  const change = report.change ? {
    ...report.change,
    candidate_len: report.candidate.len,
    label: null,
    origin: 'derived_from_candidate',
  } : null;
  return {
    run_id: 'run-p3',
    project_id: 'proj_p3',
    status: report.summary.passed ? 'passed' : 'failed',
    exit_code: report.summary.exit_code,
    report_available: true,
    bundle_sha256: report.bundle.sha256,
    corpus_sha256: report.bundle.corpus_sha256,
    baseline_sha256: report.bundle.baseline_sha256,
    candidate_sha256: report.candidate.sha256,
    change,
    ...overrides,
  };
}

const view = (report, run = runFor(report)) =>
  analysisView({ run, report, resolution: resolveChange(run, report, null) });
const dim = (model, key) => model.dimensions.find(d => d.key === key);

/** Visible text only. */
const visible = html => html.replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');
/** The ordinary view: everything except the technical details layer. */
const ordinary = html => visible(html.replace(/<section[^>]*data-technical[\s\S]*?<\/section>/g, ''));
const technical = html => html.match(/<section[^>]*data-technical[\s\S]*?<\/section>/)?.[0] ?? '';

/** Follow a run through the real page, with the server stubbed. */
async function page(run, report, spec = null) {
  store.clear();
  store.set('eplyx-operator-token', 'operator-token');
  globalThis.fetch = async url => {
    const path = String(url).replace('http://api.test', '');
    const answer = value => value
      ? { ok: true, status: 200, json: async () => value }
      : { ok: false, status: 404, json: async () => ({ error: 'no' }) };
    if (path === `/v1/runs/${run.run_id}`) return answer(run);
    if (path.endsWith('/report.json')) return answer(report);
    if (path.endsWith('/change_spec.json')) return answer(spec);
    if (path.startsWith('/v1/projects/')) return answer({ project: { name: 'Pilot project' } });
    throw new Error(`unexpected request ${path}`);
  };
  let html = '';
  const stop = attachReport(run.run_id, markup => { html = markup; });
  await new Promise(resolve => setTimeout(resolve, 40));
  stop();
  return html;
}

// A safety verdict, or an absence dressed as one. "Not evidence of no impact"
// and "does not treat them as harmless" are the honest negations and pass.
const SAFE = /\b(safe|unsafe)\b|no (economic )?impact (was )?(detected|found)|no changes detected/i;

// --- 1. matched replay + semantic coverage + no findings ------------------
await check('1. Drift U14 baseline: execution verified, impact evaluated, zero findings', async () => {
  const model = view(F.driftSemantic);
  assert.equal(model.state, 'no_changes_found');
  assert.equal(dim(model, 'execution').value, 'Verified');
  assert.equal(dim(model, 'impact').value, 'Evaluated');
  assert.equal(dim(model, 'findings').value, 'None');
  assert.equal(dim(model, 'unexplained').value, 'None');
  // The PnL subject is visible as what was evaluated, with no invented value.
  const subjects = model.groups.flatMap(g => g.subjects);
  const pnl = subjects.find(s => s.key.endsWith('/pnl_settled'));
  assert.equal(pnl.title, 'PnL settled');
  assert.equal(pnl.state, 'unchanged');
  assert.equal(pnl.cards.length, 0);
  const html = await page(runFor(F.driftSemantic), F.driftSemantic);
  assert.match(html, /No changes found in what Eplyx evaluated/);
  assert.match(ordinary(html), /PnL settled No change measured/);
  assert.doesNotMatch(ordinary(html), /\d\.\d{6}/, 'a PnL value was shown for a subject with no finding');
});

await check('1b. Orca semantic report: token-flow subjects render cleanly', async () => {
  const model = view(F.orcaSemantic);
  assert.equal(model.state, 'no_changes_found');
  const [group] = model.groups;
  assert.equal(group.protocol, 'Orca whirlpool');
  assert.equal(group.title, 'Swap V2');
  assert.deepEqual(group.subjects.map(s => s.title), ['User input spent', 'Vault A tokens out', 'Vault B tokens in', 'Transaction outcome']);
  assert.ok(group.subjects.every(s => s.state === 'unchanged'));
  const html = await page(runFor(F.orcaSemantic), F.orcaSemantic);
  assert.match(ordinary(html), /Vault B tokens in/);
  assert.doesNotMatch(ordinary(html), /vault_b_tokens_in/, 'raw subject names in the ordinary view');
  assert.match(technical(html), /orca-whirlpool\/swap_v2\/economic\/vault_b_tokens_in/, 'raw subject missing from technical details');
});

// --- 2. matched replay + economic findings --------------------------------
await check('2. a controlled Drift finding is shown clearly as an economic change', async () => {
  const model = view(F.reversal);
  assert.equal(model.state, 'changes_detected');
  assert.equal(model.headline.title, 'Unexpected economic changes detected.');
  assert.equal(dim(model, 'execution').value, 'Verified');
  assert.equal(dim(model, 'impact').value, 'Evaluated');
  assert.equal(dim(model, 'findings').value, '1');
  const card = model.groups[0].subjects.find(s => s.cards.length).cards[0];
  assert.equal(card.title, 'PnL settled decreased');
  assert.equal(card.statusLabel, 'Unexpected');
  const html = await page(runFor(F.reversal), F.reversal);
  assert.match(ordinary(html), /PnL settled decreased/);
  assert.match(ordinary(html), /Before Proposed Relative change/);
});

// --- 3. matched replay + NoSemanticCoverage -------------------------------
for (const [name, report, contract] of [['U13.3 none@0', F.driftReplayOnly, 3], ['U11.2 none@0', F.orcaReplayOnly, 2]]) {
  await check(`3. ${name}: execution verified, economic impact not evaluated`, async () => {
    const model = view(report);
    assert.equal(model.state, 'not_evaluated');
    assert.equal(dim(model, 'execution').value, 'Verified');
    assert.equal(dim(model, 'impact').value, 'Not evaluated');
    assert.equal(dim(model, 'findings').value, 'Not applicable');
    assert.deepEqual(model.proof.contracts, [contract]);
    const html = await page(runFor(report), report);
    assert.match(html, /Economic impact could not be evaluated for this interaction/);
    assert.match(ordinary(html), /execution Verified/i);
    assert.match(ordinary(html), /absence of findings here is not evidence of no impact/);
    assert.match(ordinary(html), /Technical details/, 'no route to the technical evidence');
  });
}

// --- 4. semantic coverage + undeclarable structural bytes ------------------
await check('4. a named change plus unexplained bytes keeps both visible', async () => {
  const model = view(F.unexplained);
  assert.equal(model.state, 'changes_detected');
  assert.equal(dim(model, 'unexplained').value, '1');
  assert.deepEqual(model.unexplained.accounts, ['user']);
  assert.equal(model.unexplained.rows[0].what, 'bytes at offset 4360');
  const html = await page(runFor(F.unexplained), F.unexplained);
  const text = ordinary(html);
  assert.match(text, /Additional state changed that Eplyx could not semantically explain/);
  assert.match(text, /user bytes at offset 4360/);
  assert.match(text, /does not treat them as harmless/);
  assert.match(text, /PnL settled increased/, 'the named change was hidden behind the unexplained one');
});

await check('4b. decoded economic changes nothing named stay visible (real stake-pool control)', async () => {
  const model = view(F.stakeRegression);
  const decoded = F.stakeRegression.undeclarable.filter(u => u.layer === 'decoded_economic');
  assert.equal(model.unexplained.decoded.length, decoded.length);
  assert.equal(model.unexplained.structural.length, F.stakeRegression.undeclarable.length - decoded.length);
  const html = await page(runFor(F.stakeRegression), F.stakeRegression);
  for (const change of F.stakeRegression.undeclarable) {
    assert.ok(ordinary(html).includes(change.description.split(' ').slice(1).join(' ')), `missing ${change.description}`);
  }
  assert.match(ordinary(html), /Decoded values no named finding speaks for/);
});

// --- 5. candidate revert ----------------------------------------------------
await check('5. a reverting proposed build is the primary impact, with no economic zeros', async () => {
  const model = view(F.revert);
  assert.equal(model.primary[0].kind, 'now_reverts');
  assert.equal(model.primary[0].title, 'The proposed build fails for 1 of 1 historical production interaction.');
  assert.equal(model.headline.title, 'The proposed build fails for historical production interactions.');
  const pnl = model.groups[0].subjects.find(s => s.key.endsWith('/pnl_settled'));
  assert.equal(pnl.state, 'not_compared');
  const html = await page(runFor(F.revert), F.revert);
  const alert = html.indexOf('class="primary-alert"');
  assert.ok(alert > 0 && alert < html.indexOf('id="impact"'), 'the failure is buried under the impact cards');
  assert.doesNotMatch(ordinary(html), /\b0\.0+\b/, 'an economic zero was shown for a failed execution');
  assert.doesNotMatch(html, /value-table/, 'a before/proposed table for a failed execution');
  assert.match(ordinary(html), /Not compared in the 1 interaction where execution failed/);
});

await check('5b. real stake-pool regression: 9 of 10 withdrawals now fail, first', async () => {
  const model = view(F.stakeRegression);
  assert.equal(model.primary[0].title, 'The proposed build fails for 9 of 10 historical production interactions.');
  assert.equal(model.primary[0].action, 'Withdraw SOL');
  // A declared failure is still a primary fact; the label says it is declared.
  const bounded = view(F.stakeBounded);
  assert.equal(bounded.primary[0].statusLabel, 'Declared');
  assert.equal(bounded.headline.title, 'Changes detected that Eplyx could not explain.');
});

// --- 6. replay mismatch -----------------------------------------------------
await check('6. a replay that did not reproduce is a verification failure, not a finding', async () => {
  const run = {
    run_id: 'run-mismatch', status: 'failed', exit_code: 2, report_available: false,
    detail: 'baseline replay for 229293976795dd2ee8eb88a9849004b25fce536d6540f459fc1848efbff63b49: baseline historical fidelity differs: post-state account user data',
    bundle_sha256: F.driftSemantic.bundle.sha256, candidate_sha256: F.driftSemantic.candidate.sha256,
    change: runFor(F.driftSemantic).change,
  };
  const model = analysisView({ run, report: null });
  assert.equal(model.state, 'not_verified');
  assert.equal(dim(model, 'execution').value, 'Not verified');
  assert.equal(dim(model, 'impact').value, 'Not evaluated');
  const html = await page(run, null);
  assert.match(html, /could not verify execution/);
  assert.match(html, /Exit code 2/);
  assert.match(html, /fidelity differs/);
  assert.doesNotMatch(html, /economic changes detected|Changes detected/i);
  // A report that ever states another proof status is not shown as verified.
  const odd = { ...F.driftSemantic, replay_proof: { ...F.driftSemantic.replay_proof, status: 'mismatched' } };
  assert.equal(view(odd).state, 'not_verified');
  assert.equal(dim(view(odd), 'execution').value, 'Not verified');
});

await check('6b. an incompatible bundle is not an execution result', () => {
  const model = analysisView({ run: { status: 'failed', exit_code: 4, report_available: false }, report: null });
  assert.equal(model.state, 'incompatible');
  assert.equal(dim(model, 'execution').value, 'Not attempted');
});

// --- 7. execution_error -----------------------------------------------------
await check('7. an infrastructure execution error is an analysis error', async () => {
  const run = { run_id: 'run-err', status: 'execution_error', exit_code: null, report_available: false, detail: 'candidate artefact failed verification', bundle_sha256: 'b', candidate_sha256: 'c', change: null };
  const model = analysisView({ run, report: null });
  assert.equal(model.state, 'analysis_error');
  assert.equal(model.headline.label, 'Analysis error');
  const html = await page(run, null);
  assert.match(html, /could not complete/);
  assert.match(html, /not a finding about the upgrade/);
  assert.doesNotMatch(html, /Exit code \d/);
});

// --- 8. legacy run with no ChangeSpec ---------------------------------------
await check('8. a legacy run with a pre-change-identity report', async () => {
  const run = runFor(F.legacyU14, { change: null });
  assert.equal(F.legacyU14.change, undefined, 'fixture should predate change identity');
  const html = await page(run, F.legacyU14);
  assert.match(html, /Legacy run/);
  assert.doesNotMatch(html, /Change ID<\/dt>/);
  assert.match(html, /No changes found in what Eplyx evaluated/);
  assert.match(technical(html), /none — legacy run/);
});

// --- 9, 10. proof contracts --------------------------------------------------
await check('9. proof contract 2: reconstructed, never called observed', async () => {
  const model = view(F.orcaSemantic);
  const lines = [...model.proof.statements, ...model.proof.qualifiers].join(' ');
  assert.match(lines, /checkpointed historical replay/);
  assert.match(lines, /reconstructed from retained validator evidence/);
  assert.match(lines, /not directly observed/);
  assert.match(lines, /does not claim exact fidelity/);
  assert.doesNotMatch(lines, /directly observed transaction boundary was/);
});

await check('10. proof contract 3: a derived target boundary is said to be derived', async () => {
  const model = view(F.driftSemantic);
  assert.deepEqual(model.proof.contracts, [3]);
  assert.ok(model.proof.boundary.includes('derived_target_boundary'));
  const lines = model.proof.qualifiers.join(' ');
  assert.match(lines, /derived by replaying the earlier transactions/);
  assert.match(lines, /not directly observed/);
  const html = await page(runFor(F.driftSemantic), F.driftSemantic);
  assert.match(technical(html), /contract 3/);
  assert.match(technical(html), /derived_target_boundary/);
  assert.match(technical(html), /checkpointed_execution_v1/);
});

// --- 11. SemanticBinding exact flag ------------------------------------------
await check('11. exact source-to-ELF verification false stays visible', async () => {
  const model = view(F.driftSemantic);
  assert.equal(model.proof.provenance.exact, false);
  assert.deepEqual(model.proof.provenance.levels, ['Execution-corroborated external interface']);
  const html = await page(runFor(F.driftSemantic), F.driftSemantic);
  assert.match(ordinary(html), /Exact build verified: no/);
  assert.match(ordinary(html), /not established/);
  assert.match(ordinary(html), /How does Eplyx know this\?/);
  assert.match(ordinary(html), /velocity-exchange\/protocol-v2 at 73d22383e621/);
  assert.match(technical(html), /exact_source_to_elf_verified<\/span><b>false/);
  // Corroborated is never promoted to exact.
  assert.doesNotMatch(ordinary(html), /verified against the exact/);
});

// --- 12. signed quantities -----------------------------------------------------
await check('12. a signed semantic quantity keeps its sign, exactly as emitted', async () => {
  const card = view(F.reversal).groups[0].subjects.find(s => s.cards.length).cards[0];
  assert.equal(card.values.length, 1);
  assert.equal(card.values[0].before.text, '+0.000203');
  assert.equal(card.values[0].proposed.text, '−0.000203');
  assert.equal(card.values[0].proposed.full, '-0.000203', 'the emitted string was altered');
  assert.equal(card.values[0].relative, null, 'a relative change was invented for a signed amount');
  const html = await page(runFor(F.reversal), F.reversal);
  assert.match(ordinary(html), /\+0\.000203 −0\.000203 not defined/);
  // No float ever touches a value: 2^64 with nine decimals survives intact.
  assert.equal(formatValue({ kind: 'quantity', quantity: '18446744073709551615.000000001' }).text, '18446744073709551615.000000001');
  assert.equal(formatBps(-21), '−21 bps (−0.21%)');
  assert.equal(formatBps(12345), '+12,345 bps (+123.45%)');
  assert.equal(formatBps(0), '0 bps (0.00%)');
  assert.equal(formatBps(null), null);
});

// --- 13, 14. truncation and technical expansion --------------------------------
await check('13. long targets and hashes are shortened in the ordinary view', async () => {
  const report = F.driftSemantic;
  const html = await page(runFor(report), report);
  const text = ordinary(html);
  assert.match(text, /dRifty…33UH/);
  for (const full of [report.change.change_spec_id, report.candidate.sha256, report.bundle.sha256, report.bundle.program_id]) {
    assert.ok(!text.includes(full), `${full.slice(0, 8)}… is shown in full in the ordinary view`);
  }
  assert.ok(text.includes(report.change.change_spec_id.slice(0, 8)), 'short change ID missing');
  assert.equal(shortId('a'.repeat(64)), 'aaaaaaaa');
  assert.equal(middle('SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy'), 'SPoo1K…kuHy');
});

await check('14. technical details expose every identity in full', async () => {
  const report = F.driftSemantic;
  const html = technical(await page(runFor(report), report));
  const binding = report.semantic_binding.observations[0].binding;
  for (const full of [
    report.change.change_spec_id, report.candidate.sha256, report.bundle.sha256,
    report.bundle.corpus_sha256, report.bundle.baseline_sha256, report.bundle.program_id,
    binding.historical_elf_sha256, binding.execution_evidence_sha256, binding.source.commit,
    ...binding.source.source_blobs.map(blob => blob.git_blob_sha1),
    report.replay_proof.boundary_proof,
  ]) {
    assert.ok(html.includes(full), `technical details lack ${full.slice(0, 12)}…`);
  }
  const findings = technical(await page(runFor(F.stakeRegression), F.stakeRegression));
  for (const finding of F.stakeRegression.findings) {
    assert.ok(findings.includes(finding.fingerprint), `raw fingerprint ${finding.fingerprint} missing`);
    for (const entity of finding.entities) assert.ok(findings.includes(entity));
  }
});

// --- 15, 16. no safety verdicts ---------------------------------------------------
await check('15. no page says "safe" — zero findings is not a safety verdict', async () => {
  for (const [name, report] of Object.entries(F)) {
    const html = await page(runFor(report), report);
    assert.doesNotMatch(ordinary(html), SAFE, `${name} renders a safety verdict`);
  }
  const passed = view(F.driftSemantic);
  assert.match(passed.headline.note, /tested interactions and subjects only/);
});

await check('16. no semantic coverage never reads as adverse or as safe', async () => {
  for (const report of [F.driftReplayOnly, F.orcaReplayOnly]) {
    const model = view(report);
    assert.equal(model.headline.tone, 'is-unknown');
    assert.ok(!['ok', 'bad'].includes(dim(model, 'impact').tone));
    assert.ok(!['ok', 'bad'].includes(dim(model, 'findings').tone));
    const html = await page(runFor(report), report);
    assert.doesNotMatch(ordinary(html), /No changes found|Changes detected|Unexpected economic/);
    assert.doesNotMatch(ordinary(html), SAFE);
  }
  // Nothing measured is "not evaluated" even if a report lacks the reason.
  const bare = { ...F.driftSemantic, coverage: [] };
  assert.equal(view(bare).state, 'not_evaluated');
});

// --- 17. affected facts come only from the report ---------------------------------
await check('17. affected counts are unions of report facts, nothing more', () => {
  const report = F.stakeRegression;
  const model = view(report);
  const touched = new Set([...report.findings.flatMap(f => f.observations), ...report.undeclarable.flatMap(u => u.observations)]);
  assert.deepEqual(model.affected.interactions, { affected: touched.size, total: report.bundle.record_count });
  assert.deepEqual(model.affected.entities, [...new Set(report.findings.flatMap(f => f.entities))].sort());
  const accountLevel = report.undeclarable.filter(u => !TRANSACTION_LEVEL_CHANGES.includes(u.description));
  assert.deepEqual(model.affected.accounts, [...new Set(accountLevel.map(u => u.description.split(' ')[0]))].sort());
  const clean = view(F.driftSemantic);
  assert.deepEqual(clean.affected, { interactions: { affected: 0, total: 1 }, entities: [], accounts: [], changedSubjects: [] });
  assert.deepEqual(view(F.driftReplayOnly).affected.interactions, { affected: 0, total: 1 });
});

// --- 18. identity consistency ------------------------------------------------------
await check('18. P1/P2 identity mismatches remain visible and suppress the result', async () => {
  const report = F.driftSemantic;
  const other = runFor(report, { change: { ...runFor(report).change, change_spec_id: 'f'.repeat(64) } });
  const html = await page(other, report);
  assert.match(html, /does not agree with itself/);
  assert.ok(html.includes('f'.repeat(64)));
  assert.doesNotMatch(html, /No changes found|CI gate Passed/);
  const swapped = runFor(report, { candidate_sha256: 'e'.repeat(64) });
  const html2 = await page(swapped, report);
  assert.match(html2, /stored candidate e{64} but its report executed/);
  assert.doesNotMatch(html2, /No changes found/);
});

// --- declarations -----------------------------------------------------------------
await check('stale and unevaluable declarations are review items, not findings', () => {
  const stale = view(F.stakeStale);
  assert.equal(stale.state, 'declarations_attention');
  assert.equal(stale.headline.title, 'A declared change no longer happens.');
  assert.equal(stale.declarations[0].statusLabel, 'Stale declaration');
  const unevaluable = view(F.stakeUnevaluable);
  assert.equal(unevaluable.headline.title, 'A declared change cannot be judged by this corpus.');
  assert.equal(unevaluable.declarations[0].statusLabel, 'Cannot be judged');
});

// --- vocabulary guards --------------------------------------------------------------
await check('undeclarable classification follows the engine’s own descriptions', () => {
  const source = readFileSync(new URL('engine/src/ci.rs', root), 'utf8');
  for (const literal of TRANSACTION_LEVEL_CHANGES) {
    assert.ok(source.includes(`"${literal}"`), `engine no longer emits "${literal}"`);
  }
  assert.ok(source.includes('"{label} bytes at offset {first}"'));
  assert.ok(source.includes('"{label} account state"'));
  assert.ok(source.includes(`"${EVALUATION_UNAVAILABLE}{reason}"`));
  assert.deepEqual(classifyUndeclarable({ description: 'transaction fee', observations: ['o'] }).scope, 'transaction');
  assert.deepEqual(classifyUndeclarable({ description: 'user bytes at offset 7', observations: [] }).account, 'user');
  assert.equal(classifyUndeclarable({ description: `${EVALUATION_UNAVAILABLE}role binding`, observations: [] }).scope, 'evaluation');
  assert.equal(classifyUndeclarable({ description: 'opaque', observations: [] }).what, 'opaque', 'an unrecognised change was dropped');
});

await check('names are formatted generically, with no protocol names in the logic', () => {
  assert.equal(humanize('settle_pnl'), 'Settle PnL');
  assert.equal(humanize('drift-settle-pnl'), 'Drift settle PnL');
  assert.equal(humanize('sol_received_by_user'), 'SOL received by user');
  assert.equal(parseFingerprint('a/b/c/d'), null);
  const source = readFileSync(new URL('frontend/src/analysis.js', root), 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/.*$/gm, '');
  for (const protocol of ['drift', 'orca', 'whirlpool', 'kamino', 'stake-pool', 'token2022']) {
    assert.ok(!source.toLowerCase().includes(protocol), `analysis.js hardcodes ${protocol}`);
  }
});

// --- 15 (history) -------------------------------------------------------------------
await check('history rows show change, execution and impact — not a hash table', () => {
  const change = runFor(F.reversal).change;
  const row = runRow({ run_id: 'r', status: 'failed', exit_code: 1, report_available: true, created_at_unix_seconds: 1, candidate_sha256: F.reversal.candidate.sha256, bundle_sha256: F.reversal.bundle.sha256, change });
  assert.match(visible(row), /Execution Verified/);
  assert.match(visible(row), /Impact Changes detected/);
  assert.match(visible(row), new RegExp(`change ${change.change_spec_id.slice(0, 8)}`));
  assert.ok(!row.includes(F.reversal.bundle.sha256.slice(0, 10)), 'bundle hash in a history row');
  assert.ok(!row.includes(F.reversal.candidate.sha256.slice(0, 8)), 'candidate hash in a history row');
  const cases = [
    [{ status: 'passed', exit_code: 0, report_available: true }, 'Verified', 'No unexpected changes'],
    [{ status: 'failed', exit_code: 2, report_available: true }, 'Verified', 'Not evaluated'],
    [{ status: 'failed', exit_code: 2, report_available: false }, 'Not verified', 'Not evaluated'],
    [{ status: 'failed', exit_code: 4, report_available: false }, 'Not run', 'Not evaluated'],
    [{ status: 'failed', exit_code: 3, report_available: true }, 'Verified', 'Stale declaration'],
    [{ status: 'failed', exit_code: 5, report_available: true }, 'Verified', 'Declaration unjudged'],
    [{ status: 'execution_error', exit_code: null }, 'Analysis error', 'Not evaluated'],
    [{ status: 'queued' }, 'Queued', 'Pending'],
  ];
  for (const [run, execution, impact] of cases) {
    const summary = runSummary(run);
    assert.equal(summary.execution.label, execution, JSON.stringify(run));
    assert.equal(summary.impact.label, impact, JSON.stringify(run));
    assert.doesNotMatch(`${summary.execution.label} ${summary.impact.label}`, SAFE);
  }
  assert.match(visible(runRow({ run_id: 'l', status: 'passed', exit_code: 0, report_available: true, created_at_unix_seconds: 1, candidate_sha256: 'c'.repeat(64), change: null })), /Legacy run/);
});

console.log(failures === 0 ? '\nanalysis impact view verified' : `\n${failures} analysis view failures`);
process.exit(failures === 0 ? 0 : 1);
