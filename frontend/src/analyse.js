import { Header, Footer } from './shell.js';
import { api, isConnected, short, ApiError } from './session.js';

const escapeHtml = value =>
  String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[c]));

export function AnalysePage() {
  return `<main id="main" class="inner-page analyse-page">${Header({ light: true })}
    <section class="analyse-shell">
      <div class="analyse-copy">
        <p class="eyebrow"><span></span> Hosted check</p>
        <h1>Analyse a candidate upgrade.</h1>
        <p>Submit a compiled Solana program to one of your projects. The project’s active bundle supplies the pinned baseline and validated historical corpus — the server resolves it, never the caller.</p>
        <div class="input-model"><span>Your candidate.so</span><b>+</b><span>Optional expected changes</span><i>→</i><strong>Project bundle</strong><i>→</i><em>Replay report</em></div>
        <a href="/runs/demo" data-link class="text-link">View the public demo report <span>↗</span></a>
      </div>
      <form class="analyse-form" id="analyse-form">
        <div class="form-head"><span>New upgrade check</span><em>Uses the real hosted API</em></div>
        <label>Project<select name="project" id="project-select" required><option value="">Loading projects…</option></select>
          <small id="project-note">Only projects with an active bundle can be checked.</small></label>
        <label class="file-drop"><input name="candidate" type="file" accept=".so,application/octet-stream" required><span><b>Candidate binary</b><em>Drop candidate.so or choose a file</em></span><strong>Choose file</strong></label>
        <label class="file-drop file-drop--optional"><input name="expectations" type="file" accept=".toml,text/plain"><span><b>Expected changes</b><em>Optional .toml declaration</em></span><strong>Choose file</strong></label>
        <div class="form-status" role="status" aria-live="polite"></div>
        <button class="button button--primary" type="submit">Run analysis <span>↗</span></button>
        <p class="form-note">The check is accepted immediately and runs on the server; you can leave this page and come back to the run.</p>
      </form>
    </section>
  </main>${Footer()}`;
}

export function attachAnalyse(navigate) {
  const form = document.querySelector('#analyse-form');
  if (!form) return;
  const status = form.querySelector('.form-status');
  const select = form.querySelector('#project-select');
  const note = form.querySelector('#project-note');
  const submit = form.querySelector('button[type="submit"]');

  document.querySelectorAll('.file-drop input').forEach(input => input.addEventListener('change', () => {
    const em = input.closest('.file-drop')?.querySelector('em');
    if (input.files?.[0] && em) em.textContent = input.files[0].name;
  }));

  if (!isConnected()) {
    select.innerHTML = '<option value="">Connect the console first</option>';
    note.textContent = 'Open Projects and connect with the operator token.';
    submit.disabled = true;
    return;
  }

  // Projects come from the server. There is no project-ID field, and no API URL
  // field: a person should not have to know either to use the product.
  api('/v1/projects').then(({ projects }) => {
    if (!projects.length) {
      select.innerHTML = '<option value="">No projects yet</option>';
      note.textContent = 'Create a project first, then upload and activate a bundle.';
      submit.disabled = true;
      return;
    }
    select.innerHTML = projects
      .map(project => {
        const ready = project.status === 'ready';
        const label = `${project.name} · ${short(project.program_id, 12)}${ready ? '' : ' — setup required'}`;
        return `<option value="${escapeHtml(project.project_id)}"${ready ? '' : ' disabled'}>${escapeHtml(label)}</option>`;
      })
      .join('');
    const first = projects.find(project => project.status === 'ready');
    if (first) {
      select.value = first.project_id;
      // A convenience only. The server decides what this project may do; the
      // remembered choice is never the source of truth.
      try {
        const last = localStorage.getItem('eplyx-last-project');
        if (last && projects.some(p => p.project_id === last && p.status === 'ready')) select.value = last;
      } catch { /* A forgotten preference is not a failure. */ }
    } else {
      note.textContent = 'No project has an active bundle yet. Upload and activate one on its project page.';
      submit.disabled = true;
    }
  }).catch(error => {
    select.innerHTML = '<option value="">Could not load projects</option>';
    note.textContent = error instanceof ApiError && error.status === 401
      ? 'The operator token was not accepted. Reconnect on the Projects page.'
      : error.message;
    submit.disabled = true;
  });

  form.addEventListener('submit', async event => {
    event.preventDefault();
    const values = new FormData(form);
    const projectId = String(values.get('project'));
    if (!projectId) return refuse('Choose a project first.');

    const body = new FormData();
    body.append('candidate', values.get('candidate'));
    const expected = values.get('expectations');
    if (expected instanceof File && expected.size) body.append('expected_changes', expected);

    submit.disabled = true;
    submit.innerHTML = 'Submitting <span class="spinner"></span>';
    status.textContent = 'Uploading the candidate…';
    status.className = 'form-status is-visible';
    try {
      try { localStorage.setItem('eplyx-last-project', projectId); } catch { /* optional */ }
      // The server answers before the analysis starts, so this waits only for
      // the upload and the run id.
      const result = await api(`/v1/projects/${encodeURIComponent(projectId)}/checks`, { method: 'POST', body });
      navigate(`/runs/${result.run_id}`);
    } catch (error) {
      refuse(describe(error));
    }
  });

  function describe(error) {
    if (!(error instanceof ApiError)) return error.message;
    if (error.status === 0) return 'Could not reach Eplyx. Check the API endpoint and its browser CORS configuration.';
    if (error.status === 401) return 'Authentication failed. Reconnect the console on the Projects page.';
    if (error.status === 413) return error.message;
    if (error.status === 409) return error.message;
    return error.message;
  }

  function refuse(message) {
    status.textContent = message;
    status.className = 'form-status is-visible is-error';
    submit.disabled = false;
    submit.innerHTML = 'Run analysis <span>↗</span>';
  }
}
