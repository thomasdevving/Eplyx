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
    await expect(page).toHaveURL(/\/start$/);
    await expect(page.getByRole('heading', { name: 'Start with your question.' })).toBeVisible();
    await page.screenshot({ path: test.info().outputPath('start-' + width + '.png'), fullPage: true });
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.getByRole('link', { name: 'Install the CLI', exact: false }).click();
    await expect(page).toHaveURL(/\/cli#install$/);
    await expect(page.getByRole('heading', { name: 'Build the CLI from source.' })).toBeVisible();
    await expect(page.locator('#migration')).toContainText('init alone does not create a complete runnable migration');
    await expect(page.locator('#sync')).toContainText('configuration');
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
    await page.getByRole('navigation', { name: 'CLI guide sections' }).getByRole('link', { name: 'Evaluate lifecycle terms', exact: true }).click();
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
    await expect(page.locator('#start-upgrades [data-technical]')).toBeVisible();
    await page.getByRole('link', { name: 'Install the CLI' }).click();
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
  await page.getByRole('link', { name: 'Install the CLI' }).click();
  await expect(page.getByRole('button', { name: 'Technical', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.locator('.guide-intro [data-technical]')).toBeVisible();
});

for (const width of [1440, 390]) {
  test('workflow selection explains inputs, availability and route at ' + width + 'px', async ({ page }) => {
    await page.setViewportSize({ width, height: 1000 });
    await page.goto('/start');
    await expect(page.locator('.workflow-panel:visible')).toHaveCount(1);
    await expect(page.locator('#start-upgrades')).toContainText('active bundle');
    await page.getByRole('radio', { name: /Token migration/ }).check();
    await expect(page).toHaveURL(/#migration$/);
    await expect(page.locator('#start-migration')).toBeVisible();
    await expect(page.locator('#start-upgrades')).toBeHidden();
    await expect(page.locator('#start-migration')).toContainText('not a general migration upload form');
    await page.reload();
    await expect(page.getByRole('radio', { name: /Token migration/ })).toBeChecked();
    await page.getByRole('radio', { name: /Lifecycle terms/ }).check();
    await expect(page.locator('#start-lifecycle')).toContainText('Changing policy time does not refresh account state');
    await page.getByRole('radio', { name: /Current token path/ }).check();
    await expect(page.locator('#start-paths')).toContainText('Liquidity-withdrawal checks currently use the CLI');
    await page.getByRole('radio', { name: /Squads upgrade proposal/ }).check();
    await expect(page.locator('#start-governance')).toContainText('does not sign, approve or execute');
    await page.locator('#start-governance').getByRole('link', { name: 'CLI commands and inputs' }).click();
    await expect(page).toHaveURL(/\/cli#governance$/);
    await expect(page.getByRole('heading', { name: 'Match a proposal to the analysed build.' })).toBeInViewport();
    await expect(page.locator('#governance')).toContainText('SOLANA_RPC_URL');
    await page.getByText('Attest deployment after execution', { exact: true }).click();
    await expect(page.locator('#command-attest')).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
  });
}

test('workflow deep links, keyboard choice and configured workspace navigation', async ({ page }) => {
  await page.route('**/public/runtime-config.js', route => route.fulfill({
    contentType: 'text/javascript', body: 'globalThis.EPLYX_API_URL = "https://api.example.test";',
  }));
  await page.goto('/start#paths');
  await expect(page.locator('#start-paths')).toBeVisible();
  await expect(page.locator('#start-paths').getByRole('link', { name: 'Open workspace' }))
    .toHaveAttribute('href', 'https://api.example.test/workspaces');
  await page.getByRole('radio', { name: /Current token path/ }).focus();
  await page.keyboard.press('ArrowUp');
  await expect(page.getByRole('radio', { name: /Lifecycle terms/ })).toBeChecked();
  await expect(page.locator('#start-lifecycle')).toBeVisible();
  await page.goto('/start#unknown');
  await expect(page.locator('#start-upgrades')).toBeVisible();
  await page.locator('#start-upgrades').getByRole('link', { name: 'Open upgrade form' }).click();
  await expect(page.getByRole('heading', { name: 'Analyse a program upgrade.' })).toBeVisible();
  await page.getByRole('link', { name: 'choose a workflow', exact: true }).click();
  await page.getByRole('link', { name: 'Explore a demo' }).click();
  await expect(page).toHaveURL(/\/runs\/demo$/);
});

test('CLI task index resolves every section and distinguishes CI and saved-result sync', async ({ page }) => {
  await page.goto('/cli');
  const links = await page.getByRole('navigation', { name: 'CLI guide sections' }).getByRole('link').all();
  for (const link of links) {
    const href = await link.getAttribute('href');
    await link.click();
    await expect(page.locator(href).getByRole('heading', { level: 2 })).toBeInViewport();
  }
  await expect(page.locator('#prepare')).toContainText('Discovery alone does not create replay-ready evidence');
  await expect(page.locator('#compare')).toContainText('Use ci check for expectation-based gating');
  await page.goto('/cli#ci');
  await page.getByText('Use an existing hosted project instead', { exact: true }).click();
  await expect(page.locator('#command-hosted-ci')).toHaveText(CLI_COMMANDS.hostedCi);
  await expect(page.locator('#sync')).toContainText('Upgrade CI uses the separate submission flow');
});
