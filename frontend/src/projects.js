// The operator console: projects, their bundles, their tokens, their runs.
//
// Every page here renders what the server said and nothing else. A project with
// no active bundle says so and offers the step that fixes it; a project whose
// program this build reads no semantics for says that too, because a red gate
// for "we did not look" is not the same as a finding.

import { Header, Footer } from './shell.js';
import { api, json, isConnected, setOperatorToken, short, when, ApiError } from './session.js';
import { changeLine } from './change.js';
import { runSummary } from './analysis.js';

const STATUS_LABEL = {
  setup: 'Setup required',
  ready: 'Ready',
  disabled: 'Disabled',
};

const escapeHtml = value =>
  String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[c]));

// ------------------------------------------------------------------ pages

export function ProjectsPage() {
  return page(`
    <div class="console-head">
      <div>
        <p class="eyebrow"><span></span> Operator console</p>
        <h1>Projects</h1>
        <p>Each project is one Solana program, one active bundle, and the tokens that may check it.</p>
      </div>
      <button type="button" class="button button--primary" id="new-project">New project <span>+</span></button>
    </div>
    <div id="console-body"><div class="empty-result">Loading projects…</div></div>`);
}

export function ProjectPage(projectId) {
  return page(`
    <div id="console-body" data-project="${escapeHtml(projectId)}">
      <div class="empty-result">Loading project…</div>
    </div>`);
}

function page(inner) {
  return `<main id="main" class="inner-page console-page">${Header({ light: true })}
    <section class="console-shell">${inner}</section>
  </main>${Footer()}`;
}

// ----------------------------------------------------------------- attach

export function attachProjects(navigate) {
  const body = document.querySelector('#console-body');
  if (!body) return () => {};
  if (!isConnected()) return connect(body, () => attachProjects(navigate));

  document.querySelector('#new-project')?.addEventListener('click', () => renderCreate(body, navigate));
  load();

  async function load() {
    try {
      const { projects } = await api('/v1/projects');
      body.innerHTML = projects.length ? list(projects) : empty();
      body.querySelectorAll('[data-open]').forEach(row =>
        row.addEventListener('click', () => navigate(`/projects/${row.dataset.open}`)));
      body.querySelector('#first-project')?.addEventListener('click', () => renderCreate(body, navigate));
    } catch (error) {
      body.innerHTML = failure(error, 'projects');
      body.querySelector('#reconnect')?.addEventListener('click', () => {
        setOperatorToken('');
        attachProjects(navigate);
      });
    }
  }
  return () => {};
}

function list(projects) {
  return `<div class="project-list">${projects.map(project => `
    <article class="project-row" data-open="${escapeHtml(project.project_id)}" role="button" tabindex="0">
      <div>
        <h3>${escapeHtml(project.name)}</h3>
        <span class="mono">${escapeHtml(short(project.program_id, 16))}</span>
      </div>
      <span class="pill pill--${escapeHtml(project.status)}">${escapeHtml(STATUS_LABEL[project.status] ?? project.status)}</span>
      <span class="mono project-row__adapter">${escapeHtml(project.adapter_id)}</span>
      <span class="mono">${escapeHtml(short(project.active_bundle?.bundle_sha256))}</span>
    </article>`).join('')}</div>`;
}

const empty = () => `<div class="empty-result">
  <p>No projects yet. A project is one Solana program Eplyx watches.</p>
  <button type="button" class="button button--primary" id="first-project">Create the first one <span>+</span></button>
</div>`;

/** Ask for the operator credential. Never public: this lists every program. */
function connect(body, retry) {
  body.innerHTML = `<div class="connect-panel">
    <h2>Connect this console</h2>
    <p>The operator token is configured on the server as <span class="mono">EPLYX_OPERATOR_TOKEN</span>. It is kept in this browser tab only, and is not a project token.</p>
    <label>Operator token<input type="password" id="operator-token" autocomplete="off" placeholder="operator token"></label>
    <button type="button" class="button button--primary" id="connect">Connect <span>↗</span></button>
  </div>`;
  const submit = () => {
    const value = body.querySelector('#operator-token').value.trim();
    if (!value) return;
    setOperatorToken(value);
    retry();
  };
  body.querySelector('#connect').addEventListener('click', submit);
  body.querySelector('#operator-token').addEventListener('keydown', event => {
    if (event.key === 'Enter') submit();
  });
  return () => {};
}

