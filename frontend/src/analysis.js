// What an analysis says, in the order a person asks it.
//
//   CiReport (+ the run record) → AnalysisViewModel
//
// This is a projection, never a second analysis. Every decision that carries
// truth — whether replay matched, whether semantic coverage exists, whether a
// finding is expected, whether a change is undeclarable — was made by the
// engine and is read here from the report. What this module adds is only
// presentation: which sentence to lead with, which facts to group, how to
// shorten an identifier. It computes no protocol quantity, and it never turns
// an absence into a reassurance: zero findings without semantic coverage is
// "not evaluated", not "nothing changed".
//
// Pure and synchronous, with no DOM and no escaping, so every rule below is
// testable against real frozen reports. The renderer escapes.

/** Exit codes, as the engine defines them (`engine/src/ci.rs`, `review.rs`). */
export const EXIT = {
  PASSED: 0,
  CHANGE: 1,
  ERROR: 2,
  STALE: 3,
  INCOMPATIBLE: 4,
  UNEVALUABLE: 5,
};

// ------------------------------------------------------------ vocabulary

/**
 * Transaction-level descriptions the structural layer emits. Every other
 * description begins with the label of the account it concerns. Kept in step
 * with `engine/src/ci.rs` by a test that reads the engine source.
 */
export const TRANSACTION_LEVEL_CHANGES = [
  'transaction outcome',
  'transaction fee',
  'execution logs',
  'inner instructions',
  'return data',
  'invocation shape',
];
export const EVALUATION_UNAVAILABLE = 'semantic evaluation unavailable: ';

/**
 * Display casing for common abbreviations inside subject names. Formatting
 * only: it changes how a word is spelled, never what is shown.
 */
const ACRONYMS = { pnl: 'PnL', sol: 'SOL', lp: 'LP', usd: 'USD', usdc: 'USDC', cpi: 'CPI', id: 'ID', spl: 'SPL', amm: 'AMM' };

const CHANGE_PHRASES = {
  increased: 'increased',
  decreased: 'decreased',
  changed: 'changed',
  now_reverts: 'now fails',
  now_succeeds: 'now succeeds',
  enabled: 'was enabled',
  disabled: 'was disabled',
  created: 'is now created',
  removed: 'is no longer created',
};

const STATUS = {
  unexpected: { label: 'Unexpected', note: 'Not declared in expected-changes.toml.', tone: 'bad' },
  expected: { label: 'Declared', note: 'Declared in expected-changes.toml and within its bounds.', tone: 'ok' },
  expected_but_exceeded: { label: 'Exceeds declaration', note: 'Declared, but larger or wider than declared.', tone: 'bad' },
  stale: { label: 'Stale declaration', note: 'Declared, measurable here, and no longer happening.', tone: 'warn' },
  unevaluable: { label: 'Cannot be judged', note: 'Declared, and this corpus cannot say whether it happened.', tone: 'warn' },
};

const REASONS = {
  no_semantic_coverage: 'Economic impact was not evaluated: nothing in this corpus could be interpreted economically.',
  undeclarable_change: 'Eplyx detected a change it cannot name, so it cannot be declared.',
  undeclared_change: 'A measured change is not declared, or is larger than declared.',
  stale_expectation: 'A declared change no longer happens.',
  unevaluable_expectation: 'A declared change cannot be judged by this corpus.',
};

const BINDING_LEVELS = {
  exact_verified_build: {
    label: 'Exact verified build',
    statement: 'The interface source was verified against the exact historical build.',
  },
  standard_program_interface: {
    label: 'Standard program interface',
    statement: 'Economic interpretation uses a standard Solana program interface.',
  },
  execution_corroborated_external_interface: {
    label: 'Execution-corroborated external interface',
    statement: 'Economic interpretation is supported by execution-corroborated external interface evidence: a pinned protocol source whose account roles and layout the historical execution corroborates.',
  },
  repository_source_claim: {
    label: 'Repository source claim',
    statement: 'Economic interpretation rests on a repository source claim that the historical execution did not corroborate.',
  },
  manual_or_unknown: {
    label: 'Manual or unknown',
    statement: 'The provenance of the economic interpretation is manual or unknown.',
  },
};

