import { Header, Footer } from './shell.js';
import { API_BASE, operatorToken } from './session.js';
import { ChangeCard, ChangeIdentityRows, resolveChange } from './change.js';

/** One request in flight at a time, and never sub-second. */
const POLL_MS = 1500;
const POLL_BACKOFF_MS = 5000;
const POLL_ATTEMPTS = 8;
const TERMINAL = ['passed', 'failed', 'execution_error'];

/**
 * A report page renders what a run actually produced, or says it cannot.
 *
 * There are no placeholder values anywhere below. An earlier version filled
 * missing fields with sample numbers — ten observations, a slot window, hash
 * prefixes and "Mainnet · fidelity matched" — so opening a copied link in a new
 * browser session rendered a failed check over evidence that did not exist.
 * Every invariant in the engine exists to keep absent measurement reported as
 * absent, and a page that invents it at the last step undoes all of them.
 *
 * The demo is a fixture, and it lives behind one explicit route.
 */
export function ReportPage(id) {
  if (id === 'demo') return renderReport(DEMO, { demo: true, id, extras: { spec: DEMO.change_spec, projectName: 'Example Stake Pool' } });

  const context = readContext();
  if (!context) return renderUnavailable(id);
  // Nothing of the submission is assumed to still be in memory: the run's own
  // state comes from the server on the first poll, a moment from now.
  return renderLifecycle(id, { status: 'loading' });
}

/**
 * Follow a run to its end, then show it.
 *
 * Polling rather than a socket, because the engine exposes no durable progress
 * and there is nothing to stream. `render` belongs to the router: this module
 * decides what a run looks like, not how a page is swapped in.
 */
export function attachReport(id, render) {
  if (id === 'demo') return () => {};
  const context = readContext();
  if (!context) return () => {};

  let stopped = false;
  let timer;
  let failures = 0;
  // Asked for once. Both are presentation: the project's display name, and the
  // run's canonical spec for the technical details. Neither is ever required to
  // render a run, and neither is ever invented when it cannot be fetched.
  let extras = null;
  const stop = () => {
    stopped = true;
    clearTimeout(timer);
  };
  const ask = path => fetch(`${context.api}${path}`, { headers: { Authorization: `Bearer ${context.token}` } });

  const tick = async () => {
    if (stopped) return;
    let run;
    try {
      const response = await ask(`/v1/runs/${encodeURIComponent(id)}`);
      if (response.status === 401) return finish(renderStalled(id, 'Authentication failed.', 'The stored token no longer authenticates this project. Submit the check again with a current token.'));
      if (response.status === 404) return finish(renderUnavailable(id));
      if (!response.ok) throw new Error(`status ${response.status}`);
      run = await response.json();
      failures = 0;
    } catch {
      // A failed poll is not a failed run. Back off, and only give up after
      // enough attempts that this is clearly not a blip.
      failures += 1;
      if (failures >= POLL_ATTEMPTS) return finish(renderStalled(id, 'Could not reach Eplyx.', 'The run is still on the server. Reload this page to resume following it.'));
      timer = setTimeout(tick, POLL_BACKOFF_MS);
      return;
    }

    if (!extras) extras = await loadExtras(run);
    if (stopped) return;

    if (!TERMINAL.includes(run.status)) {
      render(renderLifecycle(id, run, extras));
      // Chained rather than an interval, so two polls can never overlap.
      timer = setTimeout(tick, POLL_MS);
      return;
    }

    stop();
    if (!run.report_available) return render(renderIncomplete(id, run, extras));
    try {
      const response = await ask(`/v1/runs/${encodeURIComponent(id)}/report.json`);
      run.canonical_report = response.ok ? await response.json() : null;
    } catch {
      run.canonical_report = null;
    }
    render(renderReport(run, { demo: false, id, extras }));
  };

  async function loadExtras(run) {
    const found = { spec: null, projectName: null };
    const read = async path => {
      try {
        const response = await ask(path);
        return response.ok ? await response.json() : null;
      } catch {
        return null;
      }
    };
    // A legacy run has no stored spec; asking would only earn a 404.
    if (run.change) found.spec = await read(`/v1/runs/${encodeURIComponent(id)}/change_spec.json`);
    if (run.project_id) found.projectName = (await read(`/v1/projects/${encodeURIComponent(run.project_id)}`))?.project?.name ?? null;
    return found;
  }

  const finish = markup => {
    stop();
    render(markup);
  };

  tick();
  return stop;
}

