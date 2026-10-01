// A public saved result must retain its real identities, coverage and missing evidence.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { DEMO_REPORT } from './src/demo.js';
import { LandingPage } from './src/landing.js';
import { ReportPage } from './src/report.js';
const archive = JSON.parse(readFileSync(new URL('../docs/examples/phase-u3a-validation/reports/regression.json', import.meta.url)));
assert.deepEqual(DEMO_REPORT, archive, 'public demo drifted from the preserved canonical report');
globalThis.matchMedia = () => ({ matches: true });
const landing = LandingPage();
assert.match(landing, /1 economic decrease · 9 reverts/);
assert.match(landing, /9 \/ 9/);
assert.match(landing, /1 \/ 1/);
const demo = ReportPage('demo');
for (const identity of [archive.bundle.sha256, archive.bundle.corpus_sha256, archive.bundle.baseline_sha256, archive.candidate.sha256]) assert.ok(demo.includes(identity), `missing retained identity ${identity}`);
assert.match(demo, /predates ChangeSpec identities/);
assert.match(demo, /contains no replay-proof section/);
assert.doesNotMatch(demo, /60b7e1ac|5c1f0e9ad27b44c08/);
// The shared hosted report imports this module too. It must be embedded by its server.
const hosted = readFileSync(new URL('../server/src/cloud/web.rs', import.meta.url), 'utf8');
assert.match(hosted, /"demo.js" => .*frontend!\("src\/demo.js"\)/);
console.log('Public demo equals archived evidence; landing, report and hosted asset binding agree.');