const FIDELITY_PROFILES = {
  checkpointed_execution_v1: 'Checkpointed historical replay',
  complete_execution_v2: 'Complete historical execution',
  historical_replay_v1: 'Historical replay',
};

// --------------------------------------------------------------- helpers

/** `protocol/action/domain/subject/change`, or null if it is not one. */
export function parseFingerprint(text) {
  const parts = typeof text === 'string' ? text.split('/') : [];
  if (parts.length !== 5 || parts.some(part => !part)) return null;
  const [protocol, action, domain, subject, change] = parts;
  return { protocol, action, domain, subject, change, subjectKey: parts.slice(0, 4).join('/'), actionKey: `${protocol}/${action}` };
}

/** `protocol/action/domain/subject`, or null. */
export function parseSubject(text) {
  const parts = typeof text === 'string' ? text.split('/') : [];
  if (parts.length !== 4 || parts.some(part => !part)) return null;
  const [protocol, action, domain, subject] = parts;
  return { protocol, action, domain, subject, subjectKey: text, actionKey: `${protocol}/${action}` };
}

/** `settle_pnl` → "Settle PnL"; `drift-settle-pnl` → "Drift settle PnL". */
export function humanize(token) {
  if (typeof token !== 'string' || !token) return '—';
  const words = token.split(/[_-]+/).filter(Boolean)
    .map(word => ACRONYMS[word.toLowerCase()] ?? (word.length === 1 || /^v\d+$/i.test(word) ? word.toUpperCase() : word.toLowerCase()));
  if (!words.length) return token;
  const first = words[0];
  words[0] = first === first.toLowerCase() ? first[0].toUpperCase() + first.slice(1) : first;
  return words.join(' ');
}

const subjectTitle = parsed => (parsed.domain === 'execution' && parsed.subject === 'transaction'
  ? 'Transaction outcome'
  : humanize(parsed.subject));

/** A readable fingerprint. The full value lives in the technical details. */
export const shortId = (hash, length = 8) =>
  typeof hash === 'string' && hash.length > length ? hash.slice(0, length) : hash ?? '—';

export const middle = (value, head = 6, tail = 4) =>
  typeof value === 'string' && value.length > head + tail + 1 ? `${value.slice(0, head)}…${value.slice(-tail)}` : value ?? '—';

/**
 * A signed basis-point figure, with its sign, as bps and as a percentage.
 * Integer string work only: no floating point touches a reported number.
 */
export function formatBps(bps) {
  if (typeof bps !== 'number' || !Number.isInteger(bps)) return null;
  const sign = bps < 0 ? '−' : bps > 0 ? '+' : '';
  const magnitude = Math.abs(bps);
  const percent = `${Math.trunc(magnitude / 100)}.${String(magnitude % 100).padStart(2, '0')}`;
  return `${sign}${magnitude.toLocaleString('en-US')} bps (${sign}${percent}%)`;
}

/**
 * One semantic value exactly as the engine serialized it. Quantities are
 * already decimal strings carrying their own scale; the only change is a true
 * minus sign for display. Nothing is parsed into a number.
 */
export function formatValue(value) {
  if (!value || typeof value !== 'object') return null;
  switch (value.kind) {
    case 'quantity':
      return typeof value.quantity === 'string' ? { text: value.quantity, full: value.quantity } : null;
    case 'signed_quantity':
      return typeof value.quantity === 'string'
        ? { text: value.quantity.replace(/^-/, '−'), full: value.quantity, signed: true }
        : null;
    case 'flag':
      return { text: value.value ? 'Yes' : 'No', full: String(value.value) };
    case 'address':
      return { text: middle(value.address), full: value.address };
    default:
      return null;
  }
}

/** Sort and dedupe, so every list here is deterministic. */
const unique = values => [...new Set(values)].sort();

/**
 * Where a structural or decoded change lives, read from its description.
 *
 * The engine decides that a change is undeclarable; this only groups the ones
 * it reported by what they touch, so "which accounts?" has an answer. Anything
 * not recognised stays whole, with its description, and is never dropped.
 */
