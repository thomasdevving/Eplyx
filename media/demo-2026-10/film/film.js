// Film runtime: background, HUD, scene scheduling, transitions, preview player
// and the deterministic export entry point window.renderAt(t).
import { W, H, FPS, clamp, lerp, seg, ease, h, put, rand } from './core.js';
import { scenes, chapters } from './scenes.js';

const MARK = {
  crescent: 'M355 68C316 26 265 10 219 11 124 9 44 69 18 152 4 197 8 242 17 270c3 10 12 11 26 9 30-3 63-19 87-43-14-35-11-65 5-95 22-42 58-76 99-93 44-18 86-7 121 20Z',
  wave: 'M24 291c31 13 60 9 93-12 31-20 64-48 93-69 40-29 81-45 122-43 36 1 71 16 84 34 7 10 4 19-5 24-35 23-61 53-85 84-33 43-65 76-107 90-79 27-157-21-195-108Z',
};
let markN = 0;
export function markSVG(cls = '') {
  const id = 'mk' + (++markN);
  return `<svg class="${cls}" viewBox="0 0 440 440"><defs><linearGradient id="${id}" x1="40" y1="40" x2="370" y2="400" gradientUnits="userSpaceOnUse"><stop stop-color="#ffffff"/><stop offset=".54" stop-color="#eeedf3"/><stop offset="1" stop-color="#c4c1d2"/></linearGradient></defs><path class="mk-c" fill="url(#${id})" d="${MARK.crescent}"/><path class="mk-w" fill="url(#${id})" d="${MARK.wave}"/></svg>`;
}
window.markSVG = markSVG;

const stage = document.getElementById('stage');
const params = new URLSearchParams(location.search);
const exporting = params.has('export');
if (exporting) document.body.classList.add('export');

// --- timeline -------------------------------------------------------------
let at = 0;
for (const s of scenes) { s.at = at; s.end = at + s.dur; at = s.end; }
export const DURATION = at;
window.filmDuration = DURATION;
window.filmScenes = scenes.map(s => ({ id: s.id, at: s.at, end: s.end, chapter: s.chapter }));

// --- background -------------------------------------------------------------
const bg = h('div', { class: 'bg' });
const bgBase = h('div', { class: 'bg__base' });
const glowA = h('div', { class: 'bg__glow', style: { background: 'radial-gradient(circle, rgba(139,92,246,.55) 0%, rgba(109,49,242,.18) 40%, transparent 70%)' } });
const glowB = h('div', { class: 'bg__glow', style: { background: 'radial-gradient(circle, rgba(196,181,253,.28) 0%, rgba(124,58,237,.1) 45%, transparent 70%)' } });
const grid = h('div', { class: 'bg__grid' });
bg.append(bgBase, glowA, glowB, grid);
const grain = h('canvas', { class: 'grain', width: 512, height: 512 });
{ const c = grain.getContext('2d'), img = c.createImageData(512, 512); for (let i = 0; i < img.data.length; i += 4) { const v = rand(i * .25 + 1) * 255; img.data[i] = img.data[i + 1] = img.data[i + 2] = v; img.data[i + 3] = 255; } c.putImageData(img, 0, 0); }
const grainLayer = h('div', { class: 'grain', style: { backgroundImage: `url(${grain.toDataURL()})`, backgroundSize: '512px 512px' } });
const vignette = h('div', { class: 'vignette' });
const sceneLayer = h('div', { class: 'layer' });
const sweep = h('div', { class: 'sweep' });
const hud = h('div', { class: 'hud' });
stage.append(bg, sceneLayer, vignette, sweep, hud, grainLayer);

// --- HUD ------------------------------------------------------------------
const hudBrand = h('div', { class: 'hud__brand', html: `${markSVG()}<span>Eplyx</span>` });
const hudChips = h('div', { class: 'hud__chip' });
const chipChannel = h('span', { class: 'chip' }); const chipCase = h('span', { class: 'chip chip--solid' });
hudChips.append(chipChannel, chipCase);
const rail = h('div', { class: 'hud__rail' });
const railSegs = chapters.map(c => { const s = h('div', { class: 'rail__seg' }, h('span')); rail.append(s); return s; });
const caption = h('div', { class: 'hud__caption' });
hud.append(hudBrand, hudChips, rail, caption);