function failure(error, what) {
  if (error instanceof ApiError && error.status === 401) {
    return `<div class="empty-result is-error"><p>That operator token was not accepted.</p>
      <button type="button" class="button button--primary" id="reconnect">Try another <span>↗</span></button></div>`;
  }
  return `<div class="empty-result is-error"><p>Could not load ${escapeHtml(what)}: ${escapeHtml(error.message)}</p></div>`;
}

// ------------------------------------------------------------- create form

function renderCreate(body, navigate) {
  body.innerHTML = `<form class="console-form" id="create-project">
    <h2>New project</h2>
    <label>Project name<input name="name" required maxlength="80" placeholder="Example Lending"></label>
    <label>Solana program ID<input name="program_id" required placeholder="SPoo1Ku8…" spellcheck="false"></label>
    <label>Adapter<select name="adapter_id" required><option value="">Loading…</option></select>
      <small id="adapter-note">The adapter is decided by the program, not chosen. This confirms which one this build speaks for it.</small></label>
    <div class="form-status" role="status" aria-live="polite"></div>
    <div class="console-form__actions">
      <button type="submit" class="button button--primary">Create project <span>↗</span></button>
    </div>
  </form>`;

  const form = body.querySelector('#create-project');
  const select = form.querySelector('[name=adapter_id]');
  const status = form.querySelector('.form-status');
  let adapters = [];

  api('/v1/adapters').then(({ adapters: list }) => {
    adapters = list;
    select.innerHTML = list
      .map(a => `<option value="${escapeHtml(a.adapter_id)}">${escapeHtml(a.adapter_id)}${a.speaks_semantics ? '' : ' — no semantic coverage'}</option>`)
      .join('');
    matchProgram();
  }).catch(() => {
    select.innerHTML = '<option value="">Could not load adapters</option>';
  });

  // The engine picks the adapter from the program, so typing a known program
  // selects it. Leaving the choice open would invite a declaration the server
  // is going to refuse anyway.
  const matchProgram = () => {
    const program = form.querySelector('[name=program_id]').value.trim();
    const match = adapters.find(a => a.program_id === program);
    const fallback = adapters.find(a => !a.speaks_semantics);
    if (match) select.value = match.adapter_id;
    else if (program && fallback) select.value = fallback.adapter_id;
    const chosen = adapters.find(a => a.adapter_id === select.value);
    form.querySelector('#adapter-note').textContent = chosen && !chosen.speaks_semantics
      ? 'This build reads no semantics for that program. Checks will run and report no semantic coverage, which fails the gate: that is Eplyx saying it did not look, not that nothing is wrong.'
      : 'The adapter is decided by the program, not chosen. This confirms which one this build speaks for it.';
  };
  form.querySelector('[name=program_id]').addEventListener('input', matchProgram);
  select.addEventListener('change', matchProgram);

  form.addEventListener('submit', async event => {
    event.preventDefault();
    const values = new FormData(form);
    status.className = 'form-status is-visible';
    status.textContent = 'Creating…';
    try {
      const project = await api('/v1/projects', {
        method: 'POST',
        ...json({
          name: String(values.get('name')),
          program_id: String(values.get('program_id')).trim(),
          adapter_id: String(values.get('adapter_id')),
        }),
      });
      navigate(`/projects/${project.project_id}`);
    } catch (error) {
      status.className = 'form-status is-visible is-error';
      status.textContent = error.message;
    }
  });
}

// ------------------------------------------------------------ project page