export function classifyUndeclarable(row) {
  const description = String(row?.description ?? '');
  const observations = Array.isArray(row?.observations) ? row.observations : [];
  const layer = row?.layer ?? null;
  if (TRANSACTION_LEVEL_CHANGES.includes(description)) {
    return { scope: 'transaction', description, what: humanize(description.replace(/ /g, '_')), account: null, layer, observations };
  }
  if (description.startsWith(EVALUATION_UNAVAILABLE)) {
    return { scope: 'evaluation', description, what: description.slice(EVALUATION_UNAVAILABLE.length), account: null, layer, observations };
  }
  const space = description.indexOf(' ');
  if (space > 0) {
    return { scope: 'account', description, account: description.slice(0, space), what: description.slice(space + 1), layer, observations };
  }
  return { scope: 'other', description, what: description, account: null, layer, observations };
}

// ------------------------------------------------------------ the model

/**
 * The whole page's worth of presentation, from what the run and its report
 * say. `report` is the canonical `report.json`, or null when there is none.
 */
export function analysisView({ run = {}, report = null, resolution = null } = {}) {
  if (resolution?.conflicts?.length) return mismatchView(resolution);
  if (!report) return withoutReport(run);
  return withReport(run, report);
}

function mismatchView(resolution) {
  return {
    state: 'identity_mismatch',
    headline: {
      title: 'This run’s change identity does not agree with itself.',
      note: 'The run record, its stored change spec and its report must name the same proposed change. They do not, so no result is shown.',
      label: 'Not shown',
      tone: 'is-unknown',
    },
    conflicts: resolution.conflicts,
    dimensions: [],
  };
}

function withoutReport(run) {
  const exit = run.exit_code;
  if (run.status === 'execution_error') {
    return {
      state: 'analysis_error',
      headline: {
        title: 'The analysis could not complete.',
        note: 'Eplyx obtained no result for this change, so none is shown. This is a failure of the analysis service or its inputs, not a finding about the upgrade.',
        label: 'Analysis error',
        tone: 'is-unknown',
      },
      dimensions: [
        dimension('execution', 'Execution', 'Not determined', 'unknown', 'No execution result was obtained.'),
        dimension('impact', 'Economic impact', 'Not evaluated', 'unknown', 'Nothing was evaluated.'),
      ],
    };
  }
  if (run.status === 'failed' && exit === EXIT.INCOMPATIBLE) {
    return {
      state: 'incompatible',
      headline: {
        title: 'This change cannot be checked against the pinned bundle.',
        note: 'The check stopped before anything executed: the proposed change does not fit the baseline this bundle proves (exit code 4). Refresh or re-pin the bundle, or correct the change.',
        label: 'Not run',
        tone: 'is-unknown',
      },
      dimensions: [
        dimension('execution', 'Execution', 'Not attempted', 'unknown', 'Stopped at preflight, before any replay.'),
        dimension('impact', 'Economic impact', 'Not evaluated', 'unknown', 'Nothing was evaluated.'),
      ],
    };
  }
  if (run.status === 'failed' && exit === EXIT.ERROR) {
    return {
      state: 'not_verified',
      headline: {
        title: 'Eplyx could not verify execution for this change.',
        note: 'The check stopped before producing a report (exit code 2): a configuration error, or a historical replay that did not reproduce its recorded result. No economic impact was evaluated, and nothing here is a finding about the upgrade.',
        label: 'Not verified',
        tone: 'is-unknown',
      },
      dimensions: [
        dimension('execution', 'Execution', 'Not verified', 'bad', 'No verified execution was produced.'),
        dimension('impact', 'Economic impact', 'Not evaluated', 'unknown', 'Nothing was evaluated.'),
      ],
    };
  }
  return {
    state: 'report_unavailable',
    headline: {
      title: 'This run’s report could not be retrieved.',
      note: 'The run finished, but its canonical report was not available to this page, so no result is shown here. Fetch report.json with the project token.',
      label: 'Report unavailable',
      tone: 'is-unknown',
    },
    dimensions: [
      dimension('execution', 'Execution', 'Not shown', 'unknown', 'The report was not retrieved.'),
      dimension('impact', 'Economic impact', 'Not shown', 'unknown', 'The report was not retrieved.'),
    ],
  };
}

