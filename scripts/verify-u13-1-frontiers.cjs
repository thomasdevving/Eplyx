// Offline verification of U13.1 checkpoint, LUT, and balance evidence.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const base = path.resolve(__dirname, '../docs/examples/phase-u13-1-causal-closure');
const prior = path.resolve(__dirname, '../docs/examples/phase-u13-drift-witness/raw/accounts');
const closure = JSON.parse(fs.readFileSync(path.join(base, 'closure.json')));
const acquisition = JSON.parse(fs.readFileSync(path.join(base, 'acquisition.json')));
const blockRaw = fs.readFileSync(path.resolve(__dirname,
  '../docs/examples/phase-u13-drift-witness/raw/block-409942000.json'));
const block = JSON.parse(blockRaw).result;
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
function assert(ok, description) { if (!ok) throw Error(description); }
assert(sha(blockRaw) === closure.source.sha256, 'source block differs');
const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function encode58(bytes) {
  let n = BigInt('0x' + bytes.toString('hex'));
  let text = '';
  while (n) { text = alphabet[Number(n % 58n)] + text; n /= 58n; }
  let zeros = 0;
  while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
  return '1'.repeat(zeros) + text;
}
const summary = { schema: 'U13_1FrontierAuditV1',
  sourceSha256: sha(blockRaw), accountChanges: [],
  parentReceiptCount: 0, terminalReceiptCount: 0,
  balanceChecks: 0, lutResolutionChecks: 0, receiptHashChecks: 0,
  programData: null, nativeRuntimeInputs: [] };
const acquiredByName = new Map(acquisition.receipts.map(row => [row.name, row]));
for (const receipt of acquisition.receipts) {
  const bytes = fs.readFileSync(path.join(base, 'raw', receipt.name));
  assert(bytes.length === receipt.bytes && sha(bytes) === receipt.sha256,
    `${receipt.name}: receipt hash differs`);
  if (receipt.method === 'getAccountInfo') {
    const parsed = JSON.parse(bytes);
    assert(receipt.params[1].slot === receipt.requestedSlot
      && parsed.result.context.slot === receipt.requestedSlot,
      `${receipt.name}: request/response slot differs`);
    if (receipt.name.startsWith('account-'))
      assert(receipt.name === `account-${receipt.requestedSlot}-${receipt.params[0]}.json`,
        `${receipt.name}: account request identity differs`);
  }
  summary.receiptHashChecks++;
}
function account(slot, address) {
  if (address === 'ComputeBudget111111111111111111111111111111') {
    summary.nativeRuntimeInputs.push(address);
    return null;
  }
  let file = path.join(prior, `${slot}-${address}.json`);
  if (address === 'dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH'
    && slot === 409941999) file = path.join(prior, '409941999-program.json');
  if (!fs.existsSync(file)) {
    const name = `account-${slot}-${address}.json`;
    assert(acquiredByName.has(name), `missing acquisition receipt ${name}`);
    file = path.join(base, 'raw', name);
  }
  const result = JSON.parse(fs.readFileSync(file)).result;
  assert(result.context?.slot === slot, `${address}: wrong context slot`);
  const value = result.value;
  if (value === null) return { address, present: false };
  assert(value.data[1] === 'base64', `${address}: encoding differs`);
  const data = Buffer.from(value.data[0], 'base64');
  assert(data.length === value.space, `${address}: account data incomplete`);
  return { address, present: true, owner: value.owner, lamports: value.lamports,
    executable: value.executable, rentEpoch: value.rentEpoch,
    space: value.space, dataSha256: sha(data), data };
}
const parent = new Map();
for (const address of closure.parentStateFrontier) {
  if (address === acquisition.programData.address) continue;
  const value = account(409941999, address);
  if (value) { parent.set(address, value); summary.parentReceiptCount++; }
}
const terminal = new Map();
for (const address of closure.terminalStateFrontier) {
  terminal.set(address, account(409942000, address));
  summary.terminalReceiptCount++;
}

const frozenHeader = JSON.parse(fs.readFileSync(path.join(prior,
  '409941999-programdata-header.json'))).result.value;
const programChunks = [];
for (const chunk of acquisition.programData.chunks) {
  const parsed = JSON.parse(fs.readFileSync(path.join(base, 'raw', chunk.receipt))).result;
  assert(parsed.context.slot === 409941999 && parsed.value.space === acquisition.programData.space
    && parsed.value.owner === frozenHeader.owner
    && parsed.value.lamports === frozenHeader.lamports
    && parsed.value.executable === frozenHeader.executable
    && parsed.value.rentEpoch === frozenHeader.rentEpoch,
    `${chunk.receipt}: context or space differs`);
  const data = Buffer.from(parsed.value.data[0], 'base64');
  assert(data.length === chunk.length && sha(data) === chunk.dataSha256,
    `${chunk.receipt}: ProgramData slice differs`);
  programChunks.push(data);
}
const programData = Buffer.concat(programChunks);
assert(programData.length === acquisition.programData.space
  && sha(programData) === acquisition.programData.accountSha256
  && programData.subarray(0, 64).equals(Buffer.from(frozenHeader.data[0], 'base64')),
  'historical ProgramData reassembly differs');
