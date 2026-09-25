// What a run is an analysis *of*.
//
// Every analysis names one proposed change. The server fixes it when the run is
// accepted, the engine names it again in the report, and this module renders
// what those two said — it never derives a change identity of its own. The
// canonical report's `change` is the authority for what was analysed; the run's
// `change` is the registry's record of what was accepted; if the two disagree,
// the page says so instead of showing a verdict.
//
// Rendering is by kind so a later kind can say what *it* targets without the
// page learning about it anywhere else. Only one kind exists, so only one is
// described: nothing here offers a change Eplyx cannot analyse.

const KINDS = {
  program_upgrade: {
    title: 'Program upgrade',
    targetLabel: 'Target program',
    target: change => change.target_program_id,
    candidateLabel: 'Candidate build',
  },
};

const escapeHtml = value =>
  String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[c]));

/** A readable fingerprint. The full value lives in the technical details. */
export const fingerprint = (hash, length = 8) =>
  typeof hash === 'string' && hash.length > length ? hash.slice(0, length) : hash ?? '—';

export const middle = (value, head = 6, tail = 4) =>
  typeof value === 'string' && value.length > head + tail + 1 ? `${value.slice(0, head)}…${value.slice(-tail)}` : value ?? '—';

export function kindOf(change) {
  return KINDS[change?.kind] ?? {
    title: 'Unrecognised change',
    targetLabel: 'Target',
    target: () => null,
    candidateLabel: 'Candidate',
  };
}

export function bytes(length) {
  if (typeof length !== 'number' || !Number.isFinite(length)) return null;
  if (length < 1024) return `${length} B`;
  if (length < 1024 * 1024) return `${(length / 1024).toFixed(1)} KB`;
  return `${(length / (1024 * 1024)).toFixed(2)} MB`;
}

/**
 * Resolve what a run analysed, and whether its three records agree.
 *
 *   registry (run.change) == stored spec (spec.change_spec_id) == report.change
 *
 * `report.change` wins when present, because it is what the engine evaluated.
 * A run recorded before change identity has no `run.change`; it is legacy, and
 * shows the report's own change when the report carries one rather than an
 * identity computed here.
 */
export function resolveChange(run, report, spec) {
  const registered = run?.change ?? null;
  const reported = report?.change ?? null;
  const stated = spec?.change_spec_id ?? null;
  const conflicts = [];
  if (registered && reported && registered.change_spec_id !== reported.change_spec_id) {
    conflicts.push(`the run was accepted for ${registered.change_spec_id} but its report is about ${reported.change_spec_id}`);
  }
  if (registered && stated && registered.change_spec_id !== stated) {
    conflicts.push(`the run was accepted for ${registered.change_spec_id} but its stored spec is ${stated}`);
  }
  const change = reported ? { ...registered, ...reported } : registered;
  return {
    change,
    legacy: !registered,
    conflicts,
    verified: Boolean(registered && reported && stated && conflicts.length === 0),
  };
}

/**
 * The compact identity card: what is being changed, into what, under which ID.
 * Hashes are short here on purpose; the full values are one section down.
 */
export function ChangeCard({ change, legacy = false, targetName, spec, candidateSha } = {}) {
  if (!change) {
    return `<section class="change-card is-legacy" aria-label="Proposed change">
      <header><span>Proposed change</span><em>Legacy run</em></header>
      <p class="change-card__note">Recorded before Eplyx gave every analysis a change identity, so it has none. Only its candidate fingerprint was kept.</p>
      <dl>${term('Candidate', candidateSha ? `<b class="mono" title="${escapeHtml(candidateSha)}">${escapeHtml(fingerprint(candidateSha))}</b>` : '<b>—</b>')}</dl>
    </section>`;
  }
  const kind = kindOf(change);
  const target = kind.target(change);
  const activation = spec?.activation;
  return `<section class="change-card" aria-label="Proposed change">
    <header><span>Proposed change</span><em>${escapeHtml(kind.title)}</em></header>
    ${change.label ? `<h3>${escapeHtml(change.label)}</h3>` : ''}
    <dl>
      ${term('Target', `<b>${targetName ? `${escapeHtml(targetName)} · ` : ''}<span class="mono" title="${escapeHtml(target ?? '')}">${escapeHtml(middle(target))}</span></b>`)}
      ${term('Candidate', `<b class="mono" title="${escapeHtml(change.candidate_sha256 ?? '')}">${escapeHtml(fingerprint(change.candidate_sha256))}</b>${bytes(change.candidate_len) ? `<small>${escapeHtml(bytes(change.candidate_len))}</small>` : ''}`)}
      ${term('Change ID', `<b class="mono" title="${escapeHtml(change.change_spec_id ?? '')}">${escapeHtml(fingerprint(change.change_spec_id))}</b>`)}
      ${activation ? term('Proposed effective', `<b>${escapeHtml(activationLabel(activation))}</b>`) : ''}
    </dl>
    ${activation ? '<p class="change-card__note">Analysis is counterfactual: the candidate is replayed over validated historical state, whenever the change would take effect.</p>' : ''}
    ${legacy ? '<p class="change-card__note">Legacy run: this identity comes from its report, not from the run record.</p>' : ''}
  </section>`;
}

