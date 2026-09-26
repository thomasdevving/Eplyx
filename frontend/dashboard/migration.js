// Token Migration V1 pages. Every status comes from the engine's report.json
// through the local API; this module only chooses wording and layout. The
// default Overview answers the operator's questions in order; Solana detail
// (addresses, programs, extensions, hashes) stays behind Technical mode.
import { esc, pill, gatePill, tone, words, sentence, raw, count, short, addr, ident, kv, panel, empty, commandLine, tile } from './ui.js';
import { BASE, API } from './env.js';

export const isMigration = run => run?.kind === 'token_migration';

const PROVENANCE = {
 Observed:['Captured state', 'An exact account from a read-only mainnet capture.'],
 ObservedPopulationRehearsal:['Captured state', 'An exact captured holder executed by the sequential population rehearsal.'],
 SyntheticPopulationRehearsal:['Synthetic fixture', 'An exact fixture holder executed by the sequential population rehearsal. Not chain state.'],
 Synthetic:['Synthetic fixture', 'An account a local fixture recipe built with the real token programs. Not chain state.'],
 DerivedFromObserved:['Derived from capture', 'A typed local change of a captured account. Not an observed mainnet state.'],
 DerivedFromSynthetic:['Derived from fixture', 'A typed local change of a fixture account. Not chain state.'],
};
export const provenanceTag = p => {
 const [label, title] = PROVENANCE[p] ?? [words(p), ''];
 return `<span class="tag ${String(p).startsWith('Derived') ? 'tag--soft' : ''}" title="${esc(title)}">${esc(label)}</span>`;
};

const CLASSES = {
 Migratable:'Can migrate under the proposed mechanism',
 InsufficientReserve:'The reserve runs out before this holder (plan order)',
 ZeroBalance:'No balance to migrate',
 OutsideEligibility:'Outside the declared eligibility',
 OutputBelowMinimum:'Converts to less than the minimum output (dust)',
 Frozen:'Source account is frozen',
 UninitializedSource:'Source account is not initialized',
 UnsupportedTokenSemantics:'Token-2022 state the mechanism cannot execute faithfully',
 AuthorityPathUnavailable:'No signer the specification allows can authorize it',
 UnverifiableAuthority:'Owner could not be verified or was not inspected',
 InvalidDestinationState:'Destination account cannot receive the tokens',
 UnverifiableDestination:'Destination account was not inspected',
 WindowConflict:'Outside the migration window',
 FundingPathUnavailable:'Destination funding is unavailable to the migration authority',
 MintStateBlocksMigration:'A mint state (pause, hook, extension) blocks migration',
};
const classTone = c => c === 'Migratable' ? 'Proven' : ['ZeroBalance', 'OutsideEligibility'].includes(c) ? 'NotApplicable' : ['UnverifiableAuthority', 'UnverifiableDestination'].includes(c) ? 'NotTested' : ['InsufficientReserve', 'FundingPathUnavailable', 'WindowConflict'].includes(c) ? 'Failed' : 'Unsupported';

const FINDINGS = {
 UnexpectedFailure:'The specification requires this migration; the candidate rejected it.',
 UnexpectedSuccess:'The specification requires a rejection; the candidate migrated anyway.',
 ReconciliationMismatch:'The candidate migrated, but the resulting state differs from the specification.',
 RollbackViolation:'A rejected migration left referenced state changed.',
 EligibleHolderNotMigrated:'An eligible holder the proposal cannot migrate (for example, the reserve runs out).',
};
export const findingText = f => FINDINGS[f] ?? words(f);

const axisPill = a => pill(a?.status, a?.status ? words(a.status) : 'Not recorded');
const codes = list => (list ?? []).length ? `<span class="muted">${(list ?? []).map(c => `<code>${esc(c)}</code>`).join(' ')}</span>` : '';

