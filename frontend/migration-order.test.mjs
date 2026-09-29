import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
globalThis.document = { documentElement:{dataset:{base:'/p/proj_fixture',project:'proj_fixture',cloud:'1'}} };
let source = readFileSync(new URL('./cloud/migration-order.js',import.meta.url),'utf8');
for (const name of ['env','ui']) source = source.replace(`'./${name}.js'`,JSON.stringify(new URL(`./dashboard/${name}.js`,import.meta.url).href));
const { validateSelection,previewHTML,selectionHTML,parentSection,resultHTML,submitSelection,attachSelection } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const e = {eligible:true,units:[{source_account:'source-a',owner:'owner-a',amount_raw:'9007199254740993',quote:{output_raw:'4503599627370496'},authority:{kind:'Available'}},{source_account:'source-b',owner:'owner-b',amount_raw:'120',quote:{output_raw:'60'}}],change_spec_id:'change-id',world_id:'world-id',clock:{slot:'1000',unix_timestamp:'1760000000'},runtime_id:'runtime-id'};
assert.equal(validateSelection(e,'source-a','source-b'),null);
assert.match(validateSelection(e,'source-a','source-a'),/distinct/);
assert.match(validateSelection(e,'source-a','unknown'),/eligible retained/);
assert.match(validateSelection(e,'','source-b'),/Select/);
assert.match(selectionHTML(e),/Source A/); assert.match(selectionHTML(e),/Source B/);
assert.match(selectionHTML(e),/9007199254740993/); assert.doesNotMatch(selectionHTML(e),/<input/);
const unavailable = {eligible:false,reason:'Retained Clock cannot be verified.'};
assert.doesNotMatch(selectionHTML(unavailable),/<form/); assert.match(selectionHTML(unavailable),/Retained Clock/);
assert.match(parentSection('run_parent',e,[]),/>Order Analysis</);
assert.doesNotMatch(parentSection('run_parent',unavailable,[]),/>Order Analysis</);
const preview = previewHTML(e,'source-a','source-b');
for (const wording of ['Same proposal','Same starting state','Same Clock/runtime','A alone','B alone','A → B','B → A','No time advancement','No state refresh','two operation instances']) assert.match(preview,new RegExp(wording));
assert.doesNotMatch(preview,/succeeded|rejected|winner|SharedReserveChangesSuccessfulUnit/);
let request,navigated;
await submitSelection({parent:'run_parent',eligibility:e,sourceA:'source-a',sourceB:'source-b',call:async(path,options)=>{request={path,options};return {run_id:'run_child'};},navigate:url=>{navigated=url;}});
assert.equal(request.path,'/v1/projects/proj_fixture/runs/run_parent/migration-order');
assert.deepEqual(JSON.parse(request.options.body),{source_a:'source-a',source_b:'source-b'});
assert.equal(navigated,'/p/proj_fixture/migration-orders/run_child');
await assert.rejects(submitSelection({parent:'run_parent',eligibility:e,sourceA:'source-a',sourceB:'source-a'}),/distinct/);
// Real form wiring: a changed pair invalidates its preview and failed requests
// keep selection available for correction without navigating to old evidence.
const handlers = {}; const previewNode = {innerHTML:''},errorNode={textContent:''},submitNode={disabled:true},previewButton={addEventListener:(event,fn)=>{handlers.preview=fn;}};
const form = {elements:{source_a:{value:'source-a'},source_b:{value:'source-b'}},querySelector:selector=>({'[data-order-preview]':previewNode,'[data-order-error]':errorNode,'[data-order-submit]':submitNode,'[data-preview]':previewButton})[selector],addEventListener:(event,fn)=>{handlers[event]=fn;}};
attachSelection({querySelector:()=>form},'run_parent',e);
handlers.preview(); assert.equal(submitNode.disabled,false); assert.match(previewNode.innerHTML,/Four analyses/);
form.elements.source_b.value='source-a'; handlers.change(); assert.equal(submitNode.disabled,true); assert.equal(previewNode.innerHTML,'');assert.match(errorNode.textContent,/distinct/);
form.elements.source_b.value='source-b'; handlers.preview();
globalThis.fetch = async()=>({ok:false,status:400,json:async()=>({error:'Engine unsupported composition.'})});
await handlers.submit({preventDefault(){}}); assert.equal(submitNode.disabled,false); assert.match(errorNode.textContent,/unsupported/);
const step = (unit,outcome,before,after) => ({before_state_id:before,after_state_id:after,observed_reserve_before_raw:'80',observed_reserve_after_raw:outcome==='Migrated'?'30':'30',execution:{unit_id:unit,outcome,reconciled:outcome==='Migrated',failure:outcome==='Rejected'?{error_name:'InsufficientReserve',rollback_verified:true}:null}});
function result(status) {
 const scenarios = [[step('unit-a','Migrated','s0','sa')],[step('unit-b','Migrated','s0','sb')],[step('unit-a','Migrated','s0','sa'),step('unit-b','Rejected','sa','sab')],[step('unit-b','Migrated','s0','sb'),step('unit-a','Rejected','sb','sba')]].map((steps,i)=>({steps,initial_state_id:'s0',final_state_id:'final'+i,case_id:'case'+i,run_id:'scenario'+i,stopped:null}));
 return {run_id:'run_child',parent_run_id:'run_parent',status:'completed',artifact_available:true,failure:null,analysis:{binding:{units:[{...e.units[0],unit_id:'unit-a'},{...e.units[1],unit_id:'unit-b'}],change_spec_id:'change-id',candidate:{sha256:'candidate-id'},world_id:'world-id',world_content_sha256:'world-content',clock:e.clock,reserve:'reserve-id',initial_reserve_raw:'80',runtime_id:'runtime-id',closure:['source-a','source-b']},binding_id:'pair-id',scenarios,states:{s0:{accounts:{destination:{kind:'KnownAbsent'}}}},comparison:{status,limitations:['Two selected units only.'],order_case_ids:['case2','case3'],scenario_ids:['scenario0','scenario1','scenario2','scenario3']}}};
}
for (const status of ['SharedReserveChangesSuccessfulUnit','NoSuccessfulUnitEffect','NotEstablished']) {
 const html=resultHTML(result(status));
 assert.match(html,new RegExp(status));
 for (const text of ['A alone','B alone','A → B','B → A','Parent migration run','No state refresh','intermediate state','Pair binding ID','KnownAbsent','Download portable order artifact','eplyx migration reproduce-order']) assert.match(html,new RegExp(text));
 assert.doesNotMatch(html,/\b(unfair|unsafe|vulnerable|exploitable|fairness)\b|undefined|NaN|\[object Object\]/i);
 // Overview/Technical are CSS presentations of the very same result.
 assert.match(html,/tech-only/); assert.match(html,/pair-id/); assert.match(html,/scenario2/);
}
assert.match(resultHTML(result('SharedReserveChangesSuccessfulUnit')),/changed which migration unit succeeded/);
assert.match(resultHTML(result('NoSuccessfulUnitEffect')),/no change in which selected/);
assert.match(resultHTML(result('NotEstablished')),/did not establish/);
for (const kind of ['EvidenceGap','HandoffFailure','UnsupportedComposition','InternalExecutionFailure']) {
 const html=resultHTML({run_id:'run_child',parent_run_id:'run_parent',status:'failed',analysis:null,failure:{kind,detail:'Retained evidence insufficient.'}});
 assert.match(html,new RegExp(kind)); assert.match(html,/No analytical conclusion/); assert.doesNotMatch(html,/Comparison|changed which/);
}
const x=result('NoSuccessfulUnitEffect'); x.analysis.binding.units[0].owner='<img src=x onerror=alert(1)>';
assert.match(resultHTML(x),/&lt;img/); assert.doesNotMatch(resultHTML(x),/<img/);
const pages=readFileSync(new URL('./dashboard/pages.js',import.meta.url),'utf8'); assert.match(pages,/CLOUD && !DEMO[\s\S]*withOrders/);
console.log('Hosted order frontend: eligibility, picker, preview invalidation, submission/navigation, four scenarios, authoritative classifications, technical evidence, failures and retained-state wording passed.');
