import { test, expect } from '@playwright/test';

test('a late failure from the previous route cannot replace the current page', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('main h1')).toBeVisible();
  let rejectPrevious;
  const pending = new Promise(resolve => { rejectPrevious = resolve; });
  let requested;
  const started = new Promise(resolve => { requested = resolve; });
  let calls = 0;
  await page.route('**/api/project', async route => {
    if (++calls !== 1) return route.continue();
    requested();
    await pending;
    await route.fulfill({ status: 503, json: { error: 'old request failed' } });
  });
  await page.evaluate(() => window.dashboardNavigate('/project'));
  await started;
  await page.evaluate(() => window.dashboardNavigate('/runs'));
  await expect(page.getByRole('heading', { name: 'Runs.', exact: true })).toBeVisible();
  const failed = page.waitForResponse(response => response.url().endsWith('/api/project') && response.status() === 503);
  rejectPrevious();
  await (await failed).finished();
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  await expect(page.locator('main')).not.toContainText('old request failed');
  await expect(page.getByRole('heading', { name: 'Runs.', exact: true })).toBeVisible();
});

test('leaving a page disposes its subscriptions before attaching the next page', async ({ page }) => {
  await page.route('**/assets/pages.js', route => route.fulfill({
    contentType: 'text/javascript',
    body: `
      export async function api() { return { project: { name: 'Navigation fixture' } }; }
      function view(title) {
        return { title, html: '<h1>' + title + '</h1>', attach() {
          window.activeViews = (window.activeViews || 0) + 1;
          return () => { window.activeViews--; };
        } };
      }
      export async function overview() { return view('Overview'); }
      export async function runs() { return view('Runs'); }
    `,
  }));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Overview', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.activeViews)).toBe(1);
  await page.evaluate(() => window.dashboardNavigate('/runs'));
  await expect(page.getByRole('heading', { name: 'Runs', exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.activeViews)).toBe(1);
});
