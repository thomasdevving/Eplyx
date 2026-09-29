// Phase G1: the governance proposal an analysis is bound to.
//
// A projection of one sealed governance binding, never a decision of its own.
// The engine read the Squads accounts and the loader buffer and wrote the
// outcome; this module decodes nothing from Solana and chooses only wording.
//
// Two rules shape it. A match is a statement *at a slot*: it is never shown
// without that slot and how long ago it was taken, and an old one is marked
// as old rather than left green. And a changed buffer leads, in the words the
// product commits to: "The proposal's buffer no longer matches the candidate
// Eplyx analysed."

import { fingerprint, middle } from './change.js';

const escapeHtml = value =>
  String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[c]));

/** Older than this, a match is shown as a past observation that needs repeating. */
export const FRESH_SECONDS = 15 * 60;

export const STALE_BUFFER = "The proposal's buffer no longer matches the candidate Eplyx analysed.";

const STATUS = {
  draft: 'Draft',
  active: 'Active',
  approved: 'Approved',
  rejected: 'Rejected',
  executing: 'Executing',
  executed: 'Executed',
  cancelled: 'Cancelled',
};
const TERMINAL = new Set(['rejected', 'executed', 'cancelled']);

const HEADLINES = {
  matched: { tone: 'ok', title: 'Matches analysed change' },
  stale_artifact: { tone: 'alert', title: STALE_BUFFER },
  authority_mismatch: { tone: 'alert', title: 'Not bound: an authority is not the Squads vault.' },
  different_proposal: { tone: 'alert', title: 'This proposal is not the analysed change.' },
  unsupported_proposal: { tone: 'neutral', title: 'Eplyx cannot bind this proposal.' },
  unverifiable: { tone: 'neutral', title: 'The proposal could not be verified.' },
};

export function age(seconds) {
  if (typeof seconds !== 'number' || !Number.isFinite(seconds) || seconds < 0) return null;
  if (seconds < 90) return 'just now';
  if (seconds < 90 * 60) return `${Math.round(seconds / 60)} minutes ago`;
  if (seconds < 36 * 3600) return `${Math.round(seconds / 3600)} hours ago`;
  return `${Math.round(seconds / 86400)} days ago`;
}

/** The Squads proposal a spec names, before anything was checked. */
export function deliveryOf(spec) {
  const delivery = spec?.change?.delivery;
  return delivery?.provider === 'squads_v4' ? delivery : null;
}

/**
 * The governance view for one analysed change.
 *
 * `delivery` is the proposal the run's spec is bound to; `check` is the
 * newest recorded check of that change (`{ checked_at_unix_seconds, binding }`)
 * or null; `changeSpecId` is the run's own change. Returns null when the run
 * names no proposal at all: there is nothing to say, and nothing is invented.
 */