/**
 * What a run page needs to follow a run.
 *
 * The console's own credential, not anything kept from a submission. That is
 * what makes a run openable from history, from a link, or after a reload — the
 * page never depended on having been the one that started it.
 */
function readContext() {
  const token = operatorToken();
  return token ? { api: API_BASE, token } : null;
}

/**
 * A run exists on the server, but this browser has no copy of it and no
 * credential to ask for one: the project token is a CI secret, deliberately not
 * kept after the request that used it. So this says so, rather than guessing.
 */
function renderUnavailable(id) {
  return `<main id="main" class="inner-page report-page">${Header({ light: true })}
    <section class="report-shell">
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> Run ${escapeHtml(id)}</p>
          <h1>This report is not available in this browser.</h1>
          <p>A run is followed from the browser tab that submitted it. This tab holds no credential for run <span class="mono">${escapeHtml(id)}</span>, so it cannot ask the server about it. The run itself is unaffected.</p>
        </div>
        <div class="report-verdict is-unknown"><i></i><span>Unavailable</span><em>No result loaded</em></div>
      </div>
      <div class="report-content">
        <section><div class="report-section-title"><span>01</span><h2>How to read this run</h2></div>
          <div class="empty-result">
            <p>Fetch the canonical report directly, with the project token:</p>
            <pre class="mono">curl -H "Authorization: Bearer $EPLYX_TOKEN" \\
  "$EPLYX_URL/v1/runs/${escapeHtml(id)}/report.json"</pre>
            <p>Every pull request also uploads <span class="mono">eplyx-report.json</span> and <span class="mono">eplyx-report.md</span> as build artifacts.</p>
          </div>
        </section>
        <section><div class="report-section-title"><span>02</span><h2>Example</h2></div>
          <div class="empty-result"><p><a href="/runs/demo" data-link class="text-link">See the public demo report <span>↗</span></a> for the shape of a completed check. It is a fixture, not a record of any run.</p></div>
        </section>
      </div>
    </section>
  </main>${Footer()}`;
}

const LIFECYCLE = {
  // Before the first poll answers, the only honest thing to say is that we do
  // not know yet. Opening on "queued" would announce a state for a run that may
  // have finished an hour ago.
  loading: ['Loading', 'Loading this run…', 'Asking the server where this run stands.'],
  queued: ['Queued', 'Waiting for an execution slot…', 'The run is on the server. Closing this page does not cancel it.'],
  running: ['Running', 'Replaying the candidate against the active validated corpus…', 'This takes as long as the corpus takes. There is no partial result to show.'],
};

function renderLifecycle(id, run, extras = {}) {
  const [label, headline, note] = LIFECYCLE[run.status] ?? LIFECYCLE.loading;
  const known = run.status !== 'loading';
  return shell(id, `
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> Run ${escapeHtml(id)}</p>
          <h1>${escapeHtml(headline)}</h1>
          <p>${escapeHtml(note)}</p>
        </div>
        <div class="report-verdict is-pending"><i></i><span>${escapeHtml(label)}</span><em>No result yet</em></div>
      </div>
      <div class="report-content">
        ${known ? ChangeCard({ change: run.change, legacy: !run.change, targetName: extras.projectName, spec: extras.spec, candidateSha: run.candidate_sha256 }) : ''}
        <section><div class="report-section-title"><span>01</span><h2>Inputs</h2></div>
          <div class="provenance-table">
            ${known ? ChangeIdentityRows({ change: run.change, spec: extras.spec }) : ''}
            ${row('Bundle SHA', run.bundle_sha256)}
            ${row('Corpus SHA', run.corpus_sha256)}
            ${row('Baseline SHA', run.baseline_sha256)}
            ${row('Candidate SHA', run.candidate_sha256)}
          </div>
        </section>
      </div>`);
}

