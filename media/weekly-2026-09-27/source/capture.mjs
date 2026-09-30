import { chromium, expect } from '@playwright/test';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { resolve } from 'node:path';

const root=resolve('media/weekly-2026-09-27');
const assets=resolve(root,'assets');
const chrome=process.env.EPLYX_CHROME||'/Users/thomasnguyen/Library/Caches/ms-playwright/chromium-1228/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser=await chromium.launch({executablePath:chrome,headless:true});
const context=await browser.newContext({viewport:{width:1440,height:900},deviceScaleFactor:1.5,reducedMotion:'reduce'});
await context.route('**/*',route=>['127.0.0.1','localhost'].includes(new URL(route.request().url()).hostname)?route.continue():route.abort());
await context.addInitScript(()=>{localStorage.setItem('eplyx-detail','overview');localStorage.setItem('eplyx-mode','overview');sessionStorage.setItem('eplyx-intro-seen','1');});
const page=await context.newPage();
const manifest=[];
async function ready(){await page.locator('main').waitFor();await page.evaluate(()=>document.fonts.ready);await page.waitForTimeout(500);}
async function shot(name,source,selector){
 await page.evaluate(()=>document.fonts.ready);
 if(selector){await page.locator(selector).first().evaluate(el=>{document.documentElement.style.scrollBehavior='auto';window.scrollTo(0,el.getBoundingClientRect().top+scrollY-90);});await page.waitForTimeout(200);}
 await page.screenshot({path:resolve(assets,name+'.png')});
 manifest.push({file:name+'.png',source,url:page.url(),viewport:{width:1440,height:900},captured_at:new Date().toISOString()});
 console.log('Captured '+name);
}
async function goto(url){await page.goto(url);await ready();}
await goto('http://127.0.0.1:4189/');
await page.addStyleTag({content:"@font-face{font-family:'Manrope';src:url('/public/fonts/manrope-latin.woff2')}@font-face{font-family:'DM Sans';src:url('/public/fonts/dm-sans-latin.woff2')}"});
await shot('hero','Actual public homepage, local source at a3eac9d');
await goto('http://127.0.0.1:4189/token-transitions');await shot('transitions','Actual public token-transitions page');
await goto('http://127.0.0.1:4185/');await shot('dashboard','Actual CLI dashboard; committed synthetic transition-acceptance fixture');
const ids=JSON.parse(await readFile('fixtures/dashboard/ids.json'));
await goto('http://127.0.0.1:4185/runs/'+ids.underfunded);await shot('migration','Actual migration result; deficient-reserve deadline-defect fixture');
await shot('migration-stress','Actual fixture stress table','#stress');
await shot('migration-gate','Actual fixture deployment gate','#gate');
await goto('http://127.0.0.1:4185/counterexamples/'+ids.derived);await shot('counterexample','Actual saved derived counterexample and reproduction history');
await goto('http://127.0.0.1:4185/compare?left='+ids.healthy+'&right='+ids.underfunded);await shot('comparison','Actual comparison; search domains differ');
const runs=await(await page.request.get('http://127.0.0.1:4190/api/runs')).json();
for(const row of runs.runs){
 await goto('http://127.0.0.1:4190/runs/'+row.id);
 if(row.kind==='lifecycle_change'){await shot('lifecycle','Actual lifecycle dashboard; hypothetical fixture declaration');await shot('lifecycle-policy','Actual lifecycle policy and consequences','#policy');}
 else await shot('observation','Actual saved synthetic current observation; paths untested');
}
// Render archived/fresh engine JSON with the unmodified production renderer.
async function report(name,file,label){
 const r=JSON.parse(await readFile(file));
 await goto('http://127.0.0.1:4189/runs/demo');
 await page.evaluate(async({r,label})=>{
  const {HostedReport}=await import('/src/report.js');
  document.querySelector('#app').innerHTML=HostedReport({run_id:label,status:r.summary?.passed?'passed':'failed',report_available:true,canonical_report:r,exit_code:r.summary?.exit_code??0,bundle_sha256:r.bundle.sha256,candidate_sha256:r.candidate.sha256},{projectName:label});
 },{r,label});
 await page.evaluate(()=>scrollTo(0,0));await shot(name,'Unmodified HostedReport component rendering '+file);
 await shot(name+'-detail','Unmodified report component, economic coverage and retained proof', '.report-content');
}
await report('orca','media/weekly-2026-09-27/assets/orca-report.json','Orca · SwapV2');
await report('drift','docs/examples/phase-u14-drift-validation/ci.json','Drift · settlePnl');
await goto('http://127.0.0.1:4189/runs/demo');
const binding=JSON.parse(await readFile('docs/examples/phase-g1-1-squads-mainnet/witness-primary/bind.json'));
const spec=JSON.parse(await readFile('docs/examples/phase-g1-1-squads-mainnet/witness-primary/bound-change-spec.json'));
const attestation=JSON.parse(await readFile('docs/examples/phase-g2-squads-mainnet/active-not-executed.json'));
await page.evaluate(async({binding,spec,attestation})=>{
 const {governanceView,GovernanceSection,deliveryOf}=await import('/src/governance.js');
 const view=governanceView({delivery:deliveryOf(spec),check:{binding,checked_at_unix_seconds:1790294400},attestation,changeSpecId:spec.change_spec_id});
 document.querySelector('#app').innerHTML='<main class="inner-page report-page"><section class="report-shell"><div class="report-content">'+GovernanceSection(view)+'</div></section></main>';
},{binding,spec,attestation});
await page.evaluate(()=>scrollTo(0,0));await shot('governance','Unmodified GovernanceSection; archived real mainnet G1 binding + G2 not_executed attestation. Archived date is a presentation timestamp.');
// Use the actual local hosted service, a disposable DB, and its synthetic provider.
const seed=JSON.parse(await readFile('/private/tmp/eplyx-video-cloud/cloud-seed.json'));
await page.goto(seed.base+'/login');await page.getByLabel('Email').fill(seed.email);await page.getByLabel('Password').fill(seed.password);await page.locator('button[type=submit]').click();await expect(page.locator('h1')).toHaveText('Workspaces');
await goto(seed.base+'/p/'+seed.project);await shot('hosted','Actual hosted workspace; real exact-byte sync of synthetic fixtures');
await goto(seed.base+'/p/'+seed.upgradeProject+'/runs/'+seed.upgradeRun);await expect(page.locator('.hosted-upgrade-report')).toBeVisible();await shot('upgrade','Real local hosted upgrade job using committed Stake Pool fixture candidate');
await shot('upgrade-impact','Actual hosted economic impact report','.report-content');
await goto(seed.base+'/p/'+seed.analysisProject+'/analyse');await shot('current-form','Actual current-state analysis form');
await page.getByLabel('Mint address').fill(seed.mint);await page.getByLabel('Inspection scope').selectOption('wallet');await page.getByLabel('Public owner address').fill(seed.owner);await shot('current-input','Actual form populated with the synthetic provider wallet');
await page.getByRole('button',{name:'Observe current state',exact:true}).click();await expect(page.getByLabel('Focused account')).toHaveValue(seed.source);await shot('current-observed','Actual current-state observation through synthetic test provider','[data-observation]');
await page.getByLabel('Amount',{exact:true}).selectOption('Custom');await page.getByLabel('Custom amount',{exact:true}).fill('0.0000001');await page.getByLabel('Recipient token account').fill('124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az');
await page.getByRole('button',{name:'Run offline path check'}).click();await expect(page.locator('[data-path-result]')).toContainText('Recorded result:',{timeout:60000});await shot('path-result','Actual offline Transfer result against synthetic provider state','[data-path-result]');
await page.getByLabel('Replacement mint',{exact:true}).fill('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');await page.getByRole('combobox',{name:'Candidate amount',exact:true}).selectOption('Custom');await page.getByLabel('Candidate custom amount').fill('0.0000001');await page.getByRole('button',{name:'Run candidate check'}).click();await expect(page.locator('[data-candidate-result]')).toContainText('Exact checks passed',{timeout:60000});await shot('candidate-result','Actual candidate check; synthetic provider; exact checks and incomplete evidence preserved','[data-candidate-result]');
await page.getByLabel('Saved candidate plan (optional)').selectOption({index:1});await page.getByLabel('Scenario replacement mint (optional)').fill('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');await page.getByLabel('Proposed effective time').fill('2031-01-01T12:00');await page.getByLabel('Proposed deadline (optional)').fill('2032-01-01T12:00');await page.getByRole('button',{name:'Evaluate proposed scenario'}).click();await expect(page.locator('[data-preflight-result]')).toContainText('Prepared',{timeout:60000});await shot('scenario-result','Actual hypothetical scenario; separate readiness results retained','[data-preflight-result]');
await writeFile(resolve(assets,'capture-manifest.json'),JSON.stringify(manifest,null,2));
await browser.close();