export function attachProject(projectId, navigate) {
  const body = document.querySelector('#console-body');
  if (!body) return () => {};
  if (!isConnected()) return connect(body, () => attachProject(projectId, navigate));

  let issued = null;
  let verified = null;
  load();

  async function load() {
    try {
      const [detail, bundles, runs] = await Promise.all([
        api(`/v1/projects/${projectId}`),
        api(`/v1/projects/${projectId}/bundles`),
        api(`/v1/projects/${projectId}/runs?limit=10`),
      ]);
      body.innerHTML = detail_view(detail, bundles.bundles, runs.runs, issued, verified);
      wire(detail.project, bundles.bundles);
    } catch (error) {
      body.innerHTML = failure(error, 'this project');
    }
  }

  function wire(project, bundles) {
    body.querySelectorAll('[data-run]').forEach(row =>
      row.addEventListener('click', () => navigate(`/runs/${row.dataset.run}`)));
    body.querySelector('#analyse-link')?.addEventListener('click', () => navigate('/analyse'));

    body.querySelector('#bundle-upload')?.addEventListener('change', async event => {
      const files = [...event.target.files];
      if (!files.length) return;
      const status = body.querySelector('#bundle-status');
      status.className = 'form-status is-visible';
      status.textContent = `Uploading and verifying ${files.length} file(s)…`;
      const payload = new FormData();
      for (const file of files) {
        // The browser gives the folder name as the first segment; the server
        // wants paths relative to the bundle root.
        const relative = (file.webkitRelativePath || file.name).split('/').slice(1).join('/') || file.name;
        payload.append(relative, file, relative);
      }
      try {
        // Held across the reload below: the panel that reports what
        // verification found is the point of uploading, and re-rendering the
        // page was quietly throwing it away.
        verified = await api(`/v1/projects/${projectId}/bundles`, { method: 'POST', body: payload });
        await load();
      } catch (error) {
        status.className = 'form-status is-visible is-error';
        status.textContent = error.message;
      }
    });

    body.querySelectorAll('[data-activate]').forEach(button =>
      button.addEventListener('click', async () => {
        button.disabled = true;
        try {
          await api(`/v1/projects/${projectId}/bundles/${button.dataset.activate}/activate`, { method: 'POST' });
          await load();
        } catch (error) {
          const status = body.querySelector('#bundle-status');
          status.className = 'form-status is-visible is-error';
          status.textContent = error.message;
          button.disabled = false;
        }
      }));

    body.querySelector('#create-token')?.addEventListener('click', async () => {
      const label = body.querySelector('#token-label').value.trim() || 'CI';
      try {
        const token = await api(`/v1/projects/${projectId}/tokens`, { method: 'POST', ...json({ label }) });
        // Held in memory for this render only, and deliberately never stored:
        // the server cannot show it again, so neither should this page.
        issued = token;
        await load();
        issued = null;
      } catch (error) {
        const status = body.querySelector('#token-status');
        status.className = 'form-status is-visible is-error';
        status.textContent = error.message;
      }
    });

    body.querySelector('#copy-token')?.addEventListener('click', event => {
      navigator.clipboard?.writeText(event.target.dataset.token ?? '');
      event.target.textContent = 'Copied';
    });

    const _ = project.status;
    void bundles;
  }

  return () => {};
}