export function governanceView({ delivery, check = null, attestation = null, changeSpecId = null, now = Date.now() / 1000, error = null } = {}) {
  if (!delivery) return null;
  const proposal = {
    label: `Squads #${delivery.transaction_index}`,
    multisig: delivery.multisig,
    vaultIndex: delivery.vault_index,
    transaction: delivery.transaction,
    proposal: delivery.proposal,
    message: delivery.message_sha256,
  };
  if (error) {
    return { proposal, state: 'evidence_error', tone: 'alert', title: 'Stored governance evidence could not be verified.', note: error, facts: [], rows: [] };
  }
  const binding = check?.binding;
  if (!binding) {
    return {
      proposal,
      state: 'unchecked',
      tone: 'neutral',
      title: 'Not verified against the chain yet.',
      note: 'This analysis names the proposal, but no one has checked that its buffer holds the analysed candidate. Run `eplyx governance squads verify` or the verify API before approving.',
      facts: [],
      rows: [],
    };
  }
  // A check about some other change is not a check about this one. It is
  // about this change if it was asked about it, or bound to it.
  if (changeSpecId && binding.analysed_change_spec_id !== changeSpecId && binding.bound_change_spec_id !== changeSpecId) {
    return {
      proposal,
      state: 'identity_mismatch',
      tone: 'alert',
      title: 'The recorded check is about a different change.',
      note: `It was asked about ${binding.analysed_change_spec_id} and binds ${binding.bound_change_spec_id ?? 'no decodable change'}, neither of which is ${changeSpecId}.`,
      facts: [],
      rows: technicalRows(binding),
    };
  }
  const slot = binding.observation?.slot;
  const outcome = binding.outcome;
  const heading = HEADLINES[outcome] ?? { tone: 'neutral', title: 'Unrecognised governance result.' };
  const seconds = typeof check.checked_at_unix_seconds === 'number' ? now - check.checked_at_unix_seconds : null;
  const old = seconds == null || seconds > FRESH_SECONDS;
  // Never green without its slot; an old match is a past observation.
  let tone = heading.tone;
  let state = outcome;
  if (outcome === 'matched' && (typeof slot !== 'number' || old)) {
    tone = 'dated';
    state = 'matched_dated';
  }
  const status = binding.observation?.proposal?.status ?? null;
  const reason = binding.reasons?.find(r => r.outcome === outcome) ?? null;
  const buffer = binding.observation?.buffer?.artifact ?? null;
  const facts = [
    { label: 'Proposal', value: proposal.label, detail: middle(proposal.multisig) },
    {
      label: outcome === 'matched' ? 'Buffer checked' : 'Checked',
      value: typeof slot === 'number' ? `slot ${slot}` : 'no chain read completed',
      detail: [binding.commitment, age(seconds)].filter(Boolean).join(' · '),
    },
    {
      label: 'Proposal status',
      value: status ? `${STATUS[status] ?? status}${TERMINAL.has(status) ? ' (final)' : ''}` : 'not read',
      detail: binding.observation?.proposal?.stale ? 'stale: no further votes' : null,
    },
  ];
  if (outcome === 'matched' && ['draft', 'active', 'approved', 'executing'].includes(status)) {
    facts.push({ label: 'Execution', value: 'Not executed yet', detail: null });
  }
  if (buffer && outcome !== 'matched') {
    facts.push({ label: 'Buffer holds', value: fingerprint(buffer.sha256), detail: `analysed ${fingerprint(binding.expected?.candidate?.sha256)}` });
  }
  let note;
  if (state === 'matched_dated') {
    note = `This match was observed ${age(seconds) ?? 'at an unknown time'}${typeof slot === 'number' ? ` at slot ${slot}` : ''}. The buffer can have changed since; re-verify before approving or executing.`;
  } else if (outcome === 'matched') {
    note = 'A match holds at the slot it was read. The buffer can change again only through another transaction the vault executes; re-verify immediately before approving or executing.';
  } else {
    note = reason?.detail ?? binding.statement;
  }
  const view = {
    proposal,
    state,
    tone,
    title: state === 'matched_dated' ? `Matched at slot ${slot ?? '—'}, ${age(seconds) ?? 'time unknown'}` : heading.title,
    note,
    statement: binding.statement,
    facts,
    reasons: (binding.reasons ?? []).map(r => ({ code: r.code, detail: r.detail })),
    rows: technicalRows(binding),
  };
  if (attestation && attestation.change_spec_id === changeSpecId && attestation.binding_id === binding.binding_id) {
    const execution = attestation.execution;
    const outcomes = {
      deployed_match: ['ok', 'Analysed candidate was deployed', `Executed at slot ${execution?.slot ?? '—'}. Transaction ${middle(execution?.signature ?? '')}.`],
      deployed_mismatch: ['alert', 'The code deployed by this proposal does not match the candidate Eplyx analysed.', `Executed at slot ${execution?.slot ?? '—'}. Transaction ${middle(execution?.signature ?? '')}.`],
      superseded: ['dated', 'This proposal executed, but the program has since been upgraded again.', `Executed at slot ${execution?.slot ?? '—'}. Transaction ${middle(execution?.signature ?? '')}.`],
      not_executed: ['neutral', 'Execution not established at this observation', 'No failed deployment or byte mismatch is claimed.'],
      unsupported: ['neutral', 'Deployment attestation is unsupported for this proposal.', attestation.reasons?.join(' ') ?? ''],
      unverifiable: ['neutral', 'Deployment could not be verified.', attestation.reasons?.join(' ') ?? ''],
    };
    const [tone, title, note] = outcomes[attestation.outcome] ?? outcomes.unverifiable;
    view.tone = tone;
    view.title = title;
    view.note = note;
    view.state = attestation.outcome;
    view.facts.push({ label: 'Execution', value: execution ? `slot ${execution.slot}` : 'not proved', detail: execution?.signature ? middle(execution.signature) : null });
    view.rows.push(['Attestation ID', attestation.attestation_id]);
    view.rows.push(['Deployment outcome', attestation.outcome]);
    if (execution) view.rows.push(['Execution signature', execution.signature]);
    if (attestation.deployed) {
      view.rows.push(['ProgramData deployment slot', attestation.deployed.deploy_slot]);
      view.rows.push(['ProgramData SHA-256', attestation.deployed.account_sha256]);
      view.rows.push(['Zero padding bytes', attestation.deployed.zero_padding_len]);
    }
  }
  return view;
}