export function migrationTiles(run) {
 const r = run.readiness ?? {}, m = run.migration ?? {}, s = run.stress ?? {};
 const cx = run.search;
 return `<div class="tiles">
  ${tile({ label:'Candidate', value:`<code>${short(run.candidate_program_sha256, 8)}</code>`, sub:`Input <code>${short(run.analysis_input_sha256, 8)}</code>`, href:`${BASE}/runs/${run.id}#migration` })}
  ${tile({ label:'Can it execute?', value:axisPill({ status:r.mechanism }), sub:`${count(m.migrated)} of ${count(m.attempted)} attempted holders migrated`, status:r.mechanism, href:`${BASE}/runs/${run.id}#execution` })}
  ${tile({ label:'Population', value:count(run.population?.positive_balance_accounts_observed), sub:`holders with a balance · ${axisPill({ status:r.population })}`, status:r.population, href:`${BASE}/production?run=${run.id}` })}
  ${tile({ label:'Funding', value:axisPill({ status:r.funding }), sub:m.required_reserve_raw ? `needs ${raw(m.required_reserve_raw)} · has ${raw(m.available_reserve_raw)} raw` : esc(words(m.funding ?? '')), status:r.funding, href:`${BASE}/runs/${run.id}#accounting` })}
  ${tile({ label:'Stress cases', value:`${count(s.behaving)} / ${count(s.executed)}`, sub:'behave as specified', status:s.executed && s.behaving === s.executed ? 'Proven' : s.deviating ? 'Failed' : 'Indeterminate', href:`${BASE}/runs/${run.id}#stress` })}
  ${tile({ label:'Counterexamples', value:cx?.state === 'Recorded' ? count(cx.total) : '—', sub:cx?.state === 'Recorded' ? `${count(cx.observed)} rehearsal · ${count(cx.derived)} derived` : 'No search recorded · run <code>eplyx migration search</code>', status:cx?.state === 'Recorded' ? (cx.total ? 'Failed' : 'Proven') : '', href:`${BASE}/runs/${run.id}#search` })}
 </div>`;
}

export const migrationRunCells = run => ({
 conversion:`<span class="tag" title="Token Migration V1">Migration</span> ${pill(run.readiness?.mechanism)}`,
 stress:`<span class="${Number(run.stress?.deviating) ? 'text-crit' : ''}">${count(run.stress?.behaving)}/${count(run.stress?.executed)}</span>`,
});

function side(s) {
 if (!s) return '—';
 return `${ident(s.mint, 'addr')} <span class="muted">${esc(s.token_program_label ?? '')} · ${esc(s.decimals ?? '?')} decimals</span><span class="tech-only muted"> · program <code>${esc(s.token_program ?? '')}</code></span>`;
}

// The engine states each answer's status; the detail sentences here only
// reword the report fields that answer carries, in the same order.
const ANSWER_DETAIL = [
 a => `<p>${esc(a.detail)}</p>`,
 a => `<p>${esc(a.detail)}</p>`,
 (a, d) => {
  const reasons = d.migration_detail?.readiness?.population?.reasons ?? [];
  return reasons.length ? `<ul class="limits">${reasons.map(r => `<li>${esc(r)}</li>`).join('')}</ul><p class="tech-only">${codes(a.detail)}</p>` : '<p>Every positive holder in scope can migrate.</p>';
 },
 (a, d) => {
  const eq = d.migration_detail?.reconciliation?.equations ?? [];
  return `<p>${esc(sentence(a.detail))}: ${count(eq.filter(e => e.holds).length)} of ${count(eq.length)} accounting equations hold.</p>`;
 },
 a => `<p>${a.detail?.required_raw ? `Needs ${raw(a.detail.required_raw)} destination raw units for the migratable holders; ${raw(a.detail.available_raw)} available.` : 'The destination is minted, not paid from a reserve.'}</p>${(a.detail?.codes ?? []).length ? `<p>${codes(a.detail.codes)}</p>` : ''}`,
 a => `<p>${count(a.detail?.deviating_stress_cases ?? 0)} stress case${Number(a.detail?.deviating_stress_cases) === 1 ? ' deviates' : 's deviate'} from the specification; ${a.detail?.counterexamples == null ? 'no counterexample search is recorded' : `${count(a.detail.counterexamples)} counterexample${a.detail.counterexamples === 1 ? '' : 's'} in the recorded search`}.</p>`,
 a => `<ul class="limits">${(Array.isArray(a.detail) ? a.detail : [a.detail]).map(l => `<li>${esc(l)}</li>`).join('')}</ul>`,
 (a, d) => {
  const first = d.search_detail?.counterexamples?.find(c => c.saved);
  return first ? `${commandLine(`eplyx migration reproduce ${first.id}`)}<p class="muted">Replays the saved run and search in the local VM with the RPC environment removed.</p>` : `${commandLine(`eplyx migration gate --run ${d.id}`)}<p class="muted">No saved counterexample. Replays this run offline and re-evaluates the gate.</p>`;
 },
];

const ANSWER_STATUS = { Found:['Failed', 'Found'], 'None found':['Proven', 'None found'] };
const answerStatus = status => ANSWER_STATUS[status] ? pill(...ANSWER_STATUS[status]) : ['Stated', 'Offline'].includes(status) ? `<span class="tag">${esc(status)}</span>` : pill(status);
const answerTone = status => ANSWER_STATUS[status] ? tone(ANSWER_STATUS[status][0]) : ['Stated', 'Offline'].includes(status) ? 'neutral' : tone(status);

function answers(detail) {
 return `<ol class="answers">${(detail.answers ?? []).map((a, n) => `<li class="answer answer--${answerTone(a.status)}"><span class="answer__n">${n + 1}</span><div><strong>${esc(a.question)}</strong>${(ANSWER_DETAIL[n] ?? ANSWER_DETAIL[0])(a, detail)}</div>${answerStatus(a.status)}</li>`).join('')}</ol>`;
}

function classTable(impact) {
 const rows = (impact?.classes ?? []).slice().sort((a, b) => (a.class === 'Migratable' ? -1 : b.class === 'Migratable' ? 1 : b.accounts - a.accounts));
 return `<div class="table-wrap"><table class="table"><thead><tr><th>Class</th><th>What it means</th><th class="num">Accounts</th><th class="num tech-only">Balance (raw)</th></tr></thead><tbody>${rows.map(c => `<tr><td>${pill(classTone(c.class), words(c.class))}</td><td>${esc(CLASSES[c.class] ?? '')}<code class="tech-only"> ${esc(c.code)}</code></td><td class="num">${count(c.accounts)}</td><td class="num tech-only">${raw(c.balance_raw)}</td></tr>`).join('')}</tbody></table></div>
 <p class="note">${esc(impact?.note ?? '')}</p>`;
}

function stranded(impact) {
 const rows = impact?.not_migratable_examples ?? [];
 if (!rows.length) return '';
 return `<details class="tech-only"><summary>First ${count(rows.length)} positive holders that cannot migrate</summary><div class="table-wrap"><table class="table"><thead><tr><th>Source account</th><th>Class</th><th class="num">Balance</th><th>Reason</th></tr></thead><tbody>${rows.map(r => `<tr><td>${ident(r.source_account, 'addr')}</td><td>${esc(words(r.class))}</td><td class="num">${raw(r.balance_raw)}</td><td class="muted">${(r.reasons ?? []).map(x => `<code>${esc(x.code)}</code> ${esc(x.detail)}`).join('<br>')}</td></tr>`).join('')}</tbody></table></div></details>`;
}

function accounting(rec) {
 if (!rec) return empty('No reconciliation recorded.');
 const n = key => raw(rec[key]);
 return `<p>${pill(rec.status === 'Mismatch' ? 'Failed' : rec.status === 'FullyReconciled' ? 'Proven' : rec.status === 'NothingExecuted' ? 'NotTested' : 'Indeterminate', words(rec.status))} ${rec.status === 'ReconciledForExecutedUnits' ? '<span class="muted">Every equation holds for the executed holders; the population scope is not complete, so this is not reported as fully reconciled.</span>' : ''}</p>
 ${(rec.not_fully_reconciled_because ?? []).length ? `<ul class="limits">${rec.not_fully_reconciled_because.map(r => `<li>${esc(r)}</li>`).join('')}</ul>` : ''}
 ${kv([
  ['Source consumed', `${n('simulated_consumed_raw')} <span class="muted">raw</span>`],
  ['Burned / escrowed', `${n('source_burned_raw')} / ${n('source_escrowed_gross_raw')}`],
  ['Destination released / minted', `${n('destination_released_raw')} / ${n('destination_minted_raw')}`],
  ['Credited to holders (net)', n('destination_credited_net_raw')],
  ['Token-2022 fees withheld', `source ${n('source_escrow_withheld_fee_raw')} · destination ${n('destination_withheld_fee_raw')}`],
  ['Migration fee (source units)', n('migration_fee_source_raw')],
  ['Rounding dust', `${esc(rec.rounding_delta_numerator ?? '0')} / ${esc(rec.rounding_denominator ?? '1')} <span class="muted">destination raw</span>`, 'tech-only'],
  ['Reserve required / available / remaining', `${n('required_reserve_raw')} / ${n('available_reserve_raw')} / ${n('remaining_reserve_raw')}`],
  ['Attempted but not migrated', n('failed_attempted_source_raw')],
 ])}
 <h3 class="subhead">Equations.</h3><ul class="equations">${(rec.equations ?? []).map(e => `<li>${pill(e.holds ? 'Satisfied' : 'Violated', e.holds ? 'holds' : 'fails')} ${esc(e.name)} <code class="tech-only">${raw(e.left)} = ${raw(e.right)}</code></li>`).join('')}</ul>`;
}

function compatibility(c) {
 if (!c) return empty('No compatibility section recorded.');
 const finding = f => `<tr><td>${esc(words(f.side))}</td><td>${esc(f.extension)}</td><td>${esc(f.operation)}</td><td>${pill(f.support === 'Supported' ? 'Proven' : f.support === 'SupportedWithSpecialSemantics' ? 'Satisfied' : f.support === 'Unverifiable' ? 'NotTested' : 'Unsupported', words(f.support))}</td><td class="muted">${esc(f.semantics)}</td></tr>`;
 return `${kv([['Source token program', side(c.source)], ['Destination token program', side(c.destination)]])}
 <p class="muted">${esc(c.token_programs_independent ?? '')}</p>
 ${(c.mint_findings ?? []).length ? `<div class="table-wrap"><table class="table"><thead><tr><th>Side</th><th>Extension</th><th>Operation</th><th>Support</th><th>Semantics</th></tr></thead><tbody>${c.mint_findings.map(finding).join('')}</tbody></table></div>` : '<p class="muted">Neither mint carries a Token-2022 extension.</p>'}
 ${(c.account_findings ?? []).length ? `<div class="tech-only"><h3 class="subhead">Account extensions.</h3><div class="table-wrap"><table class="table"><thead><tr><th>Side</th><th>Code</th><th>Support</th><th class="num">Accounts</th></tr></thead><tbody>${c.account_findings.map(f => `<tr><td>${esc(f.side)}</td><td><code>${esc(f.code)}</code></td><td>${esc(words(f.support))}</td><td class="num">${count(f.accounts)}</td></tr>`).join('')}</tbody></table></div></div>` : ''}
 <details class="tech-only"><summary>Support matrix (${esc(c.matrix_version ?? '')})</summary><ul class="limits">${(c.support_matrix ?? []).map(r => `<li><strong>${esc(r.extension)}</strong>: ${esc(r.classification)}</li>`).join('')}</ul></details>`;
}

function authority(a) {
 if (!a) return empty('No authority section recorded.');
 return `${kv([
  ['Migration authority', `${esc(words(a.migration_authority?.kind))} ${ident(a.migration_authority?.address, 'addr')}`],
  ['Fee payer', esc(a.fee_payer_role ?? 'relayer')],
  ['Key possession', esc(a.key_possession ?? 'Unknown')],
  ['Program-controlled owners', count(a.program_controlled_owners)],
 ])}
 ${(a.external_capabilities ?? []).length ? `<h3 class="subhead">External capabilities this migration needs.</h3><ul class="limits">${a.external_capabilities.map(c => `<li>${esc(c)}</li>`).join('')}</ul>` : ''}
 <div class="grid-2"><div><h3 class="subhead">Owners of positive holders.</h3>${kv(Object.entries(a.owner_classes_positive_holders ?? {}).map(([k, v]) => [words(k), count(v)]))}</div>
 <div><h3 class="subhead">Signer roles for migratable holders.</h3>${kv(Object.entries(a.required_signer_roles_migratable_units ?? {}).map(([k, v]) => [words(k), count(v)]))}</div></div>
 ${(a.expectations ?? []).length ? `<div class="tech-only"><h3 class="subhead">Declared authority expectations.</h3>${kv(a.expectations.map(e => [e.check, `${pill(e.satisfied ? 'Satisfied' : 'Violated')} expected <code>${esc(e.expected)}</code> · captured <code>${esc(e.observed)}</code>`]))}</div>` : ''}`;
}

function stressTable(execution, coverage) {
 const cases = execution?.stress_cases ?? [];
 if (!cases.length) return empty('No stress case executed.');
 return `<p>${count(coverage?.stress?.behaving)} of ${count(coverage?.stress?.executed)} frozen cases behave as specified. ${esc(coverage?.stress?.rule ?? '')}</p>
 <div class="table-wrap"><table class="table"><thead><tr><th>Case</th><th>Provenance</th><th>Specification</th><th>Candidate</th><th class="tech-only">Error</th><th>Verdict</th></tr></thead><tbody>${cases.map(c => `<tr class="${c.behaves_as_specified ? '' : 'is-changed'}"><td>${esc(sentence(c.kind))}<code class="tech-only"> ${esc(c.case_id)}</code></td><td>${provenanceTag(c.provenance)}</td><td>${esc(c.expected === 'Migrate' ? 'must migrate' : 'must reject')}</td><td>${esc(words(c.outcome))}</td><td class="tech-only"><code>${esc(c.error_name ?? '')}</code></td><td>${c.behaves_as_specified ? pill('Satisfied', 'as specified') : pill('Violated', words(c.finding))}</td></tr>`).join('')}</tbody></table></div>
 ${(coverage?.stress?.not_reachable ?? []).length ? `<details><summary>${count(coverage.stress.not_reachable.length)} cases are unreachable and were not fabricated</summary><ul class="limits">${coverage.stress.not_reachable.map(n => `<li><strong>${esc(sentence(n.kind))}</strong>: ${esc(n.reason)}</li>`).join('')}</ul></details>` : ''}`;
}

function rehearsal(execution, coverage) {
 const pop = execution?.population_rehearsal ?? {};
 const failures = pop.not_migrated ?? [];
 return `${kv([
  ['Mode', esc(pop.mode ?? '')],
  ['Attempted / migrated', `${count(coverage?.rehearsal?.attempted)} / ${count(coverage?.rehearsal?.migrated)}`],
  ['Rejected / mismatched', `${count(coverage?.rehearsal?.rejected)} / ${count(coverage?.rehearsal?.mismatched)}`],
  ['Budget', `${count(coverage?.rehearsal?.max_units)} units${coverage?.rehearsal?.truncated ? ' · <strong class="text-warn">truncated</strong>' : ''}`],
  ['VM', `${esc(execution?.vm?.runtime ?? '')}`, 'tech-only'],
  ['Signatures', esc(execution?.vm?.signature_verification ?? ''), 'tech-only'],
  ['Clock', `${esc(words(execution?.vm?.clock?.basis))} · slot ${raw(execution?.vm?.clock?.clock?.slot)} · unix ${esc(execution?.vm?.clock?.clock?.unix_timestamp ?? '')}`],
  ['Compute units (max)', count(pop.compute_units_max), 'tech-only'],
 ])}
 ${failures.length ? `<h3 class="subhead">Attempted holders that did not migrate.</h3><div class="table-wrap"><table class="table"><thead><tr><th>Source account</th><th class="num">Amount</th><th>Outcome</th><th>Error</th></tr></thead><tbody>${failures.map(f => `<tr><td>${ident(f.source_account, 'addr')}</td><td class="num">${raw(f.amount_raw)}</td><td>${esc(words(f.outcome))}</td><td><code>${esc(f.failure?.error_name ?? f.failure?.error ?? (f.failed_checks ?? []).map(c => c.check ?? c.name ?? '').join(', '))}</code></td></tr>`).join('')}</tbody></table></div>` : ''}
 <div class="tech-only"><h3 class="subhead">Loaded executables.</h3><div class="table-wrap"><table class="table"><thead><tr><th>Program</th><th>Loader</th><th>ELF SHA-256</th><th>Origin</th></tr></thead><tbody>${(execution?.vm?.programs ?? []).map(p => `<tr><td><code>${esc(p.program_id)}</code></td><td><code>${esc(p.loader)}</code></td><td><code>${short(p.elf_sha256, 16)}</code></td><td>${esc(words(p.origin))}</td></tr>`).join('')}</tbody></table></div></div>`;
}

function search(detail) {
 const s = detail.search_detail;
 if (!s) return `${empty('No bounded counterexample search is recorded for this run.')}${commandLine(`eplyx migration search --run ${detail.id}`)}`;
 return `<p><strong>${esc(s.conclusion)}</strong></p>
 ${kv([
  ['Dimensions explored', (s.explored_dimensions ?? []).map(d => `<span class="tag">${esc(sentence(d))}</span>`).join(' ')],
  ['Derived VM probes', `${count(s.budget?.probes)} of ${count(s.budget?.max_probes)} · minimization ${count(s.budget?.minimization)} of ${count(s.budget?.max_minimization)} · population rehearsals ${count(s.budget?.population_rehearsals)}`],
  ['Stopping condition', esc(s.stopping_condition ?? '')],
 ])}
 ${(s.counterexamples ?? []).length ? `<div class="cx-list">${s.counterexamples.map(c => `<a class="cx-row cx-row--${c.kind === 'Derived' ? 'derived' : 'observed'}" href="${BASE}/counterexamples/${esc(c.id)}" data-link><span class="cx-row__kind">${esc(c.kind.toUpperCase())}</span><span class="cx-row__main"><strong>${esc(sentence(c.dimension ?? 'Population rehearsal'))}</strong><span class="muted">${esc(findingText(c.finding))}</span></span><span class="cx-row__detail">${c.derived_value_raw ? `value ${raw(c.derived_value_raw)}` : c.observed_amount_raw ? `${raw(c.observed_amount_raw)} raw` : ''}</span><span class="cx-row__failure"><code>${esc(c.failure?.error_name ?? words(c.failure?.outcome ?? ''))}</code></span><span class="cx-row__id"><code>${short(c.id, 12)}</code>${c.saved ? '' : ' <span class="tag tag--soft">not saved</span>'}</span></a>`).join('')}</div>` : ''}
 <details class="tech-only"><summary>Declared domains (${count((s.domains ?? []).length)})</summary><ul class="limits">${(s.domains ?? []).map(d => `<li><strong>${esc(sentence(d.dimension))}</strong>: ${esc(d.description)}${d.low_raw ? ` <code>${raw(d.low_raw)}…${raw(d.high_raw)}</code>` : ''}</li>`).join('')}</ul></details>
 <p class="note">A bounded search with no finding states only its recorded domains and budget; it does not show that no counterexample exists.</p>`;
}

function crossCheck(c) {
 if (!c) return '<p class="muted">This run predates the unsigned-plan cross-check.</p>';
 const all = c.executed_units === c.matching_units;
 return `<p>${pill(all ? 'Verified' : 'Failed', all ? 'Descriptors reproduce the rehearsal' : 'Descriptors diverge from the rehearsal')} ${count(c.matching_units)} of ${count(c.executed_units)} rehearsed holders re-executed identically from the plan alone in a separate local VM.</p><p class="muted tech-only">${esc(c.method ?? '')}</p>`;
}

function windowText(w) {
 const edge = b => b ? `${b.kind === 'slot' ? 'slot' : 'unix time'} ${esc(b.value)}` : null;
 const from = edge(w?.activation), to = edge(w?.deadline);
 if (!from && !to) return 'Always open';
 return `${from ? `from ${from} (inclusive)` : 'open now'}${to ? ` · until ${to} (exclusive)` : ' · no deadline'}`;
}

export function migrationRunDetail(detail, h) {
 const m = detail.migration_detail ?? {};
 const change = detail.release?.change ?? {};
 const conversion = change.conversion ?? {};
 const sections = [['answers', 'Answers'], ['migration', 'Migration'], ['impact', 'Who is affected'], ['accounting', 'Accounting'], ['compatibility', 'Token programs'], ['authority', 'Authorities'], ['execution', 'Rehearsal'], ['stress', 'Stress'], ['search', 'Search'], ['invariants', 'Invariants'], ['gate', 'Gate'], ['evidence', 'Evidence']];
 const html = `
 <div class="page-head page-head--run">
  <div><span class="eyebrow">Run #${esc(detail.number)} · Token Migration V1 rehearsal</span><h1>${esc(h.GATE[detail.gate?.outcome] ?? 'No gate result')}.</h1>
  <p class="muted">${h.gateSentence(detail.gate?.outcome, detail.gate?.policy)}</p>
  <p class="meta"><code>${esc(detail.id)}</code> ${h.copy(detail.id)} · ${esc(words(m.state?.world_kind))} (${esc(m.state?.cluster ?? '')})</p>${h.syncedLine(detail)}</div>
  <div class="page-head__actions">${gatePill(detail.gate?.outcome, 'lg')}${detail.previous_run ? `<a class="button button--ghost" href="${BASE}/compare?left=${esc(detail.previous_run)}&right=${esc(detail.id)}" data-link>Compare with previous run</a>` : ''}</div>
 </div>
 <div class="alert alert--info"><strong>Local rehearsal only.</strong> Nothing was signed or sent and no funds moved. Official transition: ${esc(words(detail.official_transition ?? 'NotTested'))}. Signatures were assumed locally; whether anyone holds those keys is unknown.</div>
 ${detail.problems?.length ? `<div class="alert"><strong>${esc(words(detail.state))} run.</strong><ul>${detail.problems.map(p => `<li>${esc(p)}</li>`).join('')}</ul></div>` : ''}
 <nav class="subnav" aria-label="Run sections">${sections.map(([id, label], n) => `<a href="#${id}"><span>${String.fromCharCode(65 + n)}</span>${label}</a>`).join('')}</nav>
 ${panel({ id:'answers', eyebrow:'A', title:'The questions this run answers', body:answers(detail) })}
 ${panel({ id:'migration', eyebrow:'B', title:'Proposed migration', body:kv([
  ['Source', side(detail.migration?.source)],
  ['Destination', side(detail.migration?.destination)],
  ['Terms', `${esc(conversion.numerator ?? '?')} : ${esc(conversion.denominator ?? '?')} (${esc(conversion.ratio_basis ?? '')} basis) · ${esc(conversion.rounding ?? '')} · fee ${esc(conversion.fee?.kind === 'source_bps' ? `${conversion.fee.bps} bps` : 'none')} · min output ${raw(conversion.minimum_output_raw)}`],
  ['Source leaves by', esc({ burn:'burning the source tokens', escrow:'moving the source tokens into escrow' }[detail.migration?.disposition] ?? words(detail.migration?.disposition))],
  ['Destination funded by', esc({ reserveTransfer:'a transfer from the destination reserve', mintTo:'minting new destination tokens' }[detail.migration?.funding] ?? words(detail.migration?.funding))],
  ['Window', windowText(m.migration?.window)],
  ['Effective raw terms', `<code>${esc(JSON.stringify(m.migration?.conversion?.effective_raw_terms ?? {}))}</code>`, 'tech-only'],
  ['Candidate program', ident(detail.candidate_program_sha256)],
  ['Analytical input', ident(detail.analysis_input_sha256)],
  ['Program ID', `<code>${esc(detail.release?.program_id ?? '—')}</code>`, 'tech-only'],
  ['Adapter', `<code>${esc(detail.release?.adapter ?? '')}</code>`, 'tech-only'],
  ['Migration authority', ident(m.migration?.overlay?.migration_authority, 'addr'), 'tech-only'],
  ['Config account', ident(m.migration?.overlay?.config, 'addr'), 'tech-only'],
  ['Reserve vault', `${ident(m.migration?.overlay?.reserve_vault, 'addr')} <span class="muted">${esc(words(m.migration?.overlay?.reserve_origin ?? ''))}</span>`, 'tech-only'],
  ['Provenance', `${esc(words(detail.release?.provenance))} · ${esc(words(detail.release?.deployment_origin))}`, 'tech-only'],
 ]) })}
 ${panel({ id:'impact', eyebrow:'C', title:'Who is affected', body:`${kv([['Token accounts', count(m.impact?.population?.token_accounts)], ['With a balance', count(m.impact?.population?.positive_balance_accounts)], ['Enumeration', pill(m.impact?.population?.enumeration)], ['Undecodable rows', count(m.impact?.population?.undecoded_rows), 'tech-only']])}${classTable(m.impact)}${stranded(m.impact)}` })}
 ${panel({ id:'accounting', eyebrow:'D', title:'Source → destination accounting', body:accounting(m.reconciliation) })}
 ${panel({ id:'compatibility', eyebrow:'E', title:'Token programs and extensions', body:compatibility(m.compatibility) })}
 ${panel({ id:'authority', eyebrow:'F', title:'Authorities and signers', body:authority(m.authority) })}
 ${panel({ id:'execution', eyebrow:'G', title:'Sequential population rehearsal', body:rehearsal(m.execution, m.coverage) })}
 ${panel({ id:'stress', eyebrow:'H', title:'Stress matrix', body:stressTable(m.execution, m.coverage) })}
 ${panel({ id:'search', eyebrow:'I', title:'Counterexample search', body:search(detail) })}
 ${panel({ id:'invariants', eyebrow:'J', title:'Invariants', body:h.invariantList(detail.invariant_results, detail.release?.invariant_definitions) })}
 ${panel({ id:'gate', eyebrow:'K', title:'Deployment gate', body:`${h.gateBlock(detail)}${codes(detail.gate_detail?.saved?.reason_codes)}` })}
 ${panel({ id:'evidence', eyebrow:'L', title:'Evidence, replay and the unsigned plan', body:`<p>${count(m.unsigned_plan?.units)} holders in the <strong>unsigned</strong> execution plan: instruction descriptors and signer roles only, with no keys, signatures or blockhash. Nothing was submitted.</p>${crossCheck(m.unsigned_plan?.cross_check)}${h.evidenceBlock(detail)}` })}
 <p class="note">${(m.limitations ?? []).map(esc).join(' ')}</p>`;
 return { title:`Run #${detail.number}`, crumbs:[['Runs', '/runs'], [`#${detail.number}`]], html };
}

