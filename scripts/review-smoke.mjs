// Real service only: no HTTP mocks, cookie injection, or operator browser token.
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
import { chromium } from '@playwright/test';
import { fileURLToPath } from 'node:url';

export async function reviewTarget(path) {
 assert.ok(path,'EPLYX_REVIEW_TARGET must explicitly name a verified non-production target JSON');
 const target=JSON.parse(await readFile(path,'utf8'));
 const url=new URL(target.url);
 assert.equal(url.origin,target.url,'Use an exact origin without a path');
 assert.equal(target.selection,'isolated-review','Explicit non-production selection required');
 assert.ok(!url.username && !url.password);
 if (!['127.0.0.1','localhost','[::1]'].includes(url.hostname)) {
  assert.equal(url.protocol,'https:');
  for(const k of ['account_id','project_id','environment_id','service_id','identity_storage_id','analytical_storage_id']) assert.ok(target[k],`Missing verified target ${k}`);
  assert.equal(target.owner_confirmed_isolation,true,'Owner must verify target and both stores, including deployment automation');
 }
 return target;
}
const hash=b=>createHash('sha256').update(b).digest('hex');
export async function smoke({target,credentials,project,parent,deniedProject,proposalPath,out,existingRun}) {
 assert.ok(project && parent && deniedProject && project!==deniedProject);
 await mkdir(out,{recursive:true,mode:0o700});
 const browser=await chromium.launch({headless:true,...(process.env.EPLYX_CHROME?{executablePath:process.env.EPLYX_CHROME}:{})});
 const context=await browser.newContext({baseURL:target.url,acceptDownloads:true,serviceWorkers:'block'});
 const page=await context.newPage();
 const failures=[];page.on('pageerror',e=>failures.push(e.message));
 let submissions=0;
 page.on('request',r=>{if(r.method()==='POST' && new URL(r.url()).pathname===`/v1/projects/${project}/runs/${parent}/interactions`)submissions++;});
 const json=async(path,options)=>{const r=await context.request.get(path,options);assert.ok(r.ok(),`GET ${path}: ${r.status()}`);return r.json();};
 const checks={};
 try {
  assert.equal((await context.cookies()).length,0);
  await page.goto('/login');
  assert.deepEqual(await page.evaluate(()=>[localStorage.length,sessionStorage.length]),[0,0]);
  await page.locator('input[name=email]').fill(credentials.email);
  await page.locator('input[name=password]').fill(credentials.password);
  await page.locator('button[type=submit]').click();
  await page.waitForURL(target.url+'/');
  await page.getByRole('heading',{name:'Workspaces',exact:true}).waitFor();
  const access=await json(`/v1/projects/${project}/workspace`);
  assert.equal(access.role,'member','Reviewer must use the existing minimum member role');
  const cookie=(await context.cookies()).find(c=>c.name==='eplyx_session');
  assert.ok(cookie?.httpOnly);assert.equal(cookie.sameSite,'Strict');
  if(new URL(target.url).protocol==='https:')assert.ok(cookie.secure);
  checks.login='real password form; HttpOnly SameSite=Strict session; member';
  await page.locator(`a.project-row[href="/p/${project}"]`).click();
  await page.goto(`/p/${project}/runs/${parent}`);
  await page.getByRole('link',{name:'Compare code and fee',exact:true}).waitFor();
  const eligibility=await json(`/v1/projects/${project}/runs/${parent}/interactions/eligibility`);
  assert.equal(eligibility.eligible,true);
  const record='mainnet-spl-stake-pool-151010f709e113e7';
  assert.ok(eligibility.records.some(r=>r.record_id===record));
  const listPath=`/v1/projects/${project}/runs/${parent}/interactions`;
  const before=await json(listPath);
  let child=existingRun,preview;
  if(!child) {
   await page.getByRole('link',{name:'Compare code and fee',exact:true}).click();
   await page.locator('select[name=record_id]').selectOption(record);
   await page.locator('textarea[name=proposal]').fill(await readFile(proposalPath,'utf8'));
   const previewResponse=page.waitForResponse(r=>r.request().method()==='POST' && new URL(r.url()).pathname===listPath+'/preview');
   await page.getByRole('button',{name:'Preview retained inputs'}).click();
   const pr=await previewResponse;assert.ok(pr.ok());preview=await pr.json();
   assert.equal(preview.record_id,record);assert.equal(String(preview.slot),'447850493');
   assert.equal(preview.v2.sha256,'a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d');
   assert.ok(preview.upgrade_change_spec_id && preview.parameter_change_spec_id);
   await page.getByRole('button',{name:'Technical',exact:true}).click();
   await page.locator('[data-interaction-preview]').getByText('Inspect both original proposals').click();
   assert.ok((await page.locator('[data-interaction-preview]').innerText()).includes(preview.parameter_change_spec_id));
   await page.getByRole('button',{name:'Overview',exact:true}).click();
   const acceptedResponse=page.waitForResponse(r=>r.request().method()==='POST' && new URL(r.url()).pathname===listPath);
   await page.getByRole('button',{name:'Submit interaction analysis',exact:true}).click();
   const accepted=await acceptedResponse;assert.equal(accepted.status(),202);
   child=(await accepted.json()).run_id;assert.match(child,/^run_/);
   await page.waitForURL(`**/p/${project}/interactions/${child}`);
   checks.status_at_refresh=(await json(`/v1/projects/${project}/interactions/${child}`)).status;
   await page.reload();
  } else await page.goto(`/p/${project}/interactions/${child}`);
  const resultPath=`/v1/projects/${project}/interactions/${child}`;
  let result;
  for(let i=0;i<180;i++) {
   result=await json(resultPath);
   if(!['queued','running'].includes(result.status))break;
   await new Promise(r=>setTimeout(r,1000));
  }
  assert.equal(result.status,'completed');assert.equal(result.analysis.status,'no_measured_interaction');
  assert.equal(result.parent_run_id,parent);assert.equal(result.artifact_available,true);
  if(preview)for(const k of ['upgrade_change_spec_id','parameter_change_spec_id'])assert.equal(result.analysis.facts[k],preview[k]);
  const a=result.analysis;
  assert.deepEqual(a.measurements.recipient_account_credit_raw.map(q=>q.value),['760985008','753375157','760985008','753375157']);
  assert.deepEqual(Object.fromEntries(Object.entries(a.effects.recipient_account_credit_raw).map(([k,q])=>[k,q.value])),{parameter_v1:'-7609851',parameter_v2:'-7609851',code_c0:'0',code_c1:'0',combined:'-7609851',interaction:'0'});
  for(const k of ['k1','k2','r00','r01','r10','r11'])assert.equal(a[k].state,'verified');
  assert.equal(a.r01.parent.stage,'K1');assert.equal(a.r11.parent.stage,'K2');
  assert.ok(a.measurements.referral_account_credit_raw.every(q=>q.value===null && q.unavailable_reason));
  await page.reload();
  await page.getByRole('link',{name:'Download portable artifact',exact:true}).waitFor();
  const text=await page.locator('main').innerText();
  for(const s of ['760985008','753375157','-7609851','Retained','Unavailable','constructed'])assert.ok(text.toLowerCase().includes(s.toLowerCase()),`Rendered result missing ${s}`);
  // Overview and Technical use the same server result.
  await page.getByRole('button',{name:'Technical',exact:true}).click();
  assert.equal(await page.locator('html').getAttribute('data-mode'),'technical');
  for(const value of [a.report_sha256,a.analysis_input_sha256,a.facts.upgrade_change_spec_id,a.facts.parameter_change_spec_id])assert.ok((await page.locator('main').innerText()).includes(value));
  const dl=page.waitForEvent('download');
  await page.getByRole('link',{name:'Download portable artifact',exact:true}).click();
  const download=await dl;assert.equal(download.suggestedFilename(),'interaction.tar');
  const archive=join(out,'interaction.tar');await download.saveAs(archive);
  checks.download_sha256=hash(await readFile(archive));
  checks.refresh='same durable occurrence; direct result navigation and artifact download';
  const after=await json(listPath);
  assert.equal(after.interactions.length,before.interactions.length+(existingRun?0:1));
  assert.equal(submissions,existingRun?0:1);
  const signedOut=await browser.newContext({baseURL:target.url});
  for(const path of [resultPath,resultPath+'/artifact',`/v1/projects/${project}/view/project`])assert.ok([401,403,404].includes((await signedOut.request.get(path)).status()));
  await signedOut.close();
  for(const path of [`/v1/projects/${deniedProject}`,`/v1/projects/${deniedProject}/runs`, `/v1/projects/${deniedProject}/runs/${parent}/interactions`, `/v1/projects/${deniedProject}/interactions/${child}`,`/v1/projects/${deniedProject}/interactions/${child}/artifact`])assert.ok([401,403,404].includes((await context.request.get(path)).status()),`Cross-project read allowed: ${path}`);
  const denied=await context.request.post(`/v1/projects/${deniedProject}/runs/${parent}/interactions`,{headers:{origin:target.url},data:{record_id:record,parameter_change_spec:JSON.parse(await readFile(proposalPath)),request_key:crypto.randomUUID()}});
  assert.ok([401,403,404].includes(denied.status()));
  assert.equal((await json(resultPath)).analysis.report_sha256,a.report_sha256);
  checks.authorization='signed-out read/download denied; member cross-project read/list/create/download denied; authorized result still readable';
  assert.deepEqual(failures,[]);
  const receipt={url:target.url,project,parent,child,analysis_input_sha256:a.analysis_input_sha256,report_sha256:a.report_sha256,upgrade_change_spec_id:a.facts.upgrade_change_spec_id,parameter_change_spec_id:a.facts.parameter_change_spec_id,checks};
  await writeFile(join(out,'smoke-receipt.json'),JSON.stringify(receipt,null,2)+'\n',{mode:0o600});
  console.log(JSON.stringify(receipt));return receipt;
 } catch(error) {
  await writeFile(join(out,'failure-page.txt'),await page.locator('body').innerText(),{mode:0o600}).catch(()=>{});
  await writeFile(join(out,'failure-browser-errors.json'),JSON.stringify(failures),{mode:0o600}).catch(()=>{});
  throw error;
 } finally {await context.close();await browser.close();}
}
if(process.argv[1] && resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
 const target=await reviewTarget(process.env.EPLYX_REVIEW_TARGET);
 const credentials={email:process.env.EPLYX_REVIEW_EMAIL,password:process.env.EPLYX_REVIEW_PASSWORD};
 assert.ok(credentials.email && credentials.password,'Designated test credentials required');
 await smoke({target,credentials,project:process.env.EPLYX_REVIEW_PROJECT,parent:process.env.EPLYX_REVIEW_PARENT,deniedProject:process.env.EPLYX_REVIEW_DENIED_PROJECT,proposalPath:process.env.EPLYX_REVIEW_PARAMETER,out:process.env.EPLYX_REVIEW_OUTPUT,existingRun:process.env.EPLYX_REVIEW_RUN_ID});
}
