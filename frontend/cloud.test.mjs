// The served cloud assets share /assets with dashboard modules. Resolve those
// same imports locally while rendering their pure presentation functions.
import assert from 'node:assert/strict';
import {readFileSync,readdirSync} from 'node:fs';
globalThis.document={documentElement:{dataset:{base:'/p/proj_fixture',project:'proj_fixture',cloud:'1'}}};
globalThis.localStorage={getItem:()=>null,setItem(){}};
let source=readFileSync(new URL('./cloud/proposal.js',import.meta.url),'utf8');
for(const name of ['env','ui'])source=source.replace(`'./${name}.js'`,JSON.stringify(new URL(`./dashboard/${name}.js`,import.meta.url).href));
const {proposalHTML,candidateResult,preflightResult,stressResult}=await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const banned=/\b(safe|unsafe|approved by eplyx|can never|will never)\b|no (economic )?impact (was )?(detected|found)|no changes detected|risk score/i;
const attack='<img src=x onerror=alert(1)>';
const htmls=[proposalHTML(),candidateResult({status:'Proven',amount_raw:'18446744073709551615',replacement_mint:attack,execution_performed:true,execution:{reconciled:true},official_transition:'NotTested'}),preflightResult({preparation_status:'Prepared',successor_verification:{status:'MintObserved'},replacement_conversion:{status:'NotTested'},views:[{id:'BeforeProposedEvent',readiness:null},{id:'ProposedActive',readiness:{mobility:{status:'Ready'},candidate_plan:{status:'Incomplete'},full_transition:{status:'Incomplete'}}}]}),stressResult({report:{coverage:{exact_accounts_selected:2,exact_accounts_executed:1,exact_accounts_proven:1,population_rollout_readiness:'Incomplete'},official_transition:'NotTested'}})];
for(const html of htmls){assert.doesNotMatch(html.replace(/<[^>]*>/g,' '),banned);assert.doesNotMatch(html,/undefined|NaN|\[object Object\]|<img/);}
assert.match(htmls[1],/18446744073709551615/);assert.match(htmls[1],/&lt;img/);assert.match(htmls[2],/Before event; not evaluated/);assert.match(htmls[3],/No peer inherits evidence/);
for(const name of readdirSync(new URL('./cloud/',import.meta.url)).filter(n=>n.endsWith('.js'))){const text=readFileSync(new URL(`./cloud/${name}`,import.meta.url),'utf8');assert.doesNotMatch(text,banned,name);assert.doesNotMatch(text,/<input[^>]*type=["']file/i,name);}
console.log('Cloud presentation: exact amounts, escaping, separate assurance, neutral claims and terms-only forms verified.');
