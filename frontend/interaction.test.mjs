// Presentation/form tests use labelled mock HTTP responses, not VM qualification.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
globalThis.document={documentElement:{dataset:{base:'/p/proj_fixture',project:'proj_fixture',cloud:'1'}}};
let source=readFileSync(new URL('./cloud/interaction.js',import.meta.url),'utf8');
for(const name of ['env','ui'])source=source.replace(`'./${name}.js'`,JSON.stringify(new URL(`./dashboard/${name}.js`,import.meta.url).href));
source=source.replace("'/assets/capabilities.js'",JSON.stringify(new URL('./src/capabilities.js',import.meta.url).href));
const {parseProposal,previewHTML,selectionHTML,attachSelection,resultHTML}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const proposal=JSON.parse(readFileSync(new URL('../docs/examples/stake-pool-parameter-change.json',import.meta.url)));
const ready={kind:'upgrade_parameter_interaction',status:'ready',supported:true,can_submit:true,missing:[]};
const facts={upgrade_change_spec_id:'upgrade-id',parameter_change_spec_id:'parameter-id',analysis_input_sha256:'analysis-id',record_id:'record',pool:'pool',slot:'447850493',v1:{sha256:'historical'},v2:{sha256:'constructed',profile:{id:'constructed-step10b-config-deposit-v1'}},operation:proposal.change.operation,upgrade_change_spec:{change:{kind:'program_upgrade'}},parameter_change_spec:proposal,clock:{epoch:'0'},runtime:{revision:'pinned'},manager_assumption:{origin:'assumed_simulation_only'},fee_payer:{origin:'assumed_simulation_only'}};
const eligible={eligible:true,records:[facts]};
assert.deepEqual(parseProposal(JSON.stringify(proposal)),proposal);assert.throws(()=>parseProposal('{}'),/ChangeSpec/);assert.throws(()=>parseProposal('bad'));
assert.match(selectionHTML(eligible,ready),/value="">Select/);assert.match(selectionHTML(eligible,null),/controls remain unavailable/);assert.doesNotMatch(selectionHTML({eligible:false,reason:'Evidence missing.'},ready),/<form/);
for(const s of ['upgrade-id','parameter-id','447850493','K1','K2','epoch 0','test code','current and proposed rational'])assert.ok(previewHTML(facts).includes(s),s);
// Stateful form mocks test async invalidation, errors and exact key retries.
const handlers={};const record={value:''},text={value:JSON.stringify(proposal)},preview={innerHTML:''},error={textContent:''},submit={disabled:true},button={addEventListener:(n,fn)=>handlers.preview=fn};
const file={files:[],addEventListener:(n,fn)=>handlers.file=fn};
const form={elements:{record_id:record,proposal:text,file},querySelector:s=>({'[data-interaction-preview]':preview,'[data-interaction-error]':error,'[data-submit]':submit,'[data-preview]':button})[s],addEventListener:(n,fn)=>handlers[n]=fn};
let fail=false,posted=[],destination,resolvePreview;
const call=async(path,options)=>{if(path.endsWith('/preview')){if(resolvePreview)return new Promise(resolve=>resolvePreview(resolve));return facts;}posted.push(JSON.parse(options.body));if(fail)throw new Error('Server admission rejected.');return {run_id:'run_child'};};
attachSelection({querySelector:()=>form},'run_parent',eligible,ready,{call,key:()=> 'stable-request-key',navigate:url=>destination=url});
await handlers.preview();assert.match(error.textContent,/explicit/);assert.equal(submit.disabled,true);
record.value='record';await handlers.preview();assert.equal(submit.disabled,false);
fail=true;await handlers.submit({preventDefault(){}});assert.equal(text.value,JSON.stringify(proposal));assert.match(error.textContent,/rejected/);assert.equal(submit.disabled,false);fail=false;await handlers.submit({preventDefault(){}});assert.deepEqual(posted[0],posted[1]);assert.equal(destination,'/p/proj_fixture/interactions/run_child');
handlers.input();assert.equal(submit.disabled,true);assert.equal(preview.innerHTML,'');
let release;resolvePreview=r=>{release=r;};const pending=handlers.preview();handlers.input();release(facts);await pending;assert.equal(submit.disabled,true);resolvePreview=null;
const q=value=>({value,unavailable_reason:value==null?'No independent referral split':null});
const stage={state:'verified',input_sha256:'stage-in',execution_sha256:'stage-out',parent:null,derived:{reconciled:true}};
const analysis={facts,status:'no_measured_interaction',limitations:'retained only',analysis_input_sha256:'analysis-id',report_sha256:'report-id',k1:stage,k2:stage,r00:stage,r01:{...stage,parent:{stage:'K1',pool_sha256:'pool-k1'}},r10:stage,r11:{...stage,parent:{stage:'K2',pool_sha256:'pool-k2'}},measurements:{recipient_account_credit_raw:[q('760985008'),q('753375157'),q('760985008'),q('753375157')],referral_account_credit_raw:[q(null),q(null),q(null),q(null)],huge:[q('-900719925474099312345'),q('900719925474099312345'),q(null),q(null)]},effects:{recipient_account_credit_raw:{parameter_v1:q('-7609851'),parameter_v2:q('-7609851'),code_c0:q('0'),code_c1:q('0'),combined:q('-7609851'),interaction:q('0')}}};
const result={run_id:'run_child',parent_run_id:'run_parent',status:'completed',artifact_available:true,analysis,binding:{upgrade_change_spec_id:'upgrade-id',parameter_change_spec_id:'parameter-id'}};
const html=resultHTML(result);for(const s of ['K1 ·','K2 ·','R00','R01','R10','R11','-7609851','equal in this retained case','No additional interaction','No independent referral split','-900719925474099312345','upgrade-id','parameter-id','pool-k1','pool-k2','/v1/projects/proj_fixture/interactions/run_child/artifact','tech-only'])assert.ok(html.includes(s),s);
assert.doesNotMatch(html,/undefined|\bNaN\b|\[object Object\]|safe upgrade|deployment approved|no impact/i);
for(const state of ['rejected','unreconciled','unavailable','not_executed']){const copy=structuredClone(result);copy.analysis.r11.state=state;copy.analysis.r11.reason='Mock presentation branch';copy.analysis.measurements.recipient_account_credit_raw[3]=q(null);copy.analysis.effects.recipient_account_credit_raw.interaction=q(null);assert.match(resultHTML(copy),/Mock presentation branch/);assert.doesNotMatch(resultHTML(copy),/No additional interaction/);}
const failed=resultHTML({...result,failure:{kind:'evidence_integrity',detail:'Missing retained evidence'}});assert.match(failed,/No analytical conclusion/);assert.doesNotMatch(failed,/Measured ledgers/);
assert.match(resultHTML({...result,analysis:null,status:'queued'}),/same occurrence/);
console.log('Hosted interaction: structural import, explicit selection, exact preview, async invalidation, input retention, stable retry, navigation, matrix, stage handoffs, raw signed integers and unavailable quantities passed (mock presentation).');
