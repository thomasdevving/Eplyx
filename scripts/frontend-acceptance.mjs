// §48 production frontend acceptance, driven against the deployed site.
//
// Not a local frontend pointed at a local API: the URL below is the deployed
// one, and the API it talks to is whatever its baked runtime-config names.
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';

const SITE = process.argv[2];
const TOKEN = process.argv[3];
const PROJECT = process.argv[4];
const CANDIDATE = process.argv[5];
const PORT = 9333;

let failures = 0, checks = 0;
const ok = (name, cond, detail = '') => {
  checks++;
  if (cond) console.log(`  ok   ${name}`);
  else { failures++; console.log(`  FAIL ${name}${detail ? `\n       ${detail}` : ''}`); }
};

const chrome = spawn('/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', [
  `--remote-debugging-port=${PORT}`, '--headless=new', '--no-first-run',
  '--user-data-dir=' + process.env.TMPDIR + 'eplyx-cdp', 'about:blank',
], { stdio: 'ignore' });

const sleep = ms => new Promise(r => setTimeout(r, ms));
let ws, id = 0;
const pending = new Map();
const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const msg = { id: ++id, method, params };
  if (sessionId) msg.sessionId = sessionId;
  pending.set(msg.id, { resolve, reject });
  ws.send(JSON.stringify(msg));
});

