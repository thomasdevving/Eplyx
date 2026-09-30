import {test,expect} from '@playwright/test';
import {readFileSync} from 'node:fs';
const ROOT='http://127.0.0.1:4190', project='proj_parameter', parent='run_transfer', child='run_parameter';
const q=JSON.parse(readFileSync(new URL('../../../docs/examples/protocol-parameter-change-qualification.json',import.meta.url)));
const e={schema_version:1,eligible:true,project_id:project,run_id:parent,operation:q.change.operation.kind,program_id:q.change.target.program_id,mint:q.change.target.config_account,current_basis_points:50,schedule_epoch:'1032',captured_epoch:'1037',maximum_fee_raw:'18446744073709551615',mint_data_sha256:q.change.operation.expected_current.account_data_sha256,capture_sha256:q.observed_capture_sha256,token_2022_elf_sha256:q.programs[0].elf_sha256,transfer:{source:'741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs',destination:'124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az',amount_raw:'10000',decimals:9}};
const capabilities={schema_version:1,project_id:project,analyses:[{kind:'protocol_parameter_change',status:'ready',supported:true,can_submit:true,missing:[]}]};
const side=s=>({execution:{success:s.success},reconciliation:{reconciled:s.reconciled,input_debited_raw:s.source_debit_raw,output_received_raw:s.recipient_credit_raw,token_accounts:[{}, {withheld_fee_change_raw:s.destination_withheld_increment_raw}]}});
const report={kind:'protocol_parameter_change',status:q.status,execution_performed:true,change:q.change,proposed_declaration:q.declaration,observed_current_state:{mint_sha256:e.mint_data_sha256},shared_execution:{programs:q.programs},baseline:side(q.sides.baseline),proposed:side(q.sides.proposed),findings:q.findings,limitations:q.limits};
const parentDetail={id:parent,number:1,kind:'current_path',state:'Complete',status:'Proven',hosted:{run_id:parent,project_id:project,status:'completed',report_available:true},analysis:{report:{kind:'current-execution',path:'Transfer',status:'Proven',execution_performed:true,mint:e.mint,source:e.transfer.source,recipient:e.transfer.destination,amount_raw:'10000'}}};
const childDetail={id:child,number:2,kind:'protocol_parameter_change',state:'Complete',status:report.status,hosted:{run_id:child,project_id:project,status:'completed',report_available:true},analysis:{report}};
test('retained transfer → one rate → exact preview → durable existing result',async({page})=>{
 page.on('pageerror',error=>{throw error;});
 let accepted=false,resultReads=0,posts=[],reject=true;
 const model=JSON.parse(readFileSync(new URL('../../../fixtures/dashboard/view-models.json',import.meta.url))).projects['analytical-kinds']['/api/project'];
 const shell=readFileSync(new URL('../../dashboard/index.html',import.meta.url),'utf8').replace('<html lang="en" data-mode="overview">',`<html lang="en" data-mode="overview" data-cloud="1" data-project="${project}" data-base="/p/${project}" data-api="/api">`).replace('</head>','<link rel="stylesheet" href="/assets/cloud.css"></head>');
 await page.route('**/*',async route=>{
  const req=route.request(),url=new URL(req.url()),path=url.pathname;
  if(url.origin!==ROOT)return route.abort();
  const json=(value,status=200)=>route.fulfill({status,contentType:'application/json',body:JSON.stringify(value)});
  if(path.startsWith(`/p/${project}/`))return route.fulfill({contentType:'text/html',body:shell});
  if(path==='/api/project')return json(model);
  if(path==='/v1/workspaces')return json({workspaces:[]});
  if(path===`/api/runs/${parent}`)return json(parentDetail);
  if(path===`/api/runs/${child}`){resultReads++;return json(!accepted||resultReads>1?childDetail:{...childDetail,state:'Unfinished',hosted:{...childDetail.hosted,status:'running',report_available:false},analysis:null});}
  if(path.endsWith('/parameter-change/eligibility'))return json(e);
  if(path.endsWith('/capabilities'))return json(capabilities);
  if(path===`/v1/projects/${project}/runs/${parent}/parameter-changes`){
   posts.push(req.postDataJSON());
   if(reject){reject=false;return json({error:'current_state_mismatch: retained expectation mismatch'},400);}
   accepted=true;return json({run_id:child,status:'queued',status_url:`/v1/runs/${child}`},202);
  }
  if(path.startsWith('/assets/')){
   const file=path.slice('/assets/'.length);
   if(['parameter.js','parameter-model.js','settings.js'].includes(file))return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL(`../../cloud/${file}`,import.meta.url),'utf8')});
   if(file==='capabilities.js')return route.fulfill({contentType:'text/javascript',body:readFileSync(new URL('../../src/capabilities.js',import.meta.url),'utf8')});
   if(file==='cloud.css')return route.fulfill({contentType:'text/css',body:readFileSync(new URL('../../cloud/cloud.css',import.meta.url),'utf8')});
  }
  return route.continue();
 });
 await page.goto(`${ROOT}/p/${project}/runs/${parent}`);
 await page.getByRole('link',{name:'Parameter Change',exact:true}).click();
 await expect(page.locator('h1')).toHaveText('Parameter Change.');
 const form=page.locator('[data-parameter-form]'), input=page.getByLabel('Proposed transfer fee (basis points)');
 await expect(form.locator('input')).toHaveCount(1);await expect(page.locator('#parameter-observed')).toContainText('50 bps');
 for(const invalid of ['','-1','10001','0.5']){await input.fill(invalid);await page.getByRole('button',{name:'Preview Parameter Change'}).click();await expect(page.locator('[data-parameter-error]')).not.toBeEmpty();await expect(page.getByRole('button',{name:'Submit for analysis',exact:true})).toBeDisabled();}
 for(const endpoint of ['0','10000']){await input.fill(endpoint);await page.getByRole('button',{name:'Preview Parameter Change'}).click();await expect(page.getByRole('button',{name:'Submit for analysis',exact:true})).toBeEnabled();}
 await input.fill('200');await page.getByRole('button',{name:'Preview Parameter Change'}).click();
 await expect(page.locator('[data-parameter-preview]')).toContainText('Ready to submit for analysis.');
 await expect(page.locator('[data-parameter-preview]')).not.toContainText('9,800');
 await page.getByRole('button',{name:'Technical',exact:true}).click();
 await expect(input).toHaveValue('200');await expect(page.locator('[data-parameter-preview]')).toContainText('"schedule_epoch": "1032"');
 await page.getByRole('button',{name:'Overview',exact:true}).click();await expect(input).toHaveValue('200');
 await page.setViewportSize({width:390,height:844});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
 await page.screenshot({path:test.info().outputPath('parameter-preview-mobile.png'),fullPage:true});
 await page.setViewportSize({width:1280,height:900});
 await page.getByRole('button',{name:'Submit for analysis',exact:true}).click();
 await expect(page.locator('[data-parameter-error]')).toContainText('current_state_mismatch');await expect(input).toHaveValue('200');
 await page.getByRole('button',{name:'Submit for analysis',exact:true}).click();
 await expect(page).toHaveURL(`${ROOT}/p/${project}/runs/${child}`);
 await expect(page.locator('#paths')).toContainText('9,950 → 9,800',{timeout:10000});
 await expect(page.locator('#paths')).toContainText('50 → 200');await expect(page.locator('#observation')).toContainText('Program bytes unchanged.');
 expect(posts).toHaveLength(2);expect(posts[1]).toEqual(posts[0]);
 expect(posts[1].change_spec).toEqual({schema_version:1,change:{kind:'protocol_parameter_change',target:q.change.target,operation:q.change.operation}});
 await page.reload();await expect(page.locator('#paths')).toContainText('9,950 → 9,800');
 await page.setViewportSize({width:390,height:844});
 expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
 await page.screenshot({path:test.info().outputPath('parameter-result-mobile.png'),fullPage:true});
});