function detail_view(detail, bundles, runs, issued, verified) {
  const project = detail.project;
  const setup = project.status === 'setup';
  return `
    <div class="console-head">
      <div>
        <p class="eyebrow"><span></span> ${escapeHtml(project.chain)} · ${escapeHtml(project.adapter_id)}</p>
        <h1>${escapeHtml(project.name)}</h1>
        <p class="mono">${escapeHtml(project.program_id)}</p>
      </div>
      <span class="pill pill--${escapeHtml(project.status)}">${escapeHtml(STATUS_LABEL[project.status] ?? project.status)}</span>
    </div>

    ${setup ? `<div class="callout">Upload and activate a verified Eplyx bundle before checks can run.</div>` : ''}
    ${project.speaks_semantics ? '' : `<div class="callout callout--quiet">This build reads no semantics for this program. Checks run, and report no semantic coverage — Eplyx saying it did not look, not that nothing is wrong.</div>`}

    <section class="console-section">
      <h2>Configuration</h2>
      <div class="provenance-table">
        <div><span>Project</span><b class="mono">${escapeHtml(project.project_id)}</b></div>
        <div><span>Program</span><b class="mono">${escapeHtml(project.program_id)}</b></div>
        <div><span>Adapter</span><b>${escapeHtml(project.adapter_id)}</b></div>
        <div><span>Created</span><b>${escapeHtml(when(project.created_at_unix_seconds))}</b></div>
        <div><span>Active bundle</span><b class="mono">${escapeHtml(short(project.active_bundle?.bundle_sha256, 16))}</b></div>
      </div>
    </section>

    <section class="console-section">
      <h2>Bundle</h2>
      <p class="console-note">Activation changes what every future check is measured against, so it is always a separate step. Previous bundles are kept.</p>
      <label class="file-drop">
        <input type="file" id="bundle-upload" webkitdirectory directory multiple>
        <span><b>Upload a bundle</b><em>Choose the .eplyx/bundle directory</em></span><strong>Choose folder</strong>
      </label>
      <div class="form-status" id="bundle-status" role="status" aria-live="polite"></div>
      ${verified ? `<div class="callout verify-result">
        <b>Verified.</b> This bundle opened, its hashes check out, and it is for this project's program and adapter.
        <dl>
          <dt>Bundle</dt><dd class="mono">${escapeHtml(verified.bundle_sha256)}</dd>
          <dt>Baseline</dt><dd class="mono">${escapeHtml(verified.baseline_sha256)}</dd>
          <dt>Records</dt><dd>${escapeHtml(verified.record_count)}</dd>
          <dt>Adapter</dt><dd>${escapeHtml(verified.adapter_id)}</dd>
          <dt>Semantic schema</dt><dd>v${escapeHtml(verified.semantic_schema_version)}</dd>
        </dl>
        Activate it below when you want future checks measured against it.
      </div>` : ''}
      ${bundles.length ? `<div class="bundle-list">${bundles.map(bundle => `
        <article class="bundle-row${bundle.active ? ' is-active' : ''}">
          <div><span class="mono">${escapeHtml(short(bundle.bundle_sha256, 14))}</span><small>baseline ${escapeHtml(short(bundle.baseline_sha256))} · ${bundle.record_count} record(s) · ${escapeHtml(bundle.adapter_id)}</small></div>
          <span>${escapeHtml(when(bundle.created_at_unix_seconds))}</span>
          ${bundle.active ? '<span class="pill pill--ready">Active</span>' : `<button type="button" class="button button--text" data-activate="${escapeHtml(bundle.bundle_id)}">Activate</button>`}
        </article>`).join('')}</div>` : '<div class="empty-result">No bundles uploaded yet.</div>'}
    </section>

    <section class="console-section">
      <h2>API access</h2>
      ${issued ? `<div class="token-issued">
        <p><b>${escapeHtml(issued.label)}</b> — copy it now. You won’t be able to view this token again.</p>
        <code class="mono">${escapeHtml(issued.token)}</code>
        <button type="button" class="button button--text" id="copy-token" data-token="${escapeHtml(issued.token)}">Copy token</button>
      </div>` : ''}
      <div class="token-create">
        <input id="token-label" placeholder="Label, e.g. GitHub Actions" maxlength="80">
        <button type="button" class="button button--text" id="create-token">Create token <span>+</span></button>
      </div>
      <div class="form-status" id="token-status" role="status" aria-live="polite"></div>
      <p class="console-note">Project tokens are for pipelines. This console uses the operator credential and never stores a project token.</p>
    </section>

    <section class="console-section">
      <h2>Recent runs</h2>
      ${runs.length ? `<div class="run-list">${runs.map(runRow).join('')}</div>`
        : `<div class="empty-result"><p>No runs yet.</p>${project.status === 'ready' ? '<button type="button" class="button button--primary" id="analyse-link">Analyse a program upgrade <span>↗</span></button>' : ''}</div>`}
    </section>`;
}

/**
 * One scannable history line: what was changed, whether execution verified,
 * and what the analysis could say about impact. Hashes stay in the run page.
 */
export function runRow(run) {
  const summary = runSummary(run);
  return `
        <article class="run-row" data-run="${escapeHtml(run.run_id)}" role="button" tabindex="0">
          <span>${escapeHtml(when(run.created_at_unix_seconds))}</span>
          ${changeLine(run)}
          <span class="chip chip--${escapeHtml(summary.execution.tone)}"><small>Execution</small>${escapeHtml(summary.execution.label)}</span>
          <span class="chip chip--${escapeHtml(summary.impact.tone)}"><small>Impact</small>${escapeHtml(summary.impact.label)}</span>
          <span class="run-row__exit">${run.exit_code == null ? '—' : `exit ${escapeHtml(run.exit_code)}`}</span>
        </article>`;
}
