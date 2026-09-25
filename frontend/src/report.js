import { Header, Footer } from './shell.js';
import { API_BASE, operatorToken } from './session.js';
import { ChangeCard, ChangeIdentityRows, resolveChange } from './change.js';
import { analysisView, shortId } from './analysis.js';

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
  const [label, title, stateNote] = LIFECYCLE[run.status] ?? LIFECYCLE.loading;
  // A run the server resumed after a restart is the same run: same id, same
  // change, same inputs. Saying so explains a second attempt without implying a
  // second run.
  const interrupted = Array.isArray(run.attempts) && run.attempts.some(attempt => attempt.end === 'interrupted');
  const note = interrupted ? `${stateNote} Resumed after a server restart; this is the same run.` : stateNote;
  const known = run.status !== 'loading';
  return shell(id, `
      <div class="report-top">
        <div>
          <p class="eyebrow"><span></span> Run ${escapeHtml(id)}</p>
          <h1>${escapeHtml(title)}</h1>
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
 * Three different things land here and none is a finding. A preflight abort
 * is Eplyx answering — exit 4 means the change does not fit the pinned bundle,
 * exit 2 that no verified execution was produced. An execution error is this
 * service failing to answer at all, and blames nothing about the candidate.
 */
function renderIncomplete(id, run, extras = {}) {
  const view = analysisView({ run, report: null });
  return shell(id, `
      ${top(id, view, { exit: run.exit_code, gate: null })}
      ${dimensions(view)}
      <div class="report-content">
        ${ChangeCard({ change: run.change, legacy: !run.change, targetName: extras.projectName, spec: extras.spec, candidateSha: run.candidate_sha256 })}
        <section><div class="report-section-title"><span>01</span><h2>Reason</h2></div>
          <div class="empty-result"><p>${run.detail ? escapeHtml(run.detail) : 'No further detail was recorded.'}</p></div>
        </section>
        <section data-technical><div class="report-section-title"><span>02</span><h2>Inputs</h2></div>
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

/** The headline block. The verdict box names the state, then the gate's code. */
function top(id, view, { exit, gate, demo = false }) {
  const code = exit == null ? 'No exit code' : `Exit code ${escapeHtml(exit)}`;
  const gateText = gate == null ? '' : ` · CI gate ${gate ? 'Passed' : 'Failed'}`;
  return `<div class="report-top">
        <div>
          <p class="eyebrow"><span></span> ${demo ? 'Public demo report · fixture' : `Run ${escapeHtml(id)}`}</p>
          <h1>${escapeHtml(view.headline.title)}</h1>
          <p>${escapeHtml(view.headline.note)}</p>
        </div>
        <div class="report-verdict ${escapeHtml(view.headline.tone)}"><i></i><span>${escapeHtml(view.headline.label)}</span><em>${code}${gateText}</em></div>
      </div>`;
}

/** Separate confidence dimensions, never one badge. */
function dimensions(view) {
  if (!view.dimensions?.length) return '';
  return `<div class="analysis-dimensions" aria-label="Analysis dimensions">${view.dimensions.map(item => `
        <div class="analysis-dimension is-${escapeHtml(item.tone)}">
          <span>${escapeHtml(item.label)}</span>
          <strong>${escapeHtml(item.value)}</strong>
          ${item.note ? `<em>${escapeHtml(item.note)}</em>` : ''}
        </div>`).join('')}
      </div>`;
}

/**
 * The three records of what a run analysed disagree. There is no result to
 * show: a result about some other proposal is not a result about this one.
 */
function renderIdentityConflict(id, live, resolution) {
  const view = analysisView({ run: live, resolution });
  return shell(id, `
      ${top(id, view, { exit: null, gate: null }).replace('No exit code', 'Identity mismatch')}
      <div class="report-content">
        <section><div class="report-section-title"><span>01</span><h2>Mismatch</h2></div>
          <div class="empty-result is-error">${view.conflicts.map(c => `<p class="mono">${escapeHtml(c)}</p>`).join('')}</div>
        </section>
        <section data-technical><div class="report-section-title"><span>02</span><h2>Inputs</h2></div>
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
  const view = analysisView({ run: live, report });
  if (!report) {
    // The run says a report exists, and this page could not fetch it.
    return shell(id, `
      ${top(id, view, { exit: live.exit_code, gate: null, demo })}
      ${dimensions(view)}
      <div class="report-content">
        ${ChangeCard({ change: resolution.change, legacy: resolution.legacy, targetName: extras.projectName, spec: extras.spec, candidateSha: live.candidate_sha256 })}
        <div class="empty-result">The canonical report could not be retrieved for this run, so its findings are not shown here. Fetch <span class="mono">report.json</span> with the project token.</div>
      </div>`);
  }

  return `<main id="main" class="inner-page report-page">${Header({ light: true })}
    <section class="report-shell">
      ${top(id, view, { exit: live.exit_code ?? view.gate.exitCode, gate: view.gate.passed, demo })}
      ${dimensions(view)}
      <div class="report-grid">
        <aside class="report-nav"><span>Analysis</span>
          <a href="#change" class="active">Proposed change</a>
          <a href="#result">Result</a>
          <a href="#impact">Impact</a>
          <a href="#unresolved">Not explained <b>${view.unexplained.rows.length + view.declarations.length}</b></a>
          <a href="#verification">Verification</a>
          <a href="#technical">Technical details</a>
          ${field('Corpus', view.corpus.total == null ? null : `${view.corpus.total} ${view.corpus.total === 1 ? 'interaction' : 'interactions'}`, slotLabel(view.corpus.slots))}
        </aside>
        <div class="report-content">
          <section id="change"><div class="report-section-title"><span>01</span><h2>Proposed change</h2></div>
            ${ChangeCard({ change: resolution.change, legacy: resolution.legacy, targetName: extras.projectName, spec: extras.spec, candidateSha: live.candidate_sha256 })}
          </section>
          <section id="result"><div class="report-section-title"><span>02</span><h2>Result</h2></div>
            ${renderPrimary(view)}
            ${renderResult(view)}
          </section>
          <section id="impact"><div class="report-section-title"><span>03</span><h2>Impact</h2></div>
            ${view.notEvaluated ? renderNotEvaluated(view) : renderImpact(view)}
            ${renderAffected(view)}
          </section>
          <section id="unresolved"><div class="report-section-title"><span>04</span><h2>Not explained or not evaluated</h2></div>
            ${renderUnexplained(view)}
            ${renderDeclarations(view)}
            <h3 class="analysis-subhead">What this corpus does not cover</h3>
            ${renderLimitations(view.limitations)}
          </section>
          <section id="verification"><div class="report-section-title"><span>05</span><h2>Verification</h2></div>
            ${renderProof(view)}
          </section>
          <section id="technical" data-technical><div class="report-section-title"><span>06</span><h2>Technical details</h2></div>
            ${renderTechnical({ live, report, resolution, extras })}
          </section>
        </div>
      </div>
    </section>
  </main>${Footer()}`;
}

/** An execution failure leads the page, above any economic card. */
function renderPrimary(view) {
  if (!view.primary.length) return '';
  return view.primary.map(item => `<article class="primary-alert" role="alert">
      <span>${escapeHtml(item.statusLabel)}${item.action ? ` · ${escapeHtml(item.action)}` : ''}</span>
      <h3>${escapeHtml(item.title)}</h3>
      <p>${escapeHtml(item.note)}</p>
    </article>`).join('');
}

function renderResult(view) {
  const lines = [...view.proof.statements.slice(0, 1)];
  if (view.notEvaluated) lines.push('Economic impact was not evaluated: Eplyx has no semantic coverage for this interaction.');
  else if (view.evaluated) {
    lines.push(`Economic impact was evaluated for ${view.groups.reduce((n, group) => n + group.subjects.length, 0)} named subjects${view.primary.some(item => item.kind === 'now_reverts') ? '; economic values are not compared where the proposed build fails' : ''}.`);
  }
  const reasons = view.gate.reasons.length
    ? `<h3 class="analysis-subhead">Why the CI gate did not pass</h3><ul class="plain-list">${view.gate.reasons.map(reason => `<li>${escapeHtml(reason.text)}</li>`).join('')}</ul>`
    : '<p class="analysis-note">The CI gate passed. That is a statement about the declarations and the evaluated subjects, not a deployment approval.</p>';
  return `<ul class="plain-list">${lines.map(line => `<li>${escapeHtml(line)}</li>`).join('')}</ul>${reasons}`;
}

/** Keep the P2 wording, and say what the reader can still do. */
function renderNotEvaluated(view) {
  return `<div class="not-evaluated">
      <h3>Economic impact could not be evaluated for this interaction.</h3>
      <ul class="plain-list">
        <li>Execution ${view.dimensions[0]?.value === 'Verified' ? 'was verified: the historical replay reproduced before the proposed build ran.' : 'was not verified.'}</li>
        <li>Eplyx has no semantic coverage for this interaction, so it cannot say whether balances, positions or other economic outcomes changed. The absence of findings here is not evidence of no impact.</li>
        <li>${view.unexplained.rows.length ? `State differences it detected are listed under <a href="#unresolved" class="text-link">Not explained</a>.` : 'No other state difference was detected.'} The raw evidence is under <a href="#technical" class="text-link">Technical details</a>.</li>
      </ul>
      <p class="analysis-note">Next step: a protocol adapter with semantic coverage for this program and interaction is needed before its economic impact can be evaluated.</p>
    </div>`;
}

function renderImpact(view) {
  if (!view.groups.length) return '<div class="empty-result">No subjects were evaluated in this report.</div>';
  return view.groups.map(group => `<article class="impact-group">
      <header><span>${escapeHtml(group.protocol)}</span><h3>${escapeHtml(group.title)}</h3></header>
      ${group.subjects.map(subject => subject.cards.length
        ? subject.cards.map(renderCard).join('')
        : `<div class="subject-row is-${escapeHtml(subject.state)}"><b>${escapeHtml(subject.title)}</b><span>${escapeHtml(subject.stateLabel)}</span><em>${subject.measured == null ? '' : `measured in ${escapeHtml(subject.measured)} ${subject.measured === 1 ? 'interaction' : 'interactions'}`}</em></div>`).join('')}
    </article>`).join('');
}

function renderCard(card) {
  const values = card.values.length ? `<div class="value-table" role="table">
        <div role="row" class="value-table__head"><span role="columnheader">Before</span><span role="columnheader">Proposed</span><span role="columnheader">Relative change</span></div>
        ${card.values.slice(0, 5).map(entry => `<div role="row">
          <b role="cell" title="${escapeHtml(entry.before?.full ?? '')}">${escapeHtml(entry.before?.text ?? 'not measured')}</b>
          <b role="cell" title="${escapeHtml(entry.proposed?.full ?? '')}">${escapeHtml(entry.proposed?.text ?? 'not measured')}</b>
          <b role="cell">${escapeHtml(entry.relative ?? 'not defined')}</b>
        </div>`).join('')}
        ${card.values.length > 5 ? `<p class="analysis-note">${card.values.length - 5} more in the technical details.</p>` : ''}
      </div>` : card.largest ? `<div class="value-table"><div><span>Largest relative change</span><b>${escapeHtml(card.largest)}</b></div></div>` : '';
  return `<div class="impact-card is-${escapeHtml(card.tone)}">
      <div class="impact-card__head"><span class="tag">${escapeHtml(card.statusLabel)}</span><em>${escapeHtml(card.reach)}</em></div>
      <h4>${escapeHtml(card.title)}</h4>
      <p>${card.reason ? `Declared: ${escapeHtml(card.reason)}` : escapeHtml(card.statusNote)}</p>
      ${values}
      ${card.breaches.map(text => `<p class="analysis-note">${escapeHtml(text)}</p>`).join('')}
    </div>`;
}

function renderAffected(view) {
  const a = view.affected;
  // Without semantic coverage a count of zero would read as "no impact".
  if (view.notEvaluated && !view.unexplained.rows.length) return '';
  const items = [
    `<div><span>${view.notEvaluated ? 'Interactions with a detected state difference' : 'Interactions with a detected change'}</span><b>${a.interactions.total == null ? escapeHtml(a.interactions.affected) : `${escapeHtml(a.interactions.affected)} of ${escapeHtml(a.interactions.total)}`}</b></div>`,
  ];
  if (a.changedSubjects.length) items.push(`<div><span>Named subjects that changed</span><b>${a.changedSubjects.map(escapeHtml).join(', ')}</b></div>`);
  if (a.entities.length) items.push(`<div><span>Economic entities named</span><b>${escapeHtml(a.entities.length)}</b></div>`);
  if (a.accounts.length) items.push(`<div><span>Accounts with unexplained changes</span><b>${escapeHtml(a.accounts.length)} · ${a.accounts.map(escapeHtml).join(', ')}</b></div>`);
  return `<h3 class="analysis-subhead">What is affected</h3><div class="affected-list">${items.join('')}</div>`;
}

function renderUnexplained(view) {
  const rows = view.unexplained.rows;
  if (!rows.length) return '<p class="analysis-note">Eplyx reported no state change outside what it named.</p>';
  const describe = item => item.scope === 'account'
    ? `<b class="mono">${escapeHtml(item.account)}</b> ${escapeHtml(item.what)}`
    : item.scope === 'evaluation' ? `Semantic evaluation unavailable: ${escapeHtml(item.what)}` : escapeHtml(item.what);
  const list = items => items.map(item => `<li>${describe(item)}<em>${escapeHtml(item.observations.length)} ${item.observations.length === 1 ? 'interaction' : 'interactions'}</em></li>`).join('');
  const structural = view.unexplained.structural;
  const decoded = view.unexplained.decoded;
  return `<div class="unexplained">
      <h3>Additional state changed that Eplyx could not semantically explain.</h3>
      <p>${escapeHtml(rows.length)} ${rows.length === 1 ? 'change' : 'changes'}${view.unexplained.accounts.length ? ` across ${escapeHtml(view.unexplained.accounts.length)} ${view.unexplained.accounts.length === 1 ? 'account' : 'accounts'}` : ''}. Eplyx does not know what these mean, and does not treat them as harmless. They cannot be declared in expected-changes.toml until the subject is promoted. <a href="#technical" class="text-link">Technical details</a></p>
      ${structural.length ? `<ul class="unexplained-list">${list(structural)}</ul>` : ''}
      ${decoded.length ? `<h4>Decoded values no named finding speaks for</h4><ul class="unexplained-list">${list(decoded)}</ul>` : ''}
    </div>`;
}

function renderDeclarations(view) {
  if (!view.declarations.length) return '';
  return `<h3 class="analysis-subhead">Declarations that matched nothing</h3>${view.declarations.map(item => `<div class="impact-card is-warn">
      <div class="impact-card__head"><span class="tag">${escapeHtml(item.statusLabel)}</span><em>${escapeHtml(item.covered)} can measure it</em></div>
      <h4>${escapeHtml(item.title)}</h4>
      <p>${escapeHtml(item.statusNote)}${item.reason ? ` Declared: ${escapeHtml(item.reason)}` : ''}</p>
    </div>`).join('')}`;
}

function renderLimitations(limitations) {
  if (!limitations.length) return '<p class="analysis-note">This bundle reported no limitations. That is unusual; check the canonical report.</p>';
  return `<ul class="plain-list">${limitations.map(item => `<li>${escapeHtml(item)}</li>`).join('')}</ul>`;
}

/** The proof statement, then how the interpretation is known. */
function renderProof(view) {
  const proof = view.proof;
  const provenance = proof.provenance;
  const how = [
    'Deterministic comparison: the current and proposed builds executed the same historical interactions from identical state, and their results were compared.',
    proof.adapter ? `Interpretation: ${proof.adapter}.` : 'Interpretation: none. This bundle carries no semantic interpreter.',
  ];
  if (provenance) {
    how.push(`Provenance tier: ${provenance.levels.join(', ')}.`);
    for (const source of provenance.sources) {
      how.push(source.repository
        ? `Protocol interface source: ${source.repository} at ${shortId(source.commit, 12)} (${source.files} pinned ${source.files === 1 ? 'file' : 'files'}).`
        : `Standard program: ${source.program}.`);
    }
    how.push(`Exact build verified: ${provenance.exact === true ? 'yes' : provenance.exact === false ? 'no' : 'not stated'}.`);
    if (provenance.facts.length) how.push(`Corroborated by the historical execution: ${provenance.facts.join(', ').toLowerCase()}.`);
  }
  return `<ul class="plain-list proof-list">${[...proof.statements, ...proof.qualifiers].map(line => `<li>${escapeHtml(line)}</li>`).join('')}</ul>
    ${view.evaluated || provenance ? `<details class="how-known"><summary>How does Eplyx know this?</summary><ul class="plain-list">${how.map(line => `<li>${escapeHtml(line)}</li>`).join('')}</ul></details>` : ''}`;
}

/** Everything, in full, one layer down. Nothing technical was removed. */
function renderTechnical({ live, report, resolution, extras }) {
  const bundle = report.bundle ?? {};
  const replay = report.replay_proof;
  const binding = report.semantic_binding;
  const findings = report.findings ?? [];
  const undeclarable = report.undeclarable ?? [];
  const unmatched = report.unmatched ?? [];
  const reasons = Array.isArray(report.summary?.failure_reasons) ? report.summary.failure_reasons : report.failures ?? [];
  const bindingRows = (binding?.observations ?? []).map(entry => {
    const b = entry.binding ?? {};
    return [
      row('Observation', entry.observation_id),
      row('Level', b.level),
      b.source ? row('Repository', `${b.source.repository} @ ${b.source.commit}`) : '',
      ...(b.source?.source_blobs ?? []).map(blob => row('Source blob', `${blob.path} (${blob.git_blob_sha1})`)),
      b.facts ? row('Corroborated facts', b.facts.join(', ')) : '',
      b.historical_elf_sha256 ? row('Historical ELF SHA', b.historical_elf_sha256) : '',
      b.execution_evidence_sha256 ? row('Execution evidence SHA', b.execution_evidence_sha256) : '',
      b.program_id ? row('Program', b.program_id) : '',
    ].join('');
  }).join('');
  return `<details class="technical"><summary>Show identities, proof and raw evidence</summary>
      <h3 class="provenance-heading">Proposed change</h3>
      <div class="provenance-table">${ChangeIdentityRows({ change: resolution.change, spec: extras.spec, resolution })}</div>
      <h3 class="provenance-heading">Evidence</h3>
      <div class="provenance-table">
        ${row('Program', bundle.program_id)}
        ${row('Bundle SHA', live.bundle_sha256 ?? bundle.sha256)}
        ${row('Corpus SHA', live.corpus_sha256 ?? bundle.corpus_sha256)}
        ${row('Baseline SHA', live.baseline_sha256 ?? bundle.baseline_sha256)}
        ${row('Candidate SHA', live.candidate_sha256 ?? report.candidate?.sha256)}
        ${row('Adapter', bundle.adapter ? `${bundle.adapter} v${bundle.adapter_version} · semantic schema v${bundle.semantic_schema_version}` : null)}
        ${row('Source slots', bundle.source_slot_range ? `${bundle.source_slot_range.first} → ${bundle.source_slot_range.last}` : null)}
      </div>
      <h3 class="provenance-heading">Replay proof</h3>
      <div class="provenance-table">${replay ? [
        row('Fidelity profile', replay.profile),
        row('Status', replay.status),
        row('Observations', replay.observations),
        row('Proof contract', replay.proof_contract_versions?.length ? replay.proof_contract_versions.map(v => `contract ${v}`).join(', ') : 'contract 1 (not listed)'),
        replay.boundary_proof ? row('Boundary proof', replay.boundary_proof) : '',
      ].join('') : row('Replay proof', null, 'not reported: historical replay fidelity gate only')}</div>
      <h3 class="provenance-heading">SemanticBinding</h3>
      <div class="provenance-table">${binding ? `${row('exact_source_to_elf_verified', String(binding.exact_source_to_elf_verified))}${bindingRows}` : row('SemanticBinding', null, 'not reported')}</div>
      <h3 class="provenance-heading">CI gate</h3>
      <div class="provenance-table">
        ${row('Exit code', report.summary?.exit_code ?? live.exit_code)}
        ${reasons.length ? reasons.map(reason => `<div><span class="mono">${escapeHtml(reason)}</span><b>${escapeHtml(REASON_TEXT[reason] ?? 'See the canonical report.')}</b></div>`).join('') : row('Failure reasons', 'none')}
      </div>
      <h3 class="provenance-heading">Findings</h3>
      ${findings.length ? findings.map(finding => `<div class="provenance-table raw-finding">
        ${row('Fingerprint', finding.fingerprint)}
        ${row('Severity', finding.severity)}
        ${row('Status', finding.status)}
        ${row('Observations', `${(finding.observations ?? []).length} of ${finding.covered_observations ?? '—'}: ${(finding.observations ?? []).join(', ')}`)}
        ${finding.entities?.length ? row('Entities', finding.entities.join(', ')) : ''}
        ${finding.max_relative_delta_bps != null ? row('max_relative_delta_bps', finding.max_relative_delta_bps) : ''}
        ${(finding.values ?? []).map(entry => row('Values', `${entry.observation_id}: ${JSON.stringify(entry.baseline ?? null)} → ${JSON.stringify(entry.candidate ?? null)}${entry.relative_delta_bps != null ? ` (${entry.relative_delta_bps} bps)` : ''}`)).join('')}
        ${finding.reason ? row('Declared reason', finding.reason) : ''}
        ${finding.breaches?.length ? row('Breaches', JSON.stringify(finding.breaches)) : ''}
        ${finding.unevaluable ? row('Unevaluable', JSON.stringify(finding.unevaluable)) : ''}
      </div>`).join('') : '<p class="analysis-note">No findings in the report.</p>'}
      <h3 class="provenance-heading">Changes that cannot be declared</h3>
      ${undeclarable.length ? `<div class="provenance-table">${undeclarable.map(change => `<div><span class="mono">${escapeHtml(change.layer)}</span><b>${escapeHtml(change.description)} — ${escapeHtml((change.observations ?? []).join(', '))}</b></div>`).join('')}</div>` : '<p class="analysis-note">None.</p>'}
      ${unmatched.length ? `<h3 class="provenance-heading">Unmatched declarations</h3><div class="provenance-table">${unmatched.map(item => `<div><span class="mono">${escapeHtml(item.status)}</span><b>${escapeHtml(item.fingerprint)} — ${escapeHtml(item.reason ?? '')}</b></div>`).join('')}</div>` : ''}
      <h3 class="provenance-heading">Semantic coverage</h3>
      <div class="provenance-table">${(report.coverage ?? []).length ? report.coverage.map(item => `<div><span class="mono">${escapeHtml(item.subject)}</span><b>${escapeHtml(item.observations)} observations</b></div>`).join('') : row('Semantic coverage', null, 'none reported')}</div>
    </details>`;
}

/** A value, or an explicit statement of its absence. Never a stand-in. */
function row(label, value, absent = 'not reported') {
  return `<div><span>${escapeHtml(label)}</span><b>${value == null ? `<em>${escapeHtml(absent)}</em>` : escapeHtml(value)}</b></div>`;
}

function field(label, value, note) {
  if (value == null) return '';
  return `<div><span>${escapeHtml(label)}</span><strong>${escapeHtml(value)}</strong>${note ? `<small>${escapeHtml(note)}</small>` : ''}</div>`;
}

function slotLabel(slots) {
  return slots ? `Slots ${slots.first} → ${slots.last}` : null;
}

/** The gate's own reasons, by their technical names, for the details. */
const REASON_TEXT = {
  no_semantic_coverage: 'No semantic coverage: nothing in this corpus could be evaluated economically, so a pass would mean “we did not look”.',
  undeclarable_change: 'A detected change that no expectation can name.',
  undeclared_change: 'A change nothing declared, or one larger than declared.',
  stale_expectation: 'A declaration for behaviour that no longer happens.',
  unevaluable_expectation: 'A declaration this corpus cannot judge.',
};

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
      {
        severity: 'HIGH', status: 'unexpected', fingerprint: 'spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased', observations: ['e'], covered_observations: 6, max_relative_delta_bps: -21,
        values: [{ observation_id: 'e', baseline: { kind: 'quantity', quantity: '10.412337901' }, candidate: { kind: 'quantity', quantity: '10.390472189' }, relative_delta_bps: -21 }]
      }
    ],
    undeclarable: [
      { layer: 'decoded_economic', description: 'manager-fee amount', observations: ['a', 'b', 'c', 'd', 'e'] }
    ],
    unmatched: []
  }
};