assert(programData.readUInt32LE(0) === 3 && programData.subarray(45, 49)
  .equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])), 'ProgramData/ELF layout differs');
summary.programData = { address: acquisition.programData.address,
  accountSha256: sha(programData), elfSha256: sha(programData.subarray(45)),
  elfBytes: programData.length - 45 };

for (const address of closure.terminalStateFrontier) {
  const before = parent.get(address), after = terminal.get(address);
  assert(before && after && before.present && after.present,
    `${address}: output presence differs`);
  summary.accountChanges.push({ address,
    lamportsChanged: before.lamports !== after.lamports,
    dataChanged: before.dataSha256 !== after.dataSha256,
    ownerChanged: before.owner !== after.owner,
    executableChanged: before.executable !== after.executable,
    spaceChanged: before.space !== after.space,
    rentEpochChanged: before.rentEpoch !== after.rentEpoch,
    parentDataSha256: before.dataSha256, terminalDataSha256: after.dataSha256 });
}

for (const row of closure.transactions) {
  const full = block.transactions[row.index];
  const keys = full.transaction.message.accountKeys.concat(
    full.meta.loadedAddresses.writable, full.meta.loadedAddresses.readonly);
  assert(full.transaction.signatures[0] === row.signature
    && full.meta.preBalances.length === keys.length
    && full.meta.postBalances.length === keys.length,
    `${row.index}: transaction balance envelope differs`);
  for (const source of row.inputSources) {
    const keyIndex = keys.indexOf(source.account);
    if (keyIndex < 0) continue; // LUT and ProgramData are implicit runtime inputs.
    const predecessor = source.transactionIndex === null
      ? parent.get(source.account) : null;
    let expectedLamports = predecessor?.present ? predecessor.lamports : null;
    if (source.transactionIndex !== null) {
      const priorTx = block.transactions[source.transactionIndex];
      const priorKeys = priorTx.transaction.message.accountKeys.concat(
        priorTx.meta.loadedAddresses.writable, priorTx.meta.loadedAddresses.readonly);
      const priorIndex = priorKeys.indexOf(source.account);
      assert(priorIndex >= 0, `${row.index}: predecessor omitted account`);
      expectedLamports = priorTx.meta.postBalances[priorIndex];
    }
    if (expectedLamports !== null) {
      assert(full.meta.preBalances[keyIndex] === expectedLamports,
        `${row.index}: pre-balance chain differs for ${source.account}`);
      summary.balanceChecks++;
    }
  }
  for (const lookup of full.transaction.message.addressTableLookups || []) {
    const lut = parent.get(lookup.accountKey);
    assert(lut?.present && lut.owner === 'AddressLookupTab1e1111111111111111111111111',
      `${row.index}: LUT missing or owner differs`);
    const entries = lut.data.subarray(56);
    assert(entries.length % 32 === 0, `${row.index}: LUT address data malformed`);
    const selected = indexes => indexes.map(index => {
      assert((index + 1) * 32 <= entries.length, `${row.index}: LUT index out of range`);
      return encode58(entries.subarray(index * 32, (index + 1) * 32));
    });
    const allLookups = full.transaction.message.addressTableLookups;
    const expectedWritable = allLookups.flatMap(item => {
      const value = parent.get(item.accountKey);
      return item.writableIndexes.map(index =>
        encode58(value.data.subarray(56 + index * 32, 56 + (index + 1) * 32)));
    });
    const expectedReadonly = allLookups.flatMap(item => {
      const value = parent.get(item.accountKey);
      return item.readonlyIndexes.map(index =>
        encode58(value.data.subarray(56 + index * 32, 56 + (index + 1) * 32)));
    });
    assert(JSON.stringify(expectedWritable) === JSON.stringify(full.meta.loadedAddresses.writable)
      && JSON.stringify(expectedReadonly) === JSON.stringify(full.meta.loadedAddresses.readonly),
      `${row.index}: resolved LUT addresses differ`);
    selected(lookup.writableIndexes.concat(lookup.readonlyIndexes));
    summary.lutResolutionChecks++;
  }
  for (const address of row.committedWrites) {
    if (!terminal.has(address)) continue;
    const later = closure.accountWriterCensus.find(c => c.account === address)
      .committedWriterIndexes.some(i => i > row.index);
    if (later) continue;
    const keyIndex = keys.indexOf(address);
    assert(keyIndex >= 0 && terminal.get(address).lamports === full.meta.postBalances[keyIndex],
      `${row.index}: terminal balance differs for ${address}`);
    summary.balanceChecks++;
  }
}
const out = path.join(base, 'frontier-audit.json');
fs.writeFileSync(out, JSON.stringify(summary, null, 2) + '\n');
console.log(JSON.stringify({ parentReceipts: summary.parentReceiptCount,
  terminalReceipts: summary.terminalReceiptCount,
  lutResolutionChecks: summary.lutResolutionChecks,
  balanceChecks: summary.balanceChecks, receiptHashChecks: summary.receiptHashChecks,
  accountChanges: summary.accountChanges.length }));
