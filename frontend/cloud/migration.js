// Guided entry for one prepared Token Migration V1 shape. The engine and the
// existing hosted multipart route remain authoritative for every conclusion.
import { PROJECT } from './env.js';
import { esc, panel, kv, ident, words } from './ui.js';
import {
  capabilityFor, canSubmitCapability, capabilityHTML, capabilityFailureHTML, fetchProjectCapabilities,
} from '/assets/capabilities.js';
import {
  LEGACY_TOKEN_PROGRAM, TOKEN_2022_PROGRAM, canEnterPreparedMigration,
  validateMigrationDraft, buildMigrationChangeSpec, proposalDocument, digestFile,
  unixToUtc, submitPreparedMigration, describeMigrationSubmissionError,
} from './migration-model.js';

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
const select = (label, name, options, note = '') => `<label class="analysis-field"><span>${esc(label)}</span><select name="${name}">${options.map(([value, text, selected]) => `<option value="${esc(value)}"${selected ? ' selected' : ''}>${esc(text)}</option>`).join('')}</select>${note ? `<small>${esc(note)}</small>` : ''}</label>`;
const file = (label, name, accept, note) => `<label class="analysis-field migration-file"><span>${esc(label)}</span><input name="${name}" type="file" accept="${esc(accept)}" required><small>${esc(note)}</small></label>`;

function formHTML(ready) {
  return `<form class="analysis-form migration-form" data-migration-form novalidate>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Token identities</legend>
      <div class="migration-grid">
        ${input('Source token', 'source_mint', '', 'required maxlength="44" autocomplete="off"', 'Canonical Solana mint address.')}
        ${select('Source token program', 'source_token_program', [[LEGACY_TOKEN_PROGRAM, 'SPL Token', true], [TOKEN_2022_PROGRAM, 'Token-2022']])}
        ${input('Source decimals', 'source_decimals', '6', 'required inputmode="numeric"')}
        ${input('Replacement token', 'destination_mint', '', 'required maxlength="44" autocomplete="off"', 'Canonical Solana mint address.')}
        ${select('Replacement token program', 'destination_token_program', [[TOKEN_2022_PROGRAM, 'Token-2022', true], [LEGACY_TOKEN_PROGRAM, 'SPL Token']])}
        ${input('Replacement decimals', 'destination_decimals', '9', 'required inputmode="numeric"')}
      </div>
    </fieldset>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Exchange terms</legend>
      <p class="muted">The ratio applies to whole-token UI units and is retained as exact canonical integer strings.</p>
      <div class="migration-grid">
        ${input('Exchange ratio numerator', 'numerator', '1', 'required inputmode="numeric"')}
        ${input('Exchange ratio denominator', 'denominator', '1', 'required inputmode="numeric"')}
        ${select('Rounding behavior', 'rounding', [['floor', 'Floor', true], ['ceiling', 'Ceiling']])}
        ${select('Fee', 'fee_kind', [['none', 'No migration fee', true], ['source_bps', 'Source fee (basis points)']])}
        <div data-fee-bps hidden>${input('Source fee (basis points)', 'fee_bps', '0', 'inputmode="numeric"')}</div>
        ${input('Minimum replacement output (raw)', 'minimum_output_raw', '1', 'required inputmode="numeric"')}
        ${input('Proposed reserve (raw)', 'reserve_raw', '1000000', 'required inputmode="numeric"', 'Declared funding amount; not evidence that a reserve exists.')}
      </div>
    </fieldset>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Migration window</legend>
      <p class="muted">These fields are explicitly UTC. Effective time is inclusive; the deadline is exclusive.</p>
      <div class="migration-grid">
        ${input('Effective date/time (UTC)', 'activation_utc', '', 'type="datetime-local" step="1" required')}
        ${input('Migration deadline (UTC)', 'deadline_utc', '', 'type="datetime-local" step="1" required')}
      </div>
    </fieldset>
    <fieldset${ready ? '' : ' disabled'}>
      <legend>Mechanism and prepared state</legend>
      <div class="migration-grid">
        ${input('Mechanism program', 'mechanism_program_id', '', 'required maxlength="44" autocomplete="off"', 'Program ID under which the supplied SBF is intended to execute.')}
        ${file('Compiled migration mechanism', 'candidate', '.so,application/octet-stream', 'Exact SBF shared object; its SHA-256 and length become proposal identity.')}
        ${file('Prepared state descriptor', 'state_input', '.json,application/json', 'Existing migration state.json. Eplyx validates its schema and configuration.')}
        ${file('Prepared state artifact', 'state_artifact', '.json,application/json', 'The fixture.json or world.json named by the prepared descriptor.')}
      </div>
    </fieldset>
    <aside class="migration-fixed"${ready ? '' : ' aria-disabled="true"'}>
      <strong>Supported browser shape</strong>
      <p>Full source balance · owner authorization · wallet and multisig owners · burn source · proposed reserve transfer · program-derived migration authority · relayer fee payer · block-only gate policy.</p>
      <p class="tech-only"><code>ratio_basis=ui</code> · <code>minimum_source_balance_raw=1</code> · <code>excluded_accounts=[]</code> · <code>authorities.expected={}</code></p>
    </aside>
    <div data-migration-errors class="migration-errors" role="alert" aria-live="polite"></div>
    <section data-migration-preview aria-live="polite"></section>
    <p data-migration-status class="form-error" role="status" aria-live="polite"></p>
    <button class="button button--primary" type="submit" disabled>Submit for analysis</button>
  </form>`;
}

