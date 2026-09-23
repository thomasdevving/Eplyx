// Offline consistency audit for the isolated sequence feasibility receipt.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../docs/examples/phase-u13-2c-sequence');
const prior = path.resolve(__dirname, '../docs/examples/phase-u13-1-causal-closure');
const witness = path.resolve(__dirname, '../docs/examples/phase-u13-drift-witness/raw');
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const read = file => JSON.parse(fs.readFileSync(file));
const assert = (condition, message) => { if (!condition) throw Error(message); };
const closure = read(path.join(prior, 'closure.json'));
const acquisition = read(path.join(root, 'lut-acquisition.json'));
const receipt = read(path.join(root, 'feasibility.json'));
const checksums = fs.readFileSync(path.join(root, 'checksums.sha256'), 'utf8').trim().split('\n');
for (const line of checksums) {
  const match = /^([0-9a-f]{64})  (.+)$/.exec(line.trim());
  assert(match, 'checksum manifest syntax');
  assert(sha(fs.readFileSync(path.join(root, match[2]))) === match[1],
    `checksum ${match[2]}`);
}
const blockBytes = fs.readFileSync(path.join(witness, 'block-409942000.json'));
const block = JSON.parse(blockBytes).result;
assert(sha(blockBytes) === closure.source.sha256, 'frozen block hash');
assert(JSON.stringify(closure.transactionIndexes) === JSON.stringify([73, 428, 431, 438, 1245]), 'closure indexes');
assert(closure.dependencyEdges.length === 11 && closure.parentStateFrontier.length === 24
  && closure.terminalStateFrontier.length === 9, 'closure frontier and graph');
assert(receipt.runtimeProfileId === '82d133c01dfdffd1d28d4115f969e7ecf73178c41829ccdc3b882910c270e639', 'runtime profile');
assert(receipt.featureSetHash === '9e20e164cd1299081370bf19eacc592f2990ecdd30d1854a9dfd3ff851a0d18c', 'historical features');
assert(acquisition.receipts.length === 2 && receipt.luts.length === 2, 'LUT count');
for (const item of acquisition.receipts) {
  const raw = fs.readFileSync(path.join(root, 'raw', item.name));
  const parsed = JSON.parse(raw);
  const account = parsed.result.value;
  const parent = read(path.join(prior, 'raw', `account-409941999-${item.address}.json`)).result.value;
  assert(sha(raw) === item.responseSha256 && parsed.result.context.slot === 409942000,
    `LUT receipt ${item.address}`);
  assert(account.data[1] === 'base64' && Buffer.from(account.data[0], 'base64').length === account.space,
    `LUT full data ${item.address}`);
  assert(sha(Buffer.from(account.data[0], 'base64')) === item.accountDataSha256,
    `LUT data hash ${item.address}`);
  assert(JSON.stringify(account) === JSON.stringify(parent), `LUT parent equality ${item.address}`);
}
assert(receipt.baseline.allEnvelopesMatch && receipt.baseline.allTerminalAccountsMatch, 'baseline reconciliation');
assert(receipt.baseline.transactions.length === 5 && receipt.messageResolutions.length === 5,
  'five resolved and executed messages');
for (let i = 0; i < 5; i++) {
  const index = closure.transactionIndexes[i];
  const row = receipt.baseline.transactions[i];
  const resolution = receipt.messageResolutions[i];
  const source = block.transactions[index];
  assert(row.index === index && row.signature === source.transaction.signatures[0]
    && resolution.signature === row.signature && resolution.index === index, `tx ${index} identity`);
  assert(row.success && row.envelopeMatch && row.fee === source.meta.fee
    && row.computeUnits === source.meta.computeUnitsConsumed
    && JSON.stringify(row.logs) === JSON.stringify(source.meta.logMessages), `tx ${index} envelope`);
  assert(row.innerInstructionCount === 0 && row.returnDataBytes === 0, `tx ${index} inner/return`);
  assert(Object.keys(row.derivedState).length === 23, `tx ${index} state frontier`);
  assert(row.derivedSpotDataSha256 === row.derivedState[closure.target.contaminatedOutput].dataSha256,
    `tx ${index} SpotMarket state`);
  assert(resolution.fullAccountKeys.length === closure.transactions[i].accountReads.length
    && JSON.stringify(resolution.fullAccountKeys) === JSON.stringify(closure.transactions[i].accountReads),
    `tx ${index} resolved keys`);
  assert(JSON.stringify(resolution.invokedProgramIds) === JSON.stringify(closure.transactions[i].programIds),
    `tx ${index} program census`);
}
assert(receipt.baseline.transactions[0].derivedSpotDataSha256 ===
  'edceea1f1e0ef731ebb9903d5651c5e1bee47ca219a4b13b306598a729cf8f85', 'target SpotMarket');
assert(Object.keys(receipt.baseline.terminalComparisons).length === 9, 'terminal count');
for (const key of closure.terminalStateFrontier) {
  const local = path.join(prior, 'raw', `account-409942000-${key}.json`);
  const archived = read(fs.existsSync(local) ? local
    : path.join(witness, 'accounts', `409942000-${key}.json`)).result.value;
  const comparison = receipt.baseline.terminalComparisons[key];
  assert(comparison.fullAccountEqual &&
    comparison.actualAccountSha256 === comparison.expectedAccountSha256 &&
    comparison.actualDataSha256 === comparison.expectedDataSha256 &&
    comparison.expectedDataSha256 === sha(Buffer.from(archived.data[0], 'base64')) &&
    comparison.actualLamports === archived.lamports &&
    comparison.expectedLamports === archived.lamports,
  `terminal ${key}`);
}
for (const index of [428, 431, 438, 1245])
  assert(receipt.controls[`omit_${index}`].detectedByExecution, `omit ${index}`);
for (const name of ['mutate_428_instruction_data', 'mutate_later_only_parent_account', 'reset_spot_after_73'])
  assert(receipt.controls[name].detectedByExecution, name);
assert(receipt.controls.mutate_lut_used_by_428.detectedByResolution, 'LUT mutation');
assert(receipt.controls.reorder_428_431.detectedByDependencyResolution
  && receipt.controls.reorder_428_431.violatedEdges.length === 4, 'reorder dependency');
assert(receipt.controls.current_default_features_diagnostic.completeExecutionEqual
  && !receipt.controls.current_default_features_diagnostic.historicalFeatureEvidenceSatisfied,
  'default feature diagnostic');
process.stdout.write(JSON.stringify({ indexes: closure.transactionIndexes,
  resolvedLuts: acquisition.receipts.length, envelopes: 5, terminalAccounts: 9,
  stateHashesPerTransaction: 23, controls: Object.keys(receipt.controls).length,
  files: checksums.length,
  mode: 'verified' }) + '\n');
