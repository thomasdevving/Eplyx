// Deterministic motion engine. Every visual property is a pure function of the
// film time t, so a frame can be rendered at any t, in any order, any number of
// times, and always produce the same pixels. Nothing runs on a wall clock.

export const W = 1920, H = 1080, FPS = 30;

export const clamp = (v, a = 0, b = 1) => Math.max(a, Math.min(b, v));
export const lerp = (a, b, p) => a + (b - a) * p;

export const ease = {
  linear: p => p,
  inQuad: p => p * p,
  outQuad: p => 1 - (1 - p) * (1 - p),
  inOut: p => (p < .5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2),
  out: p => 1 - Math.pow(1 - p, 3),
  outQuint: p => 1 - Math.pow(1 - p, 5),
  outExpo: p => (p >= 1 ? 1 : 1 - Math.pow(2, -10 * p)),
  inExpo: p => (p <= 0 ? 0 : Math.pow(2, 10 * p - 10)),
  inOutExpo: p => p <= 0 ? 0 : p >= 1 ? 1 : p < .5 ? Math.pow(2, 20 * p - 10) / 2 : (2 - Math.pow(2, -20 * p + 10)) / 2,
  outBack: p => { const c1 = 1.70158, c3 = c1 + 1; return 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2); },
  outBackSoft: p => { const c1 = .9, c3 = c1 + 1; return 1 + c3 * Math.pow(p - 1, 3) + c1 * Math.pow(p - 1, 2); },
  inOutSine: p => -(Math.cos(Math.PI * p) - 1) / 2,
};

/** Eased progress of t through [a, b]. */
export function seg(t, a, b, e = ease.inOut) {
  if (b <= a) return t >= b ? 1 : 0;
  return e(clamp((t - a) / (b - a)));
}

/** Piecewise interpolation through keyframes [[t, value, ease?], ...]; values may be numbers or objects of numbers. */
export function keys(t, frames, defaultEase = ease.inOut) {
  if (t <= frames[0][0]) return frames[0][1];
  for (let i = 1; i < frames.length; i++) {
    const [t1, v1, e] = frames[i];
    if (t <= t1) {
      const [t0, v0] = frames[i - 1];
      const p = (e || defaultEase)(clamp((t - t0) / (t1 - t0 || 1)));
      if (typeof v0 === 'number') return lerp(v0, v1, p);
      const out = {};
      for (const k of Object.keys(v1)) out[k] = lerp(v0[k] ?? v1[k], v1[k], p);
      return out;
    }
  }
  return frames[frames.length - 1][1];
}

/** Linear piecewise map (no easing): used to map scene time to source time. */
export function pw(t, points) {
  if (t <= points[0][0]) return points[0][1];
  for (let i = 1; i < points.length; i++) {
    if (t <= points[i][0]) {
      const [a, va] = points[i - 1], [b, vb] = points[i];
      return lerp(va, vb, (t - a) / (b - a || 1));
    }
  }
  return points[points.length - 1][1];
}

/** Deterministic pseudo-random in [0,1) from an integer seed. */
export function rand(seed) {
  let x = Math.sin(seed * 127.1 + 311.7) * 43758.5453;
  return x - Math.floor(x);
}

export function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') el.className = v;
    else if (k === 'style' && typeof v === 'object') Object.assign(el.style, v);
    else if (k === 'html') el.innerHTML = v;
    else el.setAttribute(k, v);
  }
  for (const c of children.flat()) if (c != null) el.append(c.nodeType ? c : document.createTextNode(c));
  return el;
}

export function svg(tag, attrs = {}, ...children) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  for (const c of children.flat()) if (c) el.append(c);
  return el;
}

/** Apply a transform/opacity description to an element. */
export function put(el, { x = 0, y = 0, s = 1, sx, sy, r = 0, rx = 0, ry = 0, o, blur = 0, z = 0, origin } = {}) {
  el.style.transform = `translate3d(${x.toFixed(2)}px,${y.toFixed(2)}px,${z}px) ` +
    (rx || ry ? `perspective(1600px) rotateX(${rx.toFixed(3)}deg) rotateY(${ry.toFixed(3)}deg) ` : '') +
    `rotate(${r.toFixed(3)}deg) scale(${(sx ?? s).toFixed(4)},${(sy ?? s).toFixed(4)})`;
  if (o !== undefined) el.style.opacity = clamp(o).toFixed(4);
  el.style.filter = blur > 0.05 ? `blur(${blur.toFixed(2)}px)` : '';
  if (origin) el.style.transformOrigin = origin;
}

/** Split text into word spans for staggered reveals. Returns the spans. */
export function splitWords(el, text, cls = 'w') {
  el.textContent = '';
  const spans = [];
  text.split(/(\s+)/).forEach(part => {
    if (/^\s+$/.test(part)) el.append(document.createTextNode(part));
    else if (part) { const s = h('span', { class: cls }, h('span', { class: cls + 'i' }, part)); el.append(s); spans.push(s.firstChild); }
  });
  return spans;
}

/** Staggered rise-in for word spans (masked by their parent .w overflow hidden). */
export function riseWords(spans, t, start, { stagger = .045, dur = .7, dy = 1.05, out = null } = {}) {
  spans.forEach((sp, i) => {
    const p = seg(t, start + i * stagger, start + i * stagger + dur, ease.outQuint);
    let y = (1 - p) * dy * 100, o = p;
    if (out) {
      const q = seg(t, out + i * stagger * .5, out + i * stagger * .5 + .45, ease.inOut);
      y -= q * dy * 100; o *= 1 - q;
    }
    sp.style.transform = `translateY(${y.toFixed(2)}%)`;
    sp.style.opacity = o.toFixed(3);
  });
}

export const fmt = n => n.toLocaleString('en-US');