function dimension(key, label, value, tone, note) {
  return { key, label, value, tone, note };
}

function withReport(run, report) {
  const summary = report.summary ?? {};
  const reasons = Array.isArray(summary.failure_reasons) ? summary.failure_reasons
    : Array.isArray(report.failures) ? report.failures : [];
  const findings = Array.isArray(report.findings) ? report.findings : [];
  const coverage = Array.isArray(report.coverage) ? report.coverage : [];
  const undeclarable = (Array.isArray(report.undeclarable) ? report.undeclarable : []).map(classifyUndeclarable);
  const unmatched = Array.isArray(report.unmatched) ? report.unmatched : [];
  const bundle = report.bundle ?? {};
  const total = typeof bundle.record_count === 'number' ? bundle.record_count : null;
  const proofSummary = report.replay_proof ?? null;

  // A report exists only after the engine's baseline fidelity gate passed; a
  // replay that did not reproduce aborts with exit 2 and no report at all. The
  // report's own proof status is still honoured if it ever says otherwise.
  const executionVerified = !proofSummary || proofSummary.status === 'matched';
  // Zero findings with nothing measured is "we did not look". The engine
  // fails such a gate itself; a report that somehow lacks the reason is read
  // the same way rather than as a clean result.
  const notEvaluated = reasons.includes('no_semantic_coverage') || coverage.length === 0;
  const evaluated = !notEvaluated;

  const cards = findings.map(finding => findingCard(finding, total));
  const adverse = cards.filter(card => card.status !== 'expected');
  const reverts = cards.filter(card => card.parsed?.domain === 'execution' && card.parsed?.change === 'now_reverts');
  const outcomeUnnamed = undeclarable.filter(row => row.description === 'transaction outcome');

  const primary = [
    ...reverts.map(card => ({
      kind: 'now_reverts',
      title: `The proposed build fails for ${count(card.observations.length, total)} historical production ${plural(card.observations.length, 'interaction')}.`,
      note: 'These transactions succeeded historically and fail under the proposed build. Their economic values are not compared, because there is no successful proposed execution to compare.',
      action: card.parsed ? humanize(card.parsed.action) : null,
      status: card.status,
      statusLabel: card.statusLabel,
      fingerprint: card.fingerprint,
    })),
    ...outcomeUnnamed.map(row => ({
      kind: 'outcome_changed',
      title: `The transaction outcome changed under the proposed build for ${count(row.observations.length, total)} ${plural(row.observations.length, 'interaction')}.`,
      note: 'Eplyx has no semantic subject for this outcome, so it cannot name how it changed. It is listed with the unexplained changes below.',
      action: null,
      status: 'unexpected',
      statusLabel: 'Unexplained',
      fingerprint: null,
    })),
  ];

  const state = !executionVerified ? 'not_verified'
    : notEvaluated ? 'not_evaluated'
      : summary.passed === true ? (cards.length ? 'declared_changes_only' : 'no_changes_found')
        : reasons.includes('undeclared_change') || reasons.includes('undeclarable_change') ? 'changes_detected'
          : 'declarations_attention';

  const headline = headlineFor({ state, reasons, adverse, reverts, undeclarable, total, coverage });
  const groups = impactGroups(coverage, cards);
  const affected = affectedFacts({ cards, undeclarable, total, groups });
  const proof = proofFacts(report, executionVerified);

  return {
    state,
    headline,
    gate: {
      passed: summary.passed === true,
      exitCode: summary.exit_code ?? run.exit_code ?? null,
      reasons: reasons.map(code => ({ code, text: REASONS[code] ?? 'See the canonical report.' })),
    },
    dimensions: [
      dimension('execution', 'Execution', executionVerified ? 'Verified' : 'Not verified', executionVerified ? 'ok' : 'bad',
        reverts.length ? `${proof.executionNote}; the proposed build fails in ${count(unique(reverts.flatMap(card => card.observations)).length, total)}` : proof.executionNote),
      notEvaluated
        ? dimension('impact', 'Economic impact', coverage.length ? 'Not fully evaluated' : 'Not evaluated', 'unknown',
          coverage.length ? 'Some interactions could not be interpreted economically.' : 'No semantic coverage for this interaction.')
        : evaluated
          ? dimension('impact', 'Economic impact', 'Evaluated', 'ok', reverts.length
            ? `${coverage.length} ${plural(coverage.length, 'subject')}; not compared where the proposed build fails`
            : `${coverage.length} ${plural(coverage.length, 'subject')} measured`)
          : dimension('impact', 'Economic impact', 'Not evaluated', 'unknown', 'No subject was measured.'),
      notEvaluated
        ? dimension('findings', 'Named changes', 'Not applicable', 'unknown', 'Nothing can be named without semantic coverage.')
        : dimension('findings', 'Named changes', cards.length ? String(cards.length) : 'None', adverse.length ? 'bad' : cards.length ? 'ok' : 'neutral',
          cards.length ? `${adverse.length} unexpected · ${cards.length - adverse.length} declared` : 'In the subjects evaluated'),
      dimension('unexplained', 'Unexplained changes', undeclarable.length ? String(undeclarable.length) : 'None',
        undeclarable.length ? 'warn' : 'neutral', undeclarable.length ? 'State Eplyx could not explain' : 'No other state changed'),
    ],
    primary,
    notEvaluated,
    evaluated,
    groups,
    unexplained: {
      rows: undeclarable,
      accounts: unique(undeclarable.filter(row => row.scope === 'account').map(row => row.account)),
      decoded: undeclarable.filter(row => row.layer === 'decoded_economic'),
      structural: undeclarable.filter(row => row.layer !== 'decoded_economic'),
    },
    declarations: unmatched.map(row => {
      const parsed = parseFingerprint(row.fingerprint);
      const status = STATUS[row.status] ?? { label: humanize(String(row.status ?? '')), note: '', tone: 'warn' };
      return {
        fingerprint: row.fingerprint,
        title: parsed ? `${subjectTitle(parsed)} ${CHANGE_PHRASES[parsed.change] ?? humanize(parsed.change)}` : row.fingerprint,
        context: parsed ? `${humanize(parsed.protocol)} · ${humanize(parsed.action)}` : null,
        status: row.status,
        statusLabel: status.label,
        statusNote: status.note,
        reason: row.reason ?? null,
        covered: row.covered_observations ?? 0,
      };
    }),
    affected,
    proof,
    limitations: (Array.isArray(bundle.limitations) ? bundle.limitations : []).map(item => item.detail ?? item.code),
    corpus: { total, slots: bundle.source_slot_range ?? null, adapter: bundle.adapter ?? null },
  };
}

