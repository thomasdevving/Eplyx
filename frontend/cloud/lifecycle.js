// Guided entry for one prepared lifecycle declaration. The engine and the
// existing hosted multipart route remain authoritative for every conclusion.
import { PROJECT } from './env.js';
import { esc, panel, kv, ident, words } from './ui.js';
import {
  capabilityFor, canSubmitCapability, capabilityHTML, capabilityFailureHTML, fetchProjectCapabilities,
} from '/assets/capabilities.js';
import {
  canEnterPreparedLifecycle, validateLifecycleDraft, buildLifecycleDocuments,
  digestLifecycleFile, submitPreparedLifecycle, lifecycleSubmissionError,
} from './lifecycle-model.js';

class RequestError extends Error {
  constructor(status, body) {
    super(body?.error || `Request failed (${status})`);
    this.status = status;
    this.body = body;
  }
}

async function request(path, options = {}) {
  let response;
  try {
    response = await fetch(path, { credentials: 'same-origin', ...options });
  } catch {
    throw new RequestError(0, { error: 'Could not reach Eplyx.' });
  }
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new RequestError(response.status, body);
  return body;
}

const input = (label, name, value = '', extra = '', note = '') => `<label class="analysis-field"><span>${esc(label)}</span><input name="${name}" value="${esc(value)}" ${extra}>${note ? `<small>${esc(note)}</small>` : ''}</label>`;
const select = (label, name, options, note = '') => `<label class="analysis-field"><span>${esc(label)}</span><select name="${name}">${options.map(([value, text]) => `<option value="${esc(value)}">${esc(text)}</option>`).join('')}</select>${note ? `<small>${esc(note)}</small>` : ''}</label>`;

function formHTML(ready) {
  return `<form class="analysis-form lifecycle-form" data-lifecycle-form novalidate>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Declared policy</legend>
      <p class="muted">State only the hypothetical policy terms you want Eplyx to evaluate. This declaration is not issuer verification.</p>
      <div class="lifecycle-grid">
        ${input('Scenario ID', 'scenario_id', '', 'required maxlength="120" autocomplete="off"', 'A stable label for this proposal.')}
        ${input('Declaration or source reference', 'reference', '', 'required autocomplete="off"', 'For example, an internal proposal ID or source URL.')}
        <label class="analysis-field lifecycle-wide"><span>Policy change description</span><textarea name="description" required maxlength="1000"></textarea><small>Free-text provenance; it does not alter the proposal identity.</small></label>
        ${input('Current asset mint', 'asset_mint', '', 'required maxlength="44" autocomplete="off"', 'Canonical Solana mint address.')}
        ${input('Policy effective date/time (UTC)', 'effective_at', '', 'type="datetime-local" step="1" required')}
        ${input('Declaration captured date/time (UTC)', 'captured_at', '', 'type="datetime-local" step="1" required', 'Used as the explicit scenario and source capture time.')}
      </div>
      <div class="lifecycle-transition"><code>Active</code><span aria-hidden="true">→</span><code>TransitionRequired</code></div>
    </fieldset>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Optional declared terms</legend>
      <label class="lifecycle-switch"><input name="has_successor" type="checkbox"> Declare a successor asset</label>
      <div class="lifecycle-grid" data-successor-fields hidden>
        ${input('Successor asset mint', 'successor_mint', '', 'maxlength="44" autocomplete="off"')}
        ${input('Successor description', 'successor_description', '', 'maxlength="500"')}
      </div>
      <label class="lifecycle-switch"><input name="has_deadline" type="checkbox"> Declare a post-effective deadline</label>
      <div class="lifecycle-grid" data-deadline-fields hidden>
        ${input('Deadline date/time (UTC)', 'deadline_at', '', 'type="datetime-local" step="1"')}
        ${select('Status after deadline', 'post_deadline_status', [
          ['PostDeadlineTransitionRequired', 'Post-deadline transition required'],
          ['Expired', 'Expired'],
          ['NoIssuerEntitlement', 'No issuer entitlement'],
          ['Unknown', 'Unknown'],
        ], 'A declared policy status, not a browser-derived conclusion.')}
      </div>
    </fieldset>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Immutable observed state</legend>
      <label class="analysis-field lifecycle-file"><span>Lifecycle snapshot</span><input name="snapshot" type="file" accept=".json,application/json" required><small>The existing snapshot JSON is uploaded byte-for-byte; the browser does not rewrite it.</small></label>
    </fieldset>
    <aside class="lifecycle-fixed"${ready ? '' : ' aria-disabled="true"'}>
      <strong>Supported browser shape</strong>
      <p>Scenario assumption source · <code>browser-prepared/v1</code> · eligibility unknown · no conversion ratio · no retained execution or readiness checks.</p>
      <p>The comparison is explicit: one second before the effective time versus the effective time. Optional deadline and successor terms are included only when selected.</p>
    </aside>
    <div data-lifecycle-errors class="lifecycle-errors" role="alert" aria-live="polite"></div>
    <section data-lifecycle-preview aria-live="polite"></section>
    <p data-lifecycle-status class="form-error" role="status" aria-live="polite"></p>
    <button class="button button--primary" type="submit" disabled>Submit for analysis</button>
  </form>`;
}