function technicalRows(binding) {
  const observed = binding.observation ?? {};
  const delivery = observed.delivery ?? binding.expected?.delivery ?? {};
  const rows = [
    ['Binding ID', binding.binding_id],
    ['Outcome', binding.outcome],
    ['Observed slot', observed.slot],
    ['Commitment', binding.commitment],
    ['Bound change ID', binding.bound_change_spec_id],
    ['Analysed change ID', binding.analysed_change_spec_id],
    ['Squads program', binding.decoder?.squads_program_id],
    ['Decoder', binding.decoder ? `${binding.decoder.decoder} · ${binding.decoder.source_repository} @ ${binding.decoder.source_revision}` : null],
    ['Multisig', delivery.multisig ?? binding.request?.multisig],
    ['Vault', delivery.vault ? `${delivery.vault} (index ${delivery.vault_index})` : null],
    ['Vault transaction', delivery.transaction],
    ['Proposal account', delivery.proposal],
    ['Message SHA-256', delivery.message_sha256],
    ['Expected target program', binding.expected?.target_program_id],
    ['Expected ProgramData', binding.expected?.programdata_address],
    ['Target program', observed.upgrade?.program],
    ['ProgramData', observed.upgrade?.programdata],
    ['Buffer', observed.upgrade?.buffer],
    ['Buffer authority', observed.buffer ? observed.buffer.authority ?? 'none' : null],
    ['Buffer SHA-256', observed.buffer?.artifact?.sha256],
    ['Analysed candidate', binding.expected?.candidate?.sha256],
    ['Candidate length', binding.expected?.candidate?.len],
    ['Upgrade authority', observed.current_program ? observed.current_program.upgrade_authority ?? 'none' : null],
  ];
  for (const account of observed.accounts ?? []) {
    rows.push([`Account · ${account.role}`, account.present ? `${account.address} · ${account.data_len} bytes · ${account.data_sha256}` : `${account.address} · absent`]);
  }
  return rows.filter(([, value]) => value != null);
}

/** The compact section under the change card. Null renders nothing. */
export function GovernanceSection(view) {
  if (!view) return '';
  const facts = view.facts.map(fact => `<div><dt>${escapeHtml(fact.label)}</dt><dd><b>${escapeHtml(fact.value)}</b>${fact.detail ? `<small>${escapeHtml(fact.detail)}</small>` : ''}</dd></div>`).join('');
  const role = view.tone === 'alert' ? ' role="alert"' : '';
  return `<section class="governance-card is-${escapeHtml(view.tone)}" aria-label="Governance proposal"${role}>
    <header><span>Governance proposal</span><em>${escapeHtml(view.proposal.label)}</em></header>
    <h3>${view.state === 'matched' || view.state === 'deployed_match' ? '<span aria-hidden="true">✓</span> ' : ''}${escapeHtml(view.title)}</h3>
    ${facts ? `<dl>${facts}</dl>` : ''}
    <p class="governance-card__note">${escapeHtml(view.note)}</p>
  </section>`;
}

