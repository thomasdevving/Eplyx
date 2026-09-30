import {chromium} from '@playwright/test';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve} from 'node:path';
const root=resolve('media/weekly-2026-09-27');
const browser=await chromium.launch({executablePath:process.env.EPLYX_CHROME||'/Users/thomasnguyen/Library/Caches/ms-playwright/chromium-1228/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing',headless:true});
const page=await browser.newPage({viewport:{width:1200,height:900},deviceScaleFactor:1.5,reducedMotion:'reduce'});
await page.route('**/*',r=>new URL(r.request().url()).hostname==='127.0.0.1'?r.continue():r.abort());
const manifest=JSON.parse(await readFile(resolve(root,'assets/capture-manifest.json')));
for(const name of ['orca','drift']){
 const r=JSON.parse(await readFile(resolve(root,'assets/'+name+'-report.json')));
 await page.goto('http://127.0.0.1:4189/runs/demo');
 await page.evaluate(async({r,name})=>{const {HostedReport}=await import('/src/report.js');document.querySelector('#app').innerHTML=HostedReport({run_id:name,status:r.summary.passed?'passed':'failed',report_available:true,canonical_report:r,exit_code:r.summary.exit_code,bundle_sha256:r.bundle.sha256,candidate_sha256:r.candidate.sha256},{projectName:name});},{r,name});
 await page.evaluate(()=>document.fonts.ready);
 await page.locator('#impact').screenshot({path:resolve(root,'assets/'+name+'-impact.png')});
 manifest.push({file:name+'-impact.png',source:'Unmodified production #impact component rendering assets/'+name+'-report.json',captured_at:new Date().toISOString()});
}
await writeFile(resolve(root,'assets/capture-manifest.json'),JSON.stringify(manifest,null,2));
await browser.close();