function pageHTML(capability, error) {
  const ready = canEnterPreparedMigration(capability);
  const availability = error
    ? `${capabilityFailureHTML('Eplyx could not determine prepared token migration availability.')}<button type="button" class="button button--ghost" data-capabilities-retry>Retry</button>`
    : capabilityHTML(capability, { label: 'Prepared token migration' });
  return `<div class="page-head"><h1>Prepare a token migration.</h1><p>Enter one supported set of declared migration terms, review the exact proposal, then submit it into the existing hosted rehearsal path.</p></div>
    <p><a href="/p/${esc(PROJECT)}/analyse" data-link>← Choose another analysis</a></p>
    ${panel({ title: 'Analysis availability', body: availability })}
    ${panel({ title: 'Migration proposal', body: `${formHTML(ready)}<p class="note">The preview is proposed input, not a verdict. Eplyx still validates the ChangeSpec, prepared state, mechanism and their relationship before creating a run.</p>` })}`;
}

function draftFrom(form) {
  const value = name => String(form.elements[name]?.value ?? '');
  const chosen = name => {
    const selected = form.elements[name]?.files?.[0];
    return selected?.size ? selected : null;
  };
  return {
    source_mint: value('source_mint'), source_token_program: value('source_token_program'), source_decimals: value('source_decimals'),
    destination_mint: value('destination_mint'), destination_token_program: value('destination_token_program'), destination_decimals: value('destination_decimals'),
    numerator: value('numerator'), denominator: value('denominator'), rounding: value('rounding'), fee_kind: value('fee_kind'), fee_bps: value('fee_bps'),
    minimum_output_raw: value('minimum_output_raw'), reserve_raw: value('reserve_raw'), activation_utc: value('activation_utc'), deadline_utc: value('deadline_utc'),
    mechanism_program_id: value('mechanism_program_id'), candidate: chosen('candidate'), state_input: chosen('state_input'), state_artifact: chosen('state_artifact'),
  };
}

function previewHTML(spec, draft, digests) {
  const fee = spec.change.conversion.fee.kind === 'none' ? 'None' : `${spec.change.conversion.fee.bps} source basis points`;
  const rows = [
    ['Migration', `${ident(spec.change.source.mint, 'addr')} → ${ident(spec.change.destination.mint, 'addr')}`],
    ['Exchange ratio', `${esc(spec.change.conversion.numerator)} / ${esc(spec.change.conversion.denominator)} whole tokens`],
    ['Effective', `${esc(unixToUtc(String(spec.activation.unix_timestamp)))} · Unix ${esc(spec.activation.unix_timestamp)}`],
    ['Deadline', `${esc(unixToUtc(spec.change.deadline.value))} · Unix ${esc(spec.change.deadline.value)}`],
    ['Rounding / fee', `${esc(words(spec.change.conversion.rounding))} · ${esc(fee)}`],
    ['Reserve', `${esc(spec.change.destination_funding.reserve.funded_raw)} raw · proposed`],
    ['Authority', 'Program-derived migration authority · relayer fee payer'],
    ['Optional declarations omitted', 'Expected mint/freeze authorities; excluded accounts; metadata'],
  ];
  return `<div class="migration-preview"><p class="eyebrow">Exact proposal preview</p>${kv(rows)}
    <p><strong>Ready to submit for analysis.</strong> This is not a safety, approval or verification result.</p>
    <div class="tech-only"><h3>Exact submitted ChangeSpec</h3><pre>${esc(proposalDocument(spec))}</pre>
      ${kv([
        ['Mechanism SHA-256', `<code>${esc(digests.candidate)}</code>`],
        ['Prepared state descriptor', `${esc(draft.state_input.name)} · <code>${esc(digests.state_input)}</code>`],
        ['Prepared state artifact', `${esc(draft.state_artifact.name)} · <code>${esc(digests.state_artifact)}</code>`],
        ['Gate policy', '<code>block-only</code>'],
      ])}</div></div>`;
}

