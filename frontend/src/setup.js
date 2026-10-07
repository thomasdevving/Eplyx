// The guided path to a project's first upgrade check, rendered from
// `GET /v1/projects/{id}/setup`. The browser derives no readiness of its own:
// every status, detail and command is the server's, shown once and escaped.

/** @typedef {{label:string,actor:'operator'|'workspace_member'|'repository',command?:string}} SetupAction */
/** @typedef {{id:string,title:string,required:boolean,status:string,detail:string,evidence?:object,actions:SetupAction[]}} SetupStep */
/** @typedef {{schema_version:1,project_id:string,kind:string,ready_for_first_check:boolean,first_check_complete:boolean,next_step:string|null,steps:SetupStep[],repository:object}} ProjectSetup */

const STATUSES = new Set(['done', 'attention', 'in_progress', 'todo', 'blocked', 'optional']);
const ACTORS = new Set(['operator', 'workspace_member', 'repository']);

const escapeHTML = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[character]));

/**
 * Validate the versioned payload once, without changing its meaning.
 * @param {unknown} value
 * @param {string} expectedProjectId
 * @returns {ProjectSetup}
 */
export function parseProjectSetup(value, expectedProjectId = '') {
  const invalid = () => { throw new Error('Eplyx returned an invalid project setup response.'); };
  const setup = /** @type {any} */ (value);
  if (!setup || setup.schema_version !== 1 || typeof setup.project_id !== 'string'
      || (expectedProjectId && setup.project_id !== expectedProjectId)
      || typeof setup.ready_for_first_check !== 'boolean'
      || typeof setup.first_check_complete !== 'boolean'
      || !(setup.next_step === null || typeof setup.next_step === 'string')
      || !Array.isArray(setup.steps)) invalid();
  const seen = new Set();
  for (const step of setup.steps) {
    if (!step || typeof step.id !== 'string' || seen.has(step.id) || typeof step.title !== 'string'
        || typeof step.required !== 'boolean' || !STATUSES.has(step.status)
        || typeof step.detail !== 'string' || !Array.isArray(step.actions)) invalid();
    seen.add(step.id);
    for (const action of step.actions) {
      if (!action || typeof action.label !== 'string' || !ACTORS.has(action.actor)
          || !(action.command === undefined || typeof action.command === 'string')) invalid();
    }
  }
  if (setup.next_step !== null && !seen.has(setup.next_step)) invalid();
  return /** @type {ProjectSetup} */ (setup);
}

/**
 * @param {string} projectId
 * @param {(path:string) => Promise<unknown>} request
 * @returns {Promise<ProjectSetup>}
 */
export async function fetchProjectSetup(projectId, request) {
  return parseProjectSetup(await request(`/v1/projects/${encodeURIComponent(projectId)}/setup`), projectId);
}

const STATUS_LABEL = {
  done: 'Done',
  attention: 'Done — note',
  in_progress: 'In progress',
  todo: 'To do',
  blocked: 'Waiting',
  optional: 'Optional',
};

const ACTOR_LABEL = {
  operator: 'Service operator',
  workspace_member: 'Workspace member',
  repository: 'Repository',
};

/** @param {ProjectSetup} setup */
export function setupHeadline(setup) {
  if (setup.first_check_complete) return 'The first upgrade check has reached a verdict.';
  if (setup.ready_for_first_check) return 'Everything a first upgrade check needs is in place.';
  const next = setup.steps.find(step => step.id === setup.next_step);
  return next ? `Next: ${next.title.toLowerCase()}.` : 'Some prerequisites are missing.';
}

/** @param {ProjectSetup} setup */
export function projectSetupHTML(setup) {
  const steps = setup.steps.map((step, index) => {
    const actions = step.actions.map(action => `<li>
        <span>${escapeHTML(action.label)}</span>
        <small>${escapeHTML(ACTOR_LABEL[action.actor] ?? action.actor)}</small>
        ${action.command ? `<code>${escapeHTML(action.command)}</code>` : ''}
      </li>`).join('');
    const isNext = step.id === setup.next_step;
    return `<li class="setup-step setup-step--${escapeHTML(step.status)}${isNext ? ' is-next' : ''}" data-setup-step="${escapeHTML(step.id)}" data-setup-status="${escapeHTML(step.status)}">
      <span class="setup-step__index" aria-hidden="true">${index + 1}</span>
      <div>
        <header><strong>${escapeHTML(step.title)}</strong><span>${escapeHTML(STATUS_LABEL[step.status] ?? step.status)}${step.required ? '' : ' · not required'}</span></header>
        <p>${escapeHTML(step.detail)}</p>
        ${actions ? `<ul class="setup-actions">${actions}</ul>` : ''}
        <code class="tech-only">${escapeHTML(step.id)}</code>
      </div>
    </li>`;
  }).join('');
  return `<div class="setup" data-ready="${setup.ready_for_first_check}" data-complete="${setup.first_check_complete}">
    <p class="setup__headline">${escapeHTML(setupHeadline(setup))}</p>
    <ol class="setup-steps">${steps}</ol>
  </div>`;
}
