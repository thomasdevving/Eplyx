// Building blocks that display recorded evidence: browser footage (frame
// sequences recorded from the real product), terminal casts (real PTY bytes
// played back through xterm.js), window chrome, cursor and highlights.
import { h, put, clamp, lerp, seg, ease, keys, pw } from './core.js';

const cache = new Map();
async function json(url) { if (!cache.has(url)) cache.set(url, fetch(url).then(r => { if (!r.ok) throw new Error(url); return r.json(); })); return cache.get(url); }

export const ICON = {
  lock: '<svg viewBox="0 0 16 16" width="13" height="13"><path fill="currentColor" d="M5 7V5a3 3 0 0 1 6 0v2h.5A1.5 1.5 0 0 1 13 8.5v5A1.5 1.5 0 0 1 11.5 15h-7A1.5 1.5 0 0 1 3 13.5v-5A1.5 1.5 0 0 1 4.5 7zm1.5 0h3V5a1.5 1.5 0 0 0-3 0z"/></svg>',
};

/** A window: macOS-like chrome in the Eplyx palette. kind: 'browser' | 'terminal'. */
export function windowFrame({ kind = 'browser', title = '', url = '', w, h: hh, bar = 44 }) {
  const root = h('div', { class: `win win--${kind}`, style: { width: w + 'px', height: hh + 'px' } });
  const top = h('div', { class: 'win__bar', style: { height: bar + 'px' } },
    h('span', { class: 'win__dots' }, h('i'), h('i'), h('i')),
    kind === 'browser'
      ? h('span', { class: 'win__url', html: `${ICON.lock}<b></b>` })
      : h('span', { class: 'win__title' }, title));
  const body = h('div', { class: 'win__body', style: { height: hh - bar + 'px' } });
  root.append(top, body);
  const urlEl = root.querySelector('.win__url b');
  if (urlEl) urlEl.textContent = url;
  return { root, body, setUrl: u => { if (urlEl && urlEl.textContent !== u) urlEl.textContent = u; }, setTitle: t => { const el = root.querySelector('.win__title'); if (el) el.textContent = t; } };
}

