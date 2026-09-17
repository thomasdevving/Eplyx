import { Mark } from './brand.js';

// Two orbital planes around the mark: the changes Eplyx tracks, and the
// consequences it measures. Each ring keeps its own inclination and travel
// direction, so the toggle reads as a change of axis rather than a swap of
// labels. Depth is the ring angle alone — the far half passes behind the mark.
const rings = {
  changes: {
    label: 'Changes',
    caption: 'What Eplyx tracks · one live layer, three planned',
    tilt: -17,
    direction: -1,
    duration: 38,
    bodies: [
      ['Program Upgrade', 'live', 'A proposed program binary, replayed against validated historical production state.'],
      ['Governance Change', 'planned', 'Proposals that change the rules, evaluated before they execute.'],
      ['Privilege / Authority', 'planned', 'Who holds upgrade, freeze and withdrawal authority over the program.'],
      ['Parameter Change', 'planned', 'Fees, rates and limits the program can move without a new binary.'],
    ],
  },
  consequences: {
    label: 'Consequences',
    caption: 'What Eplyx measures · two live layers, two planned',
    tilt: 19,
    direction: 1,
    duration: 34,
    bodies: [
      ['Economic Impact', 'live', 'Measured user output, fees and protocol balances across the tested corpus.', 'pool_tokens_received', '↓ 21 bps'],
      ['CI Decision', 'live', 'An undeclared or out-of-bounds change fails the gate.', 'Unexpected', 'FAIL'],
      ['Authority Surface', 'planned', 'Which authorities a change exposes, widens or removes.'],
      ['Exitability', 'planned', 'Whether users can still withdraw once the change is live.'],
    ],
  },
};

const orbitBody = (ring, [name, status, detail, metric, value], index) =>
  `<button type="button" class="orbit-body orbit-body--${status}" data-ring="${ring}" data-index="${index}" data-name="${name}" data-status="${status === 'live' ? 'Live' : 'Planned'}" data-detail="${detail}"${metric ? ` data-metric="${metric}" data-value="${value}"` : ''}>
    <span class="orbit-body__rock orbit-body__rock--${index}" aria-hidden="true"></span>
    <span class="visually-hidden">${name}. ${status === 'live' ? 'Live layer' : 'Planned layer'}. ${detail}</span>
  </button>
  <div class="orbit-label orbit-label--${status}" data-ring="${ring}" data-index="${index}" aria-hidden="true">
    <svg class="orbit-label__leader"><path/><circle r="2"/></svg>
    <span class="orbit-body__name">${name}</span>
    <small class="orbit-body__status">${status === 'live' ? 'Live' : 'Planned'}</small>
  </div>`;

const orbitPlane = side => `<svg class="orbit-plane orbit-plane--${side}" aria-hidden="true">
  ${side === 'front' ? `<defs>
    <linearGradient id="orbit-main"><stop offset="0" stop-color="#b98cf0"/><stop offset=".38" stop-color="#fdf6ff"/><stop offset=".72" stop-color="#d3b0fb"/><stop offset="1" stop-color="#8646dd"/></linearGradient>
    <linearGradient id="orbit-sheen"><stop offset="0" stop-color="#fffaff" stop-opacity=".1"/><stop offset=".45" stop-color="#fffaff" stop-opacity=".9"/><stop offset="1" stop-color="#f0dfff" stop-opacity=".15"/></linearGradient>
    <filter id="orbit-bloom" x="-25%" y="-70%" width="150%" height="240%"><feGaussianBlur stdDeviation="5"/></filter>
  </defs>` : ''}
  ${side === 'front' ? '<path class="orbit-line orbit-line--bloom"/>' : ''}
  <path class="orbit-line orbit-line--main"/>
  ${side === 'front' ? '<path class="orbit-line orbit-line--sheen"/><path class="orbit-pulse" pathLength="1"/>' : ''}
</svg>`;

