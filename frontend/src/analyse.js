import { Header, Footer } from './shell.js';
import { api, isConnected, short, ApiError } from './session.js';
import { bytes, fingerprint, middle } from './change.js';
import {
  fetchProjectCapabilities, capabilityFor, canSubmitCapability,
  capabilityHTML, capabilityLoadingHTML, capabilityFailureHTML,
} from './capabilities.js';

const escapeHtml = value =>
  String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[c]));

// One proposed change, analysed. The project already fixes which program would
// be upgraded, so the only thing a person supplies is the build they intend to
// deploy. Everything Eplyx needs to name the change exactly — the target, the
// content hash, the change ID — is derived, shown for confirmation, and never
// typed. A caller that already holds a change spec submits it through the API.
export function AnalysePage() {
  return `<main id="main" class="inner-page analyse-page">${Header({ light: true })}
    <section class="analyse-shell">
      <div class="analyse-copy">
        <p class="eyebrow"><span></span> Proposed change</p>
        <h1>Analyse a program upgrade.</h1>
        <p>Choose the project and upload the build you intend to deploy. Eplyx compares it with the project’s pinned baseline and reports measured differences for the interactions covered by that bundle.</p>
        <div class="input-model"><span>Target program</span><b>+</b><span>Candidate build</span><i>→</i><strong>Proposed change</strong><i>→</i><em>Effects report</em></div>
        <p class="analysis-choice">Use this form with an existing hosted project and active bundle. For a local check, follow the <a href="/cli#upgrades" data-link>upgrade CLI guide</a>. For another kind of change, <a href="/start" data-link>choose a workflow</a>.</p>
        <a href="/runs/demo" data-link class="text-link">View the public demo report <span>↗</span></a>
      </div>
      <form class="analyse-form" id="analyse-form">
        <div class="form-head"><span>Program upgrade</span><em>Uses the real hosted API</em></div>
        <label>Project<select name="project" id="project-select" required disabled><option value="">Loading projects…</option></select>
          <small id="project-note">The project decides which program the upgrade targets. Availability comes from the project’s hosted capability record.</small></label>
        <div id="project-readiness" aria-live="polite">${capabilityLoadingHTML()}</div>
        <label class="file-drop"><input name="candidate" type="file" accept=".so,application/octet-stream" required><span><b>Candidate build</b><em>Drop the compiled .so or choose a file</em></span><strong>Choose file</strong></label>
        <div id="change-preview" aria-live="polite"></div>
        <label>Name <small class="inline-optional">optional</small><input name="label" maxlength="120" placeholder="e.g. v2.1 release candidate" autocomplete="off"></label>
        <label class="file-drop file-drop--optional"><input name="expectations" type="file" accept=".toml,text/plain"><span><b>Expected changes</b><em>Optional .toml declaration</em></span><strong>Choose file</strong></label>
        <div class="form-status" role="status" aria-live="polite"></div>
        <button class="button button--primary" type="submit" disabled>Analyse change <span>↗</span></button>
        <p class="form-note">After the server accepts the inputs, a run is queued. Its report page follows progress and can be reopened from project history.</p>
      </form>
    </section>
  </main>${Footer()}`;
}

/**
 * What will be analysed, before it is submitted.
 *
 * The fingerprint is computed in this browser from the chosen file. The change
 * ID is not: the server derives it from the project's pinned bundle, and the
 * run page shows the one it actually recorded.
 */
export function ProposedChangePreview({ project, candidate } = {}) {
  if (!candidate) return '';
  return `<div class="change-preview">
    <span class="change-preview__kind">Program upgrade</span>
    <dl>
      <div><dt>Target</dt><dd>${project ? `${escapeHtml(project.name)} · <span class="mono" title="${escapeHtml(project.program_id)}">${escapeHtml(middle(project.program_id))}</span>` : '<em>Choose a project</em>'}</dd></div>
      <div><dt>Candidate</dt><dd><span class="mono" title="${escapeHtml(candidate.sha256 ?? '')}">${candidate.sha256 ? escapeHtml(fingerprint(candidate.sha256)) : 'fingerprinting…'}</span> · ${escapeHtml(candidate.name)} · ${escapeHtml(bytes(candidate.size) ?? '')}</dd></div>
    </dl>
  </div>`;
}