/** Recorded browser footage, a camera, the pointer and highlight boxes. */
export class Footage {
  constructor({ clip, w, h: hh, base = '/clips/' }) {
    this.clipName = clip; this.w = w; this.h = hh; this.base = base;
    this.root = h('div', { class: 'footage', style: { width: w + 'px', height: hh + 'px' } });
    this.cam = h('div', { class: 'footage__cam', style: { width: w + 'px', height: hh + 'px' } });
    this.img = h('img', { class: 'footage__img', style: { width: w + 'px', height: hh + 'px' } });
    this.overlay = h('div', { class: 'footage__overlay' });
    this.pointer = h('div', { class: 'pointer', html: '<svg width="30" height="38" viewBox="0 0 26 34"><path fill="#fff" stroke="#160b26" stroke-width="1.6" stroke-linejoin="round" d="M2 1.5V26.5L8.7 20.2L14.3 32L19.2 29.6L13.7 18H24.5Z"/></svg>' });
    this.ripple = h('div', { class: 'ripple' });
    this.cam.append(this.img, this.overlay, this.ripple, this.pointer);
    this.root.append(this.cam);
    this.current = null;
  }
  async load() {
    this.meta = await json(`${this.base}${this.clipName}/clip.json`);
    this.k = this.w / this.meta.viewport[0];
    this.moves = this.meta.events.filter(e => e.type === 'move' || e.type === 'down' || e.type === 'up');
    this.downs = this.meta.events.filter(e => e.type === 'down');
    return this;
  }
  mark(label) { const m = this.meta.events.find(e => e.type === 'mark' && e.label === label); if (!m) throw new Error(`${this.clipName}: no mark ${label}`); return m.t; }
  frameAt(ct) {
    const f = this.meta.frames; let lo = 0, hi = f.length - 1;
    if (ct <= f[0].t) return f[0];
    while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (f[mid].t <= ct) lo = mid; else hi = mid - 1; }
    return f[lo];
  }
  async prepare(ct) {
    const fr = this.frameAt(ct);
    const src = `${this.base}${this.clipName}/${fr.file}`;
    if (this.current !== src) {
      this.current = src;
      this.img.src = src;
      try { await this.img.decode(); } catch (e) { console.warn('decode', src); }
    }
  }
  pointerAt(ct) {
    const m = this.moves; if (!m.length) return null;
    if (ct < m[0].t) return { x: m[0].x, y: m[0].y, visible: ct > m[0].t - 0.6 };
    let i = 0; while (i + 1 < m.length && m[i + 1].t <= ct) i++;
    const a = m[i], b = m[i + 1];
    if (!b) return { x: a.x, y: a.y, visible: true };
    const p = clamp((ct - a.t) / (b.t - a.t || 1));
    return { x: lerp(a.x, b.x, p), y: lerp(a.y, b.y, p), visible: true };
  }
  /** cam: {cx, cy, z} in page CSS pixels. */
  update(ct, { cam = null, pointer = true, pointerOpacity = 1 } = {}) {
    const vw = this.meta.viewport[0], vh = this.meta.viewport[1], k = this.k;
    const z = cam?.z ?? 1, cx = cam?.cx ?? vw / 2, cy = cam?.cy ?? vh / 2;
    let tx = this.w / 2 - cx * k * z, ty = this.h / 2 - cy * k * z;
    tx = clamp(tx, this.w - this.w * z, 0); ty = clamp(ty, this.h - this.h * z, 0);
    this.cam.style.transform = `translate(${tx.toFixed(2)}px,${ty.toFixed(2)}px) scale(${z.toFixed(4)})`;
    const p = pointer && this.pointerAt(ct);
    if (p && p.visible) {
      this.pointer.style.opacity = pointerOpacity;
      this.pointer.style.transform = `translate(${(p.x * k - 3).toFixed(1)}px,${(p.y * k - 2).toFixed(1)}px) scale(${(1 / z * 0.9 + 0.1).toFixed(3)})`;
      const d = [...this.downs].reverse().find(e => e.t <= ct && ct - e.t < 0.7);
      if (d) {
        const q = (ct - d.t) / 0.7;
        this.ripple.style.opacity = (1 - q) * pointerOpacity;
        this.ripple.style.transform = `translate(${(d.x * k).toFixed(1)}px,${(d.y * k).toFixed(1)}px) translate(-50%,-50%) scale(${(0.35 + q * 1.4).toFixed(3)})`;
      } else this.ripple.style.opacity = 0;
    } else { this.pointer.style.opacity = 0; this.ripple.style.opacity = 0; }
  }
  /** A highlight rectangle in page coordinates that lives in camera space. */
  box({ x, y, w, h: hh, label = '', tone = 'violet' }) {
    const k = this.k;
    const el = h('div', { class: `hl hl--${tone}`, style: { left: x * k + 'px', top: y * k + 'px', width: w * k + 'px', height: hh * k + 'px' } });
    if (label) el.append(h('span', { class: 'hl__label' }, label));
    this.overlay.append(el);
    return el;
  }
}

const THEME = {
  background: '#0d0717', foreground: '#ece6f7', cursor: '#c9b6ff', cursorAccent: '#0d0717',
  selectionBackground: '#6d31f255',
  black: '#1b1428', red: '#ff7d8c', green: '#6fe3b4', yellow: '#f6c66f', blue: '#8fb7ff', magenta: '#c7a4ff', cyan: '#80d8ff', white: '#ece6f7',
  brightBlack: '#6d6380', brightRed: '#ff9aa6', brightGreen: '#9cf0cb', brightYellow: '#ffd890', brightBlue: '#b3cdff', brightMagenta: '#dcc6ff', brightCyan: '#a9e6ff', brightWhite: '#ffffff',
};