export function migrationProduction(detail) {
 const m = detail.migration_detail ?? {};
 const pop = m.impact?.population ?? {};
 const world = m.coverage?.world ?? {};
 return `<div class="tiles">
  ${tile({ label:'Token accounts', value:count(pop.token_accounts) })}
  ${tile({ label:'With a balance', value:count(pop.positive_balance_accounts) })}
  ${tile({ label:'Enumeration', value:pill(pop.enumeration) })}
  ${tile({ label:'State', value:esc(words(world.kind)), sub:world.observed_slots ? `finalized slots ${raw(world.observed_slots[0])}–${raw(world.observed_slots[1])}` : 'no chain state (fixture)' })}
  ${tile({ label:'Destinations not inspected', value:count(m.coverage?.not_inspected?.destinations), status:m.coverage?.not_inspected?.destinations ? 'NotTested' : '' })}
  ${tile({ label:'Owners not inspected', value:count(m.coverage?.not_inspected?.owners), status:m.coverage?.not_inspected?.owners ? 'NotTested' : '' })}
 </div>
 ${panel({ title:'Holders by class', body:`${classTable(m.impact)}${stranded(m.impact)}` })}
 ${panel({ cls:'tech-only', title:'World provenance', body:`${kv(Object.entries(world).map(([k, v]) => [sentence(k), typeof v === 'object' ? `<code>${esc(JSON.stringify(v))}</code>` : esc(v)]))}<ul class="limits">${(m.state?.limitations ?? []).map(l => `<li>${esc(l)}</li>`).join('')}</ul>` })}`;
}

