// Mock hosted HTTP at the already-qualified engine report boundary.
import {test,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const ROOT='http://127.0.0.1:4190',project='proj_interaction',parent='run_upgrade',child='run_interaction';
const proposal=JSON.parse(readFileSync(new URL('../../../docs/examples/stake-pool-parameter-change.json',import.meta.url)));
const facts={upgrade_change_spec_id:'upgrade-id',parameter_change_spec_id:'parameter-id',record_id:'retained-record',pool:'retained-pool',slot:'447850493',v1:{sha256:'historical'},v2:{sha256:'constructed',profile:{id:'constructed-step10b-config-deposit-v1'}},operation:proposal.change.operation,upgrade_change_spec:{schema_version:1,change:{kind:'program_upgrade'}},parameter_change_spec:proposal,clock:{epoch:'0'},runtime:{revision:'pinned'},manager_assumption:{origin:'assumed_simulation_only'},fee_payer:{origin:'assumed_simulation_only'}};
const q=value=>({value,unavailable_reason:value==null?'No independent referral split':null});
const stage={state:'verified',input_sha256:'input',execution_sha256:'output',parent:null,derived:{reconciled:true}};
const result={kind:'upgrade_parameter_interaction',run_id:child,parent_run_id:parent,status:'completed',artifact_available:true,failure:null,binding:{upgrade_change_spec_id:'upgrade-id',parameter_change_spec_id:'parameter-id'},analysis:{facts,status:'no_measured_interaction',limitations:'Retained scope only.',analysis_input_sha256:'input-id',report_sha256:'report-id',k1:stage,k2:stage,r00:stage,r01:{...stage,parent:{stage:'K1',pool_sha256:'pool-k1'}},r10:stage,r11:{...stage,parent:{stage:'K2',pool_sha256:'pool-k2'}},measurements:{recipient_account_credit_raw:[q('760985008'),q('753375157'),q('760985008'),q('753375157')],referral_account_credit_raw:[q(null),q(null),q(null),q(null)],mock_large_signed:[q('-900719925474099312345'),q(null),q(null),q(null)]},effects:{recipient_account_credit_raw:{parameter_v1:q('-7609851'),parameter_v2:q('-7609851'),code_c0:q('0'),code_c1:q('0'),combined:q('-7609851'),interaction:q('0')}}}};
async function setup(page,{eligible=true}={}) {
 page.on('pageerror',error=>{throw error;});let posts=[],reject=true,reads=0;
 const model=JSON.parse(readFileSync(new URL('../../../fixtures/dashboard/view-models.json',import.meta.url))).projects['analytical-kinds']['/api/project'];
 const shell=readFileSync(new URL('../../dashboard/index.html',import.meta.url),'utf8').replace('<html lang="en" data-mode="overview">',`<html lang="en" data-mode="overview" data-cloud="1" data-project="${project}" data-base="/p/${project}" data-api="/api">`).replace('</head>','<link rel="stylesheet" href="/assets/cloud.css"></head>');
 await page.route('**/*',async route=>{
  const req=route.request(),path=new URL(req.url()).pathname;const json=(v,status=200)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(v)});
  if(path.startsWith(`/p/${project}/`))return route.fulfill({contentType:'text/html',body:shell});
  if(path==='/api/project')return json(model);
  if(path==='/v1/workspaces')return json({workspaces:[]});
  if(path===`/api/runs/${parent}`)return json({kind:'program_upgrade',hosted:{run_id:parent,project_id:project,status:'failed',report_available:false},report_url:'/unused'});
  if(path.endsWith('/interactions/eligibility'))return eligible?json({eligible:true,records:[facts]}):json({eligible:false,records:[],reason:'Retained candidate cannot be qualified.'});
  if(path.endsWith('/capabilities'))return json({schema_version:1,project_id:project,analyses:[{kind:'upgrade_parameter_interaction',status:'ready',supported:true,can_submit:true,missing:[]}]});
  if(path.endsWith('/interactions/preview'))return json(facts);
  if(path===`/v1/projects/${project}/runs/${parent}/interactions`){if(req.method()==='GET')return json({interactions:[]});posts.push(req.postDataJSON());if(reject){reject=false;return json({error:'Expected current state mismatch.'},400);}return json({run_id:child,status:'queued'},202);}
  if(path===`/v1/projects/${project}/interactions/${child}`){reads++;return json(reads===1?{...result,status:'queued',analysis:null,artifact_available:false}:result);}
  if(path==='/assets/interaction.js')return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL('../../cloud/interaction.js',import.meta.url),'utf8')});
  if(path==='/assets/main/report.js')return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL('../../src/report.js',import.meta.url),'utf8')});
  if(path==='/assets/main/governance.js')return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL('../../src/governance.js',import.meta.url),'utf8')});
  if(path==='/assets/main/brand.js')return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL('../../src/brand.js',import.meta.url),'utf8')});
  if(path==='/assets/capabilities.js')return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL('../../src/capabilities.js',import.meta.url),'utf8')});
  if(path==='/assets/cloud.css')return route.fulfill({contentType:'text/css',body:readFileSync(new URL('../../cloud/cloud.css',import.meta.url),'utf8')});
  if(path.startsWith('/assets/main/')){const file=path.slice('/assets/main/'.length);if(['shell.js','session.js','change.js','analysis.js','brand.js','mode.js','styles.css'].includes(file))return route.fulfill({contentType:file.endsWith('.css')?'text/css':'text/javascript',body:readFileSync(new URL(`../../src/${file}`,import.meta.url),'utf8')});return route.fulfill({contentType:'text/javascript',body:''});}
  return route.continue();
 });return posts;
}
test('parent → explicit record/import → preview/retry → durable six-cell result on desktop/mobile',async({page})=>{
 const posts=await setup(page);await page.goto(`${ROOT}/p/${project}/runs/${parent}`);await page.getByRole('link',{name:'Compare code and fee',exact:true}).click();
 const select=page.getByRole('combobox',{name:'Historical record'}),text=page.getByRole('textbox',{name:'Existing parameter ChangeSpec JSON'}),submit=page.getByRole('button',{name:'Submit interaction analysis',exact:true});
 await expect(submit).toBeDisabled();await expect(select).toHaveValue('');await text.fill(JSON.stringify(proposal));await page.getByRole('button',{name:'Preview retained inputs'}).click();await expect(page.locator('[data-interaction-error]')).toContainText('explicit');
 await select.selectOption('retained-record');await page.getByRole('button',{name:'Preview retained inputs'}).click();await expect(submit).toBeEnabled();await expect(page.locator('[data-interaction-preview]')).toContainText('parameter-id');
 await text.fill(JSON.stringify(proposal,null,2));await expect(submit).toBeDisabled();await page.getByRole('button',{name:'Preview retained inputs'}).click();
 await page.setViewportSize({width:390,height:844});expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);await page.screenshot({path:test.info().outputPath('interaction-preview-mobile.png'),fullPage:true});
 await submit.click();await expect(page.locator('[data-interaction-error]')).toContainText('mismatch');await expect(text).toHaveValue(JSON.stringify(proposal,null,2));await submit.click();await expect(page).toHaveURL(`${ROOT}/p/${project}/interactions/${child}`);await expect(page.locator('main')).toContainText('equal in this retained case',{timeout:10000});
 expect(posts).toHaveLength(2);expect(posts[1]).toEqual(posts[0]);expect(posts[1].parameter_change_spec).toEqual(proposal);
 for(const heading of ['K1 · SetFee C1 under V1.','K2 · SetFee C1 under V2.','Action matrix.'])await expect(page.getByRole('heading',{name:heading,exact:true})).toBeVisible();
 await expect(page.locator('main')).toContainText('-7609851');await expect(page.locator('main')).toContainText('No independent referral split');await expect(page.locator('main')).toContainText('-900719925474099312345');await expect(page.locator('main')).toContainText('fixed epoch 0');
 await page.getByRole('button',{name:'Technical',exact:true}).click();await expect(page.locator('main')).toContainText('pool-k1');await expect(page.locator('main')).toContainText('pool-k2');await expect(page.locator('main')).toContainText('upgrade-id');await expect(page.locator('main')).toContainText('parameter-id');
 await expect(page.getByRole('link',{name:'Download portable artifact'})).toHaveAttribute('href',`/v1/projects/${project}/interactions/${child}/artifact`);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);await page.screenshot({path:test.info().outputPath('interaction-result-mobile.png'),fullPage:true});await page.reload();await expect(page.locator('main')).toContainText('equal in this retained case');
 await page.setViewportSize({width:1280,height:900});await page.screenshot({path:test.info().outputPath('interaction-result-desktop.png'),fullPage:true});
});
test('unqualified retained parent fails closed with a factual explanation',async({page})=>{await setup(page,{eligible:false});await page.goto(`${ROOT}/p/${project}/runs/${parent}/interaction`);await expect(page.locator('main')).toContainText('cannot be qualified');await expect(page.locator('[data-interaction-form]')).toHaveCount(0);});