/** A recorded PTY session replayed through a real terminal emulator. */
export class Term {
  constructor({ cast, cols, rows, fontSize = 20, lineHeight = 1.25, base = '/casts/' }) {
    Object.assign(this, { castName: cast, cols, rows, fontSize, lineHeight, base });
    this.root = h('div', { class: 'term' });
    this.cam = h('div', { class: 'term__cam' });
    this.host = h('div', { class: 'term__host' });
    this.overlay = h('div', { class: 'term__overlay' });
    this.cam.append(this.host, this.overlay);
    this.root.append(this.cam);
    this.pos = 0; this.time = -1;
  }
  async load() {
    this.cast = await json(`${this.base}${this.castName}.json`);
    this.term = new window.Terminal({
      cols: this.cols, rows: this.rows, fontFamily: '"JetBrains Mono", monospace', fontSize: this.fontSize,
      lineHeight: this.lineHeight, letterSpacing: 0, theme: THEME, cursorBlink: false, cursorStyle: 'bar', cursorInactiveStyle: 'bar',
      disableStdin: true, scrollback: 5000, allowTransparency: false, convertEol: false, fontWeight: 400, fontWeightBold: 700, customGlyphs: true,
    });
    this.term.open(this.host);
    await new Promise(r => requestAnimationFrame(r));
    const dims = this.term._core._renderService.dimensions.css;
    this.cell = { w: dims.cell.width, h: dims.cell.height };
    this.width = dims.canvas.width; this.height = dims.canvas.height;
    return this;
  }
  marker(label, nth = 0) {
    const m = this.cast.events.filter(e => e[1] === 'm' && e[2].startsWith(label));
    if (!m[nth]) throw new Error(`${this.castName}: no marker ${label} #${nth}`);
    return m[nth][0];
  }
  get duration() { return this.cast.events.at(-1)[0]; }
  async prepare(ct) {
    if (ct < this.time) { this.term.reset(); this.pos = 0; await new Promise(r => this.term.write('', r)); }
    this.time = ct;
    let chunk = '';
    const ev = this.cast.events;
    while (this.pos < ev.length && ev[this.pos][0] <= ct) { if (ev[this.pos][1] === 'o') chunk += ev[this.pos][2]; this.pos++; }
    if (chunk) await new Promise(r => this.term.write(chunk, r));
  }
  /** Row (0..rows-1) in the viewport of the last line containing text, or -1. */
  rowOf(text, { first = false } = {}) {
    const b = this.term.buffer.active; let found = -1;
    for (let i = 0; i < this.rows; i++) {
      const line = b.getLine(b.viewportY + i); if (!line) continue;
      if (line.translateToString(true).includes(text)) { found = i; if (first) break; }
    }
    return found;
  }
  colOf(text, row) { const b = this.term.buffer.active; const line = b.getLine(b.viewportY + row); return line ? line.translateToString(true).indexOf(text) : -1; }
  update({ cam = null } = {}) {
    const z = cam?.z ?? 1;
    const cx = cam?.cx ?? this.width / 2, cy = cam?.cy ?? this.height / 2;
    let tx = this.width / 2 - cx * z, ty = this.height / 2 - cy * z;
    tx = clamp(tx, this.width - this.width * z, 0); ty = clamp(ty, this.height - this.height * z, 0);
    this.cam.style.transform = `translate(${tx.toFixed(2)}px,${ty.toFixed(2)}px) scale(${z.toFixed(4)})`;
  }
  /** Center of a text match in terminal pixel space, for camera targeting. */
  focus(text, fallback) {
    const r = this.rowOf(text); if (r < 0) return fallback;
    const c = Math.max(0, this.colOf(text, r));
    return { cx: (c + text.length / 2) * this.cell.w, cy: (r + .5) * this.cell.h };
  }
}

/** Draw a highlight band over a terminal row that contains text. */
export function termMark(term, el, text, p, { pad = 6, tone = 'violet', span = null } = {}) {
  const row = term.rowOf(text);
  if (row < 0 || p <= 0) { el.style.opacity = 0; return; }
  const col = Math.max(0, term.colOf(text, row));
  const len = span ?? text.length;
  el.style.opacity = Math.min(1, p * 1.4);
  el.style.left = (col * term.cell.w - pad) + 'px';
  el.style.top = (row * term.cell.h - 2) + 'px';
  el.style.height = (term.cell.h + 4) + 'px';
  el.style.width = ((len * term.cell.w + pad * 2) * ease.outQuint(clamp(p))) + 'px';
  el.className = `tmark tmark--${tone}`;
}
