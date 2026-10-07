// Layout and animation kit used by the scenes.
import { h, put, seg, ease, keys, pw, clamp, lerp, splitWords, riseWords } from './core.js';
import { Footage, Term, windowFrame, termMark } from './components.js';

/** Kicker + title (+ subtitle) with masked word reveals. */
export function titleBlock(root, { x, y, w = 760, kicker = '', title = '', sub = '', size = '', align = 'left' }) {
  const box = h('div', { class: 'abs', style: { left: x + 'px', top: y + 'px', width: w + 'px', textAlign: align } });
  const k = kicker ? h('div', { class: 'kicker', style: align === 'center' ? { justifyContent: 'center' } : {} }, kicker) : null;
  const t = h('h1', { class: `title ${size ? 'title--' + size : ''}`, style: { marginTop: kicker ? '22px' : 0 } });
  const tw = [];
  title.split('\n').forEach(line => {
    const lineEl = h('span', { style: { display: 'block' } });
    const [plain, accent] = line.split('*');
    const a = splitWords(h('span'), plain.trim(), 'w'), b = accent != null ? splitWords(h('span'), accent.trim(), 'w') : [];
    for (const sp of a) { lineEl.append(sp.parentNode, ' '); tw.push(sp); }
    for (const sp of b) { sp.classList.add('grad'); lineEl.append(sp.parentNode, ' '); tw.push(sp); }
    t.append(lineEl);
  });
  const s = sub ? h('p', { class: 'lead', style: { marginTop: '26px' } }) : null;
  const sw = s ? splitWords(s, sub, 'w') : [];
  box.append(...[k, t, s].filter(Boolean));
  root.append(box);
  return {
    el: box,
    update(lt, { at = 0, out = null } = {}) {
      if (k) put(k, { y: (1 - seg(lt, at, at + .6, ease.outQuint)) * 20, o: seg(lt, at, at + .5) * (out == null ? 1 : 1 - seg(lt, out, out + .4)) });
      riseWords(tw, lt, at + .12, { stagger: .05, dur: .75, out });
      riseWords(sw, lt, at + .45, { stagger: .012, dur: .7, out });
    },
  };
}

/** A terminal window replaying a recorded cast. */
export async function termWindow(root, { cast, x, y, w, cols = 100, rows = 30, title, pad = 22, fontSize }) {
  const fs = fontSize || Math.floor((w - pad * 2) / (cols * 0.6) * 10) / 10;
  const term = new Term({ cast, cols, rows, fontSize: fs, lineHeight: 1.0 });
  await term.load();
  const bodyH = term.height + pad * 2;
  const win = windowFrame({ kind: 'terminal', title: title || cast, w, h: bodyH + 44 });
  win.root.style.left = x + 'px'; win.root.style.top = y + 'px';
  term.root.style.position = 'absolute'; term.root.style.left = pad + 'px'; term.root.style.top = pad + 'px';
  term.root.style.width = term.width + 'px'; term.root.style.height = term.height + 'px';
  win.body.append(term.root);
  root.append(win.root);
  return { win: win.root, term, height: bodyH + 44, width: w };
}

/** A browser window playing recorded product footage. */
export async function webWindow(root, { clip, x, y, w, url = '' }) {
  const hh = Math.round(w * 900 / 1440);
  const foot = new Footage({ clip, w, h: hh });
  await foot.load();
  const win = windowFrame({ kind: 'browser', url, w, h: hh + 44 });
  win.root.style.left = x + 'px'; win.root.style.top = y + 'px';
  win.body.append(foot.root);
  root.append(win.root);
  return { win: win.root, foot, setUrl: win.setUrl, height: hh + 44, width: w };
}

/**
 * Map scene time to cast time using the recorder's real markers: each command
 * is typed in `type` seconds, its output appears over `out` seconds, then the
 * screen holds for `hold` seconds. Returns the mapping and per-command anchors.
 */
export function plan(term, t0, steps, { lead = .25 } = {}) {
  const ev = term.cast.events.filter(e => e[1] === 'm');
  const types = ev.filter(e => e[2].startsWith('type ')).map(e => e[0]);
  const runs = ev.filter(e => e[2] === 'run' || e[2] === 'interrupt').map(e => e[0]);
  const exits = ev.filter(e => e[2].startsWith('exit ') || e[2] === 'interrupt').map(e => e[0]);
  const pts = [[0, Math.max(0, types[0] - .05)], [t0, Math.max(0, types[0] - .05)]];
  const at = []; let t = t0;
  steps.forEach((st, i) => {
    if (st.skip) return;
    const a = types[i], r = runs[i], e = exits[i] ?? r;
    if (t > pts.at(-1)[0] + 1e-6) pts.push([t, a]);
    const typeStart = t; t += st.type ?? .8; pts.push([t, r]);
    const runAt = t; t += st.out ?? .25; pts.push([t, e + .12]);
    const outEnd = t; t += st.hold ?? 1.2; pts.push([t, e + .14]);
    at.push({ typeStart, runAt, outEnd, holdEnd: t });
    t += lead;
  });
  return { pts, at, end: t, map: lt => pw(lt, pts) };
}

