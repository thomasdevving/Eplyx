import { readFile } from 'node:fs/promises';

const files = ['index.html','src/app.js','src/landing.js','src/core-scene.js','src/sculpture.js','src/analyse.js','src/report.js','src/styles.css','public/logo.svg'];
const root = new URL('.', import.meta.url);
const contents = await Promise.all(files.map(file => readFile(new URL(file, root), 'utf8')));
const all = contents.join('\n');
const required = ['Know what changes', 'validated historical production', 'Unexpected economic changes detected', 'Evidence, not', '/analyse', '/runs/', 'prefers-reduced-motion'];
for (const text of required) if (!all.includes(text)) throw new Error(`Missing required frontend content: ${text}`);
for (const file of files) console.log(`✓ ${file}`);
console.log('Frontend structure and required product claims verified.');
