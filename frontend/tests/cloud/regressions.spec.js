import { test, expect } from '@playwright/test';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

const seedPath = join(process.env.EPLYX_CLOUD_BROWSER_DIR || join(tmpdir(), 'eplyx-cloud-browser-results'), 'cloud-seed.json');
const seed = () => JSON.parse(readFileSync(seedPath));

test.beforeAll(async () => {
 for (let n = 0; n < 250 && !existsSync(seedPath); n++) await new Promise(resolve => setTimeout(resolve, 100));
 expect(existsSync(seedPath)).toBeTruthy();
});

test.beforeEach(async ({ page }) => {
 page.on('pageerror', error => { throw error; });
 await page.goto('/login');
 await page.getByLabel('Email').fill(seed().email);
 await page.getByLabel('Password').fill(seed().password);
 await page.locator('button[type=submit]').click();
 await expect(page.locator('h1')).toHaveText('Workspaces');
});

test('completed hosted transfer exposes the authoritative Parameter Change entry', async ({ page }) => {
 await page.goto(`/p/${seed().analysisProject}/analyse`);
 await page.getByLabel('Mint address').fill(seed().mint);
 await page.getByLabel('Inspection scope').selectOption('wallet');
 await page.getByLabel('Public owner address').fill(seed().owner);
 await page.getByRole('button', { name: 'Observe current state', exact: true }).click();
 await expect(page.getByLabel('Focused account')).toHaveValue(seed().source);
 await page.getByLabel('Amount', { exact: true }).selectOption('Custom');
 await page.getByLabel('Custom amount', { exact: true }).fill('0.00001');
 await page.getByLabel('Recipient token account').fill('124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az');
 await page.getByRole('button', { name: 'Run offline path check' }).click();
 await page.locator('[data-path-result]').getByRole('link', { name: 'View complete run' }).click();
 await expect(page.getByRole('link', { name: 'Parameter Change', exact: true })).toBeVisible();
 await page.reload();
 await page.getByRole('link', { name: 'Parameter Change', exact: true }).click();
 await expect(page.locator('#parameter-observed')).toContainText('50 bps');
 await expect(page.getByLabel('Proposed transfer fee (basis points)')).toBeVisible();
});

test('hosted migration renders engine findings and retained evidence after refresh', async ({ page }) => {
 const { analysisProject, migrationRun } = seed();
 await page.goto(`/p/${analysisProject}/runs/${migrationRun}`);
 await expect(page.locator('#answers .answer')).toHaveCount(8);
 await expect(page.locator('#impact')).toContainText('Token accounts');
 await expect(page.locator('#answers .answer').nth(3)).toContainText('7 of 7 accounting equations hold');
 await expect(page.locator('#accounting .equations li')).toHaveCount(7);
 await expect(page.locator('#execution')).toContainText('Sequential population rehearsal');
 const report = page.locator(`#evidence a[href="/v1/runs/${migrationRun}/report.json"]`);
 await expect(report).toBeVisible();
 const response = await page.request.get(await report.getAttribute('href'));
 expect(response.ok()).toBeTruthy();
 expect((await response.json()).transition_kind).toBe('token_migration');
 await page.reload();
 await expect(page.locator('#answers .answer')).toHaveCount(8);
 await expect(page.locator('#evidence')).toContainText('immutable store');
});
