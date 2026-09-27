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
    await expect(page.locator('#product')).toContainText('Eplyx is in development.');
    await expect(page.locator('main')).not.toContainText('Implemented');
    await expect(page.locator('.orbit-label[data-ring="changes"][data-index="2"]')).toContainText('Planned');
    await expect(page.locator('#roadmap')).toContainText('mock providers');
    await expect(page.locator('.intro')).toHaveCount(0);
    await expect(page.locator('#roadmap .section-heading')).toHaveCSS('opacity', '1');
    expect(await page.locator('.cli-preview .section-heading > p:not(.eyebrow)').evaluate(el => el.getBoundingClientRect().width)).toBeGreaterThan(230);
    await page.screenshot({ path: test.info().outputPath(`home-${width}.png`), fullPage: true });
    await page.screenshot({ path: test.info().outputPath(`hero-${width}.png`) });
    await page.locator('#product').scrollIntoViewIfNeeded();
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.getByRole('link', { name: 'Try Eplyx' }).click();
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


test.describe('original motion and product maturity', () => {
  test.use({ contextOptions: { reducedMotion: 'no-preference' } });

  test('intro, orbit movement and scroll reveals remain animated', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/');
    await expect(page.locator('.intro')).toBeVisible();
    await expect(page.locator('.intro')).toHaveCSS('animation-name', 'introExit');
    await expect(page.locator('.intro')).toHaveCount(0, { timeout: 6000 });
    await expect(page.getByRole('link', { name: 'Try Eplyx' })).toBeVisible();
    const planet = page.locator('.orbit-body[data-ring="changes"]').first();
    const before = await planet.evaluate(el => getComputedStyle(el).transform);
    await expect.poll(() => planet.evaluate(el => getComputedStyle(el).transform)).not.toBe(before);
    await expect(page.locator('#product .section-heading')).toHaveCSS('opacity', '0');
    await page.locator('#product .section-heading').scrollIntoViewIfNeeded();
    await expect(page.locator('#product .section-heading')).toHaveCSS('opacity', '1');
    await page.screenshot({ path: test.info().outputPath('product-normal-motion.png') });

    await page.goto('/');
    await expect(page.locator('.intro')).toHaveCount(0);
    await page.getByRole('button', { name: 'Consequences', exact: true }).click();
    await expect(page.locator('.orbit-caption')).toContainText('in development');
    const future = page.locator('.orbit-body[data-name="Monitoring"]');
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Shift+Tab');
    await expect(future).toBeFocused();
    await expect(page.locator('.orbit-detail')).toBeVisible();
    await expect(page.locator('.orbit-detail__status')).toHaveText('Planned');
    await expect(page.locator('.orbit-detail__text')).toContainText('not current features');
    await page.screenshot({ path: test.info().outputPath('roadmap-orbit.png') });
  });
});

for (const width of [1440, 390]) {
  test(`presentation modes change visible content at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    const overview = page.getByRole('button', { name: 'Overview', exact: true });
    const technical = page.getByRole('button', { name: 'Technical', exact: true });
    await expect(page.locator('.hero__lead.ov-only')).toBeVisible();
    await expect(page.locator('.hero__lead.tech-only')).toBeHidden();
    await technical.click();
    await expect(technical).toHaveAttribute('aria-pressed', 'true');
    await expect(overview).toHaveAttribute('aria-pressed', 'false');
    await expect(page.locator('.hero__lead.ov-only')).toBeHidden();
    await expect(page.locator('.hero__lead.tech-only')).toBeVisible();
    await expect(page.locator('.hero__lead.tech-only')).toContainText('still in development');
    await page.screenshot({ path: test.info().outputPath(`hero-technical-${width}.png`) });
    await page.locator('#product').scrollIntoViewIfNeeded();
    await expect(page.locator('.product-grid .tech-only').first()).toBeVisible();
    await expect(page.locator('.product-grid .ov-only').first()).toBeHidden();
    await expect(page.locator('.product-boundary').first()).toBeVisible();
    await page.screenshot({ path: test.info().outputPath(`product-technical-${width}.png`) });
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.getByRole('link', { name: 'Try Eplyx' }).click();
    await expect(page.locator('.guide-intro [data-technical]')).toBeVisible();
    await page.reload();
    await expect(technical).toHaveAttribute('aria-pressed', 'true');
    await expect(page.locator('.guide-intro [data-technical]')).toBeVisible();
    await overview.click();
    await expect(page.locator('.guide-intro [data-technical]')).toBeHidden();
    await expect(page.locator('#command-install')).toBeVisible();
    await page.goto('/token-transitions');
    await expect(page.locator('.analyse-copy [data-technical]')).toBeHidden();
    await technical.click();
    await expect(page.locator('.analyse-copy [data-technical]')).toBeVisible();
    await page.goto('/runs/demo');
    await expect(page.locator('#technical')).toBeVisible();
    await overview.click();
    await expect(page.locator('#technical')).toBeHidden();
    await page.goto('/analyse');
    await expect(page.getByRole('group', { name: 'Presentation', exact: true })).toHaveCount(0);
  });
}

test('presentation choice stays consistent when storage is blocked', async ({ page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, 'localStorage', { get() { throw new Error('Storage unavailable'); } });
  });
  await page.goto('/');
  await page.getByRole('button', { name: 'Technical', exact: true }).click();
  await page.getByRole('link', { name: 'Try Eplyx' }).click();
  await expect(page.getByRole('button', { name: 'Technical', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.guide-intro [data-technical]')).toBeVisible();
});