export function EplyxCoreScene() {
  const planes = Object.entries(rings);
  return `<div class="core-scene" data-ring="changes" role="group" aria-label="The changes Eplyx tracks and the consequences it measures, orbiting the Eplyx mark. Switch between both sets and explore each layer for its current status.">
    <div class="core-stage">
      ${orbitPlane('back')}
      <div class="core-glow" aria-hidden="true"></div>
      <div class="logo-core" aria-hidden="true"><div class="sculpture-fallback">${Mark({ className: 'core-mark core-mark--face' })}</div><div class="sculpture-mount"></div></div>
      ${orbitPlane('front')}
      ${planes.map(([name, ring]) => ring.bodies.map((body, index) => orbitBody(name, body, index)).join('')).join('')}
      <div class="orbit-detail" aria-hidden="true">
        <div class="orbit-detail__head"><strong class="orbit-detail__name"></strong><em class="orbit-detail__status"></em></div>
        <p class="orbit-detail__text"></p>
        <div class="orbit-detail__metric"><span></span><b></b></div>
      </div>
    </div>
    <div class="orbit-switch">
      <i class="orbit-switch__thumb" aria-hidden="true"></i>
      ${planes.map(([key, ring], index) => `<button type="button" class="orbit-switch__option" data-target="${key}" aria-pressed="${index === 0}">${ring.label}</button>`).join('')}
    </div>
    <p class="orbit-caption">${rings.changes.caption}</p>
  </div>`;
}

