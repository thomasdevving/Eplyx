import { test,expect } from '@playwright/test';
const ROOT='http://127.0.0.1:4190';
test('lifecycle and observation views retain their distinct scope',async({page})=>{
 const requests=[];page.on('request',r=>requests.push(new URL(r.url()).hostname));
 page.on('pageerror',error=>{throw error;});
 const runs=await(await page.request.get(`${ROOT}/api/runs`)).json();
 for(const row of runs.runs){
  await page.goto(`${ROOT}/runs/${row.id}`);
  await expect(page.locator('main')).toContainText('Saved engine output');
  await expect(page.locator('main')).not.toContainText('Deployment gate');
  if(row.kind==='lifecycle_change'){
   await expect(page.locator('#policy')).toContainText('Expired');
   await expect(page.locator('#consequence')).toContainText('Stale Exposure');
   await expect(page.locator('#protocols')).toContainText('not added to entity totals');
  }else{
   await expect(page.locator('#paths tbody tr')).toHaveCount(5);
   for(const row of await page.locator('#paths tbody tr').all())await expect(row).toContainText('Not evaluated');
   await expect(page.locator('#observation')).toContainText('Cannot be judged');
  }
  await page.getByRole('button',{name:'Technical'}).click();
  await expect(page.getByRole('heading',{name:'Recorded JSON.'})).toBeVisible();
  await page.setViewportSize({width:390,height:844});
  expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
 }
 await page.screenshot({path:test.info().outputPath('observation-mobile.png'),fullPage:true});
 await page.goto(`${ROOT}/runs`);
 for(const box of await page.getByRole('checkbox').all())await box.check();
 await expect(page.getByRole('button',{name:'Select two runs of the same kind'})).toBeDisabled();
 expect(new Set(requests)).toEqual(new Set(['127.0.0.1']));
});

test('public transition route uses one presentation and responsive navigation',async({page})=>{
 page.on('pageerror',error=>{throw error;});
 // Existing public-site fonts have an external fallback. No external request is
 // needed for this local check; loopback dashboard fonts are tested separately.
 await page.route('**/*',route=>new URL(route.request().url()).hostname==='127.0.0.1'?route.continue():route.abort());
 for(const width of [1440,1024,834,390]){
  await page.setViewportSize({width,height:1000});
  await page.goto('http://127.0.0.1:4189/token-transitions');
  await expect(page.getByRole('heading',{name:'Know what a transition changes.'})).toBeVisible();
  expect(await page.evaluate(()=>document.documentElement.scrollWidth-document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
  await expect(page.getByRole('group',{name:'Presentation',exact:true})).toHaveCount(0);
  await expect(page.locator('main [data-technical]')).toBeVisible();
  await page.reload();
  await expect(page.getByRole('group',{name:'Presentation',exact:true})).toHaveCount(0);
  await page.screenshot({path:test.info().outputPath(`transitions-${width}.png`),fullPage:true});
 }
});