function headlineFor({ state, reasons, adverse, reverts, undeclarable, total, coverage }) {
  switch (state) {
    case 'not_verified':
      return {
        title: 'Eplyx could not verify execution for this change.',
        note: 'The report does not state a matched replay, so no result is shown as verified.',
        label: 'Not verified', tone: 'is-unknown',
      };
    case 'not_evaluated':
      return {
        title: 'Economic impact could not be evaluated for this interaction.',
        note: `The proposed build was executed, but Eplyx has no semantic coverage for this interaction, so it cannot say whether economic outcomes changed. This is neither a pass nor a finding.${undeclarable.length ? ' It did detect changes it cannot name; they are listed below.' : ''}`,
        label: 'Not evaluated', tone: 'is-unknown',
      };
    case 'no_changes_found':
      return {
        title: 'No changes found in what Eplyx evaluated.',
        note: `The historical execution reproduced, and the ${coverage.length} ${plural(coverage.length, 'subject')} Eplyx measured matched the current build${total == null ? '' : ` across ${total} ${plural(total, 'interaction')}`}. This covers the tested interactions and subjects only; the limitations below still apply.`,
        label: 'No changes found', tone: 'is-pass',
      };
    case 'declared_changes_only':
      return {
        title: 'Only declared changes were found.',
        note: 'Every change Eplyx measured is declared in expected-changes.toml and within its bounds. Declared changes are still changes, and they are listed below.',
        label: 'Declared only', tone: 'is-pass',
      };
    case 'changes_detected': {
      if (reverts.some(card => card.status !== 'expected')) {
        return {
          title: 'The proposed build fails for historical production interactions.',
          note: 'Transactions that succeeded in production fail under the proposed build. Other measured changes, if any, are listed below.',
          label: 'Changes detected', tone: '',
        };
      }
      if (adverse.length || reasons.includes('undeclared_change')) {
        const economic = adverse.some(card => card.parsed?.domain === 'economic');
        return {
          title: economic ? 'Unexpected economic changes detected.' : 'Unexpected changes detected.',
          note: 'Eplyx measured changes that are not declared, or that exceed what was declared.',
          label: 'Changes detected', tone: '',
        };
      }
      return {
        title: 'Changes detected that Eplyx could not explain.',
        note: 'No named subject changed unexpectedly, but other state did. Eplyx cannot describe what those changes mean, and does not treat them as harmless.',
        label: 'Changes detected', tone: '',
      };
    }
    default: {
      const stale = reasons.includes('stale_expectation');
      return {
        title: stale ? 'A declared change no longer happens.' : 'A declared change cannot be judged by this corpus.',
        note: stale
          ? 'expected-changes.toml declares behaviour that the proposed build no longer shows. Remove or correct the declaration.'
          : 'Eplyx cannot prove whether a declaration still applies, because this corpus cannot measure it.',
        label: 'Declarations need review', tone: 'is-warn',
      };
    }
  }
}

