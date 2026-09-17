import { Header, Footer } from './shell.js';

export function AnalysePage() {
  return `<main id="main" class="inner-page analyse-page">${Header({ light: true })}
    <section class="analyse-shell">
      <div class="analyse-copy">
        <p class="eyebrow"><span></span> Hosted check</p>
        <h1>Analyse a candidate upgrade.</h1>
        <p>Submit a compiled Solana program to an existing Eplyx project. The project’s active bundle supplies the pinned baseline and validated historical corpus.</p>
        <div class="input-model"><span>Your candidate.so</span><b>+</b><span>Optional expected changes</span><i>→</i><strong>Project bundle</strong><i>→</i><em>Replay report</em></div>
        <a href="/runs/demo" data-link class="text-link">View the public demo report <span>↗</span></a>
      </div>
      <form class="analyse-form" id="analyse-form">
        <div class="form-head"><span>New upgrade check</span><em>Uses the real hosted API</em></div>
        <label>API endpoint<input name="api" type="url" placeholder="https://api.eplyx.dev" required><small>The deployed Eplyx server base URL.</small></label>
        <label>Project ID<input name="project" placeholder="stake-pool" required></label>
        <label>Project token<input name="token" type="password" autocomplete="off" placeholder="Bearer token" required><small>Used for this request only. It is not saved.</small></label>
        <label class="file-drop"><input name="candidate" type="file" accept=".so,application/octet-stream" required><span><b>Candidate binary</b><em>Drop candidate.so or choose a file</em></span><strong>Choose file</strong></label>
        <label class="file-drop file-drop--optional"><input name="expectations" type="file" accept=".toml,text/plain"><span><b>Expected changes</b><em>Optional .toml declaration</em></span><strong>Choose file</strong></label>
        <div class="form-status" role="status" aria-live="polite"></div>
        <button class="button button--primary" type="submit">Run analysis <span>↗</span></button>
        <p class="form-note">Candidates run against the project’s server-controlled active bundle. A regression returns HTTP 200 with its Eplyx exit code.</p>
      </form>
    </section>
  </main>${Footer()}`;
}

export function attachAnalyse(navigate) {
  document.querySelectorAll('.file-drop input').forEach(input => input.addEventListener('change', () => {
    const label = input.closest('.file-drop');
    const em = label?.querySelector('em');
    if (input.files?.[0] && em) em.textContent = input.files[0].name;
  }));
  document.querySelector('#analyse-form')?.addEventListener('submit', async event => {
    event.preventDefault();
    const form = event.currentTarget;
    const status = form.querySelector('.form-status');
    const submit = form.querySelector('button[type="submit"]');
    const values = new FormData(form);
    const api = String(values.get('api')).replace(/\/$/, '');
    const project = encodeURIComponent(String(values.get('project')));
    const body = new FormData();
    body.append('candidate', values.get('candidate'));
    const expected = values.get('expectations');
    if (expected instanceof File && expected.size) body.append('expected_changes', expected);
    submit.disabled = true;
    submit.innerHTML = 'Running replay <span class="spinner"></span>';
    status.textContent = 'Uploading the candidate and waiting for deterministic replay…';
    status.className = 'form-status is-visible';
    try {
      const response = await fetch(`${api}/v1/projects/${project}/checks`, { method: 'POST', headers: { Authorization: `Bearer ${values.get('token')}` }, body });
      const result = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(result.error || `Request failed (${response.status})`);
      const reportPath = result.report?.json;
      if (reportPath) {
        const reportResponse = await fetch(`${api}${reportPath}`, { headers: { Authorization: `Bearer ${values.get('token')}` } });
        if (reportResponse.ok) result.canonical_report = await reportResponse.json();
      }
      sessionStorage.setItem(`eplyx-run-${result.run_id}`, JSON.stringify(result));
      navigate(`/runs/${result.run_id}`);
    } catch (error) {
      status.textContent = error instanceof TypeError ? 'Could not reach the API. Check the endpoint and its browser CORS configuration.' : error.message;
      status.className = 'form-status is-visible is-error';
      submit.disabled = false;
      submit.innerHTML = 'Run analysis <span>↗</span>';
    }
  });
}
