import { readFile } from 'node:fs/promises';

const files = ['index.html','src/app.js','src/landing.js','src/session.js','src/projects.js','src/brand.js','src/intro.js','src/core-scene.js','src/sculpture.js','src/analyse.js','src/report.js','src/change.js','src/styles.css','public/logo.svg'];
const root = new URL('.', import.meta.url);
const contents = await Promise.all(files.map(file => readFile(new URL(file, root), 'utf8')));
const all = contents.join('\n');
const required = ['Know what changes', 'validated historical production', 'Unexpected economic changes detected', 'Evidence, not', '/analyse', '/runs/', 'prefers-reduced-motion'];
for (const text of required) if (!all.includes(text)) throw new Error(`Missing required frontend content: ${text}`);
// The mark is inlined for WebKit, so the two copies must stay identical.
const logo = contents[files.indexOf('public/logo.svg')];
const brand = contents[files.indexOf('src/brand.js')];
for (const id of ['crescent', 'wave']) {
  const path = logo.match(new RegExp(`id="${id}"[^>]*\\sd="([^"]+)"`))?.[1];
  if (!path) throw new Error(`public/logo.svg has no #${id} path`);
  if (!brand.includes(path)) throw new Error(`src/brand.js has drifted from public/logo.svg for #${id}`);
}

// The page must not carry sample evidence it can fall back to. Real runs and
// the demo are different things, and a fixture that leaks into a real render is
// how "10 observations · Mainnet · fidelity matched" appeared for a run that did
// not exist.
const report = contents[files.indexOf('src/report.js')];
const demoStart = report.indexOf('const DEMO =');
if (demoStart < 0) throw new Error('src/report.js has no explicitly named demo fixture');
// Comments explain these strings; only executable code may contain them.
const live = report.slice(0, demoStart).replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
for (const invented of ['fidelity matched', 'Mainnet ·', '447338102', '60b7e1ac']) {
  if (live.includes(invented)) throw new Error(`src/report.js falls back to invented evidence: ${invented}`);
}
if (live.includes('review?.findings')) throw new Error('src/report.js reads the pre-flattened report shape');

for (const file of files) console.log(`✓ ${file}`);
console.log('Frontend structure and required product claims verified.');
