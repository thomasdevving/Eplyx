import { Logo } from './brand.js';
import { API_BASE } from './session.js';

export function workspaceHref() {
  if (!API_BASE) return '/workspaces';
  try {
    const url = new URL(API_BASE);
    if (url.protocol === 'https:' || url.protocol === 'http:') return `${url.origin}/workspaces`;
  } catch { /* Invalid configuration must not create an executable link. */ }
  return '/workspaces';
}

export function Header({ light = false } = {}) {
  return `
    <header class="site-header ${light ? 'site-header--light' : ''}">
      <a href="/" data-link class="logo-link">${Logo()}</a>
      <nav id="primary-navigation" aria-label="Primary navigation">
        <a href="/start" data-link>Start here</a>
        <a href="/token-transitions" data-link>Token transitions</a>
        <a href="/cli" data-link>CLI</a>
        <a href="/#roadmap" data-link>Roadmap</a>
        <a href="/projects" data-link>Projects</a>
        <a href="${workspaceHref()}" class="nav-cta">Workspace <span>↗</span></a>
      </nav>
      <button class="menu-button" type="button" aria-label="Open navigation" aria-controls="primary-navigation" aria-expanded="false"><span></span><span></span></button>
    </header>`;
}

export function Footer() {
  return `
    <footer class="footer">
      <div>${Logo()}</div>
      <p>Change and consequence analysis for Solana.</p>
      <div class="footer__links"><a href="/start" data-link>Start here</a><a href="/cli" data-link>CLI guide</a><a href="/runs/demo" data-link>Demo report</a><a href="https://github.com/thomasdevving/Eplyx">Source</a></div>
      <nav class="footer__legal" aria-label="Legal and privacy"><a href="/legal" data-link>Legal & privacy</a><a href="/privacy" data-link>Privacy</a><a href="/cookies" data-link>Cookies & storage</a><a href="/terms" data-link>Website use</a><a href="/contact" data-link>Contact</a><a href="/licenses" data-link>Licences</a></nav>
      <small>Evidence from tested interactions. Coverage is always explicit.</small>
    </footer>`;
}

export function attachShell() {
  const button = document.querySelector('.menu-button');
  const nav = document.querySelector('.site-header nav');
  button?.addEventListener('click', () => {
    const open = button.getAttribute('aria-expanded') === 'true';
    button.setAttribute('aria-expanded', String(!open));
    button.setAttribute('aria-label', open ? 'Open navigation' : 'Close navigation');
    nav?.classList.toggle('is-open', !open);
  });
  nav?.querySelectorAll('a').forEach(link => link.addEventListener('click', () => {
    nav.classList.remove('is-open');
    button?.setAttribute('aria-expanded', 'false');
    button?.setAttribute('aria-label', 'Open navigation');
  }));
  document.querySelector('.site-header')?.addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      nav?.classList.remove('is-open');
      button?.setAttribute('aria-expanded', 'false');
      button.setAttribute('aria-label', 'Open navigation');
    }
  });
}
