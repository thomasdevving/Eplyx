// Functional render tests for the operator console and the analyse form.
//
// The point of this phase is that a person stops typing infrastructure into the
// product: no backend URL, no project id. Those are absences, and an absence is
// exactly what a structural "does the file mention X" check cannot see. So
// these render the modules and assert on the output.

import assert from 'node:assert/strict';

const store = new Map();
globalThis.sessionStorage = {
  getItem: key => (store.has(key) ? store.get(key) : null),
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: key => store.delete(key),
};
const local = new Map();
globalThis.localStorage = {
  getItem: key => (local.has(key) ? local.get(key) : null),
  setItem: (key, value) => local.set(key, String(value)),
  removeItem: key => local.delete(key),
};
globalThis.EPLYX_API_URL = 'http://api.test';

const { AnalysePage, attachAnalyse } = await import('./src/analyse.js');
const { ProjectsPage, attachProjects } = await import('./src/projects.js');
const CONSOLE_KEY = 'eplyx-operator-token';

let failures = 0;
async function check(name, fn) {
  try {
    await fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n       ${error.message.split('\n')[0]}`);
  }
}

/**
 * A DOM small enough to render one page into.
 *
 * Not a browser: enough of one that a module which builds markup and then wires
 * it up can be driven, so these test the real pages rather than a copy of them.
 */
function mount(markup) {
  const nodes = new Map();
  const make = html => {
    const element = {
      innerHTML: html ?? '',
      value: '',
      disabled: false,
      files: [],
      dataset: {},
      textContent: '',
      className: '',
      listeners: {},
      addEventListener(event, handler) {
        (this.listeners[event] ||= []).push(handler);
      },
      querySelector: selector => find(element, selector),
      querySelectorAll: selector => findAll(element, selector),
      closest: () => null,
    };
    return element;
  };
  const root = make(markup);
  // Every selector resolves to a stable stand-in. These tests assert on what a
  // page renders and what it asks the API for, not on selector matching, and a
  // half-real matcher would only fail in ways a browser never would.
  const find = (_scope, selector) => {
    if (!nodes.has(selector)) nodes.set(selector, make());
    return nodes.get(selector);
  };
  const findAll = () => [];
  globalThis.document = {
    querySelector: selector => (selector === '#console-body' || selector === '#analyse-form' ? root : find(root, selector)),
    querySelectorAll: () => [],
  };
  return root;
}

// --- the analyse form asks for nothing infrastructural ------------------
await check('the analyse form has no backend URL and no project id field', () => {
  const html = AnalysePage();
  assert.doesNotMatch(html, /name="api"/, 'an API endpoint field survived');
  assert.doesNotMatch(html, /api\.eplyx\.dev/, 'a hostname is still in the markup');
  assert.doesNotMatch(html, /name="token"/, 'a token field survived');
  assert.doesNotMatch(html, /name="project"[^>]*type="text"/, 'project is still typed in');
  assert.match(html, /<select name="project"/, 'there is no project selector');
  assert.match(html, /name="candidate"/);
  assert.match(html, /name="expectations"/);
  assert.match(html, /Checking project analysis availability/);
  assert.match(html, /<select name="project"[^>]*disabled/);
  assert.match(html, /<button class="button button--primary" type="submit" disabled/);
});

await check('an unconnected console does not pretend it can submit', async () => {
  store.clear();
  const form = mount(AnalysePage());
  attachAnalyse(() => {});
  await new Promise(resolve => setTimeout(resolve, 10));
  const select = form.querySelector('#project-select');
  assert.match(select.innerHTML, /Connect the console/);
});

await check('the project selector is filled from the backend', async () => {
  store.clear();
  store.set(CONSOLE_KEY, 'operator-token');
  const asked = [];
  globalThis.fetch = async (url, options) => {
    asked.push(String(url));
    assert.equal(options.headers.Authorization, 'Bearer operator-token');
    const href = String(url);
    if (href.endsWith('/v1/projects/proj_A/capabilities')) return {
      ok: true,
      status: 200,
      json: async () => ({ schema_version: 1, project_id: 'proj_A', analyses: [
        { kind: 'program_upgrade', status: 'ready', supported: true, can_submit: true, missing: [] },
      ] }),
    };
    if (href.endsWith('/v1/projects/proj_B/capabilities')) return {
      ok: true,
      status: 200,
      json: async () => ({ schema_version: 1, project_id: 'proj_B', analyses: [
        { kind: 'program_upgrade', status: 'not_ready', supported: true, can_submit: false, missing: [
          { code: 'active_bundle_missing', message: 'No active analysis bundle is selected.', action: 'Upload and activate a bundle.' },
        ] },
      ] }),
    };
    return {
      ok: true,
      status: 200,
      json: async () => ({
        projects: [
          // Deliberately misleading generic statuses: capabilities must win.
          { project_id: 'proj_A', name: 'Ready One', program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy', status: 'setup' },
          { project_id: 'proj_B', name: 'Not Yet', program_id: 'HopcampquEa7pvG4d6xkVNE2fkMiT9oZmY8T77XkcMBq', status: 'ready' },
        ],
      }),
    };
  };
  const form = mount(AnalysePage());
  attachAnalyse(() => {});
  await new Promise(resolve => setTimeout(resolve, 10));

  assert.deepEqual(asked, [
    'http://api.test/v1/projects',
    'http://api.test/v1/projects/proj_A/capabilities',
    'http://api.test/v1/projects/proj_B/capabilities',
  ], 'projects and capabilities did not use the configured authenticated API');
  const options = form.querySelector('#project-select').innerHTML;
  assert.match(options, /Ready One/);
  assert.match(options, /Not Yet/);
  assert.doesNotMatch(options, /setup required|disabled/, 'generic project status still drives the options');
  const select = form.querySelector('#project-select');
  assert.equal(select.value, 'proj_A', 'the capability-ready project was not preferred');
  assert.match(form.querySelector('#project-readiness').innerHTML, /Program upgrade/);
  assert.match(form.querySelector('#project-readiness').innerHTML, /Ready/);
  select.value = 'proj_B';
  for (const handler of select.listeners.change ?? []) handler({ target: select });
  const blocked = form.querySelector('#project-readiness').innerHTML;
  assert.match(blocked, /No active analysis bundle is selected/);
  assert.match(blocked, /Upload and activate a bundle/);
  assert.match(blocked, /active_bundle_missing/);
  assert.equal(form.querySelector('button[type="submit"]').disabled, true);
});

await check('a capability request failure keeps upgrade submission closed', async () => {
  store.clear();
  store.set(CONSOLE_KEY, 'operator-token');
  globalThis.fetch = async (url, options) => {
    assert.equal(options.headers.Authorization, 'Bearer operator-token');
    if (String(url).endsWith('/capabilities')) return {
      ok: false,
      status: 503,
      json: async () => ({ error: 'capability service unavailable' }),
    };
    return {
      ok: true,
      status: 200,
      json: async () => ({ projects: [
        { project_id: 'proj_A', name: 'Project A', program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy', status: 'ready' },
      ] }),
    };
  };
  const form = mount(AnalysePage());
  attachAnalyse(() => {});
  await new Promise(resolve => setTimeout(resolve, 10));
  const readiness = form.querySelector('#project-readiness').innerHTML;
  assert.match(readiness, /could not determine project analysis availability/i);
  assert.doesNotMatch(readiness, /Unsupported/);
  assert.equal(form.querySelector('button[type="submit"]').disabled, true);
});

// --- the console keeps its credential to itself -------------------------
await check('the console asks for the operator token before listing anything', async () => {
  store.clear();
  const body = mount(ProjectsPage());
  globalThis.fetch = async () => {
    throw new Error('the console listed projects without a credential');
  };
  attachProjects(() => {});
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.match(body.innerHTML, /Connect this console/);
  assert.match(body.innerHTML, /EPLYX_OPERATOR_TOKEN/);
});

await check('a project page never renders a stored project token', () => {
  const html = ProjectsPage();
  assert.doesNotMatch(html, /eplyx_proj_/, 'a token secret is in the markup');
  assert.doesNotMatch(html, /localStorage/, 'the console keeps a secret in local storage');
});

console.log(failures === 0 ? '\nfrontend console rendering verified' : `\n${failures} console failures`);
process.exit(failures === 0 ? 0 : 1);