function pageHTML(capability, error) {
  const ready = canEnterPreparedLifecycle(capability);
  const availability = error
    ? `${capabilityFailureHTML('Eplyx could not determine prepared lifecycle availability.')}<button type="button" class="button button--ghost" data-capabilities-retry>Retry</button>`
    : capabilityHTML(capability, { label: 'Prepared lifecycle change' });
  return `<div class="page-head"><h1>Prepare a lifecycle change.</h1><p>Declare one supported hypothetical policy transition, bind it to an immutable snapshot, review the exact inputs, then submit them into the existing hosted analysis path.</p></div>
    <p><a href="/p/${esc(PROJECT)}/analyse" data-link>← Choose another analysis</a></p>
    ${panel({ title: 'Analysis availability', body: availability })}
    ${panel({ title: 'Lifecycle proposal', body: `${formHTML(ready)}<p class="note">The preview is proposed input, not a verdict. Eplyx still validates the ChangeSpec, scenario, snapshot and their relationship before creating a run.</p>` })}`;
}

function draftFrom(form) {
  const value = name => String(form.elements[name]?.value ?? '');
  const selected = form.elements.snapshot?.files?.[0];
  return {
    scenario_id: value('scenario_id'),
    reference: value('reference'),
    description: value('description'),
    asset_mint: value('asset_mint'),
    effective_at: value('effective_at'),
    captured_at: value('captured_at'),
    has_successor: form.elements.has_successor.checked,
    successor_mint: value('successor_mint'),
    successor_description: value('successor_description'),
    has_deadline: form.elements.has_deadline.checked,
    deadline_at: value('deadline_at'),
    post_deadline_status: value('post_deadline_status'),
    snapshot: selected?.size ? selected : null,
  };
}

function previewHTML(documents, draft, snapshotSha256) {
  const { scenario, changeSpec, analysisOptions } = documents;
  const successor = scenario.policy.successor
    ? ident(scenario.policy.successor.mint, 'addr')
    : 'Not declared';
  const deadline = scenario.policy.deadline
    ? `${esc(scenario.policy.deadline.at)} · ${esc(words(scenario.policy.deadline.after))}`
    : 'Not declared';
  return `<div class="lifecycle-preview">
    <p class="eyebrow">Exact input preview</p>
    ${kv([
      ['Asset', ident(scenario.policy.asset_mint, 'addr')],
      ['Policy transition', `<code>${esc(scenario.policy.before)}</code> → <code>${esc(scenario.policy.after)}</code>`],
      ['Effective', `${esc(scenario.policy.effective_at)} · Unix ${esc(changeSpec.activation.unix_timestamp)}`],
      ['Successor', successor],
      ['Deadline', deadline],
      ['Evaluation times', `${esc(analysisOptions.before)} → ${esc(analysisOptions.at)}`],
      ['Retained execution / readiness checks', 'None for this lifecycle consequence analysis'],
    ])}
    <div class="lifecycle-boundaries">
      <article><strong>Declared</strong><p>Policy terms and provenance entered above. They remain hypothetical assertions.</p></article>
      <article><strong>Observed / prepared</strong><p>${esc(draft.snapshot.name)} is preserved as immutable input bytes with SHA-256 <code>${esc(snapshotSha256)}</code>.</p></article>
      <article><strong>Derived by Eplyx</strong><p>No result yet. Consequences appear only after server validation and analysis.</p></article>
    </div>
    <p><strong>Ready to submit for analysis.</strong> This preview makes no approval, issuer-authenticity or consequence claim.</p>
    <div class="tech-only">
      <h3>Exact submitted ChangeSpec</h3><pre>${esc(documents.changeSpecDocument)}</pre>
      <h3>Exact submitted lifecycle scenario</h3><pre>${esc(documents.scenarioDocument)}</pre>
      <h3>Exact submitted analysis options</h3><pre>${esc(documents.analysisOptionsDocument)}</pre>
      ${kv([
        ['Uploaded snapshot', esc(draft.snapshot.name)],
        ['Uploaded snapshot bytes SHA-256', `<code>${esc(snapshotSha256)}</code>`],
      ])}
    </div>
  </div>`;
}