export function migrationWhy(cx) {
 const base = findingText(cx.finding);
 const where = cx.kind === 'Derived'
  ? `Eplyx derived this state from ${cx.provenance === 'DerivedFromObserved' ? 'captured' : 'fixture'} state along the ${words(cx.dimension).toLowerCase()} dimension${cx.derived_value_raw ? ` (value ${raw(cx.derived_value_raw)})` : ''}. It is a local state, not an observed mainnet failure.`
  : `The sequential population rehearsal executed this exact ${cx.provenance === 'ObservedPopulationRehearsal' ? 'captured' : 'fixture'} holder (${raw(cx.observed_amount_raw)} raw) in the local VM${cx.provenance === 'ObservedPopulationRehearsal' ? '' : '; a fixture is not chain state'}.`;
 return `${esc(base)} ${esc(where)}`;
}

export const migrationBoundary = cx => {
 const b = cx.boundary ?? {};
 if (!cx.derived_value_raw) return '';
 return b.last_behaving_value_raw ? `${raw(cx.derived_value_raw)} deviates · ${raw(b.last_behaving_value_raw)} behaves` : `${raw(cx.derived_value_raw)} deviates · no behaving value recorded`;
};
export const migrationFailure = f => f?.error_name ?? f?.error ?? (f?.outcome ? words(f.outcome) : '');

