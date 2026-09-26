// Dashboard shell: navigation, routing and the shared Overview / Technical
// presentation switch. Locally the data is read-only `.eplyx/` API output; in a
// hosted workspace the same pages show synced results under a project path.
import { initializeMode, presentationMode, setPresentationMode } from './mode.js';
import { BASE, CLOUD, DEMO, MAIN_SITE_URL } from './env.js';
import { Mark } from './brand.js';
import { esc, empty } from './ui.js';
import * as pages from './pages.js';

initializeMode();
const app = document.querySelector('#app');

const NAV = [
 ['/', 'Overview', () => true],
 ['/runs', 'Runs', () => true],
 ['/counterexamples', 'Counterexamples', () => true],
 ['/compare', 'Compare', p => p.stats?.preflights >= 2],
 ['/production', 'State', p => Boolean(p.latest)],
 ['/invariants', 'Invariants', p => Boolean(p.latest?.invariants?.total)],
 ['/gate', 'CI / gate', p => Boolean(p.latest)],
 ['/project', 'Project', () => true],
 ...(CLOUD && !DEMO ? [['/settings', 'Settings', () => true]] : []),
];

const ROUTES = [
 [/^\/$/, pages.overview],
 [/^\/runs$/, pages.runs],
 [/^\/runs\/(run_[a-z0-9_]+)$/, pages.runDetail],
 [/^\/counterexamples$/, pages.counterexamples],
 [/^\/counterexamples\/(cx_[a-z0-9_]+)$/, pages.counterexampleDetail],
 [/^\/compare$/, pages.compare],
 [/^\/production$/, pages.production],
 [/^\/invariants$/, pages.invariants],
 [/^\/gate$/, pages.gate],
 [/^\/project$/, CLOUD ? (args => cloudPages().then(m => m.projectPage(args))) : pages.projectPage],
 [/^\/settings$/, args => cloudPages().then(m => m.settingsPage(args))],
];

// Cloud-only pages load on demand and are served only by the hosted workspace.
let cloudModule;
const cloudPages = () => (cloudModule ??= import('./settings.js'));
const localPath = () => (location.pathname.startsWith(BASE) ? location.pathname.slice(BASE.length) : location.pathname).replace(/\/+$/, '') || '/';

function cloudSidebar(project) {
 const ws = project.context?.cloud?.workspace;
 if (DEMO) return `<div class="sidebar__project"><span class="eyebrow">Public demo</span><strong>${esc(project.project?.name ?? 'Demo project')}</strong><span class="muted">Published read-only by this server’s operator.</span></div>`;
 return `<div class="sidebar__project"><span class="eyebrow">${esc(ws?.name ?? 'Workspace')}</span><label class="switcher"><span class="sr-only">Project</span><select data-project-switch><option>${esc(project.project?.name ?? 'Project')}</option></select></label><a class="muted" href="/" >All workspaces</a></div>`;
}

function shell(project) {
 const path = localPath();
 const section = path === '/' ? '/' : `/${path.split('/')[1]}`;
 const mode = presentationMode();
 const home = `${BASE}/`;
 return `<div class="shell">
  <aside class="sidebar">
   <a class="brand" ${CLOUD ? `href="${MAIN_SITE_URL}/" aria-label="Eplyx home"` : `href="${home}" data-link aria-label="Eplyx local dashboard"`}>${Mark({ className:'brand__mark' })}<span class="brand__lockup"><span class="brand__word">Eplyx</span><span class="brand__sub">${CLOUD ? 'Cloud workspace' : 'Local analysis'}</span></span></a>
   ${CLOUD ? cloudSidebar(project) : `<div class="sidebar__project"><span class="eyebrow">Project</span><strong>${esc(project.project?.name ?? 'Unnamed project')}</strong></div>`}
   <nav class="nav" aria-label="Dashboard">${NAV.filter(([, , show]) => show(project)).map(([href, label]) => `<a href="${BASE}${href}" data-link ${section === href ? 'aria-current="page"' : ''}>${label}</a>`).join('')}</nav>
   <p class="sidebar__foot">${CLOUD ? 'Synced results from local and CI Eplyx CLI runs. This workspace never reruns RPC, execution or replay. Projects are private to their workspace.' : 'Read-only view of <code>.eplyx/</code> on this machine. Nothing is uploaded or sent.'}</p>
   ${CLOUD && !DEMO ? '<button type="button" class="button button--ghost sidebar__signout" data-sign-out>Sign out</button>' : ''}
  </aside>
  <div class="main-col">
   <header class="topbar">
    <div class="crumbs" data-crumbs></div>
    <div class="mode-switch" role="group" aria-label="Presentation">
     <i class="mode-switch__thumb" aria-hidden="true"></i>
     <button type="button" data-mode-option="overview" aria-pressed="${mode === 'overview'}">Overview</button>
     <button type="button" data-mode-option="technical" aria-pressed="${mode === 'technical'}">Technical</button>
    </div>
   </header>
   ${CLOUD ? '<div class="synced-banner" role="note"><strong>Synced results.</strong> This page shows engine results from a developer machine or CI. It does not run RPC, execution or replay.</div>' : ''}
   <main id="main" tabindex="-1"><div class="loading" role="status">Loading ${CLOUD ? 'synced' : 'local'} runs…</div></main>
  </div>
 </div>`;
}