function attach(node, capability) {
  const form = node.querySelector('[data-lifecycle-form]');
  if (!form) return;
  const errorsNode = form.querySelector('[data-lifecycle-errors]');
  const previewNode = form.querySelector('[data-lifecycle-preview]');
  const statusNode = form.querySelector('[data-lifecycle-status]');
  const submit = form.querySelector('button[type="submit"]');
  const cache = new WeakMap();
  let generation = 0;
  let prepared = null;
  let readinessStale = false;

  const hash = file => {
    if (!cache.has(file)) cache.set(file, digestLifecycleFile(file));
    return cache.get(file);
  };
  const showErrors = errors => {
    const messages = [...new Set(Object.values(errors))];
    errorsNode.innerHTML = messages.length ? `<ul>${messages.map(message => `<li>${esc(message)}</li>`).join('')}</ul>` : '';
  };
  async function refresh() {
    const current = ++generation;
    const draft = draftFrom(form);
    const errors = validateLifecycleDraft(draft);
    showErrors(errors);
    if (Object.keys(errors).length) {
      prepared = null;
      previewNode.replaceChildren();
      submit.disabled = true;
      return null;
    }
    let snapshotSha256;
    try { snapshotSha256 = await hash(draft.snapshot); } catch { snapshotSha256 = null; }
    if (current !== generation) return null;
    if (!snapshotSha256) {
      showErrors({ snapshot: 'The snapshot could not be read.' });
      prepared = null;
      previewNode.replaceChildren();
      submit.disabled = true;
      return null;
    }
    const documents = buildLifecycleDocuments(draft);
    prepared = { documents, draft, snapshotSha256 };
    previewNode.innerHTML = previewHTML(documents, draft, snapshotSha256);
    submit.disabled = readinessStale || !canSubmitCapability(capability);
    return prepared;
  }

  function syncOptionalFields() {
    const successor = form.elements.has_successor.checked;
    const deadline = form.elements.has_deadline.checked;
    node.querySelector('[data-successor-fields]').hidden = !successor;
    node.querySelector('[data-deadline-fields]').hidden = !deadline;
    form.elements.successor_mint.disabled = !successor;
    form.elements.successor_description.disabled = !successor;
    form.elements.deadline_at.disabled = !deadline;
    form.elements.post_deadline_status.disabled = !deadline;
  }
  form.addEventListener('input', () => { statusNode.textContent = ''; refresh(); });
  form.addEventListener('change', () => { syncOptionalFields(); statusNode.textContent = ''; refresh(); });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (!canEnterPreparedLifecycle(capability) || readinessStale) {
      statusNode.textContent = 'This project is not currently available for prepared lifecycle submission.';
      return;
    }
    const current = await refresh();
    if (!current) {
      statusNode.textContent = 'Resolve the structural guidance before submitting.';
      return;
    }
    submit.disabled = true;
    submit.textContent = 'Submitting…';
    statusNode.textContent = 'Uploading the exact lifecycle inputs…';
    try {
      const result = await submitPreparedLifecycle({ projectId: PROJECT, draft: current.draft, request });
      window.dashboardNavigate(result.route);
    } catch (error) {
      const described = lifecycleSubmissionError(error);
      readinessStale = described.kind === 'stale';
      statusNode.textContent = described.message;
      statusNode.dataset.errorKind = described.kind;
      submit.disabled = readinessStale;
      submit.textContent = 'Submit for analysis';
    }
  });
  syncOptionalFields();
}

export async function preparedLifecyclePage() {
  let capabilities;
  let error;
  try { capabilities = await fetchProjectCapabilities(PROJECT, request); } catch (caught) { error = caught; }
  const capability = capabilityFor(capabilities, 'lifecycle_change');
  return {
    title: 'Prepare lifecycle change',
    html: pageHTML(capability, error),
    attach(node) {
      node.querySelector('[data-capabilities-retry]')?.addEventListener('click', () => window.dashboardNavigate(location.pathname));
      attach(node, capability);
    },
  };
}
