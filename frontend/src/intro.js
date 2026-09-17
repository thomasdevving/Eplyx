export function IntroAnimation() {
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) return '';
  try { if (sessionStorage.getItem('eplyx-intro-seen')) return ''; } catch { /* Storage may be disabled. */ }
  return `
    <div class="intro" aria-hidden="true">
      <div class="intro__bloom"></div>
      <svg class="intro__logo" viewBox="0 0 440 440" aria-hidden="true">
        <defs>
          <mask id="intro-wave-reveal" maskUnits="userSpaceOnUse" x="0" y="0" width="440" height="440">
            <path class="intro__wave-brush" pathLength="1" d="M-120 295C-20 295 25 319 82 348S166 358 214 307 301 211 356 202 425 203 560 203" fill="none" stroke="white" stroke-width="168" stroke-linecap="round" />
            <rect class="intro__mask-complete" width="440" height="440" fill="white" />
          </mask>
          <mask id="intro-moon-reveal" maskUnits="userSpaceOnUse" x="0" y="0" width="440" height="440">
            <path class="intro__moon-brush" pathLength="1" d="M61 397C61 346 63 291 67 248 42 168 80 105 135 69 207 20 295 30 355 68L444 122" fill="none" stroke="white" stroke-width="139" stroke-linecap="round" />
            <rect class="intro__mask-complete" width="440" height="440" fill="white" />
          </mask>
        </defs>
        <g mask="url(#intro-moon-reveal)"><use href="/public/logo.svg#crescent"></use></g>
        <g mask="url(#intro-wave-reveal)"><use href="/public/logo.svg#wave"></use></g>
      </svg>
      <span class="intro__name">Eplyx</span>
    </div>`;
}

export function finishIntro() {
  const intro = document.querySelector('.intro');
  if (!intro) return;
  const hero = document.querySelector('.hero');
  hero?.classList.add('hero--intro');
  try { sessionStorage.setItem('eplyx-intro-seen', '1'); } catch { /* The intro still completes without storage. */ }
  const complete = () => {
    clearTimeout(fallback);
    hero?.classList.remove('hero--intro');
    intro.remove();
  };
  intro.addEventListener('animationstart', event => {
    if (event.target === intro && event.animationName === 'introExit') {
      hero?.classList.remove('hero--intro');
    }
  });
  intro.addEventListener('animationend', event => {
    if (event.target === intro && event.animationName === 'introExit') complete();
  });
  const fallback = setTimeout(complete, 3900);
}