function term(label, value) {
  return `<div><dt>${escapeHtml(label)}</dt><dd>${value}</dd></div>`;
}

function activationLabel(activation) {
  const parts = [];
  if (activation.slot != null) parts.push(`slot ${activation.slot}`);
  if (activation.unix_timestamp != null) {
    const date = new Date(activation.unix_timestamp * 1000);
    parts.push(Number.isNaN(date.getTime()) ? `unix ${activation.unix_timestamp}` : date.toISOString().replace('.000Z', 'Z'));
  }
  return parts.join(' · ') || '—';
}

/**
 * The complete identity, for the technical details. Everything the spec
 * states appears here in full, so nothing rigorous was lost by making the card
 * short.
 */
export function ChangeIdentityRows({ change, spec, resolution } = {}) {
  if (!change) return row('Change ID', null, 'none — legacy run');
  const upgrade = spec?.change ?? {};
  const rows = [
    row('Change ID', change.change_spec_id),
    row('Kind', change.kind),
    row(kindOf(change).targetLabel, kindOf(change).target(change)),
    row('Candidate SHA-256', change.candidate_sha256),
    row('Candidate size', change.candidate_len == null ? null : `${change.candidate_len} bytes`),
  ];
  if (upgrade.target?.programdata_address) rows.push(row('ProgramData', upgrade.target.programdata_address));
  if (upgrade.replaces) rows.push(row('Replaces', `${upgrade.replaces.sha256} (${upgrade.replaces.len} bytes)`));
  if (upgrade.expected_upgrade_authority) rows.push(row('Upgrade authority', upgrade.expected_upgrade_authority));
  if (spec?.activation?.slot != null) rows.push(row('Activation slot', spec.activation.slot));
  if (spec?.activation?.unix_timestamp != null) rows.push(row('Activation time', spec.activation.unix_timestamp));
  if (spec?.schema_version != null) rows.push(row('Spec schema', `v${spec.schema_version}`));
  if (change.origin) rows.push(row('Origin', change.origin === 'submitted' ? 'Submitted change spec' : 'Derived from the uploaded candidate'));
  if (spec?.metadata?.source) rows.push(row('Source', spec.metadata.source));
  if (resolution) {
    rows.push(row('Identity check', resolution.conflicts.length
      ? `MISMATCH — ${resolution.conflicts.join('; ')}`
      : resolution.verified
        ? 'Run record, stored spec and report name the same change'
        : 'Not every record is available to compare'));
  }
  return rows.join('');
}

function row(label, value, absent = 'not stated') {
  return `<div><span>${escapeHtml(label)}</span><b>${value == null ? `<em>${escapeHtml(absent)}</em>` : escapeHtml(value)}</b></div>`;
}

/** One line for a history listing. Legacy runs say so; nothing is invented. */
export function changeLine(run) {
  const change = run?.change;
  if (!change) {
    return `<span class="run-change"><b>Legacy run</b><small class="mono">candidate ${escapeHtml(fingerprint(run?.candidate_sha256))}</small></span>`;
  }
  const kind = kindOf(change);
  const target = kind.target(change);
  return `<span class="run-change"><b>${escapeHtml(change.label || kind.title)} → <span class="mono">${escapeHtml(middle(target))}</span></b><small class="mono">change ${escapeHtml(fingerprint(change.change_spec_id))} · candidate ${escapeHtml(fingerprint(change.candidate_sha256))}</small></span>`;
}