/** SHA-256 of a file, as lowercase hex — the same fingerprint the server records. */
export async function sha256Hex(file) {
  const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer());
  return [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Did the server accept the change this page prepared? The candidate and the
 * target are compared; a disagreement is reported rather than followed.
 */
export function acceptedMismatch(accepted, { project, candidate }) {
  const change = accepted?.change;
  if (!change) return 'The server accepted the upload but named no change.';
  if (candidate?.sha256 && change.candidate_sha256 !== candidate.sha256) {
    return `The server recorded candidate ${fingerprint(change.candidate_sha256)}, not the ${fingerprint(candidate.sha256)} chosen here.`;
  }
  if (project && change.target_program_id !== project.program_id) {
    return `The server recorded a change to ${middle(change.target_program_id)}, not to ${middle(project.program_id)}.`;
  }
  return null;
}

export function attachAnalyse(navigate) {
  const form = document.querySelector('#analyse-form');
  if (!form) return;
  const status = form.querySelector('.form-status');
  const select = form.querySelector('#project-select');
  const note = form.querySelector('#project-note');
  const readiness = form.querySelector('#project-readiness');
  const submit = form.querySelector('button[type="submit"]');
  const preview = form.querySelector('#change-preview');
  let projects = [];
  const capabilities = new Map();
  const capabilityErrors = new Map();
  let candidate = null;

  const selected = () => projects.find(project => project.project_id === select.value);
  const selectedCapability = () => capabilityFor(capabilities.get(select.value), 'program_upgrade');
  const repaintReadiness = () => {
    const project = selected();
    if (!project) {
      readiness.innerHTML = capabilityFailureHTML('Choose a project to check program-upgrade availability.');
      submit.disabled = true;
      return;
    }
    const error = capabilityErrors.get(project.project_id);
    const capability = selectedCapability();
    readiness.innerHTML = error
      ? capabilityFailureHTML('Eplyx could not determine project analysis availability.')
      : capabilityHTML(capability, { label: 'Program upgrade' });
    submit.disabled = !canSubmitCapability(capability);
  };
  const repaint = () => {
    preview.innerHTML = ProposedChangePreview({ project: selected(), candidate });
    repaintReadiness();
  };

  document.querySelectorAll('.file-drop input').forEach(input => input.addEventListener('change', () => {
    const em = input.closest('.file-drop')?.querySelector('em');
    if (input.files?.[0] && em) em.textContent = input.files[0].name;
  }));

  form.querySelector('input[name="candidate"]')?.addEventListener('change', async event => {
    const file = event.target.files?.[0];
    if (!file) { candidate = null; return repaint(); }
    const current = { name: file.name, size: file.size, sha256: null };
    candidate = current;
    repaint();
    try {
      const sha = await sha256Hex(file);
      // A later choice replaced this one while it was hashing.
      if (candidate === current) { current.sha256 = sha; repaint(); }
    } catch { /* The server still fingerprints it; the preview just cannot. */ }
  });
  select.addEventListener('change', repaint);

  if (!isConnected()) {
    select.innerHTML = '<option value="">Connect the console first</option>';
    note.textContent = 'Open Projects and connect with the operator token.';
    readiness.innerHTML = capabilityFailureHTML('Connect the console to determine project analysis availability.');
    submit.disabled = true;
    return;
  }

  // Projects come from the server. There is no project-ID field, and no API URL
  // field: a person should not have to know either to use the product.
  api('/v1/projects').then(async ({ projects: listed }) => {
    projects = listed;
    if (!projects.length) {
      select.innerHTML = '<option value="">No projects yet</option>';
      note.textContent = 'Create a project first, then upload and activate a bundle.';
      readiness.innerHTML = capabilityFailureHTML('No hosted project is available for this analysis.');
      submit.disabled = true;
      return;
    }
    select.innerHTML = projects
      .map(project => `<option value="${escapeHtml(project.project_id)}">${escapeHtml(`${project.name} · ${short(project.program_id, 12)}`)}</option>`)
      .join('');
    await Promise.all(projects.map(async project => {
      try {
        capabilities.set(project.project_id, await fetchProjectCapabilities(project.project_id, api));
      } catch (error) {
        capabilityErrors.set(project.project_id, error);
      }
    }));
    const first = projects.find(project => canSubmitCapability(capabilityFor(capabilities.get(project.project_id), 'program_upgrade'))) ?? projects[0];
    select.value = first.project_id;
    // A convenience only. The server decides what this project may do; the
    // remembered choice is never the source of truth.
    try {
      const last = localStorage.getItem('eplyx-last-project');
      if (last && projects.some(project => project.project_id === last)) select.value = last;
    } catch { /* A forgotten preference is not a failure. */ }
    select.disabled = false;
    repaint();
  }).catch(error => {
    select.innerHTML = '<option value="">Could not load projects</option>';
    note.textContent = error instanceof ApiError && error.status === 401
      ? 'The operator token was not accepted. Reconnect on the Projects page.'
      : error.message;
    readiness.innerHTML = capabilityFailureHTML();
    submit.disabled = true;
  });

  form.addEventListener('submit', async event => {
    event.preventDefault();
    const values = new FormData(form);
    const projectId = String(values.get('project'));
    if (!projectId) return refuse('Choose a project first.');
    if (!canSubmitCapability(selectedCapability())) {
      return refuse('This project is not currently ready for a hosted program-upgrade analysis.');
    }

    const body = new FormData();
    body.append('candidate', values.get('candidate'));
    const label = String(values.get('label') ?? '').trim();
    if (label) body.append('label', label);
    const expected = values.get('expectations');
    if (expected instanceof File && expected.size) body.append('expected_changes', expected);

    const prepared = { project: selected(), candidate };
    submit.disabled = true;
    submit.innerHTML = 'Submitting <span class="spinner"></span>';
    status.textContent = 'Uploading the candidate…';
    status.className = 'form-status is-visible';
    try {
      try { localStorage.setItem('eplyx-last-project', projectId); } catch { /* optional */ }
      // The server answers before the analysis starts, so this waits only for
      // the upload and the change it recorded.
      const result = await api(`/v1/projects/${encodeURIComponent(projectId)}/checks`, { method: 'POST', body });
      const mismatch = acceptedMismatch(result, prepared);
      if (mismatch) {
        return refuse(`${mismatch} The run exists as ${result.run_id}; do not rely on it.`);
      }
      navigate(`/runs/${result.run_id}`);
    } catch (error) {
      refuse(describe(error));
    }
  });

  function describe(error) {
    if (!(error instanceof ApiError)) return error.message;
    if (error.status === 0) return 'Could not reach Eplyx. Check the API endpoint and its browser CORS configuration.';
    if (error.status === 401) return 'Authentication failed. Reconnect the console on the Projects page.';
    return error.message;
  }

  function refuse(message) {
    status.textContent = message;
    status.className = 'form-status is-visible is-error';
    submit.disabled = !canSubmitCapability(selectedCapability());
    submit.innerHTML = 'Analyse change <span>↗</span>';
  }
}
