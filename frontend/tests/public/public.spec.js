import { test, expect } from '@playwright/test';
import { CLI_COMMANDS } from '../../src/cli.js';
import { LEGAL_ROUTES } from '../../src/legal.js';

test.beforeEach(async ({ page }) => {
  page.on('pageerror', error => { throw error; });
  await page.route('**/*', route => new URL(route.request().url()).hostname === '127.0.0.1'
    ? route.continue() : route.abort());
});

for (const width of [1440, 390, 320]) {
  test(`legal notices navigate, reload and fit at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await page.getByRole('navigation', { name: 'Legal and privacy' }).getByRole('link', { name: 'Legal & privacy', exact: true }).click();
    await expect(page).toHaveURL(/\/legal$/);
    for (const [path, label] of LEGAL_ROUTES) {
      await page.getByRole('navigation', { name: 'Legal documents' }).getByRole('link', { name: label, exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`${path}$`));
      await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
      await expect(page).toHaveTitle(/— Eplyx$/);
      await expect(page.getByRole('complementary', { name: 'Notice status' })).toContainText('Draft');
      if (path !== '/licenses') await expect(page.locator('main a[href="mailto:eplyxcontact@gmail.com"]')).not.toHaveCount(0);
      expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(1);
      await page.reload();
      await expect(page.getByRole('navigation', { name: 'Legal documents' }).getByRole('link', { name: label, exact: true })).toHaveAttribute('aria-current', 'page');
      if (path === '/privacy') await page.screenshot({ path: test.info().outputPath(`privacy-${width}.png`), fullPage: true });
    }
    await page.goBack();
    await expect(page).toHaveURL(/\/contact$/);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Contact & operator.');
  });
}

test('licence notices are distributed as readable text', async ({ page, request }) => {
  await page.goto('/licenses');
  for (const [name, text] of [['Read the bundled Three.js licence', 'MIT License'], ['DM Sans copyright and licence', 'SIL OPEN FONT LICENSE'], ['Manrope copyright and licence', 'SIL OPEN FONT LICENSE']]) {
    const href = await page.getByRole('link', { name, exact: true }).getAttribute('href');
    const response = await request.get(href);
    expect(response.status()).toBe(200);
    expect(response.headers()['content-type']).toContain('text/plain');
    expect(await response.text()).toContain(text);
  }
});

test('public browsing adds no cookies or persistent preference storage', async ({ page, context }) => {
  for (const path of ['/', '/runs/demo', '/privacy', '/cookies']) {
    await page.goto(path);
    await expect(page.locator('main')).toBeVisible();
    expect(await context.cookies()).toEqual([]);
    expect(await page.evaluate(() => ({ local: Object.keys(localStorage), session: Object.keys(sessionStorage) }))).toEqual({ local: [], session: [] });
  }
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

test('mobile menu closes on Escape without a presentation toggle', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/cli');
  await page.getByRole('button', { name: 'Open navigation' }).click();
  const nav = page.getByRole('navigation', { name: 'Primary navigation' });
  await nav.getByRole('link', { name: 'CLI', exact: true }).focus();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Open navigation' })).toHaveAttribute('aria-expanded', 'false');
  await page.getByRole('button', { name: 'Open navigation' }).click();
  await nav.getByRole('link', { name: 'Token transitions' }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Know what a transition changes.');
  await expect(page.getByRole('group', { name: 'Presentation', exact: true })).toHaveCount(0);
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

  test('intro and orbit stay animated while section text stays visible', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/');
    await expect(page.locator('.intro')).toBeVisible();
    await expect(page.locator('.intro')).toHaveCSS('animation-name', 'introExit');
    await expect(page.locator('.intro')).toHaveCount(0, { timeout: 6000 });
    await expect(page.getByRole('link', { name: 'Try Eplyx' })).toBeVisible();
    const planet = page.locator('.orbit-body[data-ring="changes"]').first();
    const before = await planet.evaluate(el => getComputedStyle(el).transform);
    await expect.poll(() => planet.evaluate(el => getComputedStyle(el).transform)).not.toBe(before);
    await expect(page.locator('#product .section-heading')).toHaveCSS('opacity', '1');
    await page.locator('#product .section-heading').scrollIntoViewIfNeeded();
    await expect(page.locator('#product .section-heading')).toHaveCSS('opacity', '1');
    await page.screenshot({ path: test.info().outputPath('product-normal-motion.png') });

    await page.getByRole('link', { name: 'Try Eplyx' }).click();
    await page.locator('.site-header .logo-link').click();
    await expect(page.locator('.intro')).toHaveCount(0);
    expect(await page.evaluate(() => ({ local: Object.keys(localStorage), session: Object.keys(sessionStorage) }))).toEqual({ local: [], session: [] });
    await page.getByRole('button', { name: 'Consequences', exact: true }).click();
    await expect(page.locator('.orbit-caption')).toContainText('in development');
    const future = page.locator('.orbit-body[data-name="Monitoring"]');
    await expect(future).toHaveCSS('visibility', 'visible');
    await page.getByRole('button', { name: 'Consequences', exact: true }).focus();
    await page.keyboard.press('Shift+Tab');
    await expect(page.getByRole('button', { name: 'Changes', exact: true })).toBeFocused();
    await page.keyboard.press('Shift+Tab');
    await expect(future).toBeFocused();
    await expect(page.locator('.orbit-detail')).toBeVisible();
    await expect(page.locator('.orbit-detail__status')).toHaveText('Planned');
    await expect(page.locator('.orbit-detail__text')).toContainText('not current features');
    await page.screenshot({ path: test.info().outputPath('roadmap-orbit.png') });
  });
});

for (const width of [1440, 390]) {
  test(`public content needs no presentation toggle at ${width}px`, async ({ page }) => {
    // A preference left by the separate dashboard must not switch public copy.
    await page.addInitScript(() => localStorage.setItem('eplyx-detail', 'technical'));
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await expect(page.getByRole('group', { name: 'Presentation', exact: true })).toHaveCount(0);
    await expect(page.locator('.hero__lead.ov-only')).toBeVisible();
    await expect(page.locator('.hero__lead.tech-only')).toBeHidden();
    await expect(page.locator('#evidence [data-technical]').first()).toBeVisible();
    await page.getByRole('link', { name: 'Try Eplyx' }).click();
    await expect(page.locator('#start-upgrades [data-technical]')).toBeVisible();
    await page.getByRole('link', { name: 'Install the CLI' }).click();
    await expect(page.locator('.guide-intro [data-technical]')).toBeVisible();
    await page.reload();
    await expect(page.getByRole('group', { name: 'Presentation', exact: true })).toHaveCount(0);
    await page.goto('/runs/demo');
    await expect(page.locator('#technical')).toBeVisible();
  });

  test(`orbit line cutouts follow the rock silhouettes at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    for (const ring of ['changes', 'consequences']) {
      await page.getByRole('button', { name: ring === 'changes' ? 'Changes' : 'Consequences', exact: true }).click();
      const cutouts = await page.evaluate(ring => {
        const stage = document.querySelector('.core-stage').getBoundingClientRect();
        return [...document.querySelectorAll(`.orbit-body[data-ring="${ring}"]`)].map(body => {
          const cutout = document.querySelector(`[data-rock-cutout="${body.dataset.index}"]`);
          const matrix = cutout.getCTM();
          const centre = new DOMPoint(.5, .5).matrixTransform(matrix);
          const rock = body.getBoundingClientRect();
          return {
            distance: Math.hypot(centre.x - (rock.x + rock.width / 2 - stage.x), centre.y - (rock.y + rock.height / 2 - stage.y)),
            image: cutout.querySelector('image').getAttribute('href'),
          };
        });
      }, ring);
      expect(cutouts).toHaveLength(4);
      for (const cutout of cutouts) {
        expect(cutout.distance).toBeLessThan(.2);
        expect(cutout.image).toBe('/public/orbit-rocks.png');
      }
      await expect(page.locator('.orbit-plane > g, .orbit-leaders > g')).toHaveCount(3);
      for (const lines of await page.locator('.orbit-plane > g, .orbit-leaders > g').all()) {
        await expect(lines).toHaveAttribute('mask', 'url(#orbit-rock-mask)');
      }
    }
  });
}

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
    await expect(page.locator('#start-migration')).toContainText('guided form accepts a prepared candidate');
    await page.reload();
    await expect(page.getByRole('radio', { name: /Token migration/ })).toBeChecked();
    await page.getByRole('radio', { name: /Lifecycle terms/ }).check();
    await expect(page.locator('#start-lifecycle')).toContainText('Changing policy time does not refresh account state');
    await page.getByRole('radio', { name: /Current token path/ }).check();
    await expect(page.locator('#start-paths')).toContainText('Liquidity-withdrawal checks currently use the CLI');
    await page.getByRole('radio', { name: /Protocol fee change/ }).check();
    await expect(page.locator('#start-parameters')).toBeVisible();
    await expect(page.locator('#start-parameters')).toContainText('there is no standalone guided fee form');
    await page.locator('#start-parameters').getByRole('link', { name: 'CLI commands and inputs' }).click();
    await expect(page).toHaveURL(/\/cli#parameters$/);
    await expect(page.locator('#command-parameter')).toHaveText(CLI_COMMANDS.parameter);
    await expect(page.locator('#parameter-search')).toContainText('codex/analysis-integration');
    await page.goto('/start#parameters');
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


test('saved demo uses retained evidence and leaves absent proof unavailable', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.result-summary')).toContainText('Finding categories');
  await expect(page.locator('.result-summary')).toContainText('1 economic decrease · 9 reverts');
  await page.locator('#evidence').getByRole('link', { name: 'Open demo report' }).click();
  await expect(page.locator('main')).toContainText('One deposit loses pool tokens; nine withdrawals revert');
  await expect(page.locator('main')).toContainText('predates ChangeSpec identities');
  await page.locator('#technical summary').click();
  await expect(page.locator('#technical')).toContainText('3193eabd9fe2e479109ef3b2dd7301fffd06774325133ff8f88916ed482db099');
  await expect(page.locator('#technical')).toContainText('447850493 → 447904974');
  await expect(page.locator('main')).not.toContainText('60b7e1ac');
});
