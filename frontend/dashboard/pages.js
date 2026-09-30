import { analyticalTiles, analyticalDetail, analyticalOverview, analyticalCompare, analyticalCells } from './analytical.js';
// Dashboard pages. Every status rendered here comes from the local API, which
// copies engine artifacts or runs the engine's own gate evaluator. Pages only
// choose wording, filter loaded rows and lay things out.
import {
 esc, pill, gatePill, tone, words, sentence, raw, count, short, addr, ident, copy, ago, when, exact,
 utc, kv, panel, empty, commandLine, tile, meter, invariantName, scopeText, GATE, GATE_SHORT, isRaw,
} from './ui.js';
import { BASE, API, CLOUD, DEMO } from './env.js';
import {
 isMigration, migrationTiles, migrationRunCells, migrationRunDetail, migrationProduction, migrationWhy,
 migrationCounterexampleDetail, migrationBoundary, migrationFailure,
} from './migration.js';

// Pages name local API paths; a hosted workspace maps them onto its view API.
export async function api(path) {
 const response = await fetch(path.replace(/^\/api(?=\/)/, API), { headers: { Accept: 'application/json' }, credentials:'same-origin' });
 if (response.status === 401 && CLOUD && !DEMO) { location.assign(`/login?next=${encodeURIComponent(location.pathname + location.search)}`); throw new Error('Sign in to view this workspace.'); }
 const body = await response.json().catch(() => ({}));
 if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
 return body;
}

const SOURCES = { local:'Local CLI', ci:'CI', imported:'Imported', hosted:'Hosted' };
const sourceTag = source => source ? `<span class="tag">${esc(SOURCES[source] ?? source)}</span>` : '<span class="tag tag--soft" title="Recorded before run_source existed">Not recorded</span>';
const runLabel = run => run ? `Run #${run.number ?? '?'}` : 'Run';
const runLink = run => `<a href="${BASE}/runs/${esc(run.id)}" data-link class="runref"><strong>#${esc(run.number ?? '?')}</strong><code class="tech-only">${esc(run.id)}</code></a>`;
const cxLink = id => `<a href="${BASE}/counterexamples/${esc(id)}" data-link><code>${esc(id)}</code></a>`;
// Where a hosted result came from. Synced results are copies of local or CI
// engine output; viewing them never reruns RPC, execution or replay.
const SYNC_VIA = { cli:'the Eplyx CLI', ci:'a CI token' };
function syncedLine(run) {
 if (!CLOUD || !run?.synced) return '';
 const source = run.run_source === 'ci' ? 'CI run' : run.run_source === 'local' ? 'Local run' : 'Run (source not recorded)';
 return `<p class="synced-line"><span class="tag tag--synced">Synced result</span> ${esc(source)} · ${esc(when(run.timestamp))}${run.git?.commit ? ` · commit <code>${esc(run.git.commit.slice(0, 7))}</code>` : ''} · synced by ${esc(run.synced.by)} via ${esc(SYNC_VIA[run.synced.via] ?? run.synced.via)} ${esc(ago(run.synced.at))}</p>`;
}
// Local dashboard only: whether a run has been synced to a linked workspace.
function localSyncTag(run) {
 if (CLOUD || !run?.sync) return '';
 if (run.sync.status === 'synced') return '<span class="tag" title="Copied to the linked Eplyx cloud project">synced</span>';
 return `<span class="tag tag--soft" title="${esc(run.sync.error ?? 'Last sync attempt failed')}">${run.sync.last_synced_at ? 'synced · last retry failed' : 'sync failed'}</span>`;
}
const STRESS_ORDER = ['Proven', 'Failed', 'Indeterminate', 'Unsupported', 'NotTested', 'NotApplicable'];
const stressParts = counts => STRESS_ORDER.map(status => [status, Number(counts?.[status] || 0)]);
const proven = run => Number((isMigration(run) ? run?.stress?.behaving : run?.stress?.counts?.Proven) || 0);
const selected = run => run?.stress?.selected ?? 0;
const cxTotal = run => run?.search?.state === 'Recorded' ? run.search.total : null;

function gateSentence(outcome, policy) {
 const under = policy ? ` under the ${esc(policy)} policy` : '';
 if (outcome === 'Pass') return `The deployment gate passed${under} with no warnings.`;
 if (outcome === 'Warn') return `The deployment gate did not block this candidate${under}, but recorded warnings that need review before release.`;
 if (outcome === 'Block') return `The deployment gate blocked this candidate${under}.`;
 return 'No deployment gate result is recorded for this run.';
}

function counterexampleTileValue(run) {
 if (!run.search) return { value: '—', sub: 'No search recorded · run <code>eplyx migration search</code>' };
 if (run.search.state !== 'Recorded') return { value: '—', sub: 'Search artifact unreadable' };
 const total = run.search.total;
 return total
  ? { value: count(total), sub: `${count(run.search.observed)} observed · ${count(run.search.derived)} derived`, status: 'Failed' }
  : { value: '0', sub: 'found in the latest bounded search', status: 'Proven' };
}

function invariantCounts(run) {
 const counts = run.invariants?.counts;
 if (!run.invariants?.total) return 'None declared';
 return ['Satisfied', 'Indeterminate', 'Violated', 'NotApplicable'].filter(s => counts?.[s]).map(s => `${counts[s]} ${words(s).toLowerCase()}`).join(' · ');
}

function statusTiles(run) { return isMigration(run) ? migrationTiles(run) : analyticalTiles(run); }

function runsTable(runs, { selectable = false, selectedIds = [] } = {}) {
 if (!runs.length) return empty('No runs match these filters.');
 return `<div class="table-wrap"><table class="table runs-table"><thead><tr>${selectable ? '<th class="col-select"><span class="sr-only">Compare</span></th>' : ''}<th>Run</th><th>When</th><th>Commit</th><th class="hide-sm">Branch</th><th class="hide-md">Source</th><th>Result</th><th class="hide-sm">Analysis</th><th>Stress</th><th>Counter&shy;examples</th><th class="hide-md">Candidate</th><th class="tech-only">Input</th></tr></thead><tbody>
 ${runs.map(run => `<tr data-run="${esc(run.id)}" data-gate="${esc(run.gate?.outcome ?? '')}">
  ${selectable ? `<td class="col-select"><input type="checkbox" aria-label="Select run #${esc(run.number)} for comparison" data-compare="${esc(run.id)}" ${selectedIds.includes(run.id) ? 'checked' : ''}></td>` : ''}
  <td>${runLink(run)}${run.state !== 'Complete' ? ` ${pill(run.state)}` : ''}${localSyncTag(run)}</td>
  <td><span title="${esc(run.timestamp ?? '')}">${esc(run.timestamp ? ago(run.timestamp) : '—')}</span></td>
  <td>${run.git?.commit ? `<code>${esc(run.git.commit.slice(0, 7))}</code>${run.git.dirty ? ' <span class="tag" title="Uncommitted changes when this run started">dirty</span>' : ''}` : '<span class="muted">—</span>'}</td>
  <td class="hide-sm">${esc(run.git?.branch ?? '—')}</td>
  <td class="hide-md">${sourceTag(run.run_source)}</td>
  <td>${isMigration(run) ? gatePill(run.gate?.outcome) : pill(run.status ?? run.state)}</td>
  <td class="hide-sm">${isMigration(run) ? migrationRunCells(run).conversion : analyticalCells(run)}</td>
  <td>${isMigration(run) ? migrationRunCells(run).stress : '<span class="muted">Not evaluated</span>'}</td>
  <td>${cxTotal(run) === null ? '<span class="muted" title="No search recorded">—</span>' : `<span class="${cxTotal(run) ? 'text-crit' : ''}">${count(cxTotal(run))}</span>`}</td>
  <td class="hide-md"><code>${short(run.candidate_program_sha256, 8)}</code></td>
  <td class="tech-only"><code>${short(run.analysis_input_sha256, 12)}</code></td>
 </tr>`).join('')}
 </tbody></table></div>`;
}

