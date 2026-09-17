// Functional render tests for the report page.
//
// The page is the last step between a verified result and a person reading it,
// and two defects got through structural checks: it read `report.review.findings`
// while the API sends `findings` at the top level, so real findings rendered as
// none; and it filled missing fields with sample values, so a copied link
// rendered a failed check over evidence that did not exist.
//
// Both are invisible to a "does the file contain this string" check. These
// render the module and assert on the output.
//
// Since runs became asynchronous the page has a second job: it follows a run to
// its end. So the lifecycle is rendered here too, against a stubbed transport —
// what must never happen is that a run still in flight, or one that ended
// without a report, is dressed up as a result.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// The page reads session storage; give it one, and let tests control it.
const store = new Map();
globalThis.sessionStorage = {
  getItem: key => (store.has(key) ? store.get(key) : null),
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: key => store.delete(key)
};

const { ReportPage, attachReport } = await import('./src/report.js');

const reportPath = process.argv[2];
let realReport = null;
if (reportPath) realReport = JSON.parse(readFileSync(reportPath, 'utf8'));

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n       ${error.message.split('\n')[0]}`);
  }
}

async function checkAsync(name, fn) {
  try {
    await fn();
    console.log(`  ok   ${name}`);
  } catch (error) {
    failures += 1;
    console.log(`  FAIL ${name}\n       ${error.message.split('\n')[0]}`);
  }
}

/** Follow one run to whatever the stubbed server says, and return the markup. */
async function follow(id, run, report) {
  store.clear();
  store.set(`eplyx-run-${id}`, JSON.stringify({ api: 'http://api.test', project: 'p', token: 't' }));
  globalThis.fetch = async url => {
    if (String(url).endsWith(`/v1/runs/${id}`)) return { ok: true, status: 200, json: async () => run };
    if (String(url).endsWith('report.json')) {
      return report
        ? { ok: true, status: 200, json: async () => report }
        : { ok: false, status: 409, json: async () => ({ error: 'no report' }) };
    }
    throw new Error(`unexpected request ${url}`);
  };
  let html = '';
  const stop = attachReport(id, markup => { html = markup; });
  await new Promise(resolve => setTimeout(resolve, 30));
  stop();
  return html;
}

// --- an unknown run must not invent evidence ---------------------------
check('an unknown run renders as unavailable, not as a result', () => {
  store.clear();
  const html = ReportPage('audit-not-found');
  assert.match(html, /not available in this browser/);
  assert.doesNotMatch(html, /fidelity matched/i, 'claimed a fidelity it never measured');
  assert.doesNotMatch(html, /Mainnet/, 'claimed a source it never measured');
  assert.doesNotMatch(html, /\b10 observations\b/, 'invented an observation count');
  assert.doesNotMatch(html, /447\d{6}/, 'invented a slot window');
  assert.doesNotMatch(html, /60b7e1ac/, 'invented a candidate hash');
  assert.doesNotMatch(html, /Exit code 1/, 'invented a verdict');
});

check('storage being unavailable does not fabricate a run', () => {
  const saved = globalThis.sessionStorage;
  globalThis.sessionStorage = { getItem() { throw new Error('blocked'); } };
  const html = ReportPage('some-run');
  globalThis.sessionStorage = saved;
  assert.match(html, /not available in this browser/);
});

// --- the demo stays a demo --------------------------------------------
check('the demo route renders and is labelled a fixture', () => {
  const html = ReportPage('demo');
  assert.match(html, /fixture/i);
  assert.match(html, /pool_tokens_received/);
});

// --- a run in flight is not a result -----------------------------------
check('a followable run opens on its lifecycle, not on a verdict', () => {
  store.clear();
  store.set('eplyx-run-r0', JSON.stringify({ api: 'http://api.test', project: 'p', token: 't' }));
  const html = ReportPage('r0');
  // Not "queued": before the first poll answers, the state is unknown, and a
  // completed run reloaded from a link must not flash a queue it left long ago.
  assert.match(html, /Loading this run/, 'announced a state it had not asked about');
  assert.doesNotMatch(html, /execution slot/, 'claimed queued before asking');
  assert.doesNotMatch(html, /Exit code/, 'invented a verdict before any run');
  assert.doesNotMatch(html, /60b7e1ac/, 'demo candidate hash leaked');
  assert.doesNotMatch(html, /a18d72bc/, 'demo bundle hash leaked');
});

await checkAsync('a queued run reports no verdict and no report', async () => {
  const html = await follow('r1', {
    run_id: 'r1', status: 'queued', exit_code: null, report_available: false,
    bundle_sha256: 'bbb', candidate_sha256: 'ccc'
  });
  assert.match(html, /execution slot/);
  assert.match(html, /No result yet/);
  assert.doesNotMatch(html, /Exit code/, 'invented a verdict');
  assert.doesNotMatch(html, /a18d72bc/, 'demo bundle hash leaked');
  assert.doesNotMatch(html, /\d+\s*%/, 'invented progress');
});

await checkAsync('a running run says so without claiming progress', async () => {
  const html = await follow('r2', {
    run_id: 'r2', status: 'running', exit_code: null, report_available: false,
    bundle_sha256: 'bbb', candidate_sha256: 'ccc'
  });
  assert.match(html, /Replaying/);
  assert.doesNotMatch(html, /\d+\s*%/, 'invented progress');
  assert.doesNotMatch(html, /\b\d+ \/ \d+ records\b/, 'invented per-record progress');
});

await checkAsync('an execution error does not blame the candidate', async () => {
  const html = await follow('r3', {
    run_id: 'r3', status: 'execution_error', exit_code: null, report_available: false,
    detail: 'Run interrupted by server restart; resubmit the check.',
    bundle_sha256: 'bbb', candidate_sha256: 'ccc'
  });
  assert.match(html, /could not complete/i);
  assert.match(html, /interrupted by server restart/);
  assert.doesNotMatch(html, /Exit code/, 'showed a gate code where none exists');
  assert.doesNotMatch(html, /economic changes detected/i, 'reported findings it never had');
});

await checkAsync('a preflight abort keeps its exit code and claims no findings', async () => {
  const html = await follow('r4', {
    run_id: 'r4', status: 'failed', exit_code: 4, report_available: false,
    detail: 'bundle was built under another adapter version',
    bundle_sha256: 'bbb', candidate_sha256: 'ccc'
  });
  assert.match(html, /Exit code 4/);
  assert.match(html, /another adapter version/);
  assert.doesNotMatch(html, /economic changes detected/i, 'reported findings it never had');
});

await checkAsync('demo values never leak into a real run', async () => {
  const html = await follow(
    'r5',
    { run_id: 'r5', status: 'failed', exit_code: 1, report_available: true },
    { summary: { passed: false, exit_code: 1 }, findings: [], undeclarable: [], unmatched: [], coverage: [] }
  );
  assert.doesNotMatch(html, /60b7e1ac/, 'demo candidate hash leaked');
  assert.doesNotMatch(html, /a18d72bc/, 'demo bundle hash leaked');
  assert.doesNotMatch(html, /447\d{6}/, 'demo slot window leaked');
  assert.match(html, /—/, 'missing values should render as an explicit dash');
});

// --- the real serialized contract -------------------------------------
if (realReport) {
  const run = {
    run_id: 'real', status: 'failed', report_available: true,
    exit_code: realReport.summary.exit_code,
    candidate_sha256: realReport.candidate.sha256,
    bundle_sha256: realReport.bundle.sha256
  };

  await checkAsync('real findings from a real report render as cards', async () => {
    const html = await follow('real', run, realReport);
    assert.ok(realReport.findings.length > 0, 'fixture report should carry findings');
    for (const finding of realReport.findings) {
      assert.ok(html.includes(finding.fingerprint), `missing finding ${finding.fingerprint}`);
    }
  });

  await checkAsync('undeclarable changes are rendered, not dropped', async () => {
    const html = await follow('real', run, realReport);
    assert.ok(realReport.undeclarable.length > 0, 'fixture report should carry undeclarable');
    assert.match(html, /cannot be declared/);
    for (const change of realReport.undeclarable) {
      assert.ok(html.includes(change.description), `missing ${change.description}`);
    }
  });

  await checkAsync('coverage and limitations reach the page', async () => {
    const html = await follow('real', run, realReport);
    for (const row of realReport.coverage) assert.ok(html.includes(row.subject));
    for (const limit of realReport.bundle.limitations) {
      assert.ok(html.includes(limit.detail.slice(0, 40)), `missing limitation ${limit.code}`);
    }
  });
} else {
  console.log('  ..   real-report checks skipped (no report path given)');
}

console.log(failures === 0 ? '\nfrontend report rendering verified' : `\n${failures} rendering failures`);
process.exit(failures === 0 ? 0 : 1);
