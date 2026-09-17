import { Header, Footer } from './shell.js';

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
  if (id === 'demo') return renderReport(DEMO, { demo: true, id });

  const live = readStoredRun(id);
  if (!live) return renderUnavailable(id);
  return renderReport(live, { demo: false, id });
}

/** The run this browser session submitted, if it was this browser session. */
function readStoredRun(id) {
  try {
    const stored = sessionStorage.getItem(`eplyx-run-${id}`);
    return stored ? JSON.parse(stored) : null;
  } catch {
    return null;
  }
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
          <p>Reports are shown from the session that submitted them. This browser has no copy of run <span class="mono">${escapeHtml(id)}</span>, and the project token needed to fetch one is not stored after a check runs.</p>
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

function renderReport(live, { demo, id }) {
  const report = live.canonical_report;
  const passed = live.status === 'passed';
  const bundle = report?.bundle;
  const findings = report?.findings ?? [];
  const unmatched = report?.unmatched ?? [];
  const undeclarable = report?.undeclarable ?? [];
  const summary = live.summary ?? {};

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
          <a href="#provenance">Provenance</a>
          <a href="#coverage">Coverage</a>
          ${field('Corpus', bundle?.record_count == null ? null : `${bundle.record_count} observations`, slotLabel(bundle))}
        </aside>
        <div class="report-content">
          <section id="summary"><div class="report-section-title"><span>01</span><h2>Summary</h2></div>
            <div class="metric-row">
              ${metric('Observations', bundle?.record_count, 'validated history')}
              ${metric('Unexpected', summary.unexpected, 'finding groups')}
              ${metric('Expected', summary.expected, 'within bounds')}
              ${metric('Candidate', live.candidate_sha256 ? live.candidate_sha256.slice(0, 8) : null, 'SHA-256', true)}
            </div>
          </section>
          <section id="findings"><div class="report-section-title"><span>02</span><h2>Findings</h2></div>
            ${renderFindings(findings, passed, Boolean(report))}
            ${renderUndeclarable(undeclarable)}
            ${renderUnmatched(unmatched)}
          </section>
          <section id="provenance"><div class="report-section-title"><span>03</span><h2>Provenance</h2></div>
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
  bundle_sha256: 'a18d72bc2f46…3e9c',
  corpus_sha256: 'c253aefc08d1…a901',
  baseline_sha256: '9f3a0d4be24e…8c21',
  candidate_sha256: '60b7e1ac3198…bf14',
  summary: { unexpected: 2, expected: 0 },
  canonical_report: {
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
