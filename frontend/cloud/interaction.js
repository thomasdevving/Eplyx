// Hosted presentation only: quantities, classifications and handoffs are engine
// values. Preview/admission and project readiness are separate server checks.
import { BASE, PROJECT } from './env.js';
import { esc, panel, kv, ident, pill } from './ui.js';
import { fetchProjectCapabilities, capabilityFor, canSubmitCapability, capabilityHTML } from '/assets/capabilities.js';
const endpoint = parent => `/v1/projects/${encodeURIComponent(PROJECT)}/runs/${encodeURIComponent(parent)}/interactions`;
const resultEndpoint = id => `/v1/projects/${encodeURIComponent(PROJECT)}/interactions/${encodeURIComponent(id)}`;
const resultURL = id => `${BASE}/interactions/${encodeURIComponent(id)}`;
const parentURL = id => `${BASE}/runs/${encodeURIComponent(id)}`;
export const scope = 'One retained DepositSol; fixed epoch 0. Six independent fresh banks. Constructed candidates are test code, not an upstream release. No loader Upgrade, rollout order, atomicity, authority possession or governance approval is established.';
async function request(path, options = {}) {
 const response = await fetch(path, {credentials:'same-origin', ...options});
 const body = await response.json().catch(() => ({}));
 if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
 return body;
}
export function parseProposal(text) {
 const doc = JSON.parse(text);
 if (!doc || Array.isArray(doc) || doc.schema_version !== 1 || doc.change?.kind !== 'protocol_parameter_change') throw new Error('Paste an existing schema-1 parameter ChangeSpec. Server admission remains authoritative.');
 return doc;
}
const quantity = q => q?.value == null ? `<span class="muted">Unavailable: ${esc(q?.unavailable_reason || 'measurement not established')}</span>` : `<code>${esc(q.value)}</code>`;
const json = v => `<pre class="code-block">${esc(JSON.stringify(v, null, 2))}</pre>`;
export function previewHTML(f, {pending=true}={}) {
 const operation = f.operation;
 return panel({title:pending?'Factual input preview':'Retained inputs',body:`${kv([
  ['Upgrade proposal',ident(f.upgrade_change_spec_id)], ['Parameter proposal',ident(f.parameter_change_spec_id)],
  ['Selected record',ident(f.record_id)], ['Pool',ident(f.pool,'addr')], ['Retained slot',esc(f.slot)], ['Retained window',json(f.source_slot_range ?? f.window ?? 'Bound to the retained bundle')],
  ['Historical V1',ident(f.v1?.sha256)], ['Qualified V2',`${ident(f.v2?.sha256)} · ${esc(f.v2?.profile?.id)}`],
  ['Six executions','K1 SetFee under V1 · K2 SetFee under V2 · R00 V1/C0 · R01 V1/K1 pool · R10 V2/C0 · R11 V2/K2 pool'],
 ])}<h3>Exact current and proposed rational fee</h3>${json(operation)}<p>${scope}</p><p>${pending?'These are retained inputs. Execution will establish the result.':'The stages below retain the actual configuration and action outcomes.'}</p><details><summary>Inspect both original proposals</summary>${json(f.upgrade_change_spec)}${json(f.parameter_change_spec)}</details><div class="tech-only">${json({analysis_input_sha256:f.analysis_input_sha256,clock:f.clock,runtime:f.runtime,dependencies:f.dependencies,manager_assumption:f.manager_assumption,fee_payer:f.fee_payer})}</div>`});
}
export function selectionHTML(e, capability) {
 if (!e?.eligible) return panel({title:'Interaction unavailable',body:`<p>${esc(e?.reason || 'Parent eligibility could not be verified.')}</p>${(e?.unavailable_records || []).map(r=>`<p>${esc(r.record_id)}: ${esc(r.reason)}</p>`).join('')}`});
 return `<p>${scope}</p><p>The immutable upgrade parent supplies the exact candidate, proposal and historical bundle. Project readiness is checked separately.</p>${capabilityHTML(capability,{compact:true})}<form class="analysis-form" data-interaction-form>
 <label class="analysis-field"><span>Historical record</span><select name="record_id" required><option value="">Select an eligible historical record</option>${e.records.map(r=>`<option value="${esc(r.record_id)}">${esc(r.record_id)} · pool ${esc(r.pool)} · slot ${esc(r.slot)}</option>`).join('')}</select></label>
 <label class="analysis-field"><span>Existing parameter ChangeSpec JSON</span><textarea name="proposal" rows="12" required spellcheck="false"></textarea></label>
 <label class="analysis-field"><span>Import existing ChangeSpec</span><input type="file" name="file" accept="application/json,.json"></label>
 <button type="button" class="button button--ghost" data-preview>Preview retained inputs</button><div data-interaction-preview aria-live="polite"></div><p data-interaction-error role="alert"></p><button class="button" type="submit" data-submit disabled>Submit interaction analysis</button></form>`;
}
export function attachSelection(node,parent,e,capability,{call=request,navigate=url=>window.dashboardNavigate(url),key=()=>crypto.randomUUID()}={}) {
 const form=node.querySelector('[data-interaction-form]');if(!form)return;
 const record=form.elements.record_id, proposal=form.elements.proposal, preview=form.querySelector('[data-interaction-preview]'), error=form.querySelector('[data-interaction-error]'), submit=form.querySelector('[data-submit]');
 let approved=null, generation=0, busy=false;
 const invalidate=()=>{generation++;approved=null;preview.innerHTML='';submit.disabled=true;error.textContent='';};
 form.addEventListener('input',invalidate);form.addEventListener('change',invalidate);
 form.elements.file.addEventListener('change',async()=>{const file=form.elements.file.files[0];if(!file)return;if(file.size>65536){error.textContent='ChangeSpec exceeds the 64 KiB proposal bound.';return;}try{proposal.value=await file.text();invalidate();}catch{error.textContent='The proposal file could not be read.';}});
 form.querySelector('[data-preview]').addEventListener('click',async()=>{
  invalidate();const version=generation;error.textContent='';
  try {
   if(!record.value || !e.records.some(r=>r.record_id===record.value)) throw new Error('Select an explicit eligible historical record.');
   const payload={record_id:record.value,parameter_change_spec:parseProposal(proposal.value)};
   const f=await call(`${endpoint(parent)}/preview`,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(payload)});
   if(version!==generation)return;
   approved={...payload,request_key:key()};preview.innerHTML=previewHTML(f);submit.disabled=!canSubmitCapability(capability);
  }catch(err){if(version===generation)error.textContent=err.message;}
 });
 form.addEventListener('submit',async event=>{
  event.preventDefault();if(!approved || busy || !canSubmitCapability(capability))return;
  busy=true;submit.disabled=true;error.textContent='';
  try {const accepted=await call(endpoint(parent),{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(approved)});if(!/^run_[A-Za-z0-9_-]+$/.test(accepted.run_id||''))throw new Error('The server did not return an interaction occurrence identity.');navigate(resultURL(accepted.run_id));}
  catch(err){error.textContent=err.message;}
  finally{busy=false;submit.disabled=!approved || !canSubmitCapability(capability);}
 });
}
export async function selectionPage({params}) {
 document.querySelector('link[data-upgrade-style]')?.remove();
 const parent=params[0];
 const [e,c]=await Promise.all([request(`${endpoint(parent)}/eligibility`).catch(err=>({eligible:false,reason:err.message})),fetchProjectCapabilities(PROJECT,request).catch(()=>null)]);
 const capability=capabilityFor(c,'upgrade_parameter_interaction');
 return {title:'Upgrade / parameter interaction',crumbs:[['Upgrade parent',`/runs/${parent}`],['Interaction']],html:`<div class="page-head"><h1>Upgrade / parameter interaction.</h1><a href="${parentURL(parent)}" data-link>Parent upgrade run</a></div>${selectionHTML(e,capability)}`,attach:node=>attachSelection(node,parent,e,capability)};
}
export async function withInteractions(view,parent) {
 const [e,list]=await Promise.all([request(`${endpoint(parent)}/eligibility`).catch(err=>({eligible:false,reason:err.message})),request(endpoint(parent)).catch(err=>({error:err.message,interactions:[]}))]);
 return {...view,html:`${view.html}${panel({id:'interaction-analyses',cls:'interaction-parent',title:'Upgrade / parameter interactions',body:`<p>${scope}</p>${e.eligible?`<a class="button button--ghost" href="${BASE}/runs/${encodeURIComponent(parent)}/interaction" data-link>Compare code and fee</a>`:`<p>${esc(e.reason || 'Parent eligibility could not be verified.')}</p>`}${list.error?`<p role="alert">${esc(list.error)}</p>`:''}${(list.interactions||[]).map(r=>`<p><a href="${resultURL(r.run_id)}" data-link>${esc(r.run_id)}</a> · ${esc(r.status)} · ${esc(r.analysis?.status || r.failure?.kind || 'Awaiting analysis')}</p>`).join('')}`})}`};
}
const names={parameter_v1:'Parameter effect under V1',parameter_v2:'Parameter effect under V2',code_c0:'Code effect at C0',code_c1:'Code effect at C1',combined:'Combined effect',interaction:'Interaction'};
function stageHTML(a,id,title) {
 const s=a[id];return panel({title,body:`${pill(s?.state || 'not_executed')}<p>${esc(s?.reason || '')}</p>${s?.infrastructure_error?`<p role="alert">${esc(s.infrastructure_error)}</p>`:''}<p>Reconciliation / preservation: ${esc(s?.derived?.reconciled ?? s?.derived?.preservation_verified ?? 'not established')}</p><div class="tech-only">${kv([['Stage input',ident(s?.input_sha256)],['Execution output',ident(s?.execution_sha256)],['Handoff parent',s?.parent?json(s.parent):'Independent retained root']])}</div>`});
}
export function resultHTML(r) {
 const a=r.analysis;
 let body=`<div class="page-head"><h1>Upgrade / parameter interaction.</h1><p>${pill(r.status)} · ${esc(r.run_id)}</p><a href="${parentURL(r.parent_run_id)}" data-link>Parent upgrade run</a></div>`;
 if(r.failure)return body+panel({title:'No analytical conclusion',body:`<p role="alert">${esc(r.failure.kind)}: ${esc(r.failure.detail)}</p><p>The occurrence and accepted references remain retained.</p>`});
 if(!a)return body+panel({title:'Analysis pending',body:'<p>The accepted immutable inputs are queued or running. Refreshing this page reads the same occurrence.</p>'});
 body+=panel({title:'Retained result',body:`${pill(a.status)}<p>${scope}</p><p>${esc(a.limitations)}</p>${previewHTML({...a.facts,source_slot_range:r.binding?.source_slot_range},{pending:false})}`});
 body+=`<h2>Configuration executions.</h2>${stageHTML(a,'k1','K1 · SetFee C1 under V1')}${stageHTML(a,'k2','K2 · SetFee C1 under V2')}`;
 body+=`<h2>Action matrix.</h2><div class="table-wrap"><table class="table"><thead><tr><th>Code</th><th>Observed fee C0</th><th>Proposed fee C1</th></tr></thead><tbody>${[['Historical V1','r00','r01'],['Qualified V2','r10','r11']].map(([title,x,y])=>`<tr><th>${title}</th>${[x,y].map(id=>`<td>${id.toUpperCase()} · ${esc(a[id]?.state)}<p>${esc(a[id]?.reason || '')}</p><p>Recipient credit: ${quantity(a.measurements?.recipient_account_credit_raw?.[['r00','r01','r10','r11'].indexOf(id)])}</p></td>`).join('')}</tr>`).join('')}</tbody></table></div>`;
 body+=['r00','r01','r10','r11'].map(id=>stageHTML(a,id,`${id.toUpperCase()} · action ledger`)).join('');
 body+=panel({title:'Measured ledgers and effects',body:`<p>Signed raw integer values are supplied by the engine. Configuration transaction fees are excluded from action metrics. Unavailable referral splits remain unavailable.</p>${Object.entries(a.measurements||{}).map(([metric,cells])=>`<h3>${esc(metric)}</h3><div class="table-wrap"><table class="table"><thead><tr>${['R00','R01','R10','R11'].map(x=>`<th>${x}</th>`).join('')}</tr></thead><tbody><tr>${cells.map(q=>`<td>${quantity(q)}</td>`).join('')}</tr></tbody></table></div>${kv(Object.entries(names).map(([key,label])=>[label,quantity(a.effects?.[metric]?.[key])]))}`).join('')}`});
 // Wording follows authoritative values, without arithmetic or Number coercion.
 const effect=a.effects?.recipient_account_credit_raw;
 if(effect?.parameter_v1?.value?.startsWith('-') && effect.parameter_v1.value===effect.parameter_v2?.value && effect.interaction?.value==='0')body+=`<p>The fee change reduces recipient-account credit under both code versions. Its measured effect is equal in this retained case. No additional interaction is measured for this quantity.</p>`;
 body+=panel({title:'Portable evidence',body:`${r.artifact_available?`<a class="button button--ghost" href="${resultEndpoint(r.run_id)}/artifact">Download portable artifact</a>`:'<p>No verified portable artifact is available.</p>'}<pre class="code-block">eplyx interaction verify --artifact interaction --format json\neplyx interaction reproduce --artifact interaction --format json</pre><p>After extraction, neither command needs this project or its original bundle directory.</p>`});
 body+=`<div class="tech-only">${panel({title:'Identities and runtime',body:kv([['Upgrade ChangeSpec ID',ident(a.facts?.upgrade_change_spec_id)],['Parameter ChangeSpec ID',ident(a.facts?.parameter_change_spec_id)],['Analysis input ID',ident(a.analysis_input_sha256)],['Report identity',ident(a.report_sha256)],['Hosted input / parent binding',json(r.binding)],['Runtime / Clock / dependencies',json({runtime:a.facts?.runtime,clock:a.facts?.clock,dependencies:a.facts?.dependencies})],['Manager / separate payer assumptions',json({manager:a.facts?.manager_assumption,payer:a.facts?.fee_payer})]])})}</div>`;
 return body;
}
export async function resultPage({params}) {
 document.querySelector('link[data-upgrade-style]')?.remove();
 const id=params[0];let r=await request(resultEndpoint(id));
 return {title:'Upgrade / parameter interaction',html:resultHTML(r),attach(node){let stopped=false,timer;const tick=async()=>{try{r=await request(resultEndpoint(id));if(stopped)return;node.innerHTML=resultHTML(r);if(['queued','running'].includes(r.status)&&!r.failure)timer=setTimeout(tick,1500);}catch(err){if(!stopped)node.innerHTML=panel({title:'Interaction evidence unavailable',body:`<p role="alert">${esc(err.message)}</p>`});}};if(['queued','running'].includes(r.status)&&!r.failure)timer=setTimeout(tick,1500);return()=>{stopped=true;clearTimeout(timer);};}};
}
