// Real product footage, one named clip per invocation:
//   node capture-web.mjs <clip> [outdir]
// Pages are the unmodified local services: the public site on :4173 (API on
// :4390), the hosted workspace on :4390, the local dashboard on :4185.
import { launch, Clip } from './recorder.mjs';
import { readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';

const [, , name, outArg] = process.argv;
const OUT = outArg || '/tmp/claude-0/-home-user-Eplyx/72216ad1-bced-5eac-8781-c0ed5a8f02ef/scratchpad/clips';
const SITE = 'http://127.0.0.1:4173', API = 'http://127.0.0.1:4390', DASH = 'http://127.0.0.1:4185';
const seed = existsSync('/srv/eplyx-demo/seed.json') ? JSON.parse(await readFile('/srv/eplyx-demo/seed.json', 'utf8')) : {};
const operator = existsSync('/srv/eplyx-demo/operator-token') ? (await readFile('/srv/eplyx-demo/operator-token', 'utf8')).trim() : '';
const browser = await launch();

async function signedIn() {
  const ctx = await browser.newContext();
  const p = await ctx.newPage();
  await p.goto(API + '/login');
  await p.getByLabel('Email').fill(seed.email); await p.getByLabel('Password').fill(seed.password);
  await p.locator('button[type=submit]').click(); await p.waitForURL(u => !u.pathname.startsWith('/login'));
  const state = await ctx.storageState(); await ctx.close(); return state;
}

const clips = {
  // Public: choose a question on /start.
  async start(c, page) {
    await page.goto(SITE + '/start', { waitUntil: 'networkidle' }); c.mark('loaded');
    await c.wait(900);
    const options = ['Token migration', 'Lifecycle terms', 'Current token path', 'Protocol fee change', 'Squads upgrade proposal', 'Program upgrade'];
    for (const o of options) { await c.click(page.locator('label, button, [role=radio]').filter({ hasText: o }).first(), 'choose ' + o, 520); await c.wait(650); }
    await c.wait(500);
  },
  // Public: the saved demo report, no account.
  async demo(c, page) {
    await page.goto(SITE + '/runs/demo', { waitUntil: 'networkidle' }); c.mark('loaded');
    await c.wait(1400);
    await c.wheel(520, 'scroll analysis', 1400); await c.wait(900);
    await c.wheel(560, 'scroll impact', 1400); await c.wait(1100);
    await c.wheel(700, 'scroll not explained', 1500); await c.wait(900);
  },
  // Public: CLI guide.
  async cli(c, page) {
    await page.goto(SITE + '/cli', { waitUntil: 'networkidle' }); c.mark('loaded');
    await c.wait(1200); await c.wheel(900, 'scroll install', 1600); await c.wait(700); await c.wheel(900, 'scroll upgrades', 1600); await c.wait(900);
  },
  // Operator console before the first check: the guided setup checklist.
  async console(c, page) {
    await page.goto(SITE + '/projects', { waitUntil: 'networkidle' }); c.mark('loaded'); await c.wait(800);
    await c.type(page.locator('#operator-token'), operator, 'operator token', 10);
    await page.keyboard.press('Enter'); c.mark('connect'); await c.wait(1500);
    await c.click(page.getByText('stake-pool-upgrades').first(), 'open project', 700);
    await c.wait(1800);
    const setup = page.locator('[data-setup], .setup, section').filter({ hasText: /first check|First check/i }).first();
    if (await setup.count()) await c.scrollTo(setup, 'setup checklist', 90);
    await c.wait(2000);
  },
  // Hosted workspace: the CI-submitted run and its impact view.
  async hosted(c, page) {
    await page.goto(`${API}/p/${seed.upgradeProject}/runs`, { waitUntil: 'networkidle' }); c.mark('runs loaded');
    await page.locator('tbody tr').first().waitFor(); await c.wait(1200);
    await c.click(page.locator('tbody tr a').first(), 'open run', 700);
    await page.locator('#impact, .hosted-upgrade-report').first().waitFor(); c.mark('report loaded');
    await c.wait(1800);
    for (const id of ['#change', '#impact']) { const el = page.locator(id); if (await el.count()) { await c.scrollTo(el, 'scroll ' + id, 100); await c.wait(1500); } }
    await c.wheel(500, 'scroll impact details', 1300); await c.wait(1300);
  },
  // Local dashboard (eplyx dashboard): the failed rehearsal, its stress matrix and gate.
  async dashboard(c, page) {
    await page.goto(DASH + '/', { waitUntil: 'networkidle' }); c.mark('overview'); await c.wait(1500);
    const row = page.locator('tr').filter({ hasText: 'Failed' }).first();
    await c.scrollTo(row, 'recent runs', 300); await c.wait(700);
    await c.click(row.locator('a').first(), 'open failed run', 700);
    await page.locator('main h1').filter({ hasText: 'Failed' }).waitFor(); c.mark('run loaded'); await c.wait(1800);
    await c.scrollTo(page.locator('#stress'), 'stress matrix', 90); await c.wait(900);
    const dl = page.locator('#stress tr').filter({ hasText: 'At Deadline' }).first();
    await c.pointAt(dl, 700); c.mark('deadline row'); await c.wait(1500);
    await c.scrollTo(page.locator('#gate'), 'deployment gate', 90); await c.wait(1800);
  },
  // Local dashboard: the saved counterexample, then two runs compared.
  async dashboard2(c, page) {
    const cx = (await (await fetch(DASH + '/api/counterexamples')).json());
    const id = (cx.counterexamples || cx)[0].id;
    await page.goto(DASH + '/counterexamples/' + id, { waitUntil: 'networkidle' }); c.mark('counterexample'); await c.wait(1800);
    const cmd = page.locator('.command').filter({ hasText: 'eplyx migration reproduce' }).first();
    if (await cmd.count()) { await c.scrollTo(cmd, 'reproduce command', 200); await c.wait(500); const b = cmd.getByRole('button', { name: /Copy/ }); if (await b.count()) await c.click(b, 'copy reproduce', 600); await c.wait(1000); }
    await page.goto(DASH + '/runs', { waitUntil: 'networkidle' }); c.mark('runs'); await c.wait(900);
    await c.click(page.getByLabel('Select run #1 for comparison'), 'select #1', 600);
    await c.click(page.getByLabel('Select run #2 for comparison'), 'select #2', 500);
    await c.click(page.getByRole('button', { name: /Compare/ }).first(), 'compare', 600);
    await c.wait(2400); await c.wheel(500, 'scroll comparison', 1200); await c.wait(1400);
  },
  // Local dashboard: the lifecycle record written with --record.
  async dashboard3(c, page) {
    const runs = (await (await fetch(DASH + '/api/runs')).json()).runs;
    const lc = runs.find(r => r.kind === 'lifecycle_change');
    await page.goto(DASH + '/runs/' + lc.id, { waitUntil: 'networkidle' }); c.mark('lifecycle'); await c.wait(1800);
    await c.wheel(420, 'policy', 1200); await c.wait(1300); await c.wheel(380, 'consequences', 1100); await c.wait(1400);
  },
  // Approving the CLI's device code (run while `eplyx login` waits).
  async device(c, page) {
    await page.goto(process.env.DEVICE_URL, { waitUntil: 'networkidle' }); c.mark('device page');
    await page.locator('.device-code').waitFor(); await c.wait(1600);
    await c.click(page.getByRole('button', { name: 'Approve' }), 'approve', 800);
    await page.locator('.device-outcome').waitFor(); c.mark('approved'); await c.wait(2000);
  },
  // Workspace overview and run history after `eplyx sync`.
  async workspace(c, page) {
    await page.goto(API + '/workspaces', { waitUntil: 'networkidle' }); c.mark('workspaces'); await c.wait(1500);
    await c.click(page.getByText('token-transitions').first(), 'open project', 700); await c.wait(1800);
    await page.goto(`${API}/p/${seed.transitionsProject}/runs`, { waitUntil: 'networkidle' }); c.mark('runs'); await c.wait(1800);
    await c.pointAt(page.locator('tbody tr').first(), 600); await c.wait(1200);
  },
  // Live current state: a real mint and public owner read from mainnet now,
  // then one exact Transfer executed offline in the local VM.
  async current(c, page) {
    await page.goto(`${API}/p/${seed.transitionsProject}/analyse`, { waitUntil: 'networkidle' }); c.mark('form'); await c.wait(1200);
    await c.scrollTo(page.locator('#current-state'), 'scope', 90);
    await c.type(page.getByLabel('Mint address'), '2b1kV6DkPAnxd5ixfnxCpjxmKwqjjaYmCZfHsFu24GXo', 'mint', 18);
    await c.pointAt(page.getByLabel('Inspection scope'), 500); await page.getByLabel('Inspection scope').selectOption('wallet'); c.mark('wallet scope'); await c.wait(400);
    await c.type(page.getByLabel('Public owner address'), '5gUuDFHswKi2QMA1qJHf6FEVhNCrHnyAdfWniMaUUPE4', 'owner', 18);
    await c.click(page.getByRole('button', { name: 'Observe current state', exact: true }), 'observe', 600);
    await page.getByLabel('Focused account').waitFor({ timeout: 120000 }); c.mark('observed'); await c.wait(900);
    await c.scrollTo(page.locator('[data-observation]'), 'observation', 90); await c.wait(1800);
    await c.scrollTo(page.locator('[data-path-form]'), 'path form', 120); await c.wait(500);
    await page.getByLabel('Amount', { exact: true }).selectOption('Custom');
    await c.type(page.getByLabel('Custom amount', { exact: true }), '1', 'amount', 60);
    await c.type(page.getByLabel('Recipient token account'), '7RCBfgRgm3pMv42ZzvQNuXydThVatpRcYbcWUNvyMCJN', 'recipient', 16);
    await c.click(page.getByRole('button', { name: 'Run offline path check' }), 'run path', 600);
    await page.locator('[data-path-result]').filter({ hasText: 'Recorded result' }).waitFor({ timeout: 120000 }); c.mark('path result');
    await c.scrollTo(page.locator('[data-path-result]'), 'focus result', 140); await c.wait(2200);
    await writeFile('/srv/eplyx-demo/observation-url', page.url());
  },
  async current2(c, page) {
    const url = (await readFile('/srv/eplyx-demo/observation-url', 'utf8')).trim();
    await page.goto(url, { waitUntil: 'networkidle' }); await page.getByLabel('Focused account').waitFor(); c.mark('observation'); await c.wait(600);
    await c.scrollTo(page.locator('[data-candidate-form]'), 'candidate form', 110); await c.wait(500);
    await c.type(page.getByLabel('Replacement mint', { exact: true }), 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v', 'replacement', 16);
    await page.getByRole('combobox', { name: 'Candidate amount', exact: true }).selectOption('Custom');
    await c.type(page.getByLabel('Candidate custom amount'), '1', 'amount', 60);
    await c.click(page.getByRole('button', { name: 'Run candidate check' }), 'run candidate', 600);
    await page.locator('[data-candidate-result]').filter({ hasText: /Exact checks|Verified/ }).waitFor({ timeout: 120000 }); c.mark('candidate result');
    await c.scrollTo(page.locator('[data-candidate-result]'), 'focus candidate', 140); await c.wait(2200);
  },
  // Operator console after checks: checklist complete and the ops view.
  async consoleAfter(c, page) {
    await page.goto(SITE + '/projects', { waitUntil: 'networkidle' }); c.mark('loaded'); await c.wait(600);
    await c.type(page.locator('#operator-token'), operator, 'operator token', 10);
    await page.keyboard.press('Enter'); c.mark('connect'); await c.wait(1600);
    await page.locator('#operations').waitFor(); await c.wait(900);
    await c.scrollTo(page.locator('#operations'), 'ops view', 90);
    await c.wait(1800);
    await c.scrollTo(0, 'top'); await c.wait(500);
    await c.click(page.getByText('stake-pool-upgrades').first(), 'open project', 700);
    await c.wait(1700);
    const setup = page.locator('section').filter({ hasText: /first check|First check/i }).first();
    if (await setup.count()) await c.scrollTo(setup, 'setup checklist', 90);
    await c.wait(1800);
  },
};

const clipName = name;
const fn = clips[clipName.replace(/-.*/, '')] || clips[clipName];
if (!fn) throw new Error('unknown clip ' + clipName);
const needsAuth = ['hosted', 'workspace', 'current', 'device', 'lifecycleForm'].some(n => clipName.startsWith(n));
const c = new Clip(browser, OUT, clipName, needsAuth ? { storageState: await signedIn() } : {});
const page = await c.open();
await c.start();
try { await fn(c, page); }
finally { await c.close(); await browser.close(); }
