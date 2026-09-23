#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = 'docs/examples/phase-u13-2b-feature-universe';
const manifest = path.join(root, 'checksums.json');
const extra = [
  'docs/phase-u13-2b-historical-feature-set-runtime-binding.md',
  'scripts/acquire-u13-2b-missing-features.mjs',
  'scripts/analyze-u13-2b-feature-universe.cjs',
  'scripts/audit-u13-2b-feature-universe.cjs',
  'scripts/check-u13-2b-compatibility.cjs',
  'scripts/check-u13-2b-u10-mutations.cjs',
  'scripts/check-u13-2b-checksums.cjs',
  'engine/src/universal/historical_features.rs',
  'engine/examples/u13_2b_feasibility.rs',
  'engine/tests/historical_feature_set.rs',
];
const files = [...fs.readdirSync(root, { withFileTypes: true })
  .flatMap(entry => entry.isDirectory() ? fs.readdirSync(path.join(root, entry.name))
    .map(name => path.join(root, entry.name, name)) : [path.join(root, entry.name)])
  .filter(file => file !== manifest), ...extra].map(file => file.replaceAll('\\', '/')).sort();
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const entries = files.map(file => ({ path: file, bytes: fs.statSync(file).size,
  sha256: sha(fs.readFileSync(file)) }));
if (process.argv.includes('--write')) {
  if (fs.existsSync(manifest)) throw Error('checksum manifest already exists');
  fs.writeFileSync(manifest, `${JSON.stringify({ schema: 'U13_2BChecksumsV1', files: entries }, null, 2)}\n`);
} else {
  const retained = JSON.parse(fs.readFileSync(manifest));
  if (JSON.stringify(retained.files) !== JSON.stringify(entries)) throw Error('U13.2B checksum mismatch');
}
console.log(JSON.stringify({ files: entries.length, mode: process.argv.includes('--write') ? 'written' : 'verified' }));