/**
 * A run that ended without a report.
 *
 * Two different things land here and they are not the same. A preflight abort
 * is Eplyx answering — it carries a real exit code and produces no report by
 * design. An execution error is this service failing to answer at all, and
 * blames nothing about the candidate.
 */
function renderIncomplete(id, run, extras = {}) {
  const infrastructure = run.status === 'execution_error';
  return shell(id, `
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> Run ${escapeHtml(id)}</p>
          <h1>${infrastructure ? 'The analysis could not complete.' : 'The check stopped before it could report.'}</h1>
          <p>${infrastructure
            ? 'Eplyx reached no verdict about this candidate, so none is shown. Nothing here reflects on the upgrade.'
            : 'Eplyx reached a verdict before there was a report to put it in. Exit codes 2 and 4 are preflight aborts and produce no report.'}</p>
        </div>
        <div class="report-verdict ${infrastructure ? 'is-unknown' : ''}"><i></i><span>${infrastructure ? 'Incomplete' : 'Failed'}</span><em>${run.exit_code == null ? 'No exit code' : `Exit code ${escapeHtml(run.exit_code)}`}</em></div>
      </div>
      <div class="report-content">
        ${ChangeCard({ change: run.change, legacy: !run.change, targetName: extras.projectName, spec: extras.spec, candidateSha: run.candidate_sha256 })}
        <section><div class="report-section-title"><span>01</span><h2>Reason</h2></div>
          <div class="empty-result"><p>${run.detail ? escapeHtml(run.detail) : 'No further detail was recorded.'}</p></div>
        </section>
        <section><div class="report-section-title"><span>02</span><h2>Inputs</h2></div>
          <div class="provenance-table">
            ${ChangeIdentityRows({ change: run.change, spec: extras.spec })}
            ${row('Bundle SHA', run.bundle_sha256)}
            ${row('Candidate SHA', run.candidate_sha256)}
          </div>
        </section>
      </div>`);
}

/** Polling stopped for a reason that is about this browser, not the run. */
function renderStalled(id, headline, note) {
  return shell(id, `
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> Run ${escapeHtml(id)}</p>
          <h1>${escapeHtml(headline)}</h1>
          <p>${escapeHtml(note)}</p>
        </div>
        <div class="report-verdict is-unknown"><i></i><span>Not following</span><em>Run unaffected</em></div>
      </div>`);
}

function shell(id, inner) {
  return `<main id="main" class="inner-page report-page">${Header({ light: true })}
    <section class="report-shell">${inner}</section>
  </main>${Footer()}`;
}

/**
 * The three records of what a run analysed disagree. There is no verdict to
 * show: a result about some other proposal is not a result about this one.
 */
function renderIdentityConflict(id, live, resolution) {
  return shell(id, `
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> Run ${escapeHtml(id)}</p>
          <h1>This run’s change identity does not agree with itself.</h1>
          <p>The run record, its stored change spec and its report must name the same proposed change. They do not, so no verdict is shown.</p>
        </div>
        <div class="report-verdict is-unknown"><i></i><span>Not shown</span><em>Identity mismatch</em></div>
      </div>
      <div class="report-content">
        <section><div class="report-section-title"><span>01</span><h2>Mismatch</h2></div>
          <div class="empty-result is-error">${resolution.conflicts.map(c => `<p class="mono">${escapeHtml(c)}</p>`).join('')}</div>
        </section>
        <section><div class="report-section-title"><span>02</span><h2>Inputs</h2></div>
          <div class="provenance-table">
            ${row('Bundle SHA', live.bundle_sha256)}
            ${row('Candidate SHA', live.candidate_sha256)}
          </div>
        </section>
      </div>`);
}