/** Full binding evidence for the technical details. */
export function GovernanceRows(view) {
  if (!view?.rows?.length) return '';
  return view.rows.map(([label, value]) => `<div><span>${escapeHtml(label)}</span><b>${escapeHtml(value)}</b></div>`).join('');
}

const recordedTime = seconds => seconds == null ? 'unavailable (legacy evidence; relative recording order unknown)' : new Date(seconds * 1000).toISOString().replace('T', ' ').replace('.000Z', ' UTC');

/** Factual rows: outcomes remain verbatim; age is presentation only. */
export function trailEventView(event, now = Date.now() / 1000) {
  const g1 = event.type === 'governance_check';
  const proof = g1 ? event.binding : event.attestation;
  const delivery = g1 ? proof.observation?.delivery ?? proof.expected?.delivery : null;
  const outcome = proof.outcome;
  const slot = g1 ? proof.observation?.slot : proof.observed_slot;
  const dated = g1 && outcome === 'matched' && (event.recorded_at_unix_seconds == null || now - event.recorded_at_unix_seconds > FRESH_SECONDS);
  const proposal = delivery?.transaction_index ?? proof.transaction_index ?? proof.request?.transaction_index;
  const candidate = g1 ? proof.expected?.candidate : proof.candidate;
  const target = g1 ? proof.expected?.target_program_id : proof.target_program;
  let note;
  if (g1) {
    note = outcome === 'matched'
      ? 'Proposal binding matched at this observation. This is neither a current match nor execution or deployed-byte proof.'
      : outcome === 'stale_artifact' ? STALE_BUFFER : proof.statement;
  } else {
    note = ({
      not_executed: 'Execution had not been established as of this observation. No failed deployment or byte mismatch is claimed.',
      deployed_match: 'Execution and deployment attribution were established; the candidate bytes and zero padding matched at this observation. This is not a permanent current-state claim.',
      deployed_mismatch: 'Execution and deployment attribution were established; the attributable deployed bytes did not match the candidate.',
      superseded: 'Matching execution was established. Later ProgramData deployment state superseded that execution; its historical evidence is retained.',
      unverifiable: proof.execution
        ? 'Matching execution evidence was found, but deployment attribution or deployed-byte proof could not be established. No mismatch is claimed.'
        : 'Matching execution and deployed-byte proof could not be established. Proposal status alone does not prove execution or deployed bytes.',
      unsupported: 'Deployment attestation is unsupported for this proposal.',
    })[outcome] ?? 'Unrecognised retained outcome.';
  }
  const rows = g1 ? [
    ['Check ID', event.check?.check_id], ['Host recorded time', recordedTime(event.recorded_at_unix_seconds)],
    ['Message-read slot', proof.observation?.message_read_slot], ['Proposal status', proof.observation?.proposal?.status],
    ...technicalRows(proof),
  ] : [
    ['Occurrence ID', event.occurrence?.occurrence_id ?? 'unavailable (legacy)'],
    ['Attestation ID', proof.attestation_id], ['Binding ID used', proof.binding_id],
    ['Bound ChangeSpec ID', proof.change_spec_id], ['Outcome', outcome],
    ['Host recorded time', recordedTime(event.recorded_at_unix_seconds)], ['Observed slot', slot],
    ['Commitment', proof.commitment], ['Multisig', proof.multisig], ['Proposal account', proof.proposal],
    ['Vault transaction', proof.vault_transaction], ['Message SHA-256', proof.message_sha256],
    ['Candidate SHA-256', candidate?.sha256], ['Candidate length', candidate?.len],
    ['Target program', target], ['ProgramData', proof.programdata],
    ...Object.entries(proof.execution ?? {}).map(([k, v]) => [`Execution · ${k}`, typeof v === 'object' ? JSON.stringify(v) : v]),
    ...Object.entries(proof.deployed ?? {}).map(([k, v]) => [`ProgramData · ${k}`, v]),
    ...(proof.reasons ?? []).map(reason => ['Evidence limit', reason]),
  ];
  return { eventId: event.event_id, type: event.type, outcome, dated,
    title: `${g1 ? 'Proposal binding observation' : 'Deployment attestation'}: ${outcome}${dated ? ' · dated observation' : ''}`,
    note, proposal, candidate: candidate?.sha256, target, slot,
    recorded: recordedTime(event.recorded_at_unix_seconds),
    status: g1 ? proof.observation?.proposal?.status : null,
    bindingId: proof.binding_id,
    sourceId: g1 ? proof.analysed_change_spec_id : proof.change_spec_id,
    boundId: g1 ? proof.bound_change_spec_id : proof.change_spec_id,
    execution: g1 ? null : proof.execution,
    deploySlot: g1 ? null : proof.deployed?.deploy_slot,
    tone: ['stale_artifact', 'deployed_mismatch', 'different_proposal', 'authority_mismatch'].includes(outcome) ? 'alert' : dated || outcome === 'superseded' ? 'dated' : 'neutral',
    rows: rows.filter(([, v]) => v != null),
  };
}