function banner(cx) {
 const fixture = /Synthetic/.test(cx.provenance ?? '');
 if (cx.kind === 'Derived') return `<div class="kind kind--derived"><strong>DERIVED COUNTEREXAMPLE</strong><span>Eplyx built this state from ${fixture ? 'a synthetic fixture' : 'captured state'} by a typed local change. It is not an observed mainnet failure.</span></div>`;
 return fixture
  ? '<div class="kind kind--observed"><strong>FIXTURE COUNTEREXAMPLE</strong><span>This exact fixture holder deviated in the sequential rehearsal. A fixture is not chain state.</span></div>'
  : '<div class="kind kind--observed"><strong>OBSERVED COUNTEREXAMPLE</strong><span>This exact captured holder deviated in the sequential population rehearsal.</span></div>';
}

export function migrationCounterexampleDetail(cx, h) {
 const parent = cx.parent_summary;
 const derived = cx.kind === 'Derived';
 const b = cx.boundary ?? {};
 const inputs = cx.reproduction_inputs ?? {};
 const html = `
 <div class="cx-hero cx-hero--${derived ? 'derived' : 'observed'}">
  ${banner(cx)}
  <span class="eyebrow">Token Migration V1 counterexample</span>
  <h1><code>${esc(cx.id)}</code> ${h.copy(cx.id)}</h1>
  <p class="meta">${esc(cx.claim ?? '')} · from run ${parent ? h.runLink(parent) : `<code>${esc(cx.parent_run)}</code>`}</p>
 </div>
 ${cx.state !== 'Valid' ? '<div class="alert"><strong>Identity check failed.</strong> The saved file\'s ID does not match its content, so <code>eplyx migration reproduce</code> will refuse it.</div>' : ''}
 <div class="grid-2">
  ${panel({ title:'What deviated', body:kv([
   ['Source account', ident(cx.account, 'addr')],
   !derived && ['Balance', `${raw(cx.observed_amount_raw)} <span class="muted">raw units</span>`],
   ['Specification', cx.expected === 'Reject' ? 'must reject this migration' : 'must migrate this holder'],
   ['Finding', esc(findingText(cx.finding))],
   derived && ['Dimension', esc(sentence(cx.dimension))],
   derived && ['Deviating value', `<code>${raw(cx.derived_value_raw)}</code>`],
   derived && ['Last behaving value', b.last_behaving_value_raw ? `<code>${raw(b.last_behaving_value_raw)}</code>` : '<span class="muted">none recorded</span>'],
   derived && ['Minimized', cx.minimized ? 'yes' : 'no'],
   ['Candidate outcome', `${esc(words(cx.failure?.outcome ?? ''))} ${cx.failure?.error_name ? `<code>${esc(cx.failure.error_name)}</code>` : ''}`],
   (cx.failure?.failed_checks ?? []).length && ['Failed checks', cx.failure.failed_checks.map(c => esc(c)).join('<br>')],
   ['Rollback', cx.failure?.rollback_verified === true ? pill('Verified', 'Verified') : cx.failure?.rollback_verified === false ? pill('Failed', 'Changed state') : '<span class="muted">not applicable</span>'],
   ['Provenance', provenanceTag(cx.provenance)],
   ['Candidate', ident(cx.candidate_program_sha256)],
  ]) })}
  ${panel({ title:'Why this matters', body:`<p class="why">${migrationWhy(cx)}</p><p class="muted">${esc(cx.limitations ?? '')}</p>` })}
 </div>
 ${panel({ eyebrow:'Developer action', title:'Reproduce locally', body:`${commandLine(cx.reproduce)}<p class="muted">Re-executes the saved run and search in the local VM with the RPC environment removed, then checks this counterexample and its failure signature. The dashboard does not run it.</p>${h.reproductionHistory(cx.reproductions)}` })}
 ${panel({ cls:'tech-only', title:'Reproduction inputs and identity', body:`${kv([
  ['Mutations', (inputs.mutations ?? []).length ? inputs.mutations.map(m => `<code>${esc(JSON.stringify(m))}</code>`).join('<br>') : '<span class="muted">none (exact state)</span>'],
  ['Reserve override', inputs.reserve_override_raw ? `<code>${raw(inputs.reserve_override_raw)}</code>` : '—'],
  ['Transaction variant', inputs.transaction_variant ? `<code>${esc(JSON.stringify(inputs.transaction_variant))}</code>` : '—'],
  ['Failure stage', esc(cx.failure?.stage ?? '—')],
  ['World', `<code>${esc(cx.world_sha256 ?? '')}</code>`],
  ['Package', `<code>${esc(cx.analysis_input_sha256 ?? '')}</code>`],
  ['Search SHA-256', `<code>${esc(cx.search_sha256 ?? '')}</code>`],
  ['Search artifact matches', cx.search_artifact_matches ? pill('Verified', 'Matches saved search') : pill('Failed', 'Does not match')],
  ['Replay inputs', cx.replay_inputs ? Object.values(cx.replay_inputs).map(p => `<code>${esc(p)}</code>`).join('<br>') : '—'],
 ])}<p><a href="${API}/counterexamples/${esc(cx.id)}/raw">Download saved JSON</a></p>` })}`;
 return { title:'Counterexample', crumbs:[['Counterexamples', '/counterexamples'], [short(cx.id, 14)]], html };
}