let token = 0;
async function route({ focus = false } = {}) {
 const current = ++token;
 const path = localPath();
 const query = new URLSearchParams(location.search);
 let project;
 try {
  project = await pages.api('/api/project');
 } catch (error) {
  app.innerHTML = `<main id="main" class="fatal">${empty(CLOUD ? `This workspace project could not be loaded: ${error.message}` : `The dashboard could not read this project's .eplyx/ store: ${error.message}`)}</main>`;
  return;
 }
 if (current !== token) return;
 app.innerHTML = shell(project);
 if (CLOUD && !DEMO) cloudPages().then(m => m.attachSwitcher(app, project)).catch(() => {});
 const main = app.querySelector('main');
 const match = ROUTES.map(([pattern, page]) => [path.match(pattern), page]).find(([m]) => m);
 try {
  if (!match) throw new Error(`This page does not exist in the ${CLOUD ? 'workspace' : 'local dashboard'}.`);
  const [m, page] = match;
  const view = await page({ params:m.slice(1), query, project });
  if (current !== token) return;
  main.innerHTML = view.html;
  document.title = `${view.title} — Eplyx`;
  const crumbs = [[project.project?.name ?? 'Project', '/'], ...(view.crumbs ?? (view.title === 'Overview' ? [] : [[view.title]]))];
  app.querySelector('[data-crumbs]').innerHTML = crumbs.map(([label, href], n) => href && n < crumbs.length - 1 ? `<a href="${esc(`${BASE}${href}`)}" data-link>${esc(label)}</a>` : `<span>${esc(label)}</span>`).join('<span class="crumbs__sep" aria-hidden="true">/</span>');
  view.attach?.(main);
 } catch (error) {
  main.innerHTML = `<div class="page-head"><h1>Unavailable.</h1></div>${empty(error.message)}`;
  document.title = 'Unavailable — Eplyx';
 }
 if (focus) main.focus({ preventScroll:true });
 if (location.hash) requestAnimationFrame(() => document.getElementById(location.hash.slice(1))?.scrollIntoView());
 else if (focus) window.scrollTo(0, 0);
}

window.dashboardNavigate = url => { history.pushState({}, '', url); route({ focus:true }); };

document.addEventListener('click', event => {
 const option = event.target.closest('[data-mode-option]');
 if (option) {
  setPresentationMode(option.dataset.modeOption);
  document.querySelectorAll('[data-mode-option]').forEach(b => b.setAttribute('aria-pressed', String(b === option)));
  return;
 }
 const button = event.target.closest('[data-copy]');
 if (button) {
  const text = button.dataset.copy;
  const done = ok => { const label = button.textContent; button.textContent = ok ? 'Copied' : 'Copy failed'; button.classList.toggle('is-done', ok); setTimeout(() => { button.textContent = label; button.classList.remove('is-done'); }, 1400); };
  (navigator.clipboard?.writeText(text) ?? Promise.reject()).then(() => done(true), () => {
   const area = Object.assign(document.createElement('textarea'), { value:text }); document.body.append(area); area.select();
   let ok = false; try { ok = document.execCommand('copy'); } catch { ok = false; } area.remove(); done(ok);
  });
  return;
 }
 if (event.target.closest('[data-sign-out]')) {
  fetch('/v1/auth/logout', { method:'POST', credentials:'same-origin' }).finally(() => location.assign('/login'));
  return;
 }
 const link = event.target.closest('a[data-link]');
 if (!link || event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
 const url = new URL(link.href);
 if (url.origin !== location.origin) return;
 if (url.pathname === location.pathname && url.search === location.search && url.hash) return;
 event.preventDefault();
 window.dashboardNavigate(url.pathname + url.search + url.hash);
});
addEventListener('popstate', () => route());
route();