function boundaryText(cx) {
 if (cx.kind !== 'Derived') return '';
 if (cx.transition_kind === 'token_migration') return migrationBoundary(cx);
 const passing = cx.first_passing_value_raw ?? cx.last_passing_value_raw;
 return passing ? `${raw(cx.derived_value_raw)} fails · ${raw(passing)} passes` : `${raw(cx.derived_value_raw)} fails · no passing value recorded`;
}

function cxRows(list) {
 if (!list.length) return empty('No counterexamples have been found in recorded searches.');
 return `<div class="cx-list">${list.map(cx => `<a class="cx-row cx-row--${cx.kind === 'Derived' ? 'derived' : 'observed'}" href="${BASE}/counterexamples/${esc(cx.id)}" data-link data-kind="${esc(cx.kind ?? '')}">
  <span class="cx-row__kind">${esc(cx.kind ? cx.kind.toUpperCase() : 'UNREADABLE')}</span>
  <span class="cx-row__main"><strong>${cx.kind === 'Derived' ? esc(sentence(cx.dimension)) : cx.transition_kind === 'token_migration' ? 'Rehearsal holder' : 'Exact captured state'}</strong><span class="muted">account ${addr(cx.account)}${cx.parent ? ` · run #${esc(cx.parent.number)}` : ''}</span></span>
  <span class="cx-row__detail">${cx.kind === 'Derived' ? boundaryText(cx) : cx.observed_amount_raw ? `${raw(cx.observed_amount_raw)} raw units` : ''}</span>
  <span class="cx-row__failure"><code>${esc(cx.failure?.instruction_error ?? (cx.transition_kind === 'token_migration' ? migrationFailure(cx.failure) : null) ?? cx.problems?.[0] ?? '')}</code></span>
  <span class="cx-row__id"><code>${short(cx.id, 12)}</code>${cx.reproductions?.count ? `<span class="tag" title="Last reproduced ${esc(ago(cx.reproductions.last_timestamp))}">reproduced ×${esc(cx.reproductions.count)}</span>` : ''}</span>
 </a>`).join('')}</div>`;
}

// ---------------------------------------------------------------- Overview

function firstRunGuide(project) {
 const config = project.context?.config ?? {};
 if (CLOUD) return `<section class="welcome">
  <span class="eyebrow">${esc(project.project?.name ?? 'This project')}</span>
  <h1>No synced runs yet.</h1>
  <p>Locally produced analyses appear here when synced. Runs appear here after <code>eplyx sync</code> copies their results to this project.</p>
  <ol class="steps">
   <li>${commandLine('eplyx login', 'Sign in from the CLI (browser approval)')}</li>
   <li>${commandLine(`eplyx link --project ${project.project?.id ?? 'proj_…'}`, 'Link your local project')}</li>
   <li>${commandLine('eplyx migration analyse', 'Run the analysis locally')}</li>
   <li>${commandLine('eplyx sync', 'Upload run metadata and results')}</li>
  </ol>
 </section>`;
 return `<section class="welcome">
  <span class="eyebrow">${esc(project.project?.name ?? 'This project')}</span>
  <h1>No assurance runs yet.</h1>
  <p>Run <code>eplyx migration analyse</code> to create your first assurance run. Runs are saved under <code>.eplyx/</code> in this project and appear here automatically.</p>
  <ol class="steps">
   ${config.state !== 'Valid' ? `<li>${commandLine('eplyx init --migration', 'Create eplyx.toml')}<p class="muted">${config.state === 'Invalid' ? `eplyx.toml is invalid: ${esc(config.error ?? '')}` : 'No eplyx.toml was found in this project.'}</p></li>` : ''}
   <li>${commandLine('eplyx doctor', 'Check config, candidate and RPC')}</li>
   <li>${commandLine('eplyx migration analyse', 'Rehearse the proposal against its declared state input')}</li>
   <li>${commandLine('eplyx migration search', 'Search for counterexamples')}</li>
  </ol>
 </section>`;
}

function changeList(comparison) {
 const inputs = comparison.inputs.filter(f => f.changed && !/hash/i.test(f.label));
 const hashes = comparison.inputs.filter(f => f.changed && /hash/i.test(f.label));
 const results = comparison.results.filter(f => f.changed);
 const inv = comparison.invariants.filter(i => i.changed);
 const cx = comparison.counterexamples;
 const rows = [];
 if (comparison.gate.changed) rows.push(['Gate', `${gatePill(comparison.gate.left.outcome)} → ${gatePill(comparison.gate.right.outcome)}`]);
 rows.push(['Candidate binary', comparison.inputs[0].changed ? '<span class="text-warn">changed</span>' : 'unchanged']);
 for (const f of inputs.slice(0, 4)) rows.push([f.label, `${value(f.left)} → ${value(f.right)}`]);
 if (hashes.length && !inputs.length) rows.push(['Input identity', `${hashes.length} hash${hashes.length === 1 ? '' : 'es'} changed`]);
 for (const f of results.filter(f => !['Token accounts observed', 'Positive balances'].includes(f.label)).slice(0, 4)) rows.push([f.label, `${value(f.left)} → ${value(f.right)}`]);
 if (cx.left_total !== cx.right_total) rows.push(['Counterexamples', `${count(cx.left_total)} → ${count(cx.right_total)} ${cx.comparable ? '<span class="badge badge--ok">Equivalent search</span>' : '<span class="badge badge--warn" title="Counterexample disappearance does not prove resolution">Search domains differ</span>'}`]);
 for (const i of inv.slice(0, 3)) rows.push([invariantName(i.invariant_type), `${pill(i.left)} → ${pill(i.right)}`]);
 return rows.length > 1 || comparison.inputs[0].changed ? kv(rows) : '<p class="muted">No declared input or result changed between these runs.</p>';
}

function value(v) {
 if (v === null || v === undefined) return '<span class="muted">none</span>';
 if (typeof v === 'object') {
  if(v.reserve?.funded_raw != null) return `${raw(v.reserve.funded_raw)} raw (${esc(words(v.kind))})`;
  if(Array.isArray(v) && v.length && v.every(c=>c.class && c.accounts!=null)) return v.map(c=>`${count(c.accounts)} ${esc(words(c.class))}`).join(' · ');
  return `<span class="ov-only">Recorded structured values</span><code class="tech-only">${esc(JSON.stringify(v))}</code>`;
 }
 if (typeof v === 'boolean') return v ? 'yes' : 'no';
 if (isRaw(v)) return `<code class="nowrap">${raw(v)}</code>`;
 if (/^[0-9a-f]{64}$/.test(v)) return ident(v);
 if (/^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(v)) return ident(v, 'addr');
 return /^[A-Z][A-Za-z]+$/.test(v) ? pill(v) : esc(v);
}

