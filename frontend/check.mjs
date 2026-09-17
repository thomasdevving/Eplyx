import { readFile } from 'node:fs/promises';

const files = ['index.html','src/app.js','src/landing.js','src/brand.js','src/intro.js','src/core-scene.js','src/sculpture.js','src/analyse.js','src/report.js','src/styles.css','public/logo.svg'];
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

for (const file of files) console.log(`✓ ${file}`);
console.log('Frontend structure and required product claims verified.');
