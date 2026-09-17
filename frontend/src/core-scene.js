import { Mark } from './brand.js';

const inputs = [
  ['Program Upgrade', 'live', 0],
  ['Governance Change', 'future', 1],
  ['Privilege / Authority', 'future', 2],
  ['Parameter Change', 'future', 3],
];
const outputs = [
  ['Economic Impact', 'live', 4, '<span>pool_tokens_<wbr>received</span><b>↓ 21 bps</b>'],
  ['CI Decision', 'live', 5, '<span>UNEXPECTED</span><b class="signal-result__fail">FAIL</b>'],
  ['Authority Surface', 'future', 6],
  ['Exitability', 'future', 7],
];
// Initial gutter-only paths; attachment measures the actual label ports.
const paths = [
  'M188 172C225 172 216 283 257 283',
  'M188 306C223 306 221 298 257 298',
  'M188 384C224 384 221 318 257 318',
  'M188 462C224 462 220 340 257 340',
  'M423 283C464 283 465 172 505 172',
  'M423 314C464 314 465 301 505 301',
  'M423 333C464 333 465 419 505 419',
  'M423 352C464 352 465 486 505 486',
];

export function EplyxCoreScene() {
  const labels = (items, side) => `<div class="signal-column signal-column--${side}">
    ${items.map(([name, status, connection, result], row) => {
      const content = `<span class="signal-name">${name}</span><small class="signal-status">${status === 'live' ? 'Live' : 'Planned'}</small>`;
      const attributes = `class="signal signal--${status}" style="--signal-row:${row}" data-connection="${connection}"`;
      if (result) return `<div ${attributes}><button type="button" class="signal-trigger" aria-describedby="signal-result-${connection}">${content}</button><div id="signal-result-${connection}" class="signal-result" role="tooltip">${result}</div><i class="signal-port" aria-hidden="true"></i></div>`;
      return `<${status === 'future' ? 'button type="button"' : 'div'} ${attributes}${status === 'future' ? ` aria-label="${name}: future capability"` : ''}>${content}<i class="signal-port" aria-hidden="true"></i></${status === 'future' ? 'button' : 'div'}>`;
    }).join('')}
  </div>`;
  return `<div class="core-scene" role="group" aria-label="Change flows through Eplyx into consequences. Explore Economic Impact and CI Decision for demo outputs. Governance, authority, parameter and exitability analysis are planned.">
    <svg class="execution-traces" viewBox="0 0 690 560" preserveAspectRatio="none" aria-hidden="true">
      <defs>
        ${[0, 4, 5].map(i => `<linearGradient id="trace-live-${i}" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="690" y2="0"><stop stop-color="${i === 0 ? '#ae7cf3' : '#d9b7ff'}"/><stop offset=".36" stop-color="#f3e6ff"/><stop offset=".7" stop-color="#c295f4"/><stop offset="1" stop-color="${i === 0 ? '#eee0ff' : '#7840be'}"/></linearGradient>`).join('')}
        <linearGradient id="trace-specular" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="690" y2="0"><stop stop-color="#fffaff" stop-opacity=".2"/><stop offset=".45" stop-color="#fffaff" stop-opacity=".95"/><stop offset="1" stop-color="#f0dfff" stop-opacity=".3"/></linearGradient>
        <filter id="trace-glow" x="-30%" y="-50%" width="160%" height="200%"><feGaussianBlur stdDeviation="3"/></filter>
      </defs>
      ${[0, 4, 5].map(i => `<path class="trace-halo" data-route="${i}" d="${paths[i]}"/>`).join('')}
      ${paths.map((d, i) => `<path class="trace ${[0, 4, 5].includes(i) ? 'trace--live' : 'trace--future'}" data-trace="${i}" data-route="${i}"${[0, 4, 5].includes(i) ? ` style="stroke:url(#trace-live-${i})"` : ''} d="${d}"/>`).join('')}
      ${[0, 4, 5].map(i => `<path class="trace-specular" data-route="${i}" d="${paths[i]}"/>`).join('')}
      ${[0, 4, 5].map((i, n) => `<path class="trace-pulse trace-pulse--${['input', 'impact', 'ci'][n]}" pathLength="1" data-route="${i}" d="${paths[i]}"/>`).join('')}
      ${[0, 4, 5].map(i => `<path class="trace-arrow" data-arrow="${i}" style="fill:url(#trace-live-${i})"/>`).join('')}
    </svg>
    ${labels(inputs, 'input')}
    <div class="core-glow" aria-hidden="true"></div>
    <div class="logo-core" aria-hidden="true"><div class="sculpture-fallback">${Mark({ className: 'core-mark core-mark--face' })}</div><div class="sculpture-mount"></div></div>
    ${labels(outputs, 'output')}
    <div class="scene-footnote"><span><i></i> Live execution path</span><span>Future layers · planned</span></div>
  </div>`;
}