export async function overview({ project }) {
 if (!project.stats?.runs) return { title:'Overview', html:firstRunGuide(project) };
 const latest = project.latest;
 if (!latest) return { title:'Overview', html:`${firstRunGuide(project)}${panel({ title:'Unfinished runs', body:runsTable(project.recent_runs) })}` };
 const detail = await api(`/api/runs/${latest.id}`);
 if (!isMigration(detail)) return analyticalOverview(detail, project, { runsTable, evidenceBlock });
 let comparison = null;
 if (detail.previous_run) comparison = await api(`/api/compare?left=${detail.previous_run}&right=${latest.id}`).catch(() => null);
 const reasons = detail.gate_detail?.saved?.reasons ?? [];
 const limitations = [
  ...(detail.stress_detail?.limitations ?? []).filter(l => l !== 'No funds moved'),
  `Official transition: ${words(latest.official_transition ?? 'NotTested')}. Issuer authorization, key possession and population-wide readiness are separate questions.`,
  isMigration(latest)
   ? `Population coverage is ${words(latest.readiness?.population ?? 'unknown')}; funding is ${words(latest.readiness?.funding ?? 'unknown')}. A local rehearsal never executed a mainnet migration.`
   : `Population rollout readiness is ${words(latest.readiness?.population_rollout ?? 'unknown')}.`,
 ];
 const html = `
 <section class="hero-status hero-status--${tone(latest.gate?.outcome)}">
  <div>
   <span class="eyebrow">Latest analysis · ${esc(runLabel(latest))}</span>
   <h1>${esc(GATE[latest.gate?.outcome] ?? 'Not evaluated')}.</h1>
   <p>${gateSentence(latest.gate?.outcome, latest.gate?.policy)}</p>
   <p class="meta">${esc(ago(latest.timestamp))} · ${latest.git?.commit ? `commit <code>${esc(latest.git.commit.slice(0, 7))}</code>` : 'commit not recorded'}${latest.git?.branch ? ` · ${esc(latest.git.branch)}` : ''} ${exact(latest.timestamp)}</p>
   ${syncedLine(latest)}
  </div>
  <div class="hero-status__actions">
   <a class="button" href="${BASE}/runs/${esc(latest.id)}" data-link>Open run #${esc(latest.number)}</a>
   ${detail.previous_run ? `<a class="button button--ghost" href="${BASE}/compare?left=${esc(detail.previous_run)}&right=${esc(latest.id)}" data-link>Compare with previous</a>` : ''}
  </div>
 </section>
 ${statusTiles(latest, detail)}
 ${CLOUD ? releaseHealth(project) : ''}
 <div class="grid-2">
  ${panel({ eyebrow:'Deployment gate', title:'Why this result', body:reasons.length ? `<ul class="reasons">${reasons.slice(0, 5).map(r => `<li>${esc(r)}</li>`).join('')}</ul>${reasons.length > 5 ? `<a href="${BASE}/gate" data-link>All ${reasons.length} reasons</a>` : ''}` : '<p class="muted">The gate recorded no reasons.</p>' })}
  ${panel({ eyebrow:comparison ? `Run #${esc(comparison.left.number)} → #${esc(comparison.right.number)}` : 'History', title:'What changed recently', body:comparison ? `${changeList(comparison)}<p><a href="${BASE}/compare?left=${esc(detail.previous_run)}&right=${esc(latest.id)}" data-link>Open full comparison</a></p>` : '<p class="muted">This is the only readable run, so there is nothing to compare yet.</p>' })}
 </div>
 ${panel({ title:'Recent runs', body:runsTable(project.recent_runs.slice(0, 5)), actions:`<a href="${BASE}/runs" data-link>All runs</a>` })}
 ${panel({ title:'Recent counterexamples', body:cxRows(project.recent_counterexamples.slice(0, 5)), actions:`<a href="${BASE}/counterexamples" data-link>All counterexamples</a>` })}
 ${panel({ eyebrow:'Scope', title:'Current known limitations', body:`<ul class="limits">${limitations.map(l => `<li>${esc(l)}</li>`).join('')}</ul><p class="muted">${isMigration(latest) && latest.migration?.state === 'SyntheticFixture' ? 'No mainnet funds moved. No chain state was read: this run rehearsed a synthetic fixture in a local VM.' : 'No mainnet funds moved. Capture was read-only; candidate execution happened in a local VM.'}</p>` })}`;
 return { title:'Overview', html };
}

// Latest local and latest CI results side by side. Counts only; there is no
// combined score.
function releaseHealth(project) {
 const c = project.cloud ?? {};
 const cell = (label, r) => tile({ label, value:r ? gatePill(r.gate) : '<span class="muted">none synced</span>', sub:r ? `Run #${esc(r.number)} · ${esc(ago(r.timestamp))}${r.commit ? ` · <code>${esc(String(r.commit).slice(0, 7))}</code>` : ''}${r.branch ? ` · ${esc(r.branch)}` : ''}` : 'No run with this source has been synced', status:r?.gate, href:r ? `${BASE}/runs/${r.id}` : '' });
 const s = project.stats ?? {};
 return `<section class="release-health" aria-label="Latest synced results"><h2 class="subhead">Latest synced results.</h2><div class="tiles tiles--3">
  ${cell('Latest local run', c.latest_local)}
  ${cell('Latest CI run', c.latest_ci)}
  ${tile({ label:'Counterexamples', value:count(s.counterexamples_saved), sub:`${count(c.counterexamples_reproduced)} reproduced · ${count(s.reproductions_succeeded)} of ${count(s.offline_reproductions)} offline replays matched`, href:`${BASE}/counterexamples` })}
 </div><p class="note">${esc(c.synced_note ?? '')}</p></section>`;
}

// -------------------------------------------------------------------- Runs