function renderReport(live, { demo, id, extras = {} }) {
  const report = live.canonical_report;
  const resolution = resolveChange(live, report, extras.spec);
  if (resolution.conflicts.length) return renderIdentityConflict(id, live, resolution);
  const passed = live.status === 'passed';
  const bundle = report?.bundle;
  const findings = report?.findings ?? [];
  const unmatched = report?.unmatched ?? [];
  const undeclarable = report?.undeclarable ?? [];
  const summary = report?.summary ?? {};

  return `<main id="main" class="inner-page report-page">${Header({ light: true })}
    <section class="report-shell">
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> ${demo ? 'Public demo report · fixture' : `Run ${escapeHtml(id)}`}</p>
          <h1>${passed ? 'No unexpected economic changes detected.' : 'Unexpected economic changes detected.'}</h1>
          <p>${passed ? 'Across the tested validated historical corpus. The coverage limitations below still apply.' : 'Changes were found that are not declared, exceed what was declared, or cannot be declared at all.'}</p>
        </div>
        <div class="report-verdict ${passed ? 'is-pass' : ''}"><i></i><span>${passed ? 'Passed' : 'Failed'}</span><em>Exit code ${escapeHtml(live.exit_code ?? '—')}</em></div>
      </div>
      <div class="report-grid">
        <aside class="report-nav"><span>Report</span>
          <a href="#summary" class="active">Summary</a>
          <a href="#findings">Findings <b>${findings.length}</b></a>
          <a href="#provenance">Proof &amp; identity</a>
          <a href="#coverage">Coverage</a>
          ${field('Corpus', bundle?.record_count == null ? null : `${bundle.record_count} observations`, slotLabel(bundle))}
        </aside>
        <div class="report-content">
          <section id="summary"><div class="report-section-title"><span>01</span><h2>Summary</h2></div>
            ${ChangeCard({ change: resolution.change, legacy: resolution.legacy, targetName: extras.projectName, spec: extras.spec, candidateSha: live.candidate_sha256 })}
            <div class="metric-row">
              ${metric('Observations', bundle?.record_count, 'validated history')}
              ${metric('Unexpected', summary.unexpected, 'finding groups')}
              ${metric('Expected', summary.expected, 'within bounds')}
              ${metric('Cannot be declared', report ? undeclarable.length : null, 'detected changes')}
            </div>
          </section>
          <section id="findings"><div class="report-section-title"><span>02</span><h2>Findings</h2></div>
            ${renderFindings(findings, passed, Boolean(report))}
            ${renderUndeclarable(undeclarable)}
            ${renderUnmatched(unmatched)}
          </section>
          <section id="provenance"><div class="report-section-title"><span>03</span><h2>Proof &amp; identity</h2></div>
            <h3 class="provenance-heading">Proposed change</h3>
            <div class="provenance-table">
              ${ChangeIdentityRows({ change: resolution.change, spec: extras.spec, resolution })}
            </div>
            <h3 class="provenance-heading">Evidence</h3>
            <div class="provenance-table">
              ${row('Program', bundle?.program_id)}
              ${row('Bundle SHA', live.bundle_sha256)}
              ${row('Corpus SHA', live.corpus_sha256)}
              ${row('Baseline SHA', live.baseline_sha256)}
              ${row('Candidate SHA', live.candidate_sha256)}
              ${row('Adapter', bundle ? `${bundle.adapter} v${bundle.adapter_version} · semantic schema v${bundle.semantic_schema_version}` : null)}
            </div>
          </section>
          <section id="coverage"><div class="report-section-title"><span>04</span><h2>Coverage</h2></div>
            <div class="coverage-report">${renderCoverage(report)}
              <h3>Coverage limitations</h3>${renderLimitations(bundle)}
            </div>
          </section>
        </div>
      </div>
    </section>
  </main>${Footer()}`;
}

