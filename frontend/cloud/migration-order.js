// Hosted order selection and presentation. All eligibility, identities,
// execution outcomes and comparison classifications come from Rust.
import { BASE, PROJECT } from './env.js';
import { esc, panel, kv, ident, raw, pill, empty } from './ui.js';
const parentURL = id => `${BASE}/runs/${encodeURIComponent(id)}`;
const orderURL = id => `${BASE}/migration-orders/${encodeURIComponent(id)}`;
const endpoint = parent => `/v1/projects/${encodeURIComponent(PROJECT)}/runs/${encodeURIComponent(parent)}/migration-order`;
const resultEndpoint = id => `/v1/projects/${encodeURIComponent(PROJECT)}/migration-orders/${encodeURIComponent(id)}`;
export const retainedWording = 'This comparison uses the immutable retained parent state. No state refresh occurs. A newer state requires a new migration run.';
export const question = 'Compare two migration units from the same retained world and shared reserve to see whether transaction order changes their outcome.';
async function request(path, options = {}) {
 const response = await fetch(path, { credentials:'same-origin', ...options });
 const body = await response.json().catch(() => ({}));
 if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
 return body;
}
export function validateSelection(eligibility, a, b) {
 if (!eligibility?.eligible) return eligibility?.reason || 'Order analysis is unavailable for this parent.';
 if (!a || !b) return 'Select Source A and Source B.';
 if (a === b) return 'Select two distinct sources.';
 if (![a,b].every(s => eligibility.units.some(u => u.source_account === s))) return 'Select eligible retained units.';
 return null;
}
export function previewHTML(e, a, b) {
 const error = validateSelection(e,a,b);
 if (error) return `<p role="alert">${esc(error)}</p>`;
 return `${panel({title:'Bounded comparison preview',body:`<p>${question}</p>${kv([
  ['Same proposal',ident(e.change_spec_id)], ['Same starting state',ident(e.world_id)],
  ['Same Clock/runtime',`Fixed slot ${esc(e.clock?.slot)}; Unix ${esc(e.clock?.unix_timestamp)}. No time advancement. <code>${esc(e.runtime_id)}</code>`],
  ['Source A',ident(a,'addr')], ['Source B',ident(b,'addr')],
  ['Four analyses','A alone · B alone · A → B · B → A'],
 ])}<p>The comparison asks whether transaction ordering changes outcomes through shared state. The result will be established by execution.</p><p class="muted">${retainedWording}</p><p>One migration ChangeSpec; two operation instances.</p>`})}`;
}
export function selectionHTML(e) {
 if (!e?.eligible) return panel({title:'Order analysis unavailable',body:`<p>${esc(e?.reason || 'Parent eligibility could not be verified.')}</p>`});
 const options = e.units.map(u => `<option value="${esc(u.source_account)}">${esc(u.source_account)} · owner ${esc(u.owner)} · source ${esc(u.amount_raw)} raw · quoted output ${esc(u.quote?.output_raw ?? u.quote?.output ?? 'recorded in unit evidence')} · ${esc(u.authority?.kind ?? 'authority recorded')}</option>`).join('');
 return `<p>${question}</p><p class="muted">${retainedWording}</p><form data-order-form class="analysis-form">
 <div class="migration-grid">${['A','B'].map(label => `<label class="analysis-field"><span>Source ${label}</span><select aria-label="Source ${label}" name="source_${label.toLowerCase()}" required><option value="">Select an eligible retained unit</option>${options}</select></label>`).join('')}</div>
 <button type="button" class="button button--ghost" data-preview>Preview bounded comparison</button>
 <div data-order-preview aria-live="polite"></div><p data-order-error role="alert"></p>
 <button class="button" type="submit" data-order-submit disabled>Submit order analysis</button></form>`;
}
export async function submitSelection({ parent, eligibility, sourceA, sourceB, call = request, navigate = url => window.dashboardNavigate(url) }) {
 const error = validateSelection(eligibility, sourceA, sourceB);
 if (error) throw new Error(error);
 const accepted = await call(endpoint(parent), { method:'POST', headers:{'Content-Type':'application/json'}, body:JSON.stringify({source_a:sourceA,source_b:sourceB}) });
 if (!/^run_[A-Za-z0-9_-]+$/.test(accepted.run_id || '')) throw new Error('The server did not return an order occurrence identity.');
 navigate(orderURL(accepted.run_id));
 return accepted;
}
export function attachSelection(node, parent, eligibility) {
 const form = node.querySelector('[data-order-form]');
 if (!form) return;
 const a = form.elements.source_a, b = form.elements.source_b;
 const preview = form.querySelector('[data-order-preview]');
 const error = form.querySelector('[data-order-error]');
 const submit = form.querySelector('[data-order-submit]');
 let previewed = null;
 form.addEventListener('change', () => { previewed = null; preview.innerHTML = ''; submit.disabled = true; error.textContent = validateSelection(eligibility,a.value,b.value) || ''; });
 form.querySelector('[data-preview]').addEventListener('click', () => {
  const problem = validateSelection(eligibility,a.value,b.value);
  error.textContent = problem || '';
  if (problem) return;
  previewed = [a.value,b.value]; preview.innerHTML = previewHTML(eligibility,...previewed); submit.disabled = false;
 });
 form.addEventListener('submit', async event => {
  event.preventDefault();
  if (!previewed || previewed[0] !== a.value || previewed[1] !== b.value) { error.textContent = 'Preview the selected pair before submission.'; return; }
  submit.disabled = true; error.textContent = '';
  try { await submitSelection({parent,eligibility,sourceA:a.value,sourceB:b.value}); }
  catch (e) { error.textContent = e.message; submit.disabled = false; }
 });
}
export async function selectionPage({params}) {
 const parent = params[0];
 const e = await request(`${endpoint(parent)}/eligibility`);
 return {title:'Order Analysis',crumbs:[['Parent migration run',`/runs/${parent}`],['Order Analysis']],
  html:`<div class="page-head"><h1>Order Analysis.</h1><a href="${parentURL(parent)}" data-link>Parent migration run</a></div>${selectionHTML(e)}`,
  attach:node => attachSelection(node,parent,e)};
}
export function parentSection(parent, eligibility, orders) {
 return panel({id:'order-analyses',title:'Order analyses',body:`${eligibility.eligible
  ? `<p>${question}</p><a class="button button--ghost" href="${parentURL(parent)}/order" data-link>Order Analysis</a>`
  : `<p>Order analysis unavailable. ${esc(eligibility.reason)}</p>`}
 ${orders.length ? `<ul>${orders.map(o => `<li><a href="${orderURL(o.run_id)}" data-link>${esc(o.source_a)} vs ${esc(o.source_b)}</a> · ${esc(o.status)}${o.analysis ? ` · ${esc(o.analysis.comparison.status)}` : ''}</li>`).join('')}</ul>` : '<p class="muted">No order analyses recorded for this parent.</p>'}`});
}
export async function withOrders(view,parent) {
 try {
  const [e,list] = await Promise.all([request(`${endpoint(parent)}/eligibility`),request(endpoint(parent))]);
  return {...view,html:view.html + parentSection(parent,e,list.orders)};
 } catch (error) {
  return {...view,html:view.html + panel({title:'Order analyses unavailable',body:`<p>${esc(error.message)}</p>`})};
 }
}
const comparisonText = {
 SharedReserveChangesSuccessfulUnit:'Reversing these two transactions changed which migration unit succeeded under the retained shared-reserve state.',
 NoSuccessfulUnitEffect:'The engine established no change in which selected migration units succeeded for these two orders under the retained state.',
 NotEstablished:'The retained comparison did not establish the narrow shared-reserve ordering effect. Inspect the recorded scenarios and limitations.',
};
const outcomeText = {Migrated:'succeeded',Rejected:'rejected',ReconciliationMismatch:'reconciliation mismatch',NotAttempted:'not attempted'};
const techJSON = v => `<pre>${esc(JSON.stringify(v,null,2))}</pre>`;
function stepHTML(step,index,labels) {
 const x = step.execution;
 return `<article><h3>Step ${index + 1} · ${esc(labels[x.unit_id] || x.unit_id)} ${esc(outcomeText[x.outcome] || x.outcome)}</h3>
 ${kv([['Reserve before → after',`${raw(step.observed_reserve_before_raw)} → ${raw(step.observed_reserve_after_raw)} raw`],
 ['Reconciliation',x.reconciled ? 'exact' : x.failure?.rollback_verified ? 'rejection rollback verified (fee retained)' : 'not reconciled'],
 ['Failure signature',esc(x.failure?.error_name || x.failure?.error || 'none')],
 ['State before',ident(step.before_state_id), 'tech-only'],['State after',ident(step.after_state_id),'tech-only']])}
 <details class="tech-only"><summary>Exact execution evidence</summary>${techJSON(step)}</details></article>`;
}
export function resultHTML(r) {
 const a = r.analysis, b = a?.binding;
 const parent = `<a href="${parentURL(r.parent_run_id)}" data-link>Parent migration run</a>`;
 const heading = `<div class="page-head"><div><h1>Order Analysis.</h1><p>${esc(r.status)} · occurrence <code>${esc(r.run_id)}</code></p>${parent}</div></div><p class="muted">${retainedWording}</p>`;
 const failure = r.failure ? panel({title:r.failure.kind === 'UnsupportedComposition' ? 'Unsupported composition' : r.failure.kind === 'InternalExecutionFailure' ? 'Worker error' : 'Evidence/handoff failure',body:`<p>${esc(r.failure.detail)}</p><p>No analytical conclusion should be taken from this failure.</p><code>${esc(r.failure.kind)}</code>`}) : '';
 if (!a) return heading + failure + (!r.failure ? empty(`Order analysis is ${r.status}. The page will update when execution finishes.`) : '');
 const labels = Object.fromEntries(b.units.map((u,i) => [u.unit_id,['A','B'][i]]));
 return heading + failure + panel({title:'Initial conditions',body:kv([
  ['Parent',parent],['ChangeSpec',ident(b.change_spec_id)],['Candidate',ident(b.candidate.sha256)],
  ['Retained world',ident(b.world_id)],['Fixed Clock',`slot ${esc(b.clock.slot)} · Unix ${esc(b.clock.unix_timestamp)} · no time advancement`],
  ['Shared reserve',`${ident(b.reserve,'addr')} · ${raw(b.initial_reserve_raw)} raw`],
  ...b.units.map((u,i) => [`Source ${['A','B'][i]}`,`${ident(u.source_account,'addr')} · owner ${ident(u.owner,'addr')} · ${raw(u.amount_raw)} source raw`]),
 ])}) + a.scenarios.map((scenario,i) => panel({title:['A alone','B alone','A → B','B → A'][i],body:`${scenario.steps.map((s,n) => stepHTML(s,n,labels)).join(i >= 2 ? '<p aria-label="State handoff">→ intermediate state → next transaction</p>' : '')}
 <p class="tech-only">Initial state ${ident(scenario.initial_state_id)} · final state ${ident(scenario.final_state_id)}</p>
 ${scenario.stopped ? `<p>Stopped: ${esc(scenario.stopped)}</p>` : ''}<details class="tech-only"><summary>Scenario identities</summary>${kv([['Order-case ID',ident(scenario.case_id)],['Engine scenario ID',ident(scenario.run_id)]])}</details>`})).join('')
 + panel({title:'Comparison',body:`<p><code>${esc(a.comparison.status)}</code></p><p>${esc(comparisonText[a.comparison.status] || 'See the recorded engine classification.')}</p>${a.comparison.finding ? `<p class="tech-only">${ident(a.comparison.finding)}</p>` : ''}<ul>${a.comparison.limitations.map(l => `<li>${esc(l)}</li>`).join('')}</ul>`})
 + panel({id:'order-evidence',title:'Evidence',body:`${r.artifact_available ? `<a class="button button--ghost" href="${resultEndpoint(r.run_id)}/artifact">Download portable order artifact</a><p>Extract the archive, then run <code>eplyx migration reproduce-order ./order-case --format json</code>.</p>` : ''}
 <div class="tech-only">${kv([['Pair binding ID',ident(a.binding_id)],['Runtime identity',ident(b.runtime_id)],['World content identity',ident(b.world_content_sha256)]])}
 <details><summary>Account closure, known absence, unit identities and state evidence</summary>${techJSON({binding:b,states:a.states,comparison:a.comparison})}</details></div>`});
}
export async function resultPage({params}) {
 const id = params[0]; const r = await request(resultEndpoint(id));
 return {title:'Order Analysis',crumbs:[['Parent migration run',`/runs/${r.parent_run_id}`],['Order Analysis']],html:`<div data-order-result>${resultHTML(r)}</div>`,attach(node) {
  const target = node.querySelector('[data-order-result]');
  async function poll() {
   if (!target?.isConnected) return;
   try {
    const fresh = await request(resultEndpoint(id));
    if (!target.isConnected) return;
    target.innerHTML = resultHTML(fresh);
    if (['queued','running'].includes(fresh.status)) setTimeout(poll,1500);
   } catch (error) {
    if (target.isConnected) target.innerHTML = panel({title:'Order evidence unavailable',body:`<p>${esc(error.message)}</p><p>No analytical conclusion is available while retained evidence cannot be verified.</p>`});
   }
  }
  if (['queued','running'].includes(r.status)) setTimeout(poll,1500);
 }};
}