function findingCard(finding, total) {
  const parsed = parseFingerprint(finding.fingerprint);
  const status = STATUS[finding.status] ?? { label: humanize(String(finding.status ?? 'unreviewed')), note: '', tone: 'warn' };
  const observations = Array.isArray(finding.observations) ? finding.observations : [];
  const values = (Array.isArray(finding.values) ? finding.values : []).map(entry => ({
    observation: entry.observation_id,
    before: formatValue(entry.baseline),
    proposed: formatValue(entry.candidate),
    relative: formatBps(entry.relative_delta_bps),
  }));
  return {
    fingerprint: finding.fingerprint ?? null,
    parsed,
    title: parsed ? `${subjectTitle(parsed)} ${CHANGE_PHRASES[parsed.change] ?? humanize(parsed.change)}` : String(finding.fingerprint ?? 'Change'),
    subject: parsed ? subjectTitle(parsed) : null,
    change: parsed?.change ?? null,
    status: finding.status ?? null,
    statusLabel: status.label,
    statusNote: status.note,
    tone: status.tone,
    severity: finding.severity ?? null,
    observations,
    covered: finding.covered_observations ?? null,
    reach: `${observations.length} of ${finding.covered_observations ?? total ?? '—'} ${plural(finding.covered_observations ?? observations.length, 'interaction')}`,
    entities: Array.isArray(finding.entities) ? finding.entities : [],
    largest: formatBps(finding.max_relative_delta_bps),
    values,
    reason: finding.reason ?? null,
    breaches: (Array.isArray(finding.breaches) ? finding.breaches : []).map(breachText),
  };
}

function breachText(breach) {
  switch (breach?.bound) {
    case 'relative_delta': return `Declared at most ${formatBps(breach.limit_bps) ?? breach.limit_bps}; measured ${formatBps(breach.observed_bps) ?? breach.observed_bps}.`;
    case 'affected_observations': return `Declared at most ${breach.limit} affected interactions; measured ${breach.observed}.`;
    case 'affected_entities': return `Declared at most ${breach.limit} affected entities; measured ${breach.observed}.`;
    default: return JSON.stringify(breach);
  }
}

/**
 * Subjects grouped by protocol action, each with what was measured about it.
 * A subject with no finding is "no change measured" — unless the same action
 * now fails, in which case it was not compared and says so.
 */
