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
  if (attestation && attestation.change_spec_id === changeSpecId) {
    const execution = attestation.execution;
    const outcomes = {
      deployed_match: ['ok', 'Analysed candidate was deployed', `Executed at slot ${execution?.slot ?? '—'}. Transaction ${middle(execution?.signature ?? '')}.`],
      deployed_mismatch: ['alert', 'The code deployed by this proposal does not match the candidate Eplyx analysed.', `Executed at slot ${execution?.slot ?? '—'}. Transaction ${middle(execution?.signature ?? '')}.`],
      superseded: ['dated', 'This proposal executed, but the program has since been upgraded again.', `Executed at slot ${execution?.slot ?? '—'}. Transaction ${middle(execution?.signature ?? '')}.`],
      not_executed: ['neutral', 'Proposal matches analysed change', 'Not executed yet.'],
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
    ['Target program', observed.upgrade?.program],
    ['ProgramData', observed.upgrade?.programdata],
    ['Buffer', observed.upgrade?.buffer],
    ['Buffer authority', observed.buffer ? observed.buffer.authority ?? 'none' : null],
    ['Buffer SHA-256', observed.buffer?.artifact?.sha256],
    ['Analysed candidate', binding.expected?.candidate?.sha256],
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
