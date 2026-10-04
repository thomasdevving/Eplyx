import { test, expect } from '@playwright/test';
import { readFile } from 'node:fs/promises';
import { OVERVIEW_DESCRIPTION, OVERVIEW_TITLE, OVERVIEW_PDF } from '../../src/technical-overview.js';
import { SOURCE_URL } from '../../src/shell.js';

for (const width of [1440, 375]) {
  test(`technical overview is public, responsive and survives refresh at ${width}px`, async ({ page, request, context }) => {
    await page.setViewportSize({ width, height: 1000 });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.route('**/*', route => new URL(route.request().url()).hostname === '127.0.0.1' ? route.continue() : route.abort());
    const html = await request.get('/technical-overview');
    expect(html.status()).toBe(200);
    expect(await html.text()).toContain(`<title>${OVERVIEW_TITLE}</title>`);
    expect((await html.text()).replace(/<[^>]*>/g, '')).toContain('Simulation-backed assurance for on-chain changes');
    await page.goto('/technical-overview');
    await expect(page).toHaveTitle(OVERVIEW_TITLE);
    await expect(page.locator('meta[name="description"]')).toHaveAttribute('content', OVERVIEW_DESCRIPTION);
    await expect(page.locator('link[rel="canonical"]')).toHaveAttribute('href', 'http://127.0.0.1:4193/technical-overview');
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Simulation-backed assurance for on-chain changes');
    await expect(page.locator('.technical-vms .technical-node')).toHaveCount(2);
    await expect(page.locator('.technical-capability-list dt')).toHaveCount(7);
    await expect(page.getByRole('link', { name: 'View on GitHub' })).toHaveAttribute('href', SOURCE_URL);
    for (const label of ['Read Technical Overview', 'Open PDF', 'Download PDF']) {
      await expect(page.getByRole('link', { name: label, exact: true })).toHaveAttribute('href', OVERVIEW_PDF);
    }
    await expect(page.locator('iframe')).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.reload();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Simulation-backed assurance for on-chain changes');
    expect(await context.cookies()).toEqual([]);
    expect(await page.evaluate(() => ({ local: Object.keys(localStorage), session: Object.keys(sessionStorage) }))).toEqual({ local: [], session: [] });
    const pdf = await request.get(OVERVIEW_PDF);
    expect(pdf.status()).toBe(200);
    expect(pdf.headers()['content-type']).toBe('application/pdf');
    expect(await pdf.body()).toEqual(await readFile(new URL('../../public/technical-overview.pdf', import.meta.url)));
    const downloaded = page.waitForEvent('download');
    await page.getByRole('link', { name: 'Download PDF', exact: true }).click();
    expect((await downloaded).suggestedFilename()).toBe('Eplyx_Technical_Overview.pdf');
    await page.locator('.site-header .logo-link').click();
    await expect(page).toHaveURL('/');
    await expect(page).toHaveTitle('Eplyx — Know what changes');
    await expect(page.locator('link[rel="canonical"]')).toHaveCount(0);
    await page.locator('.footer__links').getByRole('link', { name: 'Technical Overview', exact: true }).click();
    await expect(page).toHaveURL('/technical-overview');
    await expect(page).toHaveTitle(OVERVIEW_TITLE);
    expect(errors).toEqual([]);
  });
}

test('the built overview contains readable content without JavaScript', async ({ browser, baseURL }) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  await page.goto(`${baseURL}/technical-overview`);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Open PDF', exact: true })).toBeVisible();
  await expect(page.locator('.technical-capability-list dt')).toHaveCount(7);
  await context.close();
});