export async function runs({ query }) {
 const { runs: all } = await api('/api/runs');
 if (!all.length) return { title:'Runs', html:empty('Run `eplyx migration analyse` to create your first assurance run.', 'eplyx migration analyse') };
 const branches = [...new Set(all.map(r => r.git?.branch).filter(Boolean))].sort();
 const html = `
 <div class="page-head"><h1>Runs.</h1><p class="muted">${CLOUD ? `${count(all.length)} synced runs from local and CI Eplyx CLIs, newest first. Each shows the result the engine produced on that machine.` : `${count(all.length)} local runs from <code>.eplyx/runs</code>, newest first.`}</p></div>
 <div class="filters" role="group" aria-label="Filter runs">
  ${[['all', 'All'], ['Pass', GATE.Pass], ['Warn', GATE.Warn], ['Block', GATE.Block], ['cx', 'Has counterexamples']].map(([key, label]) => `<button type="button" class="chip" data-filter="${key}" aria-pressed="false">${label}</button>`).join('')}
  ${branches.length ? `<label class="select"><span>Branch</span><select data-branch><option value="">All branches</option>${branches.map(b => `<option>${esc(b)}</option>`).join('')}</select></label>` : ''}
  <span class="filters__spacer"></span>
  <button type="button" class="button" data-compare-go disabled>Compare selected</button>
 </div>
 <div data-runs-table></div>`;
 return { title:'Runs', html, attach(root) {
  const state = { filter:query.get('filter') || 'all', branch:query.get('branch') || '', picked:[] };
  const render = () => {
   const rows = all.filter(run => (state.filter === 'all' || (state.filter === 'cx' ? (cxTotal(run) || run.saved_counterexamples?.total) : run.gate?.outcome === state.filter)) && (!state.branch || run.git?.branch === state.branch));
   root.querySelector('[data-runs-table]').innerHTML = runsTable(rows, { selectable:true, selectedIds:state.picked });
   root.querySelectorAll('[data-filter]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.filter === state.filter)));
   const go = root.querySelector('[data-compare-go]');
   const sameKind = state.picked.length===2 && all.find(r=>r.id===state.picked[0])?.kind===all.find(r=>r.id===state.picked[1])?.kind;
   go.disabled = !sameKind;
   go.textContent = state.picked.length === 2 ? (sameKind ? 'Compare selected' : 'Select two runs of the same kind') : `Select two runs to compare (${state.picked.length}/2)`;
   const next = new URLSearchParams(); if (state.filter !== 'all') next.set('filter', state.filter); if (state.branch) next.set('branch', state.branch);
   history.replaceState(history.state, '', `${BASE}/runs${next.size ? `?${next}` : ''}`);
  };
  root.addEventListener('click', event => { const b = event.target.closest('[data-filter]'); if (b) { state.filter = b.dataset.filter; render(); } });
  root.querySelector('[data-branch]')?.addEventListener('change', event => { state.branch = event.target.value; render(); });
  const branch = root.querySelector('[data-branch]'); if (branch) branch.value = state.branch;
  root.addEventListener('change', event => {
   const box = event.target.closest('[data-compare]'); if (!box) return;
   state.picked = box.checked ? [...state.picked.filter(id => id !== box.dataset.compare), box.dataset.compare].slice(-2) : state.picked.filter(id => id !== box.dataset.compare);
   render();
  });
  root.querySelector('[data-compare-go]').addEventListener('click', () => {
   const [a, b] = [...state.picked].sort(); // Older run on the left.
   window.dashboardNavigate(`${BASE}/compare?left=${a}&right=${b}`);
  });
  render();
 } };
}

// -------------------------------------------------------------- Run detail

function invariantList(results, definitions) {
 if (!results?.length) return definitions?.length ? '<p class="muted">Invariants are declared but no results were recorded.</p>' : empty('No invariants are declared in this package. Add [[invariants]] to eplyx.toml and run a new analysis.');
 return `<div class="inv-list">${results.map(i => `<article class="inv inv--${tone(i.status)}">
  <header><h3>${esc(invariantName(i.invariant_type))}</h3>${pill(i.status)}<span class="tag tag--${i.severity === 'blocking' ? 'strong' : 'soft'}">${esc(sentence(i.severity))}</span></header>
  <p>${esc(i.explanation)}</p>
  <p class="muted"><strong>Scope.</strong> ${esc(scopeText(i.scope))}</p>
  <div class="tech-only">${kv([['Invariant ID', `<code>${esc(i.invariant_id)}</code>`], ['Type', `<code>${esc(i.invariant_type)}</code>`], ['Scope', `<code>${esc(i.scope)}</code>`], ['Evaluation', `<code>${esc(i.evaluation_version)}</code>`], ['Config', `<code>${esc(JSON.stringify(i.config))}</code>`], ['Evidence', (i.evidence_refs ?? []).map(r => `<code>${esc(r)}</code>`).join('<br>')]])}</div>
 </article>`).join('')}</div>`;
}

function gateBlock(detail) {
 const g = detail.gate_detail ?? {};
 const saved = g.saved ?? {};
 return `<div class="status-line">${gatePill(saved.outcome, 'lg')} <span class="muted">policy <code>${esc(saved.policy ?? '—')}</code> · analytical readiness ${pill(saved.analytical_readiness)}</span></div>
 <ul class="reasons">${(saved.reasons ?? []).map(r => `<li>${esc(r)}</li>`).join('')}</ul>
 <p class="muted">Saved gate ${g.consistent_with_engine === true ? 'matches' : g.consistent_with_engine === false ? '<strong class="text-crit">does not match</strong>' : 'could not be checked against'} the engine gate re-evaluated over the saved report.</p>
 ${policyTable(g)}`;
}

function policyTable(g) {
 return `<div class="table-wrap"><table class="table"><thead><tr><th>Policy</th><th>Analysis gate</th><th>With saved search finding</th></tr></thead><tbody>${(g.policies ?? []).map(p => `<tr><td><code>${esc(p.policy)}</code></td><td>${p.preflight?.error ? `<span class="muted">${esc(p.preflight.error)}</span>` : gatePill(p.preflight?.outcome)}</td><td>${p.with_search == null ? '<span class="muted">no search recorded</span>' : p.with_search.error ? `<span class="muted">${esc(p.with_search.error)}</span>` : gatePill(p.with_search.outcome)}</td></tr>`).join('')}</tbody></table></div><p class="note">${esc(g.basis ?? '')}</p>`;
}

function evidenceBlock(detail) {
 const e = detail.evidence ?? {};
 const hosted = CLOUD && detail.run_source === 'hosted';
 const size = n => n == null ? 'absent' : n > 1048576 ? `${(n / 1048576).toFixed(1)} MB` : n > 1024 ? `${(n / 1024).toFixed(1)} KB` : `${n} B`;
 const primary = ['report.md', 'report.json', 'search.json', 'manifest.json', 'migration-search.json', 'migration.unsigned-plan.json'];
 return `<div class="commands">${(hosted ? [] : e.commands ?? []).map(c => commandLine(c.command, c.label)).join('')}</div>
 ${hosted ? '<p class="note">Hosted inputs and results are retained in MAIN’s immutable store. Viewing them performs no new execution. Provider configuration is not an analytical artifact.</p>' : CLOUD ? '<p class="note">Artifacts stay on the machine that ran Eplyx. This workspace holds only the synced metadata, report, bindings, ChangeSpec and state descriptor, and search result, each with its SHA-256. Candidate bytes, captures and the RPC provider never leave that machine.</p>' : ''}
 ${hosted ? '' : '<p class="note">Commands run from the project root. The dashboard never runs them.</p>'}
 <div class="table-wrap"><table class="table"><thead><tr><th>Artifact</th><th class="tech-only">Path</th><th>Size</th><th></th></tr></thead><tbody>${(e.artifacts ?? []).map(a => `<tr class="${primary.includes(a.name) ? '' : 'tech-only'}"><td>${esc(a.label)}</td><td class="tech-only"><code>${esc(hosted ? a.name : a.path)}</code></td><td class="num">${size(a.size)}</td><td>${hosted ? (['report.json','report.md','change_spec.json'].includes(a.name) && a.size != null ? `<a href="/v1/runs/${esc(detail.id)}/${esc(a.name)}" target="_blank" rel="noopener">Open</a>` : `<span class="muted">${a.size == null ? 'not retained for this run' : 'retained by service'}</span>`) : CLOUD ? (a.size == null ? '' : '<span class="muted">stays local</span>') : a.size == null ? '' : `<a href="${API}/runs/${esc(detail.id)}/artifacts/${esc(a.name)}" target="_blank" rel="noopener">Open</a> · <a href="${API}/runs/${esc(detail.id)}/artifacts/${esc(a.name)}?download=1">Download</a>`}</td></tr>`).join('')}</tbody></table></div>
 <div class="tech-only"><h3 class="subhead">Hashes.</h3>${kv(Object.entries(e.hashes ?? {}).map(([k, v]) => [sentence(k.replace(/_sha256$/, '')), v ? `<code>${esc(v)}</code>` : '<span class="muted">not recorded</span>']))}</div>`;
}

export async function runDetail({ params }) {
 const detail = await api(`/api/runs/${params[0]}`);
 if(CLOUD && !DEMO && detail.kind === "upgrade_parameter_interaction") return (await import("./interaction.js")).resultPage({params});
 if(CLOUD)document.querySelector('link[data-upgrade-style]')?.remove();
 if (isMigration(detail)) {
  const view = migrationRunDetail(detail, { GATE, gateSentence, copy, syncedLine, invariantList, gateBlock, evidenceBlock });
  if (CLOUD && !DEMO) {
   const order = await import('./migration-order.js');
   return order.withOrders(view, detail.id);
  }
  return view;
 }
 if(detail.kind==='program_upgrade' && detail.hosted){
  const {HostedReport}=await import('/assets/main/report.js');
  if(!document.querySelector('link[data-upgrade-style]')){const style=document.createElement('link');style.rel='stylesheet';style.href='/assets/main/styles.css';style.dataset.upgradeStyle='1';document.head.append(style);}
  const run=detail.hosted;const report=run.report_available?await fetch(detail.report_url,{credentials:'same-origin'}).then(r=>{if(!r.ok)throw new Error('Report unavailable');return r.json();}):null;
  const spec=run.change?await fetch(`/v1/runs/${encodeURIComponent(run.run_id)}/change_spec.json`,{credentials:'same-origin'}).then(r=>r.ok?r.json():null):null;
  const extras={spec,runBase:`${BASE}/runs`};
  if(spec?.change?.delivery?.provider==='squads_v4' && run.project_id){
   try { const response=await fetch(`/v1/projects/${encodeURIComponent(run.project_id)}/governance/changes/${encodeURIComponent(run.change.change_spec_id)}/trail`,{credentials:'same-origin'});const body=await response.json();extras.governance=response.ok?{trail:body}:{error:body.error??`HTTP ${response.status}`}; }
   catch {extras.governance={error:'the governance trail could not be fetched'};}
  }
  const {attachGovernanceTrail}=await import('/assets/main/governance.js');
  const view = {title:'Program upgrade',crumbs:[['Runs','/runs'],[run.run_id]],html:HostedReport({...run,canonical_report:report},extras),attach(root){return attachGovernanceTrail(root,path=>fetch(path,{credentials:'same-origin'}));}};
  return CLOUD && !DEMO ? (await import('./interaction.js')).withInteractions(view,run.run_id) : view;
 }
 const view = analyticalDetail(detail, { evidenceBlock });
 if(CLOUD && !DEMO && ['current_path','protocol_parameter_change'].includes(detail.kind)) {
  const parameter=await import('./parameter.js');
  if(detail.kind==='current_path')return parameter.withParameterEntry(view,detail);
  return parameter.withParameterPolling(view,detail,()=>api(`/api/runs/${params[0]}`),fresh=>analyticalDetail(fresh,{evidenceBlock}));
 }
 return view;
}

// ---------------------------------------------------------- Counterexamples

export async function counterexamples({ query }) {
 const { counterexamples: all } = await api('/api/counterexamples');
 const runFilter = query.get('run');
 const html = `
 <div class="page-head"><h1>Counterexamples.</h1><p class="muted">${count(all.length)} ${CLOUD ? 'synced from local and CI searches' : 'saved in <code>.eplyx/counterexamples</code>'}. Rehearsal failures retain their recorded provenance; derived failures are local variants of the recorded captured or synthetic state.</p></div>
 ${all.length ? `<div class="filters" role="group" aria-label="Filter counterexamples">${[['all', 'All'], ['Observed', 'Observed'], ['Derived', 'Derived']].map(([key, label]) => `<button type="button" class="chip" data-kind-filter="${key}" aria-pressed="${key === 'all'}">${label} <span class="muted">${key === 'all' ? all.length : all.filter(c => c.kind === key).length}</span></button>`).join('')}${runFilter ? `<span class="tag">run filter: <code>${esc(runFilter)}</code> <a href="${BASE}/counterexamples" data-link>clear</a></span>` : ''}</div>` : ''}
 <div data-cx-list></div>`;
 return { title:'Counterexamples', html, attach(root) {
  let kind = query.get('kind') || 'all';
  const render = () => {
   const rows = all.filter(c => (kind === 'all' || c.kind === kind) && (!runFilter || c.parent_run === runFilter));
   root.querySelector('[data-cx-list]').innerHTML = cxRows(rows);
   root.querySelectorAll('[data-kind-filter]').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.kindFilter === kind)));
  };
  root.addEventListener('click', event => { const b = event.target.closest('[data-kind-filter]'); if (b) { kind = b.dataset.kindFilter; render(); } });
  render();
 } };
}