function chapterBounds(i) {
  const list = scenes.filter(s => s.chapter === i);
  return list.length ? [list[0].at, list.at(-1).end] : [0, 0];
}
const bounds = chapters.map((_, i) => chapterBounds(i));

function updateHud(t, scene) {
  const show = scene.hud !== false;
  const o = show ? Math.min(seg(t, scene.at + .1, scene.at + .7), 1 - seg(t, scene.end - .35, scene.end + .2)) : 0;
  // the brand stays while any HUD scene is on screen; chips change per scene
  const anyHud = scenes.some(s => s.hud !== false && t >= s.at - .3 && t < s.end + .3);
  hudBrand.style.opacity = (anyHud ? 1 : 0) * (scene.hud === false ? seg(t, scene.at + .3, scene.at - .2) : 1);
  const ch = scene.channel, uc = scene.usecase;
  const chText = ch ? `<i></i>Channel <b>${String(ch[0]).padStart(2, '0')}</b> · ${ch[1]}` : '';
  const ucText = uc ? `Use case <b>${String(uc[0]).padStart(2, '0')}</b> · ${uc[1]}` : '';
  if (chipChannel.dataset.v !== chText) { chipChannel.innerHTML = chText; chipChannel.dataset.v = chText; }
  if (chipCase.dataset.v !== ucText) { chipCase.innerHTML = ucText; chipCase.dataset.v = ucText; }
  chipChannel.style.display = ch ? '' : 'none'; chipCase.style.display = uc ? '' : 'none';
  put(hudChips, { y: (1 - o) * -16, o });
  rail.style.opacity = anyHud ? .9 : 0;
  railSegs.forEach((s, i) => { const [a, b] = bounds[i]; s.firstChild.style.transform = `scaleX(${clamp((t - a) / (b - a)).toFixed(4)})`; });
  const cap = scene.caption || '';
  if (caption.dataset.v !== cap) { caption.textContent = cap; caption.dataset.v = cap; }
  caption.style.opacity = o * (cap ? 1 : 0);
}

function updateBackground(t, scene) {
  const tone = scene.bg ?? 'violet';
  const target = tone === 'ink' ? 0 : tone === 'violet' ? 1 : tone;
  // smooth tone change across boundaries
  const prev = scenes[scenes.indexOf(scene) - 1];
  const pt = prev ? (prev.bg === 'ink' ? 0 : prev.bg === undefined || prev.bg === 'violet' ? 1 : prev.bg) : target;
  const v = lerp(pt, target, seg(t, scene.at - .4, scene.at + .8));
  bgBase.style.opacity = (.35 + .65 * v).toFixed(3);
  put(glowA, { x: 900 + Math.sin(t * .11) * 260, y: -500 + Math.cos(t * .09) * 160, o: .5 + .5 * v });
  put(glowB, { x: -300 + Math.cos(t * .07) * 200, y: 300 + Math.sin(t * .13) * 140, o: .35 + .4 * v });
  grid.style.transform = `translate(${(-t * 6) % 44}px, ${(-t * 3) % 44}px)`;
  const f = Math.floor(t * FPS);
  grainLayer.style.backgroundPosition = `${Math.floor(rand(f) * 512)}px ${Math.floor(rand(f + 7) * 512)}px`;
}

function updateSweep(t) {
  let best = null;
  for (const s of scenes) if (s.sweep !== false && s.at > 0 && Math.abs(t - s.at) < .5) best = s;
  if (!best) { sweep.style.opacity = 0; return; }
  const p = clamp((t - (best.at - .5)) / 1);
  sweep.style.opacity = Math.sin(p * Math.PI) * .9;
  sweep.style.transform = `translateX(${lerp(-700, 2300, ease.inOut(p))}px) skewX(-18deg)`;
}