/** A value, or an explicit dash. Never a stand-in that reads as a measurement. */
function metric(label, value, note, mono = false) {
  const shown = value == null ? '—' : escapeHtml(value);
  return `<article><span>${escapeHtml(label)}</span><strong${mono ? ' class="mono"' : ''}>${shown}</strong><em>${escapeHtml(note)}</em></article>`;
}

function row(label, value) {
  return `<div><span>${escapeHtml(label)}</span><b>${value == null ? '<em>not reported</em>' : escapeHtml(value)}</b></div>`;
}

function field(label, value, note) {
  if (value == null) return '';
  return `<div><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong>${note ? `<small>${escapeHtml(note)}</small>` : ''}</div>`;
}

function slotLabel(bundle) {
  const slots = bundle?.source_slot_range;
  return slots ? `Slots ${slots.first} → ${slots.last}` : null;
}

function renderFindings(rows, passed, hasReport) {
  if (!hasReport) {
    return '<div class="empty-result">The canonical report could not be retrieved for this run, so its findings are not shown here. Fetch <span class="mono">report.json</span> with the project token.</div>';
  }
  if (rows.length === 0) {
    return `<div class="empty-result">${passed ? 'No findings in the tested corpus.' : 'No named findings. This check failed for a reason listed below.'}</div>`;
  }
  return rows.map(finding => {
    const severity = String(finding.severity ?? 'finding');
    const status = String(finding.status ?? 'unreviewed');
    const affected = Array.isArray(finding.observations) ? finding.observations.length : 0;
    const covered = finding.covered_observations ?? '—';
    const delta = finding.max_relative_delta_bps != null
      ? `<div class="outcome-diff"><span><em>Largest change</em><b class="bad">${escapeHtml(finding.max_relative_delta_bps)} bps</b></span></div>`
      : '';
    const breaches = Array.isArray(finding.breaches) && finding.breaches.length
      ? `<p class="mono">Exceeds: ${escapeHtml(JSON.stringify(finding.breaches))}</p>` : '';
    return `<article class="report-finding ${escapeHtml(severity.toLowerCase())}">
      <div class="report-finding__head"><span>${escapeHtml(severity)}</span><span>${escapeHtml(status.replace(/_/g, ' '))}</span><b>${affected} / ${escapeHtml(covered)} observations</b></div>
      <h3 class="mono">${escapeHtml(finding.fingerprint ?? 'Economic change')}</h3>
      <p>${finding.reason ? `Declared: ${escapeHtml(finding.reason)}` : 'Not declared in expected-changes.toml.'}</p>${delta}${breaches}</article>`;
  }).join('');
}

/** Detected, and outside what any expectation can name. Its own section,
 *  because it is frequently the sole reason a check failed. */
function renderUndeclarable(rows) {
  if (!rows.length) return '';
  return `<h3>Changes that cannot be declared</h3>
    <p class="section-note">Detected, and outside the vocabulary an expectation can name. The subject has to be promoted deliberately before one of these can be approved.</p>
    ${rows.map(row => `<article class="report-finding warning">
      <div class="report-finding__head"><span>${escapeHtml(String(row.layer ?? '').replace(/_/g, ' '))}</span><span>undeclarable</span><b>${Array.isArray(row.observations) ? row.observations.length : 0} observations</b></div>
      <h3 class="mono">${escapeHtml(row.description ?? '')}</h3></article>`).join('')}`;
}

function renderUnmatched(rows) {
  if (!rows.length) return '';
  return `<h3>Declarations that matched nothing</h3>
    ${rows.map(row => `<article class="report-finding warning">
      <div class="report-finding__head"><span>${escapeHtml(String(row.status ?? '').replace(/_/g, ' '))}</span><span>declaration</span><b>${escapeHtml(row.covered_observations ?? 0)} can measure it</b></div>
      <h3 class="mono">${escapeHtml(row.fingerprint ?? '')}</h3>
      <p>Declared: ${escapeHtml(row.reason ?? '')}</p></article>`).join('')}`;
}