export function TrailEvent(event) {
  const v = trailEventView(event);
  return `<li class="governance-card is-${escapeHtml(v.tone)}" data-event-id="${escapeHtml(v.eventId)}">
    <header><span>${escapeHtml(v.type)}</span><em>Squads #${escapeHtml(v.proposal ?? 'unknown')}</em></header>
    <h3>${escapeHtml(v.title)}</h3>
    <p>Recorded ${escapeHtml(v.recorded)} · Observed ${v.slot == null ? 'slot unavailable' : `slot ${escapeHtml(v.slot)}`}</p>
    <p>Candidate ${escapeHtml(middle(v.candidate ?? 'unknown'))} · Target ${escapeHtml(middle(v.target ?? 'unknown'))}${v.status ? ` · Proposal status: ${escapeHtml(v.status)}` : ''}</p>
    <p>${v.type === 'deployment_attestation' ? 'Based on G1 binding' : 'G1 binding'} <code>${escapeHtml(v.bindingId)}</code></p>
    ${v.type === 'governance_check' ? `<p>Asked-about change ${escapeHtml(middle(v.sourceId))} · Derived bound change ${escapeHtml(middle(v.boundId ?? 'unavailable'))}</p>` : ''}
    ${v.execution ? `<p>Matching execution: slot ${escapeHtml(v.execution.slot)} · Transaction ${escapeHtml(middle(v.execution.signature))}</p>` : ''}
    ${v.deploySlot != null ? `<p>ProgramData deployment slot: ${escapeHtml(v.deploySlot)} (separate from the observation slot)</p>` : ''}
    <p class="governance-card__note">${escapeHtml(v.note)}</p>
    <details><summary>Technical evidence</summary><div class="provenance-table">${GovernanceRows(v)}</div></details>
  </li>`;
}

function runLinks(runs, base) {
  return runs.map(run => `<li><a href="${escapeHtml(base)}/${encodeURIComponent(run.run_id)}">${escapeHtml(run.run_id)}</a> · ${escapeHtml(run.status)} · analytical exit code ${escapeHtml(run.exit_code ?? 'unavailable')}<small> · Candidate ${escapeHtml(middle(run.candidate_sha256))} · Bundle ${escapeHtml(middle(run.bundle_sha256))} · Created ${escapeHtml(recordedTime(run.created_at_unix_seconds))}${run.completed_at_unix_seconds == null ? '' : ` · Completed ${escapeHtml(recordedTime(run.completed_at_unix_seconds))}`}</small></li>`).join('');
}

