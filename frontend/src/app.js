import { LandingPage } from './landing.js';
import { AnalysePage, attachAnalyse } from './analyse.js';
import { ReportPage } from './report.js';
import { finishIntro } from './intro.js';
import { attachCoreParallax } from './core-scene.js';
import { attachShell } from './shell.js';

const app = document.querySelector('#app');
let revealObserver;
let disposeCoreScene;

function route() {
  revealObserver?.disconnect();
  disposeCoreScene?.();
  disposeCoreScene = undefined;
  const path = location.pathname.replace(/\/+$/, '') || '/';
  if (path === '/analyse') app.innerHTML = AnalysePage();
  else if (path.startsWith('/runs/')) app.innerHTML = ReportPage(decodeURIComponent(path.slice(6)));
  else app.innerHTML = LandingPage();
  if (location.hash) requestAnimationFrame(() => document.querySelector(location.hash)?.scrollIntoView());
  else window.scrollTo(0, 0);
  attachPage(path);
}

function navigate(path) {
  history.pushState({}, '', path);
  route();
}

function attachPage(path) {
  document.querySelectorAll('[data-link]').forEach(link => link.addEventListener('click', event => {
    const url = new URL(link.href, location.href);
    if (url.origin !== location.origin || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    if (url.pathname === location.pathname && url.hash) return;
    event.preventDefault();
    navigate(url.pathname + url.hash);
  }));
  attachShell();
  if (path === '/') {
    disposeCoreScene = attachCoreParallax();
    finishIntro();
  }
  if (path === '/analyse') attachAnalyse(navigate);
  revealObserver = new IntersectionObserver(entries => entries.forEach(entry => {
    if (entry.isIntersecting) { entry.target.classList.add('is-visible'); revealObserver.unobserve(entry.target); }
  }), { threshold: .12 });
  document.querySelectorAll('.reveal').forEach(el => revealObserver.observe(el));
}

addEventListener('popstate', route);
route();