function reproductionHistory(r) {
 if (!r?.count) return '<p class="muted"><strong>Not reproduced yet.</strong> Each <code>eplyx migration reproduce</code> attempt is recorded in <code>.eplyx/reproductions/</code>.</p>';
 const rows = r.history.map(h => `<tr><td><span title="${esc(h.timestamp)}">${esc(ago(h.timestamp))}</span></td><td>${pill(h.outcome === 'Reproduced' ? 'Verified' : 'Failed', h.outcome === 'Reproduced' ? 'Reproduced' : 'Failed')}</td><td>${h.failure_signature_matched ? 'yes' : 'no'}</td><td>${h.no_rpc ? 'yes' : '<span class="text-warn">no</span>'}</td><td class="tech-only">${esc(h.eplyx_version)} · <code>${short(h.engine_binary_sha256, 12)}</code></td><td class="tech-only muted">${esc(h.error ?? '')}</td></tr>`).join('');
 return `<h3 class="subhead">Reproduction history.</h3><p>Reproduced ${count(r.succeeded)} time${r.succeeded === 1 ? '' : 's'}${r.failed ? ` · ${count(r.failed)} failed attempt${r.failed === 1 ? '' : 's'}` : ''} · last ${esc(ago(r.last_timestamp))}.</p><div class="table-wrap"><table class="table"><thead><tr><th>When</th><th>Outcome</th><th>Signature matched</th><th>No RPC</th><th class="tech-only">Engine</th><th class="tech-only">Error</th></tr></thead><tbody>${rows}</tbody></table></div><p class="note">History recorded by the CLI. It shows what an earlier offline replay concluded; it is not a replacement for running it again.</p>`;
}

export async function counterexampleDetail({ params }) {
 const cx = await api(`/api/counterexamples/${params[0]}`);
 if (cx.transition_kind === 'token_migration') return migrationCounterexampleDetail(cx, { copy, runLink, reproductionHistory });
 return { title:'Counterexample', html:empty('This counterexample record cannot be read.') };
}

function equivalenceBanner(x) {
 if (x.comparable) return '<div class="equivalence equivalence--ok" role="status"><strong>Equivalent search conditions.</strong> Same search version, budget and derived domain. A counterexample absent from run B after exact re-execution is reported as resolved.</div>';
 return `<div class="equivalence equivalence--warn" role="status"><strong>Search domains differ. A missing counterexample does not prove resolution.</strong><span>${x.differences.map(esc).join(' ')}</span></div>`;
}