function impactGroups(coverage, cards) {
  const groups = new Map();
  const group = (actionKey, protocol, action) => {
    if (!groups.has(actionKey)) {
      groups.set(actionKey, { key: actionKey, title: humanize(action), protocol: humanize(protocol), subjects: new Map() });
    }
    return groups.get(actionKey);
  };
  for (const row of coverage) {
    const parsed = parseSubject(row.subject);
    if (!parsed) continue;
    group(parsed.actionKey, parsed.protocol, parsed.action).subjects.set(parsed.subjectKey, {
      key: parsed.subjectKey, title: subjectTitle(parsed), domain: parsed.domain, measured: row.observations ?? null, cards: [],
    });
  }
  for (const card of cards) {
    if (!card.parsed) continue;
    const target = group(card.parsed.actionKey, card.parsed.protocol, card.parsed.action);
    if (!target.subjects.has(card.parsed.subjectKey)) {
      target.subjects.set(card.parsed.subjectKey, {
        key: card.parsed.subjectKey, title: subjectTitle(card.parsed), domain: card.parsed.domain, measured: card.covered, cards: [],
      });
    }
    target.subjects.get(card.parsed.subjectKey).cards.push(card);
  }
  return [...groups.values()].sort((a, b) => a.key.localeCompare(b.key)).map(entry => {
    const failing = [...entry.subjects.values()]
      .flatMap(subject => subject.cards)
      .find(card => card.parsed.domain === 'execution' && card.parsed.change === 'now_reverts');
    const failed = failing ? failing.observations.length : 0;
    const subjects = [...entry.subjects.values()].sort((a, b) => a.key.localeCompare(b.key)).map(subject => ({
      ...subject,
      state: subject.cards.length ? 'changed' : failed ? 'not_compared' : 'unchanged',
      stateLabel: subject.cards.length ? 'Changed'
        : failed ? `Not compared in the ${failed} ${plural(failed, 'interaction')} where execution failed`
          : 'No change measured',
    }));
    return { key: entry.key, title: entry.title, protocol: entry.protocol, subjects };
  });
}

function affectedFacts({ cards, undeclarable, total, groups }) {
  const touched = unique([...cards.flatMap(card => card.observations), ...undeclarable.flatMap(row => row.observations)]);
  return {
    interactions: { affected: touched.length, total },
    entities: unique(cards.flatMap(card => card.entities)),
    accounts: unique(undeclarable.filter(row => row.scope === 'account').map(row => row.account)),
    changedSubjects: groups.flatMap(group => group.subjects.filter(subject => subject.state === 'changed').map(subject => `${group.title}: ${subject.title}`)),
  };
}

/**
 * The proof statement, from the report's own proof and binding fields only.
 * Checkpointed is never called observed; corroborated is never called exact.
 */