export function GovernanceTrailSection({ delivery, trail, error, changeSpecId, projectId, runBase = '/runs' }) {
  if (!delivery) return '';
  if (error || (trail && trail.change_spec_id !== changeSpecId)) return `<section class="governance-card is-alert" role="alert"><h2>Squads governance trail</h2><p>Stored governance evidence could not be verified.</p></section>`;
  if (!trail) return '<section class="governance-card"><h2>Squads governance trail</h2><p>No retained trail was loaded.</p></section>';
  const endpoint = `/v1/projects/${encodeURIComponent(projectId)}/governance/changes/${encodeURIComponent(changeSpecId)}/trail`;
  return `<section data-governance-trail data-endpoint="${escapeHtml(endpoint)}" data-root-id="${escapeHtml(changeSpecId)}" data-run-base="${escapeHtml(runBase)}">
    <h2>Analysis runs for this bound change</h2>
    <p>Analytical verdicts are separate from proposal binding, execution evidence and deployed-byte proof. Earlier unbound analyses are not analyses of this proposal.</p>
    <ul data-trail-runs>${runLinks(trail.runs ?? [], runBase)}</ul>
    ${trail.runs?.length ? '' : '<p>No linked analysis run was retained for this bound change.</p>'}
    ${trail.runs_next_cursor ? `<button type="button" data-trail-more-runs="${escapeHtml(trail.runs_next_cursor)}">More analysis runs</button>` : ''}
    <h2>Squads governance trail</h2><p>Bound ChangeSpec <code>${escapeHtml(changeSpecId)}</code></p>
    ${trail.source_unbound_change_spec_id ? `<p>Source unbound ChangeSpec <code>${escapeHtml(trail.source_unbound_change_spec_id)}</code></p>` : ''}
    <p>Recorded observations are oldest first. Legacy evidence follows with recording time unavailable; its position does not establish chronology. Viewing this trail performs no chain reads.</p>
    <ol class="governance-trail" data-trail-events>${(trail.events ?? []).map(TrailEvent).join('')}</ol>
    ${trail.events?.length ? '' : '<p>No retained governance observations.</p>'}
    ${trail.next_cursor ? `<button type="button" data-trail-more="${escapeHtml(trail.next_cursor)}">More governance observations</button>` : ''}
    <p data-trail-error role="alert"></p>
  </section>`;
}

/** Explicit page reads only. No verify/attest POST and no polling. */
export function attachGovernanceTrail(root, ask) {
  const click = async event => {
    const button = event.target.closest?.('[data-trail-more], [data-trail-more-runs]');
    if (!button || button.disabled) return;
    const section = button.closest('[data-governance-trail]');
    if (!section) return;
    const runs = button.hasAttribute('data-trail-more-runs');
    const attr = runs ? 'data-trail-more-runs' : 'data-trail-more';
    button.disabled = true;
    try {
      const response = await ask(`${section.dataset.endpoint}?${runs ? 'run_cursor' : 'cursor'}=${encodeURIComponent(button.getAttribute(attr))}`);
      if (!response.ok) throw new Error('Stored governance evidence could not be verified. Further history is unavailable.');
      const page = await response.json();
      if (page.change_spec_id !== section.dataset.rootId) throw new Error('Trail identity differs. Further history is unavailable.');
      section.querySelector(runs ? '[data-trail-runs]' : '[data-trail-events]').insertAdjacentHTML('beforeend', runs ? runLinks(page.runs, section.dataset.runBase) : page.events.map(TrailEvent).join(''));
      const cursor = runs ? page.runs_next_cursor : page.next_cursor;
      if (cursor) button.setAttribute(attr, cursor); else button.remove();
      section.querySelector('[data-trail-error]').textContent = '';
    } catch (error) {
      section.querySelector('[data-trail-error]').textContent = error.message;
    } finally { button.disabled = false; }
  };
  root.addEventListener('click', click);
  return () => root.removeEventListener('click', click);
}
