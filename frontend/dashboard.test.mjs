// MAIN-style view assertions over dashboard::view exports of the real fixtures.
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
globalThis.document={documentElement:{dataset:{mode:'overview'}}};
globalThis.localStorage={getItem:()=>null,setItem(){}};
const pages=await import('./dashboard/pages.js');
const ui=await import('./dashboard/ui.js');
const { analyticalDetail }=await import('./dashboard/analytical.js');
const { TokenTransitionsPage,TokenTransitionsSection }=await import('./src/transitions.js');
const load=path=>JSON.parse(readFileSync(new URL(path,import.meta.url)));
const models=load('../fixtures/dashboard/view-models.json').projects;
const ids=load('../fixtures/dashboard/ids.json');
const visible=html=>html.replace(/<[^>]*>/g,' ').replace(/\s+/g,' ');
const banned=/\b(safe|unsafe|approved by eplyx|can never|will never)\b|no (economic )?impact (was )?(detected|found)|no changes detected|risk score/i;
let checked=0;
const check=(html,name)=>{assert.doesNotMatch(visible(html),banned,name);assert.doesNotMatch(html,/undefined|\[object Object\]|NaN/,name);checked++;};
for (const [name,responses] of Object.entries(models)) {
 globalThis.fetch=async path=>{const value=responses[path];assert.ok(value,`${name}: unexpected fetch ${path}`);return {ok:true,status:200,json:async()=>structuredClone(value)};};
 const project=responses['/api/project'];
 for (const [method,query] of [['overview',''],['runs',''],['counterexamples',''],['production',''],['invariants',''],['gate',''],['projectPage','']]) {
  const result=await pages[method]({project,query:new URLSearchParams(query)});check(result.html,`${name}/${method}`);
 }
 for (const run of responses['/api/runs'].runs) {
  const result=await pages.runDetail({params:[run.id]});check(result.html,`${name}/${run.id}`);
  const detail=responses[`/api/runs/${run.id}`];
  if (run.kind==='lifecycle_change') {
   assert.match(result.html,/Active/);assert.match(result.html,/Expired/);assert.match(result.html,/Not evaluated/);assert.match(result.html,/1,234,567/);assert.match(result.html,/not added to entity totals/);
  } else if (run.kind==='current_observation') {
   for (const path of detail.analysis.report.paths) assert.match(result.html,new RegExp(ui.words(path.path)));
   assert.equal((result.html.match(/data-status="NotTested"/g)??[]).length,6);
   assert.doesNotMatch(visible(result.html),/Passed|Failed/);
  } else {
   assert.equal((result.html.match(/class="answer answer--/g)??[]).length,8);
   assert.match(result.html,/until slot 900000 \(exclusive\)/);
   assert.match(result.html,/\(ui basis\)/);
  }
 }
 for (const cx of responses['/api/counterexamples'].counterexamples) check((await pages.counterexampleDetail({params:[cx.id]})).html,`${name}/${cx.id}`);
 for (const path of Object.keys(responses).filter(k=>k.startsWith('/api/compare?'))) check((await pages.compare({query:new URLSearchParams(path.split('?')[1])})).html,`${name}/comparison`);
}
for(const html of [TokenTransitionsPage(),TokenTransitionsSection()])check(html,'public transition page');
assert.equal(ui.raw('18446744073709551615'),'18,446,744,073,709,551,615');
assert.equal(ui.count(undefined),'—');assert.equal(ui.count(null),'—');
assert.match(ui.pill('Unknown'),/Cannot be judged/);assert.match(ui.pill('OperatorSupplied'),/Declared only/);
assert.equal(ui.tone('Unsupported'),'neutral');assert.equal(ui.tone('NotTested'),'unknown');
assert.doesNotMatch(ui.ident('<img src=x onerror=alert(1)>'),/<img/);
const stored=models['analytical-kinds']['/api/runs'].runs.find(r=>r.kind==='current_observation');
const malformed=structuredClone(models['analytical-kinds'][`/api/runs/${stored.id}`]);
malformed.state='Unreadable';malformed.problems=['<img src=x>'];
const hidden=analyticalDetail(malformed).html;assert.doesNotMatch(hidden,/<img/);assert.doesNotMatch(hidden,/Paths remain separate/);
const walletDetail=structuredClone(models['analytical-kinds'][`/api/runs/${stored.id}`]);
walletDetail.analysis.report.wallet_observation={public_owner:'public-owner',token_accounts:[{address:'source-account',state:{raw_balance:'9007199254740993',account_state:'Initialized'}}]};
const walletHTML=analyticalDetail(walletDetail).html;assert.match(walletHTML,/9,007,199,254,740,993/);assert.match(walletHTML,/Initialized/);assert.doesNotMatch(walletHTML,/\[object Object\]/);
const failedDetail=analyticalDetail({kind:'current_path',state:'ExecutionError',hosted:{detail:'Worker stopped; no result'}}).html;assert.match(failedDetail,/Worker stopped; no result/);assert.doesNotMatch(failedDetail,/has not finished/);
const upgradeOverview=analyticalDetail({id:'run_upgrade',kind:'program_upgrade',state:'Complete',status:'failed'}).html;assert.match(upgradeOverview,/Open program-upgrade report/);assert.doesNotMatch(upgradeOverview,/Paths remain separate|Public owner/);
const css=readFileSync(new URL('./dashboard/dashboard.css',import.meta.url),'utf8');
const outside=css.replace(/:root\s*\{[^}]*\}/g,'');
assert.doesNotMatch(outside,/#(?:[a-f\d]{3,8})\b|\brgba?\(|\bhsla?\(/i);
assert.doesNotMatch(css,/https?:/);
for(const file of readdirSync(new URL('./dashboard/',import.meta.url)).filter(f=>/\.(js|html)$/.test(f))) {
 const source=readFileSync(new URL(`./dashboard/${file}`,import.meta.url),'utf8');
 assert.doesNotMatch(source,banned,file);
}
console.log(`Dashboard: ${checked} rendered views, exact quantities, glossary, escaping and role-token palette verified.`);
