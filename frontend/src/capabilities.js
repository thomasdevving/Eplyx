// Authenticated project analysis availability, copied from the server without
// deriving a second readiness model in the browser.

/** @typedef {{code:string,message:string,action:string}} MissingPrerequisite */
/** @typedef {{kind:string,status:'ready'|'not_ready'|'unsupported',supported:boolean,can_submit:boolean,missing:MissingPrerequisite[]}} AnalysisCapability */
/** @typedef {{schema_version:1,project_id:string,analyses:AnalysisCapability[]}} ProjectCapabilities */

const STATUSES = new Set(['ready', 'not_ready', 'unsupported']);

const escapeHTML = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[character]));

/**
 * Validate the versioned server payload once, without changing its meaning.
 * @param {unknown} value
 * @param {string} expectedProjectId
 * @returns {ProjectCapabilities}
 */
export function parseProjectCapabilities(value, expectedProjectId = '') {
  const invalid = () => { throw new Error('Eplyx returned an invalid project capability response.'); };
  const candidate = /** @type {any} */ (value);
  if (!candidate || candidate.schema_version !== 1 || typeof candidate.project_id !== 'string'
      || (expectedProjectId && candidate.project_id !== expectedProjectId)
      || !Array.isArray(candidate.analyses)) invalid();
  const seen = new Set();
  for (const analysis of candidate.analyses) {
    if (!analysis || typeof analysis.kind !== 'string' || seen.has(analysis.kind)
        || !STATUSES.has(analysis.status) || typeof analysis.supported !== 'boolean'
        || typeof analysis.can_submit !== 'boolean' || !Array.isArray(analysis.missing)) invalid();
    seen.add(analysis.kind);
    if ((analysis.status === 'ready') !== analysis.can_submit
        || (analysis.status === 'unsupported') !== !analysis.supported) invalid();
    for (const reason of analysis.missing) {
      if (!reason || typeof reason.code !== 'string' || typeof reason.message !== 'string'
          || typeof reason.action !== 'string') invalid();
    }
  }
  return /** @type {ProjectCapabilities} */ (candidate);
}

/**
 * Use the caller's existing authenticated request function.
 * @param {string} projectId
 * @param {(path:string) => Promise<unknown>} request
 * @returns {Promise<ProjectCapabilities>}
 */
export async function fetchProjectCapabilities(projectId, request) {
  const path = `/v1/projects/${encodeURIComponent(projectId)}/capabilities`;
  return parseProjectCapabilities(await request(path), projectId);
}

/** @param {ProjectCapabilities|null|undefined} capabilities @param {string} kind @returns {AnalysisCapability|null} */
export const capabilityFor = (capabilities, kind) =>
  capabilities?.analyses?.find(analysis => analysis.kind === kind) ?? null;

// Submission controls read the explicit server fields, never missing.length.
/** @param {AnalysisCapability|null|undefined} capability */
export const canSubmitCapability = capability =>
  capability?.supported === true && capability.status === 'ready' && capability.can_submit === true;

const labels = {
  program_upgrade: 'Program upgrade',
  token_migration: 'Prepared token migration',
  lifecycle_change: 'Prepared lifecycle analysis',
  protocol_parameter_change: 'Token-2022 active fee Parameter Change',
  current_observation: 'Observe current state',
  current_path: 'Check a current path',
  current_candidate: 'Check a candidate migration',
  current_preflight: 'Evaluate a proposed scenario',
  current_stress: 'Run bounded stress',
};

const stateLabel = capability => capability?.status === 'ready' ? 'Ready'
  : capability?.status === 'unsupported' ? 'Unsupported' : 'Not ready';

/**
 * One reusable Overview/Technical presentation over the same capability.
 * @param {AnalysisCapability|null|undefined} capability
 */
export function capabilityHTML(capability, { label = labels[capability?.kind] ?? 'Analysis', compact = false } = {}) {
  if (!capability) return capabilityFailureHTML();
  const ready = canSubmitCapability(capability);
  const reasons = capability.missing.map(reason => `<li><span>${escapeHTML(reason.message)}</span>${reason.action ? `<small>${escapeHTML(reason.action)}</small>` : ''}<code class="tech-only">${escapeHTML(reason.code)}</code></li>`).join('');
  const explanation = ready
    ? '<p>This hosted workflow is available for this project.</p>'
    : reasons
      ? `<ul class="capability-reasons">${reasons}</ul>`
      : capability.status === 'unsupported'
        ? '<p>This deployed server does not support this hosted workflow.</p>'
        : '<p>The server reported that this workflow is not ready.</p>';
  return `<article class="capability capability--${escapeHTML(capability.status)}${compact ? ' capability--compact' : ''}" data-capability-kind="${escapeHTML(capability.kind)}" data-capability-status="${escapeHTML(capability.status)}">
    <header><strong>${escapeHTML(label)}</strong><span>${escapeHTML(stateLabel(capability))}</span></header>
    ${explanation}
    <p class="capability-technical tech-only"><code>${escapeHTML(capability.kind)}</code> · status <code>${escapeHTML(capability.status)}</code> · supported <code>${capability.supported}</code> · can_submit <code>${capability.can_submit}</code></p>
  </article>`;
}

export function capabilityLoadingHTML() {
  return '<div class="capability capability--loading" role="status"><p>Checking project analysis availability…</p></div>';
}

export function capabilityFailureHTML(message = 'Eplyx could not determine project analysis availability.') {
  return `<div class="capability capability--error" role="alert"><p>${escapeHTML(message)}</p><small>Hosted analysis controls remain unavailable. Retry before starting a workflow.</small></div>`;
}

/**
 * Four product groups; current-state sub-actions remain individually visible.
 * @param {ProjectCapabilities} capabilities
 */
export function projectCapabilitiesHTML(capabilities) {
  const one = kind => capabilityHTML(capabilityFor(capabilities, kind), { compact: true });
  return `<div class="capability-groups">
    ${one('program_upgrade')}
    ${one('token_migration')}
    ${one('lifecycle_change')}
    <section class="capability-group"><h3>Current-state analysis</h3><div class="capability-subactions">
      ${['current_observation', 'current_path', 'current_candidate', 'current_preflight', 'current_stress'].map(one).join('')}
    </div></section>
  </div>`;
}
