import { BASE, PROJECT } from './env.js';
import { esc, panel, kv, ident, raw } from './ui.js';
import { fetchProjectCapabilities, capabilityFor, canSubmitCapability, capabilityHTML } from '/assets/capabilities.js';
import { eligibilityEndpoint, parseEligibility, validateProposedBps, buildParameterSpec, submitParameter, parameterSubmissionError } from './parameter-model.js';
export const comparisonContract='Eplyx will execute the same retained TransferChecked independently against the observed mint and a derived mint whose active fee rate is changed.';
export const retainedScope='Captured current-state counterfactual for one exact retained transfer. No SetTransferFee instruction, fee-authority possession or on-chain activation is established. A newer state requires a new retained transfer run.';
export async function request(path,options={}) {
 const response=await fetch(path,{credentials:'same-origin',...options});
 const body=await response.json().catch(()=>({}));
 if(!response.ok){const error=new Error(body.error||`Request failed (${response.status})`);error.body=body;throw error;}
 return body;
}
const technical = e => `<div class="tech-only">${kv([['Operation',esc(e.operation)],['Mint data SHA-256',ident(e.mint_data_sha256)],['Retained run',ident(e.run_id)],['Capture SHA-256',ident(e.capture_sha256)],['Token-2022 ELF SHA-256',ident(e.token_2022_elf_sha256)],['Exact raw amount',raw(e.transfer.amount_raw)],['Mint decimals',raw(e.transfer.decimals)]])}</div>`;
export function observedHTML(e) {
 return panel({id:'parameter-observed',title:'Current observed configuration',body:`${kv([['Mint',ident(e.mint,'addr')],['Current fee',`${raw(e.current_basis_points)} bps`],['Maximum fee (raw)',raw(e.maximum_fee_raw)],['Active schedule epoch',raw(e.schedule_epoch)],['Captured Clock epoch',raw(e.captured_epoch)],['Transfer amount',`${raw(e.transfer.amount_raw)} raw units`],['Source',ident(e.transfer.source,'addr')],['Destination',ident(e.transfer.destination,'addr')],['Program code','Same retained Token-2022 code']])}${technical(e)}<p>${retainedScope}</p>`});
}
export function previewHTML(e,value) {
 const spec=buildParameterSpec(e,value);
 return panel({title:'Parameter Change preview',body:`${kv([['Configuration',`${raw(e.current_basis_points)} bps → ${raw(spec.change.operation.proposed_basis_points)} bps`],['Maximum fee (raw)',raw(e.maximum_fee_raw)],['Schedule epoch',raw(e.schedule_epoch)],['Captured epoch',raw(e.captured_epoch)],['Exact transfer amount',`${raw(e.transfer.amount_raw)} raw units`],['Source',ident(e.transfer.source,'addr')],['Destination',ident(e.transfer.destination,'addr')],['Token-2022 program',ident(e.program_id,'addr')]])}<p>${comparisonContract}</p><p>Program code unchanged; captured state and interaction fixed. The measured transfer result will come from engine execution.</p><p><strong>Ready to submit for analysis.</strong></p><div class="tech-only"><h3>Exact ChangeSpec</h3><pre>${esc(JSON.stringify(spec,null,2))}</pre><p>The ChangeSpec ID is verified by the server and shown in the accepted run.</p></div>`});
}
export function formHTML(e,capability) {
 if(!e.eligible)return panel({title:'Parameter Change unavailable',body:`<p>${esc(e.reason)}</p><code>${esc(e.reason_code)}</code>`});
 return `${observedHTML(e)}${panel({title:'Proposed declaration',body:`${capabilityHTML(capability,{compact:true})}<form data-parameter-form class="analysis-form" novalidate><label class="analysis-field"><span>Proposed transfer fee (basis points)</span><input name="proposed_basis_points" type="text" inputmode="numeric" pattern="[0-9]+" required autocomplete="off" aria-describedby="parameter-range"></label><p id="parameter-range">Integer 0–10000 bps. An equal-rate no-op is allowed. Only the proposed basis-point rate is editable.</p><button type="button" class="button button--ghost" data-parameter-preview-button>Preview Parameter Change</button><div data-parameter-preview aria-live="polite"></div><p data-parameter-error role="alert"></p><button type="submit" class="button button--primary" data-parameter-submit disabled>Submit for analysis</button></form>`})}`;
}
export function attachForm(node,e,capability,{call=request,navigate=url=>window.dashboardNavigate(url),key=()=>crypto.randomUUID()}={}) {
 const form=node.querySelector('[data-parameter-form]');if(!form)return;
 const input=form.elements.proposed_basis_points, preview=form.querySelector('[data-parameter-preview]'), error=form.querySelector('[data-parameter-error]'), submit=form.querySelector('[data-parameter-submit]');
 let previewed=null,busy=false;
 const ready=canSubmitCapability(capability);
 form.addEventListener('input',()=>{previewed=null;preview.innerHTML='';submit.disabled=true;error.textContent=validateProposedBps(input.value)||'';});
 form.querySelector('[data-parameter-preview-button]').addEventListener('click',()=>{
  const problem=validateProposedBps(input.value);error.textContent=problem||'';if(problem)return;
  previewed={value:input.value,requestKey:key()};preview.innerHTML=previewHTML(e,input.value);submit.disabled=!ready;
 });
 form.addEventListener('submit',async event=>{
  event.preventDefault();if(busy)return;
  if(!ready){error.textContent='The project is not ready to submit this analysis.';return;}
  if(!previewed || previewed.value!==input.value){error.textContent='Preview the proposed rate before submission.';return;}
  const selected=previewed;busy=true;submit.disabled=true;input.disabled=true;
  try{await submitParameter({eligibility:e,value:selected.value,requestKey:selected.requestKey,call,navigate});}
  catch(failure){error.textContent=parameterSubmissionError(failure);}
  finally{busy=false;input.disabled=false;submit.disabled=!ready||previewed!==selected;}
 });
}
export async function parameterPage({params}) {
 const parent=params[0];
 const [e,capabilities]=await Promise.all([request(eligibilityEndpoint(PROJECT,parent)).then(v=>parseEligibility(v,PROJECT,parent)),fetchProjectCapabilities(PROJECT,path=>request(path))]);
 const capability=capabilityFor(capabilities,'protocol_parameter_change');
 return {title:'Parameter Change',crumbs:[['Retained transfer',`/runs/${parent}`],['Parameter Change']],html:`<div class="page-head"><h1>Parameter Change.</h1><a href="${BASE}/runs/${encodeURIComponent(parent)}" data-link>Retained transfer run</a></div>${formHTML(e,capability)}`,attach:node=>attachForm(node,e,capability)};
}
export async function withParameterEntry(view,detail) {
 if(!detail.hosted)return view;
 try {
  const e=parseEligibility(await request(eligibilityEndpoint(PROJECT,detail.id)),PROJECT,detail.id);
  const section=e.eligible?`<p>Compare one proposed active transfer-fee rate for this exact retained TransferChecked. Same program code and captured state.</p><a class="button button--ghost" href="${BASE}/runs/${encodeURIComponent(detail.id)}/parameter-change" data-link>Parameter Change</a>`:`<p>${esc(e.reason)}</p><code class="tech-only">${esc(e.reason_code)}</code>`;
  return {...view,html:view.html+panel({title:'Parameter Change',body:section})};
 }catch(error){return {...view,html:view.html+panel({title:'Parameter eligibility unavailable',body:`<p>${esc(error.message)}</p>`})};}
}
// Reuse the existing run page. Refresh can always recover a durable accepted job.
export function withParameterPolling(view,detail,load,render) {
 if(!['queued','running'].includes(detail.hosted?.status))return view;
 return {...view,attach(node){view.attach?.(node);async function poll(){if(!node.isConnected)return;try{const fresh=await load();if(!node.isConnected)return;node.innerHTML=render(fresh).html;if(['queued','running'].includes(fresh.hosted?.status))setTimeout(poll,1500);}catch(error){if(node.isConnected){node.innerHTML=panel({title:'Run evidence unavailable',body:`<p>${esc(error.message)}</p><p>The accepted run remains durable. Reload to view its recorded status.</p>`});}}}setTimeout(poll,1500);}};
}