function attach(node, capability) {
  const form = node.querySelector('[data-migration-form]');
  if (!form) return;
  const errorsNode = form.querySelector('[data-migration-errors]');
  const previewNode = form.querySelector('[data-migration-preview]');
  const statusNode = form.querySelector('[data-migration-status]');
  const submit = form.querySelector('button[type="submit"]');
  const cache = new WeakMap();
  let generation = 0;
  let prepared = null;
  let readinessStale = false;

  const hash = file => {
    if (!cache.has(file)) cache.set(file, digestFile(file));
    return cache.get(file);
  };
  const showErrors = errors => {
    const messages = [...new Set(Object.values(errors))];
    errorsNode.innerHTML = messages.length ? `<ul>${messages.map(message => `<li>${esc(message)}</li>`).join('')}</ul>` : '';
  };
  async function refresh() {
    const current = ++generation;
    const draft = draftFrom(form);
    const candidate = { sha256: null, len: draft.candidate?.size ?? 0 };
    if (draft.candidate) {
      try { candidate.sha256 = await hash(draft.candidate); } catch { candidate.sha256 = null; }
    }
    if (current !== generation) return null;
    const errors = validateMigrationDraft(draft, candidate);
    showErrors(errors);
    if (Object.keys(errors).length) {
      prepared = null;
      previewNode.replaceChildren();
      submit.disabled = true;
      return null;
    }
    const [stateInput, stateArtifact] = await Promise.all([hash(draft.state_input), hash(draft.state_artifact)]);
    if (current !== generation) return null;
    const spec = buildMigrationChangeSpec(draft, candidate);
    prepared = { spec, draft, candidate, digests: { candidate: candidate.sha256, state_input: stateInput, state_artifact: stateArtifact } };
    previewNode.innerHTML = previewHTML(spec, draft, prepared.digests);
    submit.disabled = readinessStale || !canSubmitCapability(capability);
    return prepared;
  }

  form.elements.fee_kind.addEventListener('change', () => {
    const enabled = form.elements.fee_kind.value === 'source_bps';
    node.querySelector('[data-fee-bps]').hidden = !enabled;
    form.elements.fee_bps.disabled = !enabled;
  });
  form.addEventListener('input', () => { statusNode.textContent = ''; refresh(); });
  form.addEventListener('change', () => { statusNode.textContent = ''; refresh(); });
  form.addEventListener('submit', async event => {
    event.preventDefault();
    if (!canEnterPreparedMigration(capability) || readinessStale) {
      statusNode.textContent = 'This project is not currently available for prepared token migration submission.';
      return;
    }
    const current = await refresh();
    if (!current) {
      statusNode.textContent = 'Resolve the structural guidance before submitting.';
      return;
    }
    submit.disabled = true;
    submit.textContent = 'Submitting…';
    statusNode.textContent = 'Uploading the exact proposal and prepared inputs…';
    try {
      const result = await submitPreparedMigration({
        projectId: PROJECT,
        spec: current.spec,
        files: { candidate: current.draft.candidate, state_input: current.draft.state_input, state_artifact: current.draft.state_artifact },
        request,
      });
      window.dashboardNavigate(result.route);
    } catch (error) {
      const described = describeMigrationSubmissionError(error);
      readinessStale = described.kind === 'readiness';
      statusNode.textContent = described.message;
      statusNode.dataset.errorKind = described.kind;
      submit.disabled = readinessStale;
      submit.textContent = 'Submit for analysis';
    }
  });
}

export async function preparedMigrationPage() {
  let capabilities;
  let error;
  try { capabilities = await fetchProjectCapabilities(PROJECT, request); } catch (caught) { error = caught; }
  const capability = capabilityFor(capabilities, 'token_migration');
  return {
    title: 'Prepare token migration',
    html: pageHTML(capability, error),
    attach(node) {
      node.querySelector('[data-capabilities-retry]')?.addEventListener('click', () => window.dashboardNavigate(location.pathname));
      attach(node, capability);
    },
  };
}