const STATUS_LABELS = { new:'New', only_right:'Only in B', persistent:'Persistent', changed:'Persistent · changed', resolved:'Resolved (equivalent search)', not_reproduced:'Re-executed without failure', only_left:'Only in A' };
const STATUS_TONES = { new:'Failed', only_right:'Indeterminate', persistent:'Failed', changed:'Indeterminate', resolved:'Proven', not_reproduced:'NotTested', only_left:'NotTested' };

function storyCell(label, a, b, changed) {
 return `<div class="story__cell${changed ? ' is-changed' : ''}"><span class="story__label">${esc(label)}</span><span class="story__values"><span>${a}</span><span class="arrow" aria-label="to">→</span><span>${b}</span></span></div>`;
}

function fieldTable(fields) {
 return `<div class="table-wrap"><table class="table diff"><thead><tr><th>Field</th><th>A</th><th>B</th></tr></thead><tbody>${fields.map(f => `<tr class="${f.changed ? 'is-changed' : 'tech-only'}"><td>${esc(f.label)}${f.changed ? ' <span class="tag tag--strong">differs</span>' : ''}</td><td>${value(f.left)}</td><td>${value(f.right)}</td></tr>`).join('')}</tbody></table></div>${fields.some(f => f.changed) ? '' : '<p class="muted">No differences.</p>'}<p class="muted ov-only">${count(fields.filter(f => !f.changed).length)} unchanged fields are shown in Technical mode.</p>`;
}

export async function compare({ query }) {
 const { runs: all } = await api('/api/runs');
 const complete = all.filter(r => r.state === 'Complete');
 if (complete.length < 2) return { title:'Compare', html:empty('Comparison needs at least two completed runs. Run `eplyx migration analyse` again after changing the candidate or config.', 'eplyx migration analyse') };
 const first=complete.find(r=>complete.filter(other=>other.kind===r.kind).length>=2);
 const left = query.get('left') || first?.id;
 const kind=complete.find(r=>r.id===left)?.kind;
 const sameKind=complete.filter(r=>r.kind===kind);
 const right=query.get('right') || sameKind.find(r=>r.id!==left)?.id;
 if(!left || !right || complete.find(r=>r.id===right)?.kind!==kind) return {title:'Compare',html:empty('Select two completed runs of the same analysis kind from the run history.')};
 const picker = (name, selectedId) => `<label class="select"><span>${name === 'left' ? 'Run A' : 'Run B'}</span><select data-side="${name}">${sameKind.map(r => `<option value="${esc(r.id)}" ${r.id === selectedId ? 'selected' : ''}>#${esc(r.number)} · ${esc(GATE_SHORT[r.gate?.outcome] ?? '—')} · ${esc(when(r.timestamp))}</option>`).join('')}</select></label>`;
 const c = await api(`/api/compare?left=${encodeURIComponent(left)}&right=${encodeURIComponent(right)}`);
 if (!isMigration(c.left)) return analyticalCompare(c, { fieldTable });
 const A = c.left, B = c.right, x = c.counterexamples;
 const cxCount = run => run.search?.state === 'Recorded' ? count(run.search.total) : '<span class="muted">no search</span>';
 const violated = run => count(run.invariants?.counts?.Violated ?? 0);
 const html = `
 <div class="page-head"><h1>Compare runs.</h1><p class="muted">Run A → Run B. Compare the saved engine results; differences do not explain what caused them.</p></div>
 <div class="filters">${picker('left', left)}<span class="arrow">→</span>${picker('right', right)}<button type="button" class="button button--ghost" data-swap>Swap</button></div>
 ${equivalenceBanner(x)}
 <section class="story" aria-label="Summary">
  <h2>Run #${esc(A.number)} → Run #${esc(B.number)} ${x.comparable ? '<span class="badge badge--ok">Equivalent search</span>' : '<span class="badge badge--warn">Search domains differ</span>'}</h2>
  <div class="story__grid">
   ${storyCell('Gate', gatePill(A.gate?.outcome), gatePill(B.gate?.outcome), A.gate?.outcome !== B.gate?.outcome)}
   ${isMigration(A) || isMigration(B)
    ? storyCell('Migration mechanism', pill(A.readiness?.mechanism), pill(B.readiness?.mechanism), A.readiness?.mechanism !== B.readiness?.mechanism)
    : storyCell('Candidate conversion', pill(A.conversion), pill(B.conversion), A.conversion !== B.conversion)}
   ${storyCell(isMigration(A) || isMigration(B) ? 'Stress behaving' : 'Stress proven', `${count(proven(A))}/${count(selected(A))}`, `${count(proven(B))}/${count(selected(B))}`, proven(A) !== proven(B) || selected(A) !== selected(B))}
   ${storyCell('Counterexamples', cxCount(A), cxCount(B), cxTotal(A) !== cxTotal(B))}
   ${storyCell('Invariants violated', violated(A), violated(B), violated(A) !== violated(B))}
   ${storyCell('Candidate binary', `<code>${short(A.candidate_program_sha256, 8)}</code>`, c.inputs[0].changed ? `<code>${short(B.candidate_program_sha256, 8)}</code>` : 'unchanged', c.inputs[0].changed)}
  </div>
 </section>
 <div class="grid-2">
  ${panel({ eyebrow:'What the developer declared', title:'Input differences', body:fieldTable(c.inputs) })}
  ${panel({ eyebrow:'What the engine observed', title:'Result differences', body:fieldTable(c.results) })}
 </div>
 ${panel({ title:'Invariants', body:c.invariants.length ? `<div class="table-wrap"><table class="table diff"><thead><tr><th>Invariant</th><th>Severity</th><th>A</th><th>B</th></tr></thead><tbody>${c.invariants.map(i => `<tr class="${i.changed ? 'is-changed' : ''}"><td>${esc(invariantName(i.invariant_type))}${i.changed ? ' <span class="tag tag--strong">changed</span>' : ''}</td><td>${esc(sentence(i.severity ?? ''))}</td><td>${i.left ? pill(i.left) : '<span class="muted">not declared</span>'}</td><td>${i.right ? pill(i.right) : '<span class="muted">not declared</span>'}</td></tr>`).join('')}</tbody></table></div>` : '<p class="muted">Neither run declares invariants.</p>' })}
 ${panel({ title:'Deployment gate', body:`<div class="status-line">${gatePill(c.gate.left.outcome)} <span class="arrow">→</span> ${gatePill(c.gate.right.outcome)} <span class="muted">policy ${esc(c.gate.left.policy ?? '—')} → ${esc(c.gate.right.policy ?? '—')}</span></div>
  ${c.gate.reasons_added.length ? `<h3 class="subhead">Reasons only in B.</h3><ul class="reasons reasons--added">${c.gate.reasons_added.map(r => `<li>${esc(r)}</li>`).join('')}</ul>` : ''}
  ${c.gate.reasons_removed.length ? `<h3 class="subhead">Reasons only in A.</h3><ul class="reasons reasons--removed">${c.gate.reasons_removed.map(r => `<li>${esc(r)}</li>`).join('')}</ul>` : ''}
  <p class="muted">${count(c.gate.reasons_kept)} reasons appear in both runs.</p>` })}
 ${panel({ title:'Counterexamples', body:`
  <div class="alert ${x.comparable ? 'alert--ok' : ''}"><strong>${x.comparable ? 'Search conditions are equivalent.' : 'Search conditions are not directly equivalent.'}</strong>${x.differences.length ? `<ul>${x.differences.map(d => `<li>${esc(d)}</li>`).join('')}</ul>` : ''}${x.comparable ? '' : '<p>A counterexample missing from one run is not reported as resolved.</p>'}</div>
  <p>${count(x.left_total)} in A → ${count(x.right_total)} in B. ${Object.entries(x.counts).map(([k, v]) => `${pill(STATUS_TONES[k], `${v} ${STATUS_LABELS[k]}`)}`).join(' ')}</p>
  ${x.items.length ? cxDiffTable(x.items) : '<p class="muted">Neither run recorded a counterexample.</p>'}
  <p class="note">${esc(x.identity)}</p>` })}
 ${panel({ title:'Git and tooling', body:fieldTable(c.git) })}
 <p class="note">${esc(c.causality)}</p>`;
 return { title:'Compare', html, attach(root) {
  const go = (l, r) => window.dashboardNavigate(`${BASE}/compare?left=${l}&right=${r}`);
  root.querySelectorAll('[data-side]').forEach(s => s.addEventListener('change', () => go(root.querySelector('[data-side=left]').value, root.querySelector('[data-side=right]').value)));
  root.querySelector('[data-swap]').addEventListener('click', () => go(right, left));
 } };
}