try {
  let targets;
  for (let i = 0; i < 40; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${PORT}/json/version`)).json(); break; }
    catch { await sleep(500); }
  }
  ws = new WebSocket(targets.webSocketDebuggerUrl);
  await new Promise(r => ws.addEventListener('open', r));
  ws.addEventListener('message', event => {
    const data = JSON.parse(event.data);
    if (data.id && pending.has(data.id)) {
      const { resolve, reject } = pending.get(data.id);
      pending.delete(data.id);
      data.error ? reject(new Error(JSON.stringify(data.error))) : resolve(data.result);
    }
  });

  const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
  const S = sessionId;
  await send('Page.enable', {}, S);
  await send('Runtime.enable', {}, S);
  await send('DOM.enable', {}, S);
  // The host serves app files with max-age=300. Without this the browser
  // happily reuses a build from a previous run, so a page that had stopped
  // rendering something would still appear to render it. Mutation testing
  // found that: the control and the mutated build were the same bytes.
  await send('Network.enable', {}, S);
  await send('Network.setCacheDisabled', { cacheDisabled: true }, S);

  const evaluate = async expression => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }, S);
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || 'eval failed');
    return r.result.value;
  };
  const goto = async path => {
    await send('Page.navigate', { url: SITE + path }, S);
    await sleep(2500);
  };

  console.log(`\n=== §48 productie-frontend: ${SITE} ===\n`);

  // --- the site itself -------------------------------------------------
  await goto('/');
  ok('landing page laadt', await evaluate('document.title.length > 0'));
  const apiBase = await evaluate('globalThis.EPLYX_API_URL');
  ok('API-URL is de gedeployde backend, geen localhost',
     typeof apiBase === 'string' && apiBase.startsWith('https://') && !apiBase.includes('localhost'), apiBase);

  // --- operator console ------------------------------------------------
  await goto('/projects');
  const beforeToken = await evaluate('document.body.innerText');
  ok('console vraagt om een credential voor hij iets toont', /Connect this console/i.test(beforeToken));
  ok('console lekt geen projectgegevens zonder credential', !beforeToken.includes(PROJECT));

  await evaluate(`sessionStorage.setItem('eplyx-operator-token', ${JSON.stringify(TOKEN)})`);
  await goto('/projects');
  const listed = await evaluate('document.body.innerText');
  ok('pilotproject zichtbaar', listed.includes('Solana Stake Pool Pilot'), listed.slice(0, 200));
  ok('status ready zichtbaar', /ready/i.test(listed));

  await goto('/projects/' + PROJECT);
  const detail = await evaluate('document.body.innerText');
  ok('program id klopt', detail.includes('SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy'));
  ok('adapter klopt', detail.includes('spl-stake-pool'));
  ok('actieve bundle-hash zichtbaar', detail.includes('5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f')
     || detail.includes('5e5b67ac'), detail.slice(0, 300));
  ok('geen ruw projecttoken op de pagina', !/eplyx_proj_[0-9a-f]{8}/.test(detail));

  // --- analyse form ----------------------------------------------------
  await goto('/analyse');
  const analyse = await evaluate('document.body.innerHTML');
  ok('projectkiezer aanwezig', /<select[^>]*name="project"/.test(analyse));
  ok('geen handmatig backend-URL-veld', !/name="api"/.test(analyse));
  ok('geen handmatig project-id-veld', !/name="project"[^>]*type="text"/.test(analyse));
  await sleep(1500);
  const options = await evaluate(`document.querySelector('#project-select')?.innerHTML || ''`);
  ok('kiezer is gevuld vanuit de backend', options.includes('Solana Stake Pool Pilot'), options.slice(0, 200));

  // --- submit a real candidate ----------------------------------------
  const { root } = await send('DOM.getDocument', {}, S);
  const { nodeId } = await send('DOM.querySelector', { nodeId: root.nodeId, selector: 'input[name="candidate"]' }, S);
  ok('kandidaat-bestandsveld gevonden', nodeId > 0);
  await send('DOM.setFileInputFiles', { files: [CANDIDATE], nodeId }, S);
  await evaluate(`(() => {
    const select = document.querySelector('#project-select');
    select.value = ${JSON.stringify(PROJECT)};
    select.dispatchEvent(new Event('change', { bubbles: true }));
  })()`);
  await evaluate(`document.querySelector('#analyse-form button[type=submit], #analyse-form button')?.click()`);
  await sleep(4000);

  let path = await evaluate('location.pathname');
  ok('meteen naar /runs/{id} genavigeerd', /^\/runs\/run_/.test(path), path);
  const runId = path.split('/').pop();

  // --- lifecycle, refresh, report --------------------------------------
  let text = '';
  for (let i = 0; i < 40; i++) {
    text = await evaluate('document.body.innerText');
    if (/passed|failed|execution error/i.test(text)) break;
    await sleep(2000);
  }
  ok('run bereikte een eindtoestand in de UI', /passed|failed/i.test(text), text.slice(0, 200));

  await goto('/runs/' + runId);
  const afterRefresh = await evaluate('document.body.innerText');

  // The baseline is the bundle's own binary, so this run must pass. Accepting
  // "passed or failed" would let a report page that renders the wrong verdict -
  // or a submission that sent the wrong bytes - go unnoticed. Read after the
  // reload, when the page has settled, rather than the instant polling stopped.
  // Case-insensitive: the verdict badge is uppercased by CSS, and Chrome
  // reflects text-transform in innerText, so the rendered word is "PASSED".
  ok('run passeerde in de UI, zoals de baseline hoort te doen',
     /\bpassed\b/i.test(afterRefresh) && /exit code 0\b/i.test(afterRefresh),
     afterRefresh.slice(0, 300));
  ok('harde refresh behoudt toegang tot de run', afterRefresh.includes(runId.slice(0, 12)) || /passed|failed/i.test(afterRefresh));
  ok('rapport toont bundle-hash', afterRefresh.includes('5e5b67ac'), '');
  ok('rapport toont baseline-hash', afterRefresh.includes('ec2dfef'), '');
  ok('rapport toont kandidaat-hash', afterRefresh.includes('ec2dfef'), '');
  ok('rapport toont corpus-hash', afterRefresh.includes('a4b66731'), '');
  ok('rapport toont record count', /10\b/.test(afterRefresh));
  // Assert on the limitations' own text, not on the word "limitation": the
  // passing headline says "The coverage limitations below still apply", so a
  // page that had hidden the list entirely still matched the looser pattern.
  // Mutation testing found that; the guard was vacuous until it did.
  const shownLimitations = [
    'Transactions that failed on mainnet are observed but not replayed',
    'outside the exact historical contract',
    'refused by the replay runtime',
    'resolves addresses through a lookup table',
  ].filter(text => afterRefresh.includes(text));
  ok('coverage-limitations zichtbaar ondanks pass', shownLimitations.length >= 3,
     `${shownLimitations.length} van 4 gevonden`);
  ok('geen "safe"/"secure"-taal', !/\b(is safe|secure|no risk|production safe)\b/i.test(afterRefresh));

  await goto('/projects/' + PROJECT);
  const history = await evaluate('document.body.innerText');
  // The history identifies a run by what it measured - candidate, bundle,
  // verdict, exit code, time - rather than by an opaque id, so assert on that.
  ok('runhistorie toont de zojuist ingediende kandidaat', history.includes('ec2dfefaa7'), history.slice(0, 200));
  ok('runhistorie toont verdict en exitcode', /PASSED/.test(history) && /exit 0/.test(history));
  ok('runhistorie toont de bundle waartegen gemeten is', history.includes('5e5b67ac13'));
  const links = await evaluate(`[...document.querySelectorAll('a[href^="/runs/"]')].length`);
  ok('historie-rijen linken naar hun run', links > 0, `${links} links`);

  console.log(`\n${failures === 0 ? 'ALLE' : failures + ' VAN DE ' + checks} frontend-controles${failures === 0 ? ' geslaagd (' + checks + ')' : ' GEFAALD'}`);
} catch (error) {
  console.log('acceptance error: ' + error.message);
  failures++;
} finally {
  chrome.kill();
}
process.exit(failures === 0 ? 0 : 1);
