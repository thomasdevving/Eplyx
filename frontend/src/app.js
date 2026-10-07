import { StartPage, attachStart } from './start.js';
import { CliPage, attachCommandCopy } from './cli.js';
import { TokenTransitionsPage } from './transitions.js';
import { LandingPage } from './landing.js';
import { AnalysePage, attachAnalyse } from './analyse.js';
import { ReportPage, attachReport } from './report.js';
import { ProjectsPage, ProjectPage, attachProjects, attachProject } from './projects.js';
import { finishIntro } from './intro.js';
import { attachCoreParallax } from './core-scene.js';
import { attachShell } from './shell.js';
import { LegalPage, legalTitle } from './legal.js';
import { attachLandingMotion } from './landing-motion.js';

const app = document.querySelector('#app');
let revealObserver;
let disposeCoreScene;
let disposeReport;
let disposeLandingMotion;

function route() {
  revealObserver?.disconnect();
  disposeLandingMotion?.();
  disposeLandingMotion = undefined;
  disposeCoreScene?.();
  disposeCoreScene = undefined;
  // Leaving a run page stops the polling; the run itself is unaffected.
  disposeReport?.();
  disposeReport = undefined;
  const path = location.pathname.replace(/\/+$/, '') || '/';
  const legal = LegalPage(path);
  document.title = legalTitle(path) || 'Eplyx — Know what changes';
  if (legal) app.innerHTML = legal;
  else if (path === '/start') app.innerHTML = StartPage();
  else if (path === '/cli') app.innerHTML = CliPage();
  else if (path === '/token-transitions') app.innerHTML = TokenTransitionsPage();
  else if (path === '/analyse') app.innerHTML = AnalysePage();
  else if (path === '/projects') app.innerHTML = ProjectsPage();
  else if (path.startsWith('/projects/')) app.innerHTML = ProjectPage(decodeURIComponent(path.slice(10)));
  else if (path.startsWith('/runs/')) app.innerHTML = ReportPage(decodeURIComponent(path.slice(6)));
  else app.innerHTML = LandingPage();
  if (location.hash && path !== '/start') requestAnimationFrame(() => document.getElementById(decodeFragment(location.hash.slice(1)))?.scrollIntoView());
  else window.scrollTo(0, 0);
  attachPage(path);
}

function decodeFragment(fragment) {
  try { return decodeURIComponent(fragment); } catch { return fragment; }
}

function navigate(path) {
  history.pushState({}, '', path);
  route();
}

/** Everything a freshly painted page needs, whoever painted it. */
function decorate() {
  document.querySelectorAll('[data-link]').forEach(link => link.addEventListener('click', event => {
    const url = new URL(link.href, location.href);
    if (url.origin !== location.origin || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    if (url.pathname === location.pathname && url.hash) return;
    event.preventDefault();
    navigate(url.pathname + url.search + url.hash);
  }));
  attachShell();
  attachCommandCopy();
  revealObserver?.disconnect();
  revealObserver = new IntersectionObserver(entries => entries.forEach(entry => {
    if (entry.isIntersecting) { entry.target.classList.add('is-visible'); revealObserver.unobserve(entry.target); }
  }), { threshold: .12 });
  document.querySelectorAll('.reveal').forEach(el => revealObserver.observe(el));
}

function attachPage(path) {
  decorate();
  if (path.startsWith('/runs/')) {
    // The run page repaints itself as the run advances. Repainting is the
    // router's job, so the poller is handed a way to do it rather than
    // reaching into the DOM on its own.
    disposeReport = attachReport(decodeURIComponent(path.slice(6)), markup => {
      app.innerHTML = markup;
      decorate();
    });
  }
  if (path === '/') {
    disposeCoreScene = attachCoreParallax();
    disposeLandingMotion = attachLandingMotion();
    finishIntro();
  }
  if (path === '/start') attachStart();
  if (path === '/analyse') attachAnalyse(navigate);
  if (path === '/projects') disposeReport = attachProjects(navigate);
  if (path.startsWith('/projects/')) disposeReport = attachProject(decodeURIComponent(path.slice(10)), navigate);
}

addEventListener('popstate', route);
route();
