import { test, expect } from '@playwright/test';

// Token Migration V1 runs of examples/migrations/minimal: a synthetic fixture
// world rehearsed with the reference candidate and a deadline-defect candidate.
const MIGRATION = 'http://127.0.0.1:4188';
import { readFileSync } from 'node:fs';
const { reference:REFERENCE, defect:DEFECT, deadline:DEADLINE } = JSON.parse(readFileSync(new URL('../../../fixtures/dashboard/ids.json', import.meta.url)));

const tile = (page, label) => page.locator('.tile').filter({ has:page.locator('.tile__label', { hasText:new RegExp(`^${label}$`, 'i') }) });

test.beforeEach(async ({ page }) => {
 page.on('pageerror', error => { throw error; });
 await page.addInitScript(() => { if (!sessionStorage.getItem('seeded')) { localStorage.setItem('eplyx-detail', 'overview'); sessionStorage.setItem('seeded', '1'); } });
});

test('a migration run answers the eight questions in order', async ({ page }) => {
 await page.goto(`${MIGRATION}/runs/${DEFECT}`);
 await expect(page.locator('main h1')).toHaveText('Failed.');
 await expect(page.locator('.alert--info')).toContainText('Nothing was signed or sent and no funds moved');
 const questions = page.locator('#answers .answer strong');
 await expect(questions).toHaveText([
  'Can this proposed migration execute for the tested states?',
  'How much of the captured population is covered?',
  'Which accounts or classes cannot migrate?',
  'Does source → destination accounting reconcile?',
  'Is the destination funding sufficient?',
  'What are the concrete failures or counterexamples?',
  'What are the evidence limitations?',
  'How can a failure be reproduced?',
 ]);
 await expect(page.locator('#answers .answer').nth(0)).toContainText('Blocked');
 await expect(page.locator('#answers .answer').nth(3)).toContainText('7 of 7 accounting equations hold');
 await expect(page.locator('#answers .answer').nth(7)).toContainText(`eplyx migration reproduce ${DEADLINE}`);
 await expect(page.locator('#impact')).toContainText('Can migrate under the proposed mechanism');
 await expect(page.locator('#stress')).toContainText('Derived from fixture');
 await expect(page.locator('#stress tr', { hasText:'At Deadline' })).toContainText('Unexpected Success');
 await expect(page.locator('#evidence')).toContainText('unsigned');
 await expect(page.locator('#evidence')).toContainText('4 of 4 rehearsed holders re-executed identically from the plan alone');
 // Solana detail stays behind Technical mode.
 const program = page.locator('#migration .tech-only', { hasText:'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA' }).first();
 await expect(program).toHaveCount(1);
 await expect(program).toBeHidden();
 await page.getByRole('button', { name:'Technical' }).click();
 await expect(program).toBeVisible();
 const text = await page.locator('main').innerText();
 expect(text).not.toMatch(/undefined|\[object Object\]|NaN/);
 expect(text).not.toMatch(/Capture was read-only/);
});

test('migration overview, list and counterexample keep provenance honest', async ({ page }) => {
 await page.goto(`${MIGRATION}/`);
 await expect(page.locator('.hero-status h1')).toHaveText('Failed.');
 await expect(tile(page, 'Stress cases')).toContainText('19 / 20');
 await expect(tile(page, 'Counterexamples')).toContainText('0 rehearsal · 1 derived');
 await expect(page.locator('main')).toContainText('this run rehearsed a synthetic fixture');
 await page.goto(`${MIGRATION}/runs`);
 await expect(page.locator('tbody tr[data-run]')).toHaveCount(2);
 await expect(page.locator(`tr[data-run="${REFERENCE}"]`)).toContainText('Passed with warnings');
 await page.goto(`${MIGRATION}/counterexamples/${DEADLINE}`);
 await expect(page.locator('.kind--derived')).toContainText('It is not an observed mainnet failure.');
 await expect(page.locator('main')).toContainText('must reject this migration');
 await expect(page.locator('main')).toContainText('Reproduced 1 time');
 const text = await page.locator('main').innerText();
 expect(text).not.toMatch(/undefined|\[object Object\]|NaN/);
});

test('migration comparison never claims a fix', async ({ page }) => {
 // Keep the pinned source's non-equivalent-search presentation assertion.
 // MAIN's regenerated minimal searches happen to share their selected domain;
 // this explicit view response represents the source's differing-domain case.
 await page.route('**/api/compare?*', async route => {
  const body = await (await route.fetch()).json();
  body.counterexamples.comparable = false;
  body.counterexamples.differences = ['The declared search domains differ.'];
  for (const item of body.counterexamples.items) if (item.status === 'resolved') item.status = 'only_left';
  delete body.counterexamples.counts.resolved;
  await route.fulfill({ json:body });
 });
 await page.goto(`${MIGRATION}/compare?left=${REFERENCE}&right=${DEFECT}`);
 const story = page.locator('.story');
 await expect(story.locator('.story__cell', { hasText:'Migration mechanism' })).toContainText('Ready');
 await expect(story.locator('.story__cell', { hasText:'Migration mechanism' })).toContainText('Blocked');
 await expect(page.locator('.equivalence--warn')).toContainText('A missing counterexample does not prove resolution.');
 await page.goto(`${MIGRATION}/compare?left=${DEFECT}&right=${REFERENCE}`);
 await expect(page.locator('main')).toContainText('Only in A');
 expect((await page.locator('main').innerText()).toLowerCase()).not.toMatch(/\bfixed\b|resolved \(equivalent search\)/);
});

test('migration pages fit a phone viewport', async ({ page }) => {
 await page.setViewportSize({ width:390, height:844 });
 for (const path of ['/', `/runs/${DEFECT}`, `/counterexamples/${DEADLINE}`, '/production']) {
  await page.goto(`${MIGRATION}${path}`);
  await expect(page.locator('main h1, main h2').first()).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
 }
});
