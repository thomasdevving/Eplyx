// Deterministic capture for animated pages (the WebGL hero): headless Chrome
// under begin-frame control and virtual time. Each output frame is produced by
// advancing the browser's virtual clock exactly 1/fps and asking the compositor
// for one frame, so requestAnimationFrame, timers and CSS animations run at
// their true speed no matter how slowly the software renderer draws.
//
//   node deterministic.mjs <url> <outdir> <seconds> [--fps 30] [--width 1440 --height 900 --scale 1.5]
//       [--script steps.json]   steps: [{"at":2.0,"scroll":600}, {"at":3,"mouse":[x,y]}]
import { spawn } from 'node:child_process';
import { mkdir, writeFile, rm, readFile } from 'node:fs/promises';
import { join } from 'node:path';

const args = process.argv.slice(2);
const opt = (k, d) => { const i = args.indexOf('--' + k); return i >= 0 ? args[i + 1] : d; };
const [url, out, seconds] = args;
const fps = +opt('fps', 30), width = +opt('width', 1440), height = +opt('height', 900), scale = +opt('scale', 1.5);
const steps = opt('script') ? JSON.parse(await readFile(opt('script'), 'utf8')) : [];
const shell = process.env.EPLYX_HEADLESS_SHELL || '/opt/pw-browsers/chromium_headless_shell-1194/chrome-linux/headless_shell';
const port = 9300 + Math.floor(Math.random() * 500);
await rm(out, { recursive: true, force: true }).catch(() => {}); await mkdir(out, { recursive: true });

const chrome = spawn(shell, [
  `--remote-debugging-port=${port}`, '--deterministic-mode', '--enable-begin-frame-control',
  '--disable-new-content-rendering-timeout', '--run-all-compositor-stages-before-draw',
  '--disable-threaded-animation', '--disable-threaded-scrolling', '--disable-checker-imaging',
  '--enable-unsafe-swiftshader', '--use-angle=swiftshader', '--hide-scrollbars', '--no-sandbox',
  `--window-size=${width},${height}`, '--user-data-dir=' + join(out, '.profile'), 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let wsUrl;
await new Promise((resolve, reject) => {
  chrome.stderr.on('data', d => { if (process.env.DEBUG) process.stderr.write(d); const m = String(d).match(/ws:\/\/\S+/); if (m && !wsUrl) { wsUrl = m[0]; resolve(); } });
  chrome.on('exit', c => reject(new Error('chrome exited ' + c)));
});

const ws = new WebSocket(wsUrl);
await new Promise(r => ws.addEventListener('open', r, { once: true }));
let id = 0; const pending = new Map(); const listeners = [];
ws.addEventListener('message', ev => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) { const { resolve, reject } = pending.get(msg.id); pending.delete(msg.id); msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result); }
  else listeners.forEach(l => l(msg));
});
const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const mid = ++id; pending.set(mid, { resolve, reject }); ws.send(JSON.stringify({ id: mid, method, params, ...(sessionId ? { sessionId } : {}) }));
});
const waitEvent = (method, sessionId) => new Promise(r => { const l = m => { if (m.method === method && (!sessionId || m.sessionId === sessionId)) { listeners.splice(listeners.indexOf(l), 1); r(m.params); } }; listeners.push(l); });

const { targetId } = await send('Target.createTarget', { url: 'about:blank', enableBeginFrameControl: true, width, height });
const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
const S = (m, p) => send(m, p, sessionId);
await S('Page.enable'); await S('Runtime.enable');
await S('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: scale, mobile: false });
await S('Fetch.enable', { patterns: [{ urlPattern: '*' }] });
listeners.push(m => {
  if (m.method !== 'Fetch.requestPaused' || m.sessionId !== sessionId) return;
  const host = new URL(m.params.request.url).hostname;
  const allowed = ['127.0.0.1', 'localhost'].includes(host);
  S(allowed ? 'Fetch.continueRequest' : 'Fetch.failRequest', allowed ? { requestId: m.params.requestId } : { requestId: m.params.requestId, errorReason: 'BlockedByClient' }).catch(() => {});
});

// Pause virtual time, navigate, then let the load proceed under virtual time.
await S('Emulation.setVirtualTimePolicy', { policy: 'pause' });
S('Page.navigate', { url });

const interval = 1000 / fps;
let ticks = 0;
const frames = [];
const total = Math.round(+seconds * fps);
// Warm up: allow network + parsing before the first frame (virtual time paused
// blocks timers, not network). Advance in small budgets until load fires.
let loaded = false; waitEvent('Page.loadEventFired', sessionId).then(() => { loaded = true; });
for (let i = 0; i < 400 && !loaded; i++) {
  await S('Emulation.setVirtualTimePolicy', { policy: 'advance', budget: 10 });
  await new Promise(r => setTimeout(r, 25));
}
await new Promise(r => setTimeout(r, 500));
for (let f = 0; f < total; f++) {
  const t = f / fps;
  for (const s of steps.filter(s => Math.abs(s.at - t) < 0.5 / fps)) {
    if (s.scroll !== undefined) await S('Runtime.evaluate', { expression: `window.scrollTo({top:${s.scroll},behavior:'smooth'})` });
    if (s.mouse) await S('Input.dispatchMouseEvent', { type: 'mouseMoved', x: s.mouse[0], y: s.mouse[1] });
    if (s.eval) await S('Runtime.evaluate', { expression: s.eval });
  }
  const budget = waitEvent('Emulation.virtualTimeBudgetExpired', sessionId);
  await S('Emulation.setVirtualTimePolicy', { policy: 'advance', budget: interval });
  await budget;
  ticks += interval;
  const r = await S('HeadlessExperimental.beginFrame', { frameTimeTicks: 1e6 + ticks, interval, screenshot: { format: 'jpeg', quality: 90 } });
  if (r.screenshotData) {
    const file = `${String(f).padStart(5, '0')}.jpg`;
    await writeFile(join(out, file), Buffer.from(r.screenshotData, 'base64'));
    frames.push({ t: +t.toFixed(4), file });
  }
  if (f % 30 === 0) console.log(`frame ${f}/${total}`);
}
await writeFile(join(out, 'clip.json'), JSON.stringify({ name: out.split('/').pop(), url, deterministic: true, fps, viewport: [width, height], scale, frame_size: [width * scale, height * scale], duration: +seconds, frames, events: [] }));
ws.close(); chrome.kill();
await rm(join(out, '.profile'), { recursive: true, force: true });
console.log(`saved ${frames.length} deterministic frames`);
