#!/usr/bin/env node
// Rebuild the backend-known historical feature state from retained raw RPC bytes.
const fs = require('node:fs');
const crypto = require('node:crypto');
const path = require('node:path');

const root = 'docs/examples/phase-u13-2b-feature-universe';
const sourcePath = 'docs/examples/phase-u9-freeze/runtime-source/agave-feature-set-4.2.2/src/lib.rs';
const oldPath = 'docs/examples/phase-u13-2a-runtime';
const source = fs.readFileSync(sourcePath);
const old = JSON.parse(fs.readFileSync(path.join(oldPath, 'acquisition.json')));
const u13Input = JSON.parse(fs.readFileSync(path.join(oldPath, 'input-audit.json')));
const fresh = JSON.parse(fs.readFileSync(path.join(root, 'acquisition.json')));
const sha = v => crypto.createHash('sha256').update(v).digest('hex');
const fail = message => { throw Error(message); };
const slot = 409942000;
if (old.requestedSlot !== slot || fresh.requestedSlot !== slot) fail('slot mismatch');
if (fresh.sourceSha256 !== sha(source)) fail('feature source hash mismatch');
const declared = [...source.toString('utf8').matchAll(/solana_pubkey::declare_id!\("([^"]+)"\)/g)]
  .map(m => m[1]);
const registrySection = source.toString('utf8').split('pub static FEATURE_NAMES:')[1]
  ?.split('pub static ID:')[0] || fail('feature registry missing');
const registeredCount = [...registrySection.matchAll(/[A-Za-z_][A-Za-z0-9_:]*::id\(\)/g)].length;
if (registeredCount !== 285 || declared.length !== 288) fail('frozen feature registry changed');
if (new Set(declared).size !== declared.length) fail('duplicate source ID');
const prior = old.receipts.filter(r => r.kind === 'feature');
if (prior.length !== 311 || fresh.receipts.length !== 38) fail('unexpected inventory length');
const union = [...prior.map(r => r.address), ...fresh.receipts.map(r => r.address)];
if (new Set(union).size !== union.length) fail('overlapping inventory');
if (declared.some(id => !union.includes(id))) fail('backend-known feature missing');
const featureOwner = 'Feature111111111111111111111111111111111111';
const rows = [];
for (const [base, receipt] of [...prior.map(r => [oldPath, r]), ...fresh.receipts.map(r => [root, r])]) {
  const raw = fs.readFileSync(path.join(base, 'raw', receipt.name));
  if (sha(raw) !== receipt.responseSha256) fail(`raw response hash: ${receipt.address}`);
  const parsed = JSON.parse(raw.toString('utf8'));
  if (receipt.httpStatus !== 200 || receipt.contextSlot !== slot || parsed.result?.context?.slot !== slot || parsed.error) {
    fail(`archive context: ${receipt.address}`);
  }
  const value = parsed.result.value;
  if (!!value !== receipt.present) fail(`presence: ${receipt.address}`);
  let state = 'absent', activationSlot = null, owner = null, dataSha256 = null;
  if (value) {
    owner = value.owner;
    if (value.data?.[1] !== 'base64') fail(`encoding: ${receipt.address}`);
    const data = Buffer.from(value.data[0], 'base64');
    if (data.toString('base64') !== value.data[0] || value.space !== data.length ||
        receipt.dataLength !== data.length || receipt.dataSha256 !== sha(data) ||
        receipt.owner !== owner || value.executable !== receipt.executable) {
      fail(`account bytes: ${receipt.address}`);
    }
    dataSha256 = sha(data);
    if (owner === featureOwner) {
      if (value.executable || data.length !== 9) fail(`feature layout: ${receipt.address}`);
      if (data[0] === 0 && data.subarray(1).equals(Buffer.alloc(8))) state = 'inactive';
      else if (data[0] === 1) {
        activationSlot = Number(data.readBigUInt64LE(1));
        if (!Number.isSafeInteger(activationSlot) || activationSlot > slot) fail(`invalid activation: ${receipt.address}`);
        state = 'active';
      } else fail(`feature variant: ${receipt.address}`);
    } else state = 'non_feature_account';
  }
  rows.push({ id: receipt.address, state, activationSlot, owner,
    responseSha256: receipt.responseSha256, dataSha256,
    source: path.join(base, 'raw', receipt.name).replaceAll('\\', '/') });
}
rows.sort((a, b) => a.id.localeCompare(b.id));
const active = rows.filter(r => r.state === 'active').map(r => [r.id, r.activationSlot]);
const counts = Object.fromEntries(['active', 'inactive', 'absent', 'non_feature_account']
  .map(state => [state, rows.filter(r => r.state === state).length]));
const report = {
  schema: 'U13_2BFeatureUniverseAuditV1', slot,
  backendDependency: 'agave-feature-set 4.2.2',
  source: sourcePath, sourceSha256: sha(source), sourceDeclarations: declared.length,
  backendRegisteredFeatureCount: registeredCount,
  priorInventoryCount: prior.length, supplementedCount: fresh.receipts.length,
  unionCount: rows.length, missingBackendKnownIds: declared.filter(id => !union.includes(id)),
  counts,
  priorCounts: Object.fromEntries(['active', 'inactive', 'absent', 'non_feature_account'].map(state =>
    [state, rows.filter(r => r.state === state && r.source.startsWith(oldPath)).length])),
  supplementCounts: Object.fromEntries(['active', 'inactive', 'absent', 'non_feature_account'].map(state =>
    [state, rows.filter(r => r.state === state && r.source.startsWith(root)).length])),
  universeHash: sha(JSON.stringify(['eplyx-feature-universe-v1', sha(source), rows.map(r => r.id)])),
  featureSetHash: sha(JSON.stringify(['eplyx-historical-feature-set-v1', slot, active])),
  active,
  backendPostTargetFeatures: u13Input.backendFeatureGap.backendFeaturesAfterTarget.map(feature => {
    const match = source.toString('utf8').match(new RegExp(`pub mod ${feature.name} \\{[\\s\\S]{0,500}?solana_pubkey::declare_id!\\("([^"]+)"\\)`));
    if (!match) fail(`backend post-target feature ID: ${feature.name}`);
    return { ...feature, id: match[1] };
  }),
  observations: rows,
};
fs.writeFileSync(path.join(root, 'audit.json'), `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify({ counts, priorCounts: report.priorCounts,
  supplementCounts: report.supplementCounts, universeHash: report.universeHash,
  featureSetHash: report.featureSetHash, missingBackendKnownIds: report.missingBackendKnownIds }));
