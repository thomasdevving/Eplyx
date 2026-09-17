import { Logo } from './brand.js';

export function Header({ light = false } = {}) {
  return `
    <header class="site-header ${light ? 'site-header--light' : ''}">
      <a href="/" data-link class="logo-link">${Logo()}</a>
      <nav id="primary-navigation" aria-label="Primary navigation">
        <a href="/#product" data-link>Product</a>
        <a href="/#how" data-link>How it works</a>
        <a href="/#evidence" data-link>Evidence</a>
        <a href="/#roles" data-link>Who it’s for</a>
        <a href="/#vision" data-link>Vision</a>
        <a href="/analyse" data-link class="nav-cta">Analyse <span>↗</span></a>
      </nav>
      <button class="menu-button" type="button" aria-label="Open navigation" aria-controls="primary-navigation" aria-expanded="false"><span></span><span></span></button>
    </header>`;
}

export function Footer() {
  return `
    <footer class="footer">
      <div>${Logo()}</div>
      <p>Economic change intelligence for Solana upgrades.</p>
      <div class="footer__links"><a href="/#how" data-link>Method</a><a href="/runs/demo" data-link>Demo report</a><a href="/analyse" data-link>Analyse</a></div>
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
  button?.addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      nav?.classList.remove('is-open');
      button.setAttribute('aria-expanded', 'false');
      button.setAttribute('aria-label', 'Open navigation');
    }
  });
}
