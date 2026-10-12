// Footage of the live site, recorded through this environment's proxy:
//   node capture-live.mjs <clip> [outdir]
// Only eplyx.dev is reachable from the clip; every other host is refused.
import { launch, Clip } from '../../demo-2026-10/source/recorder.mjs';

const [, , name, outArg] = process.argv;
const OUT = outArg || '/tmp/claude-0/-home-user-Eplyx/72216ad1-bced-5eac-8781-c0ed5a8f02ef/scratchpad/clips';
const SITE = 'https://eplyx.dev';

const clips = {
  // The CLI guide on eplyx.dev, down to the hosted-RPC block added this week,
  // then the workspace link, served from the same domain.
  async live(c, page) {
    await page.goto(SITE + '/cli', { waitUntil: 'load' });
    await page.getByText('From your terminal').first().waitFor();
    c.mark('loaded');
    await c.wait(1300);
    await c.scrollTo(page.locator('#install'), 'install', 90);
    await c.wait(700);
    await c.pointAt(page.getByText('Reading Solana needs no RPC setup.').first(), 700);
    await c.wait(1300);
    await c.pointAt(page.getByText('Unlock the hosted Solana RPC').first(), 700);
    await c.wait(1500);
    await c.scrollTo(0, 'top');
    await c.wait(400);
    await c.click(page.getByRole('link', { name: /Workspace/ }).first(), 'workspace', 700);
    await page.waitForURL(u => u.pathname.startsWith('/workspace') || u.pathname.startsWith('/login'), { timeout: 30000 });
    await page.waitForLoadState('load');
    c.mark('workspace loaded');
    await c.wait(2200);
  },
};

const fn = clips[name];
if (!fn) throw new Error('unknown clip ' + name);
const browser = await launch({ proxy: { server: process.env.HTTPS_PROXY } });
const c = new Clip(browser, OUT, name, { hosts: ['eplyx.dev'] });
const page = await c.open();
await c.start();
try { await fn(c, page); }
finally { await c.close({ url: SITE }); await browser.close(); }