export function attachCoreParallax() {
  const scene = document.querySelector('.core-scene');
  if (!scene) return () => {};
  // Long horizontal runs sit below the labels; bends stay in the gutters.
  // Measure instead of assuming a fixed relationship between text and SVG.
  const updateRoutes = () => {
    if (!scene.isConnected) return;
    const bounds = scene.getBoundingClientRect();
    if (!bounds.width || !bounds.height) return;
    const core = scene.querySelector('.logo-core').getBoundingClientRect();
    const x = value => (value - bounds.left) * 690 / bounds.width;
    const y = value => (value - bounds.top) * 560 / bounds.height;
    const coreLeft = x(core.left + core.width * .18);
    const coreRight = x(core.right - core.width * .18);
    const coreMiddle = y(core.top + core.height * .5);
    scene.querySelectorAll('.signal').forEach(label => {
      if (!label.getClientRects().length) return;
      const id = Number(label.dataset.connection);
      const port = label.querySelector('.signal-port').getBoundingClientRect();
      const portX = x(port.left + port.width / 2);
      const portY = y(port.top + port.height / 2);
      const input = id < 4;
      const coreY = coreMiddle + [-27, -9, 9, 27][id % 4];
      const startX = input ? portX : coreRight;
      const startY = input ? portY : coreY;
      const endX = input ? coreLeft : portX;
      const endY = input ? coreY : portY;
      const column = label.closest('.signal-column').getBoundingClientRect();
      const gutter = input ? x(column.right + 12) : x(column.left - 12);
      const bend = input ? (gutter + endX) / 2 : (startX + gutter) / 2;
      const d = input
        ? `M${startX} ${startY}H${gutter}C${bend} ${startY} ${bend} ${endY} ${endX} ${endY}`
        : `M${startX} ${startY}C${bend} ${startY} ${bend} ${endY} ${gutter} ${endY}H${endX}`;
      scene.querySelectorAll(`[data-route="${id}"]`).forEach(path => path.setAttribute('d', d));
      const gradient = scene.querySelector(`#trace-live-${id}`);
      if (gradient) {
        for (const [attribute, value] of Object.entries({ x1: startX, y1: startY, x2: endX, y2: endY })) gradient.setAttribute(attribute, value);
      }
      const arrow = scene.querySelector(`[data-arrow="${id}"]`);
      arrow?.setAttribute('d', `M${endX - 13} ${endY - 5.5}Q${endX - 14} ${endY - 6.5} ${endX - 11} ${endY - 5.5}L${endX} ${endY}L${endX - 11} ${endY + 5.5}Q${endX - 14} ${endY + 6.5} ${endX - 13} ${endY + 5.5}L${endX - 8.5} ${endY}Z`);
    });
  };
  const routesObserver = new ResizeObserver(updateRoutes);
  routesObserver.observe(scene);
  routesObserver.observe(scene.querySelector('.logo-core'));
  scene.querySelectorAll('.signal').forEach(label => routesObserver.observe(label));
  updateRoutes();
  document.fonts?.ready.then(updateRoutes);
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const move = ({ clientX, clientY }) => {
    if (motion.matches) return;
    const r = scene.getBoundingClientRect();
    scene.style.setProperty('--px', ((clientX - r.left) / r.width - .5).toFixed(3));
    scene.style.setProperty('--py', ((clientY - r.top) / r.height - .5).toFixed(3));
  };
  const leave = () => {
    scene.style.setProperty('--px', 0);
    scene.style.setProperty('--py', 0);
  };
  scene.addEventListener('pointermove', move);
  scene.addEventListener('pointerleave', leave);
  const connections = [];
  scene.querySelectorAll('.signal--future, .signal:has(.signal-trigger)').forEach(label => {
    const path = scene.querySelector(`[data-trace="${label.dataset.connection}"]`);
    const highlight = () => path?.classList.add('trace--highlighted');
    const reset = () => path?.classList.remove('trace--highlighted');
    for (const [event, handler] of [['pointerenter', highlight], ['focusin', highlight], ['pointerleave', reset], ['focusout', reset]]) {
      label.addEventListener(event, handler);
      connections.push(() => label.removeEventListener(event, handler));
    }
  });
  scene.querySelectorAll('.signal-trigger').forEach(trigger => {
    const label = trigger.closest('.signal');
    const dismiss = event => {
      if (event.key === 'Escape') label.classList.add('signal--dismissed');
    };
    const reset = () => label.classList.remove('signal--dismissed');
    for (const [event, handler] of [['keydown', dismiss], ['pointerleave', reset], ['focusout', reset], ['pointerenter', reset]]) {
      label.addEventListener(event, handler);
      connections.push(() => label.removeEventListener(event, handler));
    }
  });
  let disposed = false;
  let disposeSculpture;
  import('./sculpture.js').then(module => {
    if (!disposed) disposeSculpture = module.mountSculpture(scene.querySelector('.sculpture-mount'));
  }).catch(() => { /* The original vector remains visible when WebGL is unavailable. */ });
  return () => {
    disposed = true;
    routesObserver.disconnect();
    disposeSculpture?.();
    scene.removeEventListener('pointermove', move);
    scene.removeEventListener('pointerleave', leave);
    connections.forEach(dispose => dispose());
  };
}
