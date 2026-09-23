#!/usr/bin/env node
// Read-only preflight: compare the frozen Agave feature declarations with U9's
// later account inventory and U13.2A's exact-slot feature receipts.
const fs = require('node:fs');
const crypto = require('node:crypto');

const sourcePath = 'docs/examples/phase-u9-freeze/runtime-source/agave-feature-set-4.2.2/src/lib.rs';
const u9Path = 'docs/examples/phase-u9-acquisition/acquisition.json';
const u13Path = 'docs/examples/phase-u13-2a-runtime/acquisition.json';
const source = fs.readFileSync(sourcePath, 'utf8');
const u9 = JSON.parse(fs.readFileSync(u9Path, 'utf8'));
const u13 = JSON.parse(fs.readFileSync(u13Path, 'utf8'));
const declared = [...source.matchAll(/solana_pubkey::declare_id!\("([^"]+)"\)/g)]
  .map(m => ({ id: m[1], line: source.slice(0, m.index).split('\n').length }));
const known = new Set(u9.feature_snapshot.accounts.map(a => a.feature_id));
const acquired = new Set(u13.receipts.filter(r => r.kind === 'feature').map(r => r.address));
const report = {
  schema: 'U13_2BFeatureUniversePreflightV1',
  source: sourcePath,
  sourceSha256: crypto.createHash('sha256').update(fs.readFileSync(sourcePath)).digest('hex'),
  sourceDeclarations: declared.length,
  sourceUniqueIds: new Set(declared.map(d => d.id)).size,
  laterFeatureAccounts: known.size,
  exactSlotFeatureReceipts: acquired.size,
  sourceIdsMissingFromLaterInventory: declared.filter(d => !known.has(d.id)),
  sourceIdsMissingFromExactSlotReceipts: declared.filter(d => !acquired.has(d.id)),
  laterInventoryIdsMissingFromSource: [...known].filter(id => !declared.some(d => d.id === id)),
};
console.log(JSON.stringify(report, null, 2));
