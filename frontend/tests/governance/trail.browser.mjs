// Isolated browser regression: retained fixture evidence, no RPC or live service.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { chromium } from '@playwright/test';

const root = new URL('../../../', import.meta.url);
const load = async path => JSON.parse(await readFile(new URL(path, root), 'utf8'));
const fixture = 'docs/examples/phase-g1-squads-binding/';
const spec = await load(`${fixture}bound-change-spec.json`);
const binding = await load(`${fixture}binding-matched.json`);
const stale = await load(`${fixture}binding-stale-artifact.json`);
const now = Math.floor(Date.now()/1000);
const events = [binding,stale].map((b,i)=>({type:'governance_check',event_id:`gchk_${i}`,check:{check_id:`gchk_${i}`},recorded_at_unix_seconds:now-3600+i,binding:b}));
for(let i=0;i<23;i++) events.push({type:'deployment_attestation',event_id:`gocc_${i}`,occurrence:{occurrence_id:`gocc_${i}`},recorded_at_unix_seconds:now-300+i,attestation:{attestation_id:'shared-proof',binding_id:binding.binding_id,change_spec_id:spec.change_spec_id,candidate:spec.change.candidate,target_program:spec.change.target.program_id,transaction_index:42,outcome:'not_executed',observed_slot:7001,execution:null}});
const trail = {project_id:'proj_fixture',change_spec_id:spec.change_spec_id,source_unbound_change_spec_id:null,runs:[{run_id:'run_bound',status:'failed',exit_code:1,candidate_sha256:spec.change.candidate.sha256,bundle_sha256:'b'.repeat(64),created_at_unix_seconds:now}],events:events.slice(0,20),next_cursor:'page-two'};
const report=await load('docs/examples/phase-p3-impact-view/drift-semantic-baseline.json');
const change={change_spec_id:spec.change_spec_id,kind:'program_upgrade',target_program_id:spec.change.target.program_id,candidate_sha256:spec.change.candidate.sha256,delivery:spec.change.delivery};
report.change=change;report.candidate={...report.candidate,...spec.change.candidate};
const run={run_id:'run_bound',project_id:'proj_fixture',status:'failed',exit_code:1,report_available:true,bundle_sha256:report.bundle.sha256,candidate_sha256:spec.change.candidate.sha256,change:{...change,candidate_len:spec.change.candidate.len,origin:'submitted',label:null},canonical_report:report};
const browser = await chromium.launch(process.env.EPLYX_CHROME?{executablePath:process.env.EPLYX_CHROME}:{channel:'chrome'});
try {
  for (const width of [1280,390]) {
    const page=await browser.newPage({viewport:{width,height:900}});
    const requests=[]; const errors=[];
    page.on('pageerror',error=>errors.push(error.message));
    await page.addInitScript(data=>{window.fixture=data;},{spec,trail,run});
    await page.route('http://trail.test/**',async route=>{
      const url=new URL(route.request().url());requests.push({method:route.request().method(),path:url.pathname});
      if(url.pathname.endsWith('/trail')) return route.fulfill({json:{...trail,events:events.slice(20),next_cursor:null}});
      if(url.pathname==='/') return route.fulfill({contentType:'text/html',body:`<!doctype html><html><head><link rel="stylesheet" href="/src/styles.css"></head><body><main id="root"></main><script type="module">import {HostedReport} from '/src/report.js';import {attachGovernanceTrail} from '/src/governance.js';document.querySelector('#root').innerHTML=HostedReport(fixture.run,{spec:fixture.spec,governance:{trail:fixture.trail}});attachGovernanceTrail(document,path=>fetch(path));</script></body></html>`});
      if(url.pathname.startsWith('/src/') && !url.pathname.includes('..')) {
        try { return route.fulfill({contentType:url.pathname.endsWith('.js')?'text/javascript':'text/css',body:await readFile(new URL(`frontend${url.pathname}`,root))}); } catch {}
      }
      return route.fulfill({status:404,body:''});
    });
    await page.goto('http://trail.test/');
    await page.locator('[data-trail-more]').waitFor();
    assert.equal(await page.locator('[data-event-id]').count(),20);
    assert.equal(requests.filter(r=>r.path.endsWith('/trail')).length,0,'view never automatically fetches another page');
    await page.locator('[data-trail-more]').click();
    await page.waitForFunction(()=>document.querySelectorAll('[data-event-id]').length===25);
    assert.equal(await page.locator('[data-trail-more]').count(),0);
    await page.locator('[data-event-id="gocc_0"] summary').click();
    assert.ok(await page.locator('[data-event-id="gocc_0"] details').evaluate(el=>el.open));
    assert.match(await page.locator('[data-event-id="gocc_0"]').innerText(),new RegExp(binding.binding_id));
    assert.match(await page.locator('[data-trail-runs]').innerText(),/failed/);
    assert.match(await page.locator('[data-event-id="gchk_0"]').innerText(),/matched · dated observation/);
    assert.equal(requests.filter(r=>r.method!=='GET').length,0);
    const overflow=await page.locator('[data-governance-trail]').evaluate(el=>el.scrollWidth>el.clientWidth+1);
    assert.equal(overflow,false,`trail overflow at ${width}px`);
    assert.deepEqual(errors,[]);
    await page.locator('[data-governance-trail]').evaluate(el=>window.scrollTo(0,el.getBoundingClientRect().top+window.scrollY));
    await page.screenshot({path:`/tmp/eplyx-governance-trail-${width}.png`});
    await page.close();
  }
  console.log('Governance browser: desktop/mobile pagination, exact binding, dated match, analysis separation, technical expansion and read-only requests passed.');
} finally {await browser.close();}