// --- scenes ---------------------------------------------------------------
const built = [];
async function buildAll() {
  for (const s of scenes) {
    const root = h('div', { class: 'scene', 'data-scene': s.id });
    root.style.visibility = 'hidden';
    sceneLayer.append(root);
    const api = await s.build(root, s);
    built.push({ s, root, api });
  }
}

const PRE = .45, POST = .45;
function sceneEnvelope(t, s) {
  if (s.envelope === false) return { o: 1 };
  const enter = seg(t, s.at - .25, s.at + .45, ease.out);
  const exit = seg(t, s.end - .3, s.end + .3, ease.inOut);
  const first = s.at === 0, last = s === scenes.at(-1);
  const e = first ? 1 : enter, x = last ? 0 : exit;
  return { o: e * (1 - x), s: lerp(1.035, 1, e) * lerp(1, .975, x), blur: (1 - e) * 10 + x * 8 };
}

let busy = Promise.resolve();
async function renderAt(t) {
  t = clamp(t, 0, DURATION - 1 / FPS / 2);
  const current = scenes.find(s => t >= s.at && t < s.end) || scenes.at(-1);
  const active = built.filter(b => t >= b.s.at - PRE && t < b.s.end + POST);
  await Promise.all(active.map(b => b.api.prepare?.(t - b.s.at, t)));
  for (const b of built) {
    const on = active.includes(b);
    b.root.style.visibility = on ? 'visible' : 'hidden';
    if (!on) continue;
    const env = sceneEnvelope(t, b.s);
    put(b.root, { s: env.s ?? 1, o: env.o, blur: env.blur ?? 0, origin: '50% 50%' });
    b.api.update(t - b.s.at, t);
  }
  updateBackground(t, current);
  updateHud(t, current);
  updateSweep(t);
  // xterm and images paint on the next animation frame
  await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
}
window.renderAt = t => (busy = busy.then(() => renderAt(t)));

// --- audio cue list for the score (exported once by the renderer) -------------
window.audioCues = () => {
  const cues = [];
  for (const b of built) {
    cues.push({ t: b.s.at, type: 'scene', id: b.s.id, chapter: b.s.chapter });
    for (const c of b.api.cues?.() || []) cues.push({ ...c, t: b.s.at + c.t });
  }
  return cues.sort((a, b) => a.t - b.t);
};

// --- boot -----------------------------------------------------------------
await document.fonts.load('400 20px "JetBrains Mono"'); await document.fonts.load('700 20px "JetBrains Mono"');
await document.fonts.load('700 80px Manrope'); await document.fonts.load('400 20px "DM Sans"');
await document.fonts.ready;
await buildAll();
function fit() { if (exporting) return; const k = Math.min(innerWidth / W, (innerHeight - 50) / H); stage.style.transform = `scale(${k})`; }
fit(); addEventListener('resize', fit);
window.filmReady = true;
document.dispatchEvent(new Event('film-ready'));

if (!exporting) {
  const ui = document.getElementById('ui'), range = ui.querySelector('input'), label = ui.querySelector('span'), btn = ui.querySelector('button');
  range.max = DURATION; range.step = 1 / FPS;
  let playing = false, t0 = 0, wall = 0, t = +(params.get('t') || 0);
  const show = async v => { t = v; range.value = v; label.textContent = `${v.toFixed(2)} / ${DURATION.toFixed(1)}s · ${(scenes.find(s => v >= s.at && v < s.end) || {}).id || ''}`; await window.renderAt(v); };
  range.oninput = () => { playing = false; show(+range.value); };
  btn.onclick = () => { playing = !playing; t0 = t; wall = performance.now(); if (playing) loop(); };
  addEventListener('keydown', e => { if (e.code === 'Space') btn.click(); if (e.code === 'ArrowRight') show(Math.min(DURATION, t + 1)); if (e.code === 'ArrowLeft') show(Math.max(0, t - 1)); });
  async function loop() { if (!playing) return; await show(t0 + (performance.now() - wall) / 1000); if (t < DURATION) requestAnimationFrame(loop); }
  show(t);
}
