import { Header, Footer } from './shell.js';

export function ReportPage(id) {
  const stored = sessionStorage.getItem(`eplyx-run-${id}`);
  const live = stored ? JSON.parse(stored) : null;
  const isDemo = id === 'demo';
  const report = live?.canonical_report;
  const status = live?.status || 'failed';
  const passed = status === 'passed';
  const findings = isDemo ? 2 : (report?.review?.findings?.length ?? live?.summary?.unexpected ?? 0);
  const records = report?.bundle?.record_count ?? 10;
  const slots = report?.bundle?.source_slot_range;
  const slotLabel = slots ? `Slots ${slots.first} → ${slots.last}` : 'Slots 447338102 → 447512884';
  return `<main id="main" class="inner-page report-page">${Header({ light: true })}
    <section class="report-shell">
      <div class="report-top"><div><p class="eyebrow"><span></span> ${isDemo ? 'Public demo report' : `Run ${escapeHtml(id)}`}</p><h1>${passed ? 'Upgrade check passed.' : 'Unexpected economic changes detected.'}</h1><p>${passed ? 'The observed changes matched declarations and stayed within their configured bounds.' : 'Changes were found that are not declared, or that exceed what was declared.'}</p></div><div class="report-verdict ${passed ? 'is-pass' : ''}"><i></i><span>${passed ? 'Passed' : 'Failed'}</span><em>Exit code ${live?.exit_code ?? 1}</em></div></div>
      <div class="report-grid">
        <aside class="report-nav"><span>Report</span><a href="#summary" class="active">Summary</a><a href="#findings">Findings <b>${findings}</b></a><a href="#provenance">Provenance</a><a href="#coverage">Coverage</a><div><span>Corpus</span><strong>${records} observations</strong><small>${slotLabel}</small></div></aside>
        <div class="report-content">
          <section id="summary"><div class="report-section-title"><span>01</span><h2>Summary</h2></div><div class="metric-row"><article><span>Observations</span><strong>${records}</strong><em>validated history</em></article><article><span>Unexpected</span><strong>${isDemo ? 2 : (live?.summary?.unexpected ?? 0)}</strong><em>finding groups</em></article><article><span>Expected</span><strong>${live?.summary?.expected ?? 0}</strong><em>within bounds</em></article><article><span>Candidate</span><strong class="mono">${escapeHtml(live?.candidate_sha256?.slice(0, 8) || '60b7e1ac')}</strong><em>SHA-256</em></article></div></section>
          <section id="findings"><div class="report-section-title"><span>02</span><h2>Findings</h2></div>${!isDemo ? renderLiveFindings(report, passed) : `
            <article class="report-finding critical"><div class="report-finding__head"><span>Critical</span><span>Unexpected</span><b>4 / 4 observations</b></div><h3>WithdrawSol · transaction result changed</h3><p>The baseline succeeds. The candidate returns an execution error for every measurable WithdrawSol observation.</p><div class="outcome-diff"><span><em>Baseline</em><b>Success</b></span><i>→</i><span><em>Candidate</em><b class="bad">Error</b></span></div></article>
            <article class="report-finding high"><div class="report-finding__head"><span>High</span><span>Unexpected</span><b>1 / 6 observations</b></div><h3>DepositSol · pool_tokens_received decreased</h3><p>One referral interaction also changes its invocation graph. The largest measured output change is 21 bps.</p><div class="outcome-diff"><span><em>Baseline</em><b>422,891,634</b></span><i>→</i><span><em>Candidate</em><b class="bad">422,003,562</b></span></div></article>`}</section>
          <section id="provenance"><div class="report-section-title"><span>03</span><h2>Provenance</h2></div><div class="provenance-table"><div><span>Program</span><b>${escapeHtml(report?.bundle?.program_id || 'SPoo1K…akuHy')}</b></div><div><span>Bundle SHA</span><b>${escapeHtml(live?.bundle_sha256 || 'a18d72bc2f46…3e9c')}</b></div><div><span>Corpus SHA</span><b>${escapeHtml(live?.corpus_sha256 || 'c253aefc08d1…a901')}</b></div><div><span>Baseline SHA</span><b>${escapeHtml(live?.baseline_sha256 || '9f3a0d4be24e…8c21')}</b></div><div><span>Candidate SHA</span><b>${escapeHtml(live?.candidate_sha256 || '60b7e1ac3198…bf14')}</b></div><div><span>Source</span><b>Mainnet · fidelity matched</b></div></div></section>
          <section id="coverage"><div class="report-section-title"><span>04</span><h2>Coverage</h2></div><div class="coverage-report">${isDemo ? '<div><span>DepositSol</span><b>6 observations</b></div><div><span>WithdrawSol</span><b>4 observations</b></div>' : renderCoverage(report)}<h3>Coverage limitations</h3>${renderLimitations(report, isDemo)}</div></section>
        </div>
      </div>
    </section>
  </main>${Footer()}`;
}

function renderLiveFindings(report, passed) {
  const rows = report?.review?.findings;
  if (!Array.isArray(rows) || rows.length === 0) {
    return `<div class="empty-result">${passed ? 'No unexpected findings in the tested corpus.' : 'The run failed its gate. Open the canonical JSON or Markdown report for the complete engine output.'}</div>`;
  }
  return rows.map(finding => {
    const severity = String(finding.severity || 'finding');
    const status = String(finding.status || 'unreviewed');
    const affected = Array.isArray(finding.observations) ? finding.observations.length : 0;
    const covered = finding.covered_observations ?? '—';
    const delta = finding.max_relative_delta_bps != null ? `<div class="outcome-diff"><span><em>Largest change</em><b class="bad">${escapeHtml(finding.max_relative_delta_bps)} bps</b></span></div>` : '';
    return `<article class="report-finding ${severity.toLowerCase()}"><div class="report-finding__head"><span>${escapeHtml(severity)}</span><span>${escapeHtml(status)}</span><b>${affected} / ${escapeHtml(covered)} observations</b></div><h3>${escapeHtml(finding.fingerprint || 'Economic change')}</h3><p>${finding.reason ? `Declared: ${escapeHtml(finding.reason)}` : 'Execution evidence differs between the baseline and candidate.'}</p>${delta}</article>`;
  }).join('');
}

function renderCoverage(report) {
  const coverage = report?.coverage;
  if (!Array.isArray(coverage) || coverage.length === 0) return '<div><span>Corpus coverage</span><b>See canonical report</b></div>';
  return coverage.map(row => `<div><span>${escapeHtml(row.subject)}</span><b>${escapeHtml(row.observations)} observations</b></div>`).join('');
}

function renderLimitations(report, isDemo) {
  const limitations = isDemo ? [
    'Jito-tipped DepositSol is underrepresented relative to observed production activity.',
    'Failed historical originals and account-creation paths are unsupported in this corpus.',
    'The selected corpus is validated production-derived evidence; it is not claimed to represent all traffic.'
  ] : (report?.bundle?.limitations || []).map(item => item.detail || item.code);
  if (!limitations.length) return '<p>No additional limitations were supplied by this bundle.</p>';
  return `<ul>${limitations.map(item => `<li>${escapeHtml(item)}</li>`).join('')}</ul>`;
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#039;'}[c]));
}
