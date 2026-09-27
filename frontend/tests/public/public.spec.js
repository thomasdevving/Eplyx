import { test, expect } from '@playwright/test';
import { CLI_COMMANDS } from '../../src/cli.js';

test.beforeEach(async ({ page }) => {
  page.on('pageerror', error => { throw error; });
  await page.route('**/*', route => new URL(route.request().url()).hostname === '127.0.0.1'
    ? route.continue() : route.abort());
});

for (const width of [1440, 1024, 834, 390, 320]) {
  test(`public capabilities and CLI work at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    const external = [];
    page.on('request', request => { if (new URL(request.url()).hostname !== '127.0.0.1') external.push(request.url()); });
    await page.goto('/');
    await expect(page.getByRole('heading', { level: 1 })).toContainText('See who is affected.');
    await expect(page.locator('#product')).toContainText('Governance binding.');
    await expect(page.locator('#roadmap')).toContainText('mock providers');
    await expect(page.locator('.intro')).toHaveCount(0);
    await expect(page.locator('#roadmap .section-heading')).toHaveCSS('opacity', '1');
    expect(await page.locator('.cli-preview .section-heading > p:not(.eyebrow)').evaluate(el => el.getBoundingClientRect().width)).toBeGreaterThan(230);
    await page.screenshot({ path: test.info().outputPath(`home-${width}.png`), fullPage: true });
    await page.screenshot({ path: test.info().outputPath(`hero-${width}.png`) });
    await page.locator('#product').scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.getByRole('link', { name: 'Get started with the CLI' }).click();
    await expect(page).toHaveURL(/\/cli$/);
    await expect(page.getByRole('heading', { name: 'Build the CLI from source.' })).toBeVisible();
    await expect(page.locator('#migration')).toContainText('init alone does not create a complete runnable migration');
    await expect(page.locator('#sync')).toContainText('configuration');
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.getByRole('navigation', { name: 'CLI guide sections' }).getByRole('link', { name: 'Lifecycle', exact: true }).click();
    await expect(page).toHaveURL(/\/cli#lifecycle$/);
    await expect(page.getByRole('heading', { name: 'Evaluate declared terms.' })).toBeInViewport();
    await page.screenshot({ path: test.info().outputPath(`cli-${width}.png`), fullPage: true });
    await page.reload();
    await expect(page.getByRole('heading', { name: 'Evaluate declared terms.' })).toBeInViewport();
    await page.goto('/token-transitions');
    await expect(page.getByRole('link', { name: 'Use the migration CLI' })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    expect(external).toEqual([]);
  });
}

test('commands copy exactly and denied clipboard access has an honest fallback', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
      writeText: async text => { window.copiedCommand = text; },
    } });
  });
  await page.goto('/cli');
  await page.getByRole('button', { name: 'Copy Build from source commands' }).click();
  expect(await page.evaluate(() => window.copiedCommand)).toBe(CLI_COMMANDS.install);
  await expect(page.locator('.command-feedback').first()).toHaveText('Commands copied.');
  await page.evaluate(() => { navigator.clipboard.writeText = async () => { throw new Error('Permission denied'); }; });
  await page.getByRole('button', { name: 'Copy Build from source commands' }).click();
  await expect(page.locator('.command-feedback').first()).toContainText('Commands selected');
  expect(await page.evaluate(() => window.getSelection().toString())).toBe(CLI_COMMANDS.install);
});

test('mobile menu closes on Escape and mode survives page navigation', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/cli');
  await page.getByRole('button', { name: 'Technical', exact: true }).click();
  await page.getByRole('button', { name: 'Open navigation' }).click();
  const nav = page.getByRole('navigation', { name: 'Primary navigation' });
  await nav.getByRole('link', { name: 'CLI', exact: true }).focus();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Open navigation' })).toHaveAttribute('aria-expanded', 'false');
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await nav.getByRole('link', { name: 'Token transitions' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Know what a transition changes.');
  await expect(page.locator('html')).toHaveAttribute('data-mode', 'technical');
});

test('workspace links use the configured service instead of the static-site origin', async ({ page }) => {
  await page.route('**/public/runtime-config.js', route => route.fulfill({
    contentType: 'text/javascript', body: 'globalThis.EPLYX_API_URL = "https://api.example.test";',
  }));
  await page.goto('/cli');
  await expect(page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('link', { name: 'Workspace' }))
    .toHaveAttribute('href', 'https://api.example.test/workspaces');
  await expect(page.locator('#sync').getByRole('link', { name: 'Open workspace' }))
    .toHaveAttribute('href', 'https://api.example.test/workspaces');
});

test('public routes retain the existing demo and analysis pages', async ({ page }) => {
  await page.goto('/runs/demo');
  await expect(page.locator('main')).toContainText('demo');
  await page.getByRole('link', { name: 'CLI', exact: true }).click();
  await page.locator('#upgrades').getByRole('link', { name: 'Open the hosted upgrade form' }).click();
  await expect(page.getByRole('heading', { name: 'Analyse a program upgrade.' })).toBeVisible();
});
