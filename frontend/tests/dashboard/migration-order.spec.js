import {test,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const origin='http://127.0.0.1:4188',base='/p/proj_fixture';
const {reference:parent}=JSON.parse(readFileSync(new URL('../../../fixtures/dashboard/ids.json',import.meta.url)));
const e={eligible:true,parent_run_id:parent,change_spec_id:'change',world_id:'world',runtime_id:'runtime',clock:{slot:'1000',unix_timestamp:'1760000000'},units:[{source_account:'source-a',owner:'owner-a',amount_raw:'100',quote:{output_raw:'50'}},{source_account:'source-b',owner:'owner-b',amount_raw:'120',quote:{output_raw:'60'}}]};
const step=(unit_id,outcome,before,after)=>({before_state_id:before,after_state_id:after,observed_reserve_before_raw:'80',observed_reserve_after_raw:'30',execution:{unit_id,outcome,reconciled:outcome==='Migrated',failure:outcome==='Rejected'?{error_name:'InsufficientReserve',rollback_verified:true}:null}});
const r={run_id:'run_child',status:'completed',parent_run_id:parent,artifact_available:true,failure:null,analysis:{binding:{units:e.units.map((u,i)=>({...u,unit_id:['a','b'][i]})),clock:e.clock,change_spec_id:'change',world_id:'world',world_content_sha256:'content',candidate:{sha256:'candidate'},runtime_id:'runtime',reserve:'reserve',initial_reserve_raw:'80',closure:['source-a','source-b']},binding_id:'binding',states:{initial:{accounts:{destination:{kind:'KnownAbsent'}}}},scenarios:[[step('a','Migrated','initial','a')],[step('b','Migrated','initial','b')],[step('a','Migrated','initial','a'),step('b','Rejected','a','ab')],[step('b','Migrated','initial','b'),step('a','Rejected','b','ba')]].map((steps,i)=>({steps,case_id:'case'+i,run_id:'scenario'+i,initial_state_id:'initial',final_state_id:'final'+i,stopped:null})),comparison:{status:'SharedReserveChangesSuccessfulUnit',limitations:['Two selected units only.'],order_case_ids:['case2','case3']}}};
async function setup(page,{eligible=true,status='SharedReserveChangesSuccessfulUnit'}={}) {
 page.on('pageerror',error=>{throw error;});
 await page.route('**/p/proj_fixture/**',route=>route.fulfill({contentType:'text/html',body:readFileSync(new URL('../../dashboard/index.html',import.meta.url),'utf8').replace('data-mode="overview"',`data-mode="overview" data-base="${base}" data-api="/api" data-project="proj_fixture" data-cloud="1"`)}));
 for(const name of ['migration-order','settings']) await page.route(`**/assets/${name}.js`,route=>route.fulfill({contentType:'text/javascript',body:readFileSync(new URL(`../../cloud/${name}.js`,import.meta.url),'utf8')}));
 await page.route('**/v1/projects/proj_fixture/runs/**/migration-order/eligibility',route=>route.fulfill({json:eligible?e:{eligible:false,reason:'Retained world is unavailable.',units:[]}}));
 await page.route('**/v1/projects/proj_fixture/runs/**/migration-order',async route=>{
  if(route.request().method()==='POST') {expect(route.request().postDataJSON()).toEqual({source_a:'source-a',source_b:'source-b'});await route.fulfill({status:202,json:{run_id:'run_child',status:'queued'}});}
  else await route.fulfill({json:{orders:[]}});
 });
 await page.route('**/v1/projects/proj_fixture/migration-orders/run_child',route=>route.fulfill({json:{...r,analysis:{...r.analysis,comparison:{...r.analysis.comparison,status}}}}));
}
test('retained source picker previews four scenarios and navigates to the child',async({page})=>{
 await setup(page);await page.goto(`${origin}${base}/runs/${parent}`);
 await page.getByRole('link',{name:'Order Analysis',exact:true}).click();
 await expect(page.getByRole('button',{name:'Submit order analysis'})).toBeDisabled();
 await page.getByRole('combobox',{name:'Source A',exact:true}).selectOption('source-a');await page.getByRole('combobox',{name:'Source B',exact:true}).selectOption('source-a');
 await expect(page.locator('[data-order-error]')).toContainText('distinct');
 await page.getByRole('combobox',{name:'Source B',exact:true}).selectOption('source-b');await page.getByRole('button',{name:'Preview bounded comparison'}).click();
 await expect(page.locator('[data-order-preview]')).toContainText('A alone · B alone · A → B · B → A');
 await expect(page.locator('[data-order-preview]')).not.toContainText('succeeded');
 await page.getByRole('button',{name:'Submit order analysis'}).click();await expect(page).toHaveURL(new RegExp('/migration-orders/run_child$'));
 await expect(page.locator('main')).toContainText('changed which migration unit succeeded');
 await expect(page.getByRole('heading',{name:'A → B.',exact:true})).toBeVisible();
 await expect(page.getByRole('heading',{name:'B → A.',exact:true})).toBeVisible();
 await expect(page.locator('main')).toContainText('No state refresh occurs');
 await page.getByRole('button',{name:'Technical',exact:true}).click();
 await expect(page.locator('main')).toContainText('Pair binding ID');await expect(page.getByRole('link',{name:'Parent migration run',exact:true}).first()).toHaveAttribute('href',`${base}/runs/${parent}`);
 const text=await page.locator('main').innerText();expect(text).not.toMatch(/\b(unfair|unsafe|vulnerable|exploitable|fairness)\b/i);
 await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
});
for(const status of ['NoSuccessfulUnitEffect','NotEstablished']) test(`renders authoritative ${status}`,async({page})=>{await setup(page,{status});await page.goto(`${origin}${base}/migration-orders/run_child`);await expect(page.locator('main')).toContainText(status);await expect(page.getByRole('heading',{name:'A alone.',exact:true})).toBeVisible();});
test('ineligible parent shows the factual unavailable reason',async({page})=>{await setup(page,{eligible:false});await page.goto(`${origin}${base}/runs/${parent}`);await expect(page.locator('#order-analyses')).toContainText('Retained world is unavailable');await expect(page.getByRole('link',{name:'Order Analysis',exact:true})).toHaveCount(0);});