function cxDiffTable(items) {
 const row = i => `<tr><td>${pill(STATUS_TONES[i.status], STATUS_LABELS[i.status])}</td><td>${esc(i.kind)}${i.left?.dimension || i.right?.dimension ? ` · ${esc(sentence((i.left ?? i.right).dimension))}` : ''}</td><td>${ident(i.account, 'addr')}</td><td>${cxSide(i.left)}</td><td>${cxSide(i.right)}</td><td class="muted">${esc(i.note)}</td></tr>`;
 const table = rows => `<div class="table-wrap"><table class="table"><thead><tr><th>Status</th><th>Kind</th><th>Account</th><th>A</th><th>B</th><th>Note</th></tr></thead><tbody>${rows.map(row).join('')}</tbody></table></div>`;
 // Derived boundaries first, then observed failures; long lists fold away.
 const ordered = [...items.filter(i => i.kind === 'Derived'), ...items.filter(i => i.kind !== 'Derived')];
 const shown = ordered.slice(0, 6), rest = ordered.slice(6);
 return `${table(shown)}${rest.length ? `<details><summary>Show ${rest.length} more matched counterexamples</summary>${table(rest)}</details>` : ''}`;
}

function cxSide(side) {
 if (!side) return '<span class="muted">—</span>';
 const detail = side.kind === 'Derived' ? boundaryText(side) : `<code>${esc(side.failure?.instruction_error ?? migrationFailure(side.failure))}</code>`;
 return `${side.id ? `<a href="${BASE}/counterexamples/${esc(side.id)}" data-link><code>${short(side.id, 10)}</code></a>` : ''} <span class="muted">${detail}</span>`;
}

// ------------------------------------------------ Secondary summary pages

async function runForPage(query, project) {
 const id = query.get('run') || project.latest?.id;
 if (!id) return null;
 return api(`/api/runs/${id}`);
}
const runPicker = (runs, current, path) => `<label class="select"><span>Run</span><select data-run-picker="${esc(path)}">${runs.filter(r => r.state === 'Complete').map(r => `<option value="${esc(r.id)}" ${r.id === current ? 'selected' : ''}>#${esc(r.number)} · ${esc(GATE_SHORT[r.gate?.outcome] ?? '—')} · ${esc(when(r.timestamp))}</option>`).join('')}</select></label>`;
const attachPicker = root => root.querySelector('[data-run-picker]')?.addEventListener('change', event => window.dashboardNavigate(`${BASE}${event.target.dataset.runPicker}?run=${event.target.value}`));
const noRuns = title => ({ title, html:empty('Run `eplyx migration analyse` to create your first assurance run.', 'eplyx migration analyse') });

export async function production({ query, project }) {
 const detail = await runForPage(query, project);
 if (!detail) return noRuns('Production state');
 const { runs: all } = await api('/api/runs');
 if (isMigration(detail)) return { title:'Production state', html:`<div class="page-head"><div><h1>${detail.migration_detail?.coverage?.world?.kind === 'ObservedCapture' ? 'Captured state' : 'Rehearsal state'}.</h1><p class="muted">The state run #${esc(detail.number)} rehearsed against: ${esc(words(detail.migration_detail?.coverage?.world?.kind))}. Summary only; the world stays on ${CLOUD ? 'the machine that ran it' : 'disk'}.</p></div>${runPicker(all, detail.id, '/production')}</div>${migrationProduction(detail)}`, attach:attachPicker };
 return analyticalDetail(detail, { evidenceBlock });
}

export async function invariants({ query, project }) {
 const detail = await runForPage(query, project);
 if (!detail) return noRuns('Invariants');
 if (!isMigration(detail)) return { title:'Not evaluated', html:empty('This analysis has no migration deployment gate or migration invariants.') };
 const { runs: all } = await api('/api/runs');
 const results = detail.invariant_results ?? [];
 const tally = s => results.filter(i => i.status === s).length;
 const html = `
 <div class="page-head"><div><h1>Invariants.</h1><p class="muted">Declared in <code>eplyx.toml</code>, recorded with the analytical input and evaluated by the engine after execution. Rendered exactly as run #${esc(detail.number)} recorded them.</p></div>${runPicker(all, detail.id, '/invariants')}</div>
 ${results.length ? `<div class="tiles tiles--4">${['Satisfied', 'Violated', 'Indeterminate', 'NotApplicable'].map(s => tile({ label:words(s), value:count(tally(s)), status:tally(s) ? s : '' })).join('')}</div>` : ''}
 ${panel({ title:'Declared invariants', body:invariantList(results, detail.release?.invariant_definitions) })}
 ${panel({ title:'How severity meets the gate', body:'<ul class="limits"><li>A <strong>blocking</strong> invariant that is Violated blocks the gate under every policy.</li><li>Under <code>strict</code>, a blocking invariant that Cannot be judged also blocks.</li><li>A <strong>warning</strong> invariant that is Violated or Cannot be judged adds a warning.</li></ul><p class="muted">These are the engine’s rules, summarised. The dashboard renders saved findings and never re-evaluates them.</p>' })}`;
 return { title:'Invariants', html, attach:attachPicker };
}