function renderCoverage(report) {
  const coverage = report?.coverage;
  if (!Array.isArray(coverage) || coverage.length === 0) {
    return '<div><span>Semantic coverage</span><b><em>none reported</em></b></div>';
  }
  return coverage.map(row => `<div><span class="mono">${escapeHtml(row.subject)}</span><b>${escapeHtml(row.observations)} observations</b></div>`).join('');
}

function renderLimitations(bundle) {
  const limitations = bundle?.limitations ?? [];
  if (!limitations.length) return '<p>This bundle reported no limitations. That is unusual; check the canonical report.</p>';
  return `<ul>${limitations.map(item => `<li>${escapeHtml(item.detail ?? item.code)}</li>`).join('')}</ul>`;
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#039;' }[c]));
}

/**
 * The demo fixture. Reachable only from `/runs/demo`, labelled as a fixture on
 * the page, and never used to fill in a real run's missing fields.
 */
const DEMO = {
  status: 'failed',
  exit_code: 1,
  report_available: true,
  bundle_sha256: 'a18d72bc2f46…3e9c',
  corpus_sha256: 'c253aefc08d1…a901',
  baseline_sha256: '9f3a0d4be24e…8c21',
  candidate_sha256: '60b7e1ac3198…bf14',
  change: {
    change_spec_id: '5c1f0e9ad27b44c08e1d7fa3b60c2e95d8a41f7c63b09e2d15a8c4f70b3e6d21',
    kind: 'program_upgrade',
    target_program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy',
    candidate_sha256: '60b7e1ac3198…bf14',
    candidate_len: 32240,
    label: 'Fee rounding change (fixture)',
    origin: 'derived_from_candidate'
  },
  change_spec: {
    schema_version: 1,
    change_spec_id: '5c1f0e9ad27b44c08e1d7fa3b60c2e95d8a41f7c63b09e2d15a8c4f70b3e6d21',
    change: { kind: 'program_upgrade', target: { program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy' }, candidate: { sha256: '60b7e1ac3198…bf14', len: 32240 } },
    metadata: { label: 'Fee rounding change (fixture)' }
  },
  canonical_report: {
    change: {
      change_spec_id: '5c1f0e9ad27b44c08e1d7fa3b60c2e95d8a41f7c63b09e2d15a8c4f70b3e6d21',
      kind: 'program_upgrade',
      target_program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy',
      candidate_sha256: '60b7e1ac3198…bf14'
    },
    summary: { unexpected: 2, expected: 0, passed: false, exit_code: 1 },
    bundle: {
      program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy',
      record_count: 10,
      adapter: 'spl-stake-pool',
      adapter_version: 3,
      semantic_schema_version: 2,
      source_slot_range: { first: 447454329, last: 447488871 },
      limitations: [
        { code: 'deposit_replayability_materially_below_observed', detail: 'Jito-tipped DepositSol is under-represented relative to observed production activity.' },
        { code: 'failed_original_transactions_unsupported', detail: 'Transactions that failed on mainnet are observed but not replayed, so no failure path is represented.' },
        { code: 'account_creation_paths_unsupported', detail: 'Interactions that create a token account are outside the exact historical contract and are excluded.' }
      ]
    },
    coverage: [
      { subject: 'spl-stake-pool/deposit_sol/economic/pool_tokens_received', observations: 6 },
      { subject: 'spl-stake-pool/withdraw_sol/execution/transaction', observations: 4 }
    ],
    findings: [
      { severity: 'CRITICAL', status: 'unexpected', fingerprint: 'spl-stake-pool/withdraw_sol/execution/transaction/now_reverts', observations: ['a', 'b', 'c', 'd'], covered_observations: 4 },
      { severity: 'HIGH', status: 'unexpected', fingerprint: 'spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased', observations: ['e'], covered_observations: 6, max_relative_delta_bps: -21 }
    ],
    undeclarable: [
      { layer: 'decoded_economic', description: 'manager-fee amount', observations: ['a', 'b', 'c', 'd', 'e'] }
    ],
    unmatched: []
  }
};
