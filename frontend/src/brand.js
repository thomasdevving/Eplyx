// WebKit does not resolve <use> into an external document, so the mark is
// inlined rather than referenced out of public/logo.svg. That file stays the
// source for the CSS mask, the favicon and the extruded sculpture; check.mjs
// holds both copies to the same path data.
export const markPaths = {
  crescent: 'M355 68C316 26 265 10 219 11 124 9 44 69 18 152 4 197 8 242 17 270c3 10 12 11 26 9 30-3 63-19 87-43-14-35-11-65 5-95 22-42 58-76 99-93 44-18 86-7 121 20Z',
  wave: 'M24 291c31 13 60 9 93-12 31-20 64-48 93-69 40-29 81-45 122-43 36 1 71 16 84 34 7 10 4 19-5 24-35 23-61 53-85 84-33 43-65 76-107 90-79 27-157-21-195-108Z',
};

export const markGradient = id =>
  `<linearGradient id="${id}" x1="40" y1="40" x2="370" y2="400" gradientUnits="userSpaceOnUse"><stop stop-color="#ffffff"/><stop offset=".54" stop-color="#eeedf3"/><stop offset="1" stop-color="#c4c1d2"/></linearGradient>`;

// Each instance carries its own gradient so two marks never share an id.
let instances = 0;

export function Mark({ className = '' } = {}) {
  const fill = `eplyx-mark-${++instances}`;
  return `<svg class="${className}" viewBox="0 0 440 440" aria-hidden="true"><defs>${markGradient(fill)}</defs><path fill="url(#${fill})" d="${markPaths.crescent}"/><path fill="url(#${fill})" d="${markPaths.wave}"/></svg>`;
}

export function Logo({ wordmark = true, className = '' } = {}) {
  return `
    <span class="brand ${className}" aria-label="Eplyx">
      ${Mark({ className: 'brand__mark' })}
      ${wordmark ? '<span class="brand__word">Eplyx</span>' : ''}
    </span>`;
}