export async function gate({ query, project }) {
 const detail = await runForPage(query, project);
 if (!detail) return noRuns('CI / gate');
 if (!isMigration(detail)) return { title:'Not evaluated', html:empty('This analysis has no migration deployment gate or migration invariants.') };
 const { runs: all } = await api('/api/runs');
 const configured = project.context?.config?.gate_policy;
 const history = (project.gate_history ?? []).slice().reverse();
 const html = `
 <div class="page-head"><div><h1>CI / gate.</h1><p class="muted">The deployment decision recorded for run #${esc(detail.number)}. Exit codes: <code>0</code> pass or warnings, <code>1</code> policy or requirement failure, <code>3</code> stale declaration, <code>5</code> evidence insufficient for judgement, <code>4</code> compatibility or identity failure, <code>2</code> invalid input or engine failure.</p></div>${runPicker(all, detail.id, '/gate')}</div>
 <div class="tiles tiles--3">
  ${tile({ label:'Configured policy', value:`<code>${esc(configured ?? detail.gate?.policy ?? '—')}</code>`, sub:configured ? 'from eplyx.toml' : 'eplyx.toml unavailable; showing the run’s policy' })}
  ${tile({ label:`Run #${detail.number}`, value:gatePill(detail.gate?.outcome, 'lg'), sub:`under <code>${esc(detail.gate?.policy ?? '—')}</code>`, status:detail.gate?.outcome })}
  ${tile({ label:'Runs blocked', value:`${count(project.stats?.blocked)} / ${count(project.stats?.runs)}`, sub:`${count(project.stats?.warned)} warned · ${count(project.stats?.passed)} passed` })}
 </div>
 ${panel({ title:'Gate history', body:history.length ? `<ol class="history" aria-label="Gate outcomes, oldest to newest">${history.map(h => `<li><a href="${BASE}/gate?run=${esc(h.id)}" data-link class="history__cell history__cell--${tone(h.outcome)}${h.id === detail.id ? ' is-current' : ''}" title="${esc(`Run #${h.number}: ${GATE[h.outcome] ?? 'no result'} · ${when(h.timestamp)}`)}"><span>${esc(GATE_SHORT[h.outcome] ?? '—')}</span><small>#${esc(h.number)}</small></a></li>`).join('')}</ol><p class="muted">Oldest to newest, up to the last 30 runs.</p>` : '<p class="muted">No runs.</p>' })}
 ${panel({ title:'Reasons', body:gateBlock(detail) })}
 ${panel({ title:'CI usage', body:`${commandLine('eplyx migration analyse')}${commandLine('eplyx migration analyse --policy strict', 'Stricter policy for one run')}<p class="muted">Block-only permits Incomplete with warnings; strict blocks it. Neither policy changes the analytical findings.</p>` })}`;
 return { title:'CI / gate', html, attach:attachPicker };
}

export async function projectPage({ project }) {
 const ctx = project.context ?? {};
 const config = ctx.config ?? {};
 const s = project.stats ?? {};
 const latest = project.latest;
 const candidateMatch = ctx.candidate?.sha256 && latest ? (ctx.candidate.sha256 === latest.candidate_program_sha256 ? 'matches the latest run' : '<span class="text-warn">differs from the latest run. Run a new analysis.</span>') : '';
 const html = `
 <div class="page-head"><h1>Project.</h1><p class="muted">Local project and usage history from <code>.eplyx/</code> on this machine. The dashboard reads this data locally.</p></div>
 <div class="tiles">
  ${tile({ label:'Runs', value:count(s.runs), sub:`${count(s.preflights)} completed analyses${s.unfinished_or_unreadable ? ` · ${count(s.unfinished_or_unreadable)} unfinished or unreadable` : ''}` })}
  ${tile({ label:'Searches', value:count(s.searches) })}
  ${tile({ label:'Counterexamples saved', value:count(s.counterexamples_saved), sub:`${count(s.counterexample_kinds?.Observed)} observed · ${count(s.counterexample_kinds?.Derived)} derived` })}
  ${tile({ label:'Blocked releases', value:count(s.blocked), sub:`${count(s.warned)} warned · ${count(s.passed)} passed` })}
  ${tile({ label:'Offline reproductions', value:count(s.offline_reproductions ?? 0), sub:s.offline_reproductions ? `${count(s.reproductions_succeeded)} reproduced · ${count(s.reproductions_failed)} failed · last ${esc(ago(s.latest_reproduction))}` : esc(s.offline_reproductions_note ?? '') })}
  ${tile({ label:'First → latest run', value:esc(s.first_run ? when(s.first_run) : '—'), sub:esc(s.latest_run ? `latest ${ago(s.latest_run)}` : '') })}
 </div>
 ${panel({ title:'Project', body:kv([
  ['Project name', esc(project.project?.name ?? '—')],
  ['Project ID', project.project?.id ? `<code>${esc(project.project.id)}</code>` : '<span class="muted">not recorded</span>'],
  ['Config', `<code>${esc(config.path ?? 'eplyx.toml')}</code> ${pill(config.state === 'Valid' ? 'Valid' : config.state === 'Missing' ? 'NotTested' : 'Failed', config.state === 'Missing' ? 'missing' : config.state === 'Valid' ? 'valid' : 'invalid')}${config.error ? ` <span class="muted">${esc(config.error)}</span>` : ''}`],
  ['Candidate path', config.program_path ? `<code>${esc(config.program_path)}</code>` : '<span class="muted">unavailable</span>'],
  ['Current candidate hash', ctx.candidate?.sha256 ? `${ident(ctx.candidate.sha256)} ${candidateMatch}` : `<span class="muted">${esc(ctx.candidate?.error ?? 'unavailable')}</span>`],
  ['Adapter', config.adapter ? `<code>${esc(config.adapter)}</code>` : '—'],
  ['Gate policy', config.gate_policy ? `<code>${esc(config.gate_policy)}</code>` : '—'],
  ['Git branch', esc(ctx.git?.branch ?? 'not a Git checkout')],
  ['Latest commit', ctx.git?.commit ? `${ident(ctx.git.commit, 'commit')}${ctx.git.dirty ? ' <span class="tag">uncommitted changes</span>' : ''}` : '—'],
  ['Run sources', Object.entries(s.run_sources ?? {}).filter(([, n]) => n).map(([k, n]) => `${count(n)} ${esc(k === 'not_recorded' ? 'not recorded' : SOURCES[k] ?? k)}`).join(' · ') || '—'],
  ['Branches in run history', (s.branches ?? []).map(b => `<code>${esc(b)}</code>`).join(' ') || '<span class="muted">none recorded</span>'],
  ['Local run store', `<code>${esc(project.store?.path ?? '.eplyx/')}</code>`],
  ['Project root', `<code>${esc(project.store?.root_display ?? '')}</code>`, 'tech-only'],
  ['Dashboard cache', `<code>${esc(project.store?.index ?? '')}</code> <span class="muted">(summaries only; rebuilt when missing or stale; not evidence)</span>`, 'tech-only'],
  ['CLI / engine version', esc(ctx.version ?? '—')],
  ['RPC', 'not used by the dashboard; no provider URL or credential is read or shown'],
  ['Eplyx cloud', project.cloud?.linked ? `linked to <code>${esc(project.cloud.project_id)}</code> on <code>${esc(project.cloud.server)}</code> · ${count(project.cloud.synced_runs)} of ${count(s.preflights)} runs synced <span class="muted">(optional; change it with <code>eplyx link</code>)</span>` : 'not linked <span class="muted">(optional; everything here works without an account)</span>'],
 ]) })}
 ${panel({ title:'Where things live', body:`<ul class="limits"><li><code>eplyx.toml</code> is the only configuration surface. The dashboard is read-only and cannot edit terms, invariants, the gate policy or the candidate path.</li><li><code>.eplyx/runs/</code> holds immutable run inputs, reports and search output. <code>.eplyx/counterexamples/</code> holds saved counterexamples.</li><li>Replay and reproduction happen through the CLI.</li></ul>` })}`;
 return { title:'Project', html };
}