export function attachCoreParallax() {
  const scene = document.querySelector('.core-scene');
  if (!scene) return () => {};
  const stage = scene.querySelector('.core-stage');
  const detail = scene.querySelector('.orbit-detail');
  const caption = scene.querySelector('.orbit-caption');
  const bodies = [...scene.querySelectorAll('.orbit-body')];
  // Labels stay above the mark even when their rock travels behind it.
  const labels = new Map(bodies.map(body => [body, {
    element: scene.querySelector(`.orbit-label[data-ring="${body.dataset.ring}"][data-index="${body.dataset.index}"]`),
    width: 0,
    height: 0,
  }]));
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  // The inclination is tweened on a switch, so line and bodies never disagree.
  const view = { tilt: rings.changes.tilt, from: rings.changes.tilt, to: rings.changes.tilt, swing: 1 };
  const listeners = [];
  let active = 'changes';
  let width = 0, height = 0, bodyWidth = 126, spanX = 0, spanY = 0;
  let frame = 0, last = 0, elapsed = 0, held = null, onscreen = true;

  const point = (angle, spread = 1) => {
    const radians = view.tilt * Math.PI / 180;
    const x = Math.cos(angle) * spanX * spread;
    const y = Math.sin(angle) * spanY * spread;
    return {
      x: width / 2 + x * Math.cos(radians) - y * Math.sin(radians),
      y: height / 2 + x * Math.sin(radians) + y * Math.cos(radians),
      depth: Math.sin(angle),
    };
  };

  const arc = (from, to, spread) => {
    let d = '';
    for (let step = 0; step <= 44; step++) {
      const { x, y } = point(from + (to - from) * step / 44, spread);
      d += `${step ? 'L' : 'M'}${x.toFixed(1)} ${y.toFixed(1)}`;
    }
    return d;
  };

  const drawPlanes = () => {
    if (!width || !height) return;
    for (const [side, from, to] of [['back', Math.PI, Math.PI * 2], ['front', 0, Math.PI]]) {
      const plane = scene.querySelector(`.orbit-plane--${side}`);
      plane.setAttribute('viewBox', `0 0 ${width} ${height}`);
      const main = arc(from, to, 1);
      plane.querySelectorAll('.orbit-line--main, .orbit-line--sheen, .orbit-line--bloom, .orbit-pulse').forEach(path => path.setAttribute('d', main));
    }
  };

  // The panel aligns with the body's outward edge so it opens away from the
  // mark, and only flips back when that side has no room left in the stage.
  const anchor = (x, y) => {
    const half = (detail.offsetWidth || 228) / 2;
    const depth = detail.offsetHeight || 150;
    const outward = x >= width / 2 ? 1 : -1;
    const span = Math.max(width - half - 2, half + 2);
    const left = Math.min(Math.max(x + outward * (bodyWidth / 2 - half), half + 2), span);
    const gap = bodyWidth / 2 + 12;
    const below = y < height * .5 ? y + gap + depth < height : y - gap - depth < 0;
    detail.style.transform = `translate(${left.toFixed(1)}px, ${(y + (below ? gap : -gap)).toFixed(1)}px) translate(-50%, ${below ? '0' : '-100%'})`;
  };

  const place = () => {
    if (!width || !height) return;
    const ring = rings[active];
    const turn = elapsed / ring.duration * Math.PI * 2 * ring.direction;
    for (const body of bodies) {
      if (body.dataset.ring !== active) continue;
      const index = Number(body.dataset.index);
      const angle = turn - 2.25 + index / ring.bodies.length * Math.PI * 2;
      const { x, y, depth } = point(angle);
      const near = (depth + 1) / 2;
      const scale = .78 + near * .22;
      body.style.transform = `translate(${x.toFixed(1)}px, ${y.toFixed(1)}px) translate(-50%, -50%) scale(${scale.toFixed(3)})`;
      body.style.zIndex = depth > 0 ? 6 : 2;
      body.style.setProperty('--near', near.toFixed(3));
      body.style.setProperty('--rock-turn', `${(Math.sin(elapsed * .12 + index * 1.7) * 9).toFixed(2)}deg`);
      const label = labels.get(body);
      const outwardX = (x - width / 2) / spanX;
      const outwardY = (y - height / 2) / spanY;
      const radius = bodyWidth * scale * .4;
      const wantedX = x + outwardX * (radius + label.width / 2 + 12);
      const labelX = Math.min(width - label.width / 2 - 8, Math.max(label.width / 2 + 8, wantedX));
      // Blend continuously into an above/below placement at the edges.
      // Hard edge and direction thresholds made the text jump mid-orbit.
      const edge = Math.min(1, Math.abs(wantedX - labelX) / 36);
      const blend = edge * edge * (3 - 2 * edge);
      const vertical = outwardY + (Math.tanh((outwardY - .15) * 4) - outwardY) * blend;
      const wantedY = y + vertical * (radius + label.height / 2 + 14);
      const labelY = Math.min(height - label.height / 2 - 8, Math.max(label.height / 2 + 8, wantedY));
      label.element.style.transform = `translate(${labelX.toFixed(1)}px, ${labelY.toFixed(1)}px) translate(-50%, -50%)`;
      label.element.style.setProperty('--near', near.toFixed(3));
      const reach = Math.hypot(labelX - x, labelY - y) || 1;
      const dotX = x - labelX + (labelX - x) / reach * radius * .7;
      const dotY = y - labelY + (labelY - y) / reach * radius * .7;
      // Follow the nearest edge of the label, without flipping the leader
      // abruptly between its top and bottom while the label moves past it.
      const endScale = 1 / Math.max(Math.abs(x - labelX) / (label.width / 2 + 4), Math.abs(y - labelY) / (label.height / 2 + 4), 1);
      const endX = (x - labelX) * endScale;
      const endY = (y - labelY) * endScale;
      label.element.querySelector('path').setAttribute('d', `M${dotX.toFixed(1)} ${dotY.toFixed(1)} L${endX.toFixed(1)} ${endY.toFixed(1)}`);
      const dot = label.element.querySelector('circle');
      dot.setAttribute('cx', dotX.toFixed(1));
      dot.setAttribute('cy', dotY.toFixed(1));
      if (body === held) anchor(x, y);
    }
    scene.classList.add('is-placed');
  };

  const step = now => {
    frame = 0;
    const delta = last ? Math.min((now - last) / 1000, .05) : 0;
    last = now;
    if (view.swing < 1) {
      view.swing = Math.min(1, view.swing + delta / .8);
      const eased = 1 - (1 - view.swing) ** 3;
      view.tilt = view.from + (view.to - view.from) * eased;
      drawPlanes();
    }
    if (!held) elapsed += delta;
    place();
    if (onscreen && !document.hidden && !motion.matches) frame = requestAnimationFrame(step);
  };

  const start = () => {
    if (frame || motion.matches || !onscreen || document.hidden) return;
    last = 0;
    frame = requestAnimationFrame(step);
  };
  const stop = () => {
    cancelAnimationFrame(frame);
    frame = 0;
  };

  const measure = () => {
    const rect = stage.getBoundingClientRect();
    if (!rect.width || !rect.height) return;
    width = rect.width;
    height = rect.height;
    bodyWidth = parseFloat(getComputedStyle(scene).getPropertyValue('--orbit-body')) || bodyWidth;
    for (const label of labels.values()) {
      label.width = label.element.offsetWidth;
      label.height = label.element.offsetHeight;
    }
    spanX = Math.min(width * .39, width / 2 - bodyWidth / 2 - 2);
    spanY = height * .29;
    drawPlanes();
    place();
  };

  const show = body => {
    held = body;
    detail.querySelector('.orbit-detail__name').textContent = body.dataset.name;
    detail.querySelector('.orbit-detail__status').textContent = body.dataset.status;
    detail.querySelector('.orbit-detail__text').textContent = body.dataset.detail;
    const metric = detail.querySelector('.orbit-detail__metric');
    metric.style.display = body.dataset.metric ? '' : 'none';
    if (body.dataset.metric) {
      metric.querySelector('span').textContent = body.dataset.metric;
      metric.querySelector('b').textContent = body.dataset.value;
    }
    detail.dataset.status = body.dataset.status;
    detail.classList.add('is-visible');
    body.classList.add('orbit-body--held');
    place();
  };

  const hide = () => {
    held?.classList.remove('orbit-body--held');
    held = null;
    detail.classList.remove('is-visible');
  };

  const select = next => {
    if (next === active || !rings[next]) return;
    active = next;
    scene.dataset.ring = next;
    scene.querySelectorAll('.orbit-switch__option').forEach(option => option.setAttribute('aria-pressed', String(option.dataset.target === next)));
    caption.textContent = rings[next].caption;
    hide();
    view.from = view.tilt;
    view.to = rings[next].tilt;
    view.swing = motion.matches ? 1 : 0;
    if (motion.matches) view.tilt = view.to;
    drawPlanes();
    place();
    start();
  };

  const bind = (target, event, handler) => {
    target.addEventListener(event, handler);
    listeners.push(() => target.removeEventListener(event, handler));
  };

  for (const body of bodies) {
    bind(body, 'pointerenter', event => { if (event.pointerType !== 'touch') show(body); });
    bind(body, 'focus', () => { if (body.matches(':focus-visible')) show(body); });
    bind(body, 'pointerleave', event => { if (event.pointerType !== 'touch') hide(); });
    bind(body, 'blur', hide);
    bind(body, 'click', () => (held === body ? hide() : show(body)));
    bind(body, 'keydown', event => {
      if (event.key === 'Escape') hide();
    });
  }
  for (const option of scene.querySelectorAll('.orbit-switch__option')) {
    bind(option, 'click', () => select(option.dataset.target));
  }

  const move = ({ clientX, clientY }) => {
    if (motion.matches) return;
    const rect = scene.getBoundingClientRect();
    scene.style.setProperty('--px', ((clientX - rect.left) / rect.width - .5).toFixed(3));
    scene.style.setProperty('--py', ((clientY - rect.top) / rect.height - .5).toFixed(3));
  };
  const leave = () => {
    scene.style.setProperty('--px', 0);
    scene.style.setProperty('--py', 0);
  };
  bind(scene, 'pointermove', move);
  bind(scene, 'pointerleave', leave);

  const onMotion = () => {
    stop();
    if (motion.matches) {
      view.tilt = view.to;
      drawPlanes();
      place();
    } else start();
  };
  const onVisibility = () => (document.hidden ? stop() : start());
  motion.addEventListener('change', onMotion);
  document.addEventListener('visibilitychange', onVisibility);

  const sizeObserver = new ResizeObserver(measure);
  sizeObserver.observe(stage);
  const screenObserver = new IntersectionObserver(([entry]) => {
    onscreen = entry.isIntersecting;
    if (onscreen) start(); else stop();
  });
  screenObserver.observe(stage);

  measure();
  document.fonts?.ready.then(measure);
  start();

  let disposed = false;
  let disposeSculpture;
  import('./sculpture.js').then(module => {
    if (!disposed) disposeSculpture = module.mountSculpture(scene.querySelector('.sculpture-mount'));
  }).catch(() => { /* The original vector remains visible when WebGL is unavailable. */ });

  return () => {
    disposed = true;
    stop();
    sizeObserver.disconnect();
    screenObserver.disconnect();
    motion.removeEventListener('change', onMotion);
    document.removeEventListener('visibilitychange', onVisibility);
    listeners.forEach(dispose => dispose());
    disposeSculpture?.();
  };
}