/** Window entrance: rise with a slight 3D tilt; exit by settling back. */
export function enter(el, lt, t0, { dy = 60, rx = 9, dur = .9, out = null, x = 0, drift = 0 } = {}) {
  const p = seg(lt, t0, t0 + dur, ease.outQuint);
  const q = out == null ? 0 : seg(lt, out, out + .55, ease.inOut);
  // a slow push-in keeps every window alive after it lands
  const push = 1 + .018 * seg(lt, t0 + dur, t0 + dur + 9, ease.linear);
  put(el, { x: x * (1 - p), y: dy * (1 - p) - q * 30 + drift * lt, rx: rx * (1 - p) - q * 4, o: p * (1 - q), s: lerp(.96, 1, p) * lerp(1, .97, q) * push });
}

export function badge(root, { x, y, label, sub = '', tone = '' }) {
  const el = h('div', { class: `badge ${tone ? 'badge--' + tone : ''}`, style: { left: x + 'px', top: y + 'px' } });
  el.append(h('span', { class: 'dot' }), h('span', {}, sub ? h('small', {}, sub) : null, label));
  root.append(el); el.style.opacity = 0;
  return el;
}

export function pop(el, lt, t0, { out = null, from = .6 } = {}) {
  const p = seg(lt, t0, t0 + .55, ease.outBack);
  const q = out == null ? 0 : seg(lt, out, out + .35);
  put(el, { s: lerp(from, 1, p) * (1 - q * .1), o: clamp(p * 1.6) * (1 - q), y: (1 - p) * 14 });
}

export function note(root, { x, y, w = 480, kicker = '', title = '', body = '' }) {
  const el = h('div', { class: 'note', style: { left: x + 'px', top: y + 'px', width: w + 'px', maxWidth: w + 'px' } });
  if (kicker) el.append(h('div', { class: 'kicker' }, kicker));
  if (title) el.append(h('h4', {}, title));
  if (body) el.append(h('p', { html: body }));
  root.append(el); el.style.opacity = 0;
  return el;
}

export function slide(el, lt, t0, { dx = 40, dy = 0, out = null, dur = .7 } = {}) {
  const p = seg(lt, t0, t0 + dur, ease.outQuint);
  const q = out == null ? 0 : seg(lt, out, out + .4);
  put(el, { x: dx * (1 - p), y: dy * (1 - p), o: p * (1 - q) });
}

export function pills(root, { x, y, items, gap = 12, wrap = 1700 }) {
  const box = h('div', { class: 'abs', style: { left: x + 'px', top: y + 'px', display: 'flex', flexWrap: 'wrap', gap: gap + 'px', maxWidth: wrap + 'px' } });
  const els = items.map(it => { const e = h('span', { class: `pill ${it.tone ? 'pill--' + it.tone : ''}` }, h('span', { class: 'dot' }), it.text); box.append(e); return e; });
  root.append(box);
  return els;
}

export function staggerIn(els, lt, t0, { step = .08, dy = 18, out = null } = {}) {
  els.forEach((e, i) => slide(e, lt, t0 + i * step, { dx: 0, dy, out: out == null ? null : out + i * .02 }));
}

/** Terminal camera: zoom toward a text match between [a, b]. */
export function termCam(term, lt, shots, base = null) {
  let cam = base || { z: 1, cx: term.width / 2, cy: term.height / 2 };
  for (const s of shots) {
    const p = seg(lt, s.a, s.a + (s.ease ?? .7), ease.inOut) * (1 - seg(lt, s.b, s.b + .55, ease.inOut));
    if (p <= 0) continue;
    const f = s.text ? term.focus(s.text, null) : s.point;
    if (!f) continue;
    cam = { z: lerp(cam.z, s.z ?? 1.6, p), cx: lerp(cam.cx, f.cx + (s.dx || 0), p), cy: lerp(cam.cy, f.cy + (s.dy || 0), p) };
  }
  return cam;
}

/** Footage camera keyframes in page coordinates. */
export function footCam(lt, frames) { return keys(lt, frames.map(([t, v, e]) => [t, v, e])); }

export function mark(term, overlay, text, opts) {
  const el = h('div', { class: 'tmark' }); overlay.append(el);
  return (p) => termMark(term, el, text, p, opts);
}