function proofFacts(report, executionVerified) {
  const statements = [];
  const qualifiers = [];
  const replay = report.replay_proof ?? null;
  const contracts = Array.isArray(replay?.proof_contract_versions) ? replay.proof_contract_versions : [];
  const boundary = typeof replay?.boundary_proof === 'string' ? replay.boundary_proof.split(';').map(part => part.trim()).filter(Boolean) : [];
  let executionNote;
  if (!executionVerified) {
    executionNote = 'The report does not state a matched replay.';
  } else if (!replay) {
    executionNote = 'Historical execution reproduced';
    statements.push('Historical execution reproduced: the current build matched each recorded production interaction before the proposed build ran.');
  } else {
    const profile = FIDELITY_PROFILES[replay.profile] ?? humanize(String(replay.profile ?? ''));
    executionNote = `${profile} matched`;
    statements.push(`Result reconstructed from a ${profile.toLowerCase()}: the current build matched the retained result in ${replay.observations} ${plural(replay.observations, 'interaction')} before the proposed build ran.`);
    if (contracts.includes(2)) qualifiers.push('The state before the transaction was reconstructed from retained validator evidence and re-verified; it was not directly observed.');
    if (contracts.includes(3) || boundary.includes('derived_target_boundary')) qualifiers.push('The transaction’s starting state was derived by replaying the earlier transactions that affect it; it was not directly observed.');
    if (!contracts.length) qualifiers.push('Replay starts from a retained checkpoint rather than the directly observed transaction boundary.');
    qualifiers.push('A checkpointed replay does not claim exact fidelity to the original validator execution.');
  }

  const binding = report.semantic_binding ?? null;
  const bindings = Array.isArray(binding?.observations) ? binding.observations : [];
  const levels = unique(bindings.map(entry => entry?.binding?.level).filter(Boolean));
  const coverage = Array.isArray(report.coverage) ? report.coverage : [];
  let provenance = null;
  if (bindings.length) {
    for (const level of levels) statements.push(BINDING_LEVELS[level]?.statement ?? `Economic interpretation provenance: ${humanize(level)}.`);
    const exact = typeof binding.exact_source_to_elf_verified === 'boolean' ? binding.exact_source_to_elf_verified : null;
    qualifiers.push(exact === true
      ? 'The interpretation source was verified against the exact deployed build.'
      : 'Exact source-to-bytecode build verification: not established. The interface is corroborated by execution, not proven to be the deployed build’s source.');
    const sources = new Map();
    for (const entry of bindings) {
      const source = entry?.binding?.source;
      if (source?.repository) sources.set(`${source.repository}@${source.commit}`, { repository: source.repository, commit: source.commit ?? null, files: Array.isArray(source.source_blobs) ? source.source_blobs.length : 0 });
      if (entry?.binding?.program_id) sources.set(entry.binding.program_id, { program: entry.binding.program_id });
    }
    provenance = {
      levels: levels.map(level => BINDING_LEVELS[level]?.label ?? humanize(level)),
      exact,
      sources: [...sources.values()],
      facts: unique(bindings.flatMap(entry => (Array.isArray(entry?.binding?.facts) ? entry.binding.facts : []))).map(humanize),
      observations: bindings.length,
    };
  } else if (coverage.length) {
    statements.push('This report records no provenance for its economic interpretation.');
  }
  return {
    executionNote,
    statements,
    qualifiers,
    provenance,
    adapter: report.bundle?.adapter && report.bundle.adapter !== 'none' ? `${report.bundle.adapter} v${report.bundle.adapter_version}` : null,
    profile: replay?.profile ?? null,
    contracts,
    boundary,
  };
}

const plural = (n, word) => (n === 1 ? word : `${word}s`);
const count = (n, total) => (total == null ? String(n) : `${n} of ${total}`);

// ------------------------------------------------------------ run history

/**
 * What a history row can say without the report: the run's status and exit
 * code, read through the engine's exit-code contract. Exit 2 *with* a report
 * is only ever `no_semantic_coverage`; without one it is a preflight abort.
 */
export function runSummary(run = {}) {
  const exit = run.exit_code;
  const withReport = run.report_available === true;
  const chip = (label, tone) => ({ label, tone });
  switch (run.status) {
    case 'queued': return { execution: chip('Queued', 'pending'), impact: chip('Pending', 'pending') };
    case 'running': return { execution: chip('Running', 'pending'), impact: chip('Pending', 'pending') };
    case 'execution_error': return { execution: chip('Analysis error', 'unknown'), impact: chip('Not evaluated', 'unknown') };
    case 'passed': return { execution: chip('Verified', 'ok'), impact: chip('No unexpected changes', 'ok') };
    case 'failed':
      if (!withReport) {
        return exit === EXIT.INCOMPATIBLE
          ? { execution: chip('Not run', 'unknown'), impact: chip('Not evaluated', 'unknown') }
          : { execution: chip('Not verified', 'bad'), impact: chip('Not evaluated', 'unknown') };
      }
      if (exit === EXIT.ERROR) return { execution: chip('Verified', 'ok'), impact: chip('Not evaluated', 'unknown') };
      if (exit === EXIT.CHANGE) return { execution: chip('Verified', 'ok'), impact: chip('Changes detected', 'bad') };
      if (exit === EXIT.STALE) return { execution: chip('Verified', 'ok'), impact: chip('Stale declaration', 'warn') };
      if (exit === EXIT.UNEVALUABLE) return { execution: chip('Verified', 'ok'), impact: chip('Declaration unjudged', 'warn') };
      return { execution: chip('Verified', 'ok'), impact: chip('Failed', 'bad') };
    default: return { execution: chip(humanize(String(run.status ?? 'unknown')), 'unknown'), impact: chip('—', 'unknown') };
  }
}
