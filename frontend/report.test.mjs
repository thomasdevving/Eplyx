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

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// The page reads session storage; give it one, and let tests control it.
const store = new Map();
globalThis.sessionStorage = {
  getItem: key => (store.has(key) ? store.get(key) : null),
  setItem: (key, value) => store.set(key, String(value)),
  removeItem: key => store.delete(key)
};

const { ReportPage } = await import('./src/report.js');

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

check('demo values never leak into a real run', () => {
  store.clear();
  store.set('eplyx-run-r1', JSON.stringify({ status: 'passed', exit_code: 0, summary: {} }));
  const html = ReportPage('r1');
  assert.doesNotMatch(html, /60b7e1ac/, 'demo candidate hash leaked');
  assert.doesNotMatch(html, /a18d72bc/, 'demo bundle hash leaked');
  assert.match(html, /—/, 'missing values should render as an explicit dash');
});

// --- the real serialized contract -------------------------------------
if (realReport) {
  check('real findings from a real report render as cards', () => {
    store.clear();
    store.set('eplyx-run-r2', JSON.stringify({
      status: 'failed',
      exit_code: realReport.summary.exit_code,
      candidate_sha256: realReport.candidate.sha256,
      summary: realReport.summary,
      canonical_report: realReport
    }));
    const html = ReportPage('r2');
    assert.ok(realReport.findings.length > 0, 'fixture report should carry findings');
    for (const finding of realReport.findings) {
      assert.ok(html.includes(finding.fingerprint), `missing finding ${finding.fingerprint}`);
    }
  });

  check('undeclarable changes are rendered, not dropped', () => {
    const html = ReportPage('r2');
    assert.ok(realReport.undeclarable.length > 0, 'fixture report should carry undeclarable');
    assert.match(html, /cannot be declared/);
    for (const change of realReport.undeclarable) {
      assert.ok(html.includes(change.description), `missing ${change.description}`);
    }
  });

  check('coverage and limitations reach the page', () => {
    const html = ReportPage('r2');
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
