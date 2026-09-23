// Offline research audit. This deliberately does not create a replay observation.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const root = path.resolve(__dirname, '../docs/examples/phase-u13-drift-witness');
const raw = path.join(root, 'raw');
const accounts = path.join(raw, 'accounts');
const signature = '2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK';
const drift = 'dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH';
const programData = '7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP';
const spot = '6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3';
const slot = 409942000;
const parent = 409941999;
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const read = file => fs.readFileSync(path.join(root, file));
const json = file => JSON.parse(read(file));
function requireFact(ok, description) {
  if (!ok) throw new Error(description);
}
function decode58(value) {
  const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
  let n = 0n;
  for (const char of value) {
    const digit = alphabet.indexOf(char);
    requireFact(digit >= 0, 'invalid base58');
    n = n * 58n + BigInt(digit);
  }
  let hex = n.toString(16);
  if (hex.length % 2) hex = '0' + hex;
  const body = n ? Buffer.from(hex, 'hex') : Buffer.alloc(0);
  return Buffer.concat([Buffer.alloc(value.match(/^1*/)[0].length), body]);
}
function archive(at, address) {
  const receipt = JSON.parse(fs.readFileSync(path.join(accounts, `${at}-${address}.json`)));
  requireFact(receipt.result.context.slot === at, `archive slot differs: ${address}`);
  const value = receipt.result.value;
  requireFact(value, `archive account absent: ${address}`);
  const bytes = Buffer.from(value.data[0], 'base64');
  requireFact(value.data[1] === 'base64' && bytes.length === value.space,
    `archive account is sliced or malformed: ${address}`);
  return { owner: value.owner, lamports: value.lamports, bytes: bytes.length,
    dataSha256: sha(bytes), data: bytes };
}
function writableKeys(entry) {
  const { message } = entry.transaction;
  const { header } = message;
  const result = [];
  for (let i = 0; i < message.accountKeys.length; i++) {
    const signed = i < header.numRequiredSignatures;
    const boundary = signed
      ? header.numRequiredSignatures - header.numReadonlySignedAccounts
      : message.accountKeys.length - header.numReadonlyUnsignedAccounts;
    if (i < boundary) result.push(message.accountKeys[i]);
  }
  return result.concat(entry.meta.loadedAddresses?.writable || []);
}
const blockBytes = read('raw/block-409942000.json');
const block = JSON.parse(blockBytes).result;
const predecessor = json('raw/predecessor-block-409941999.json').result;
const transaction = json('raw/target-transaction.json');
const source = read('source/lib.rs');
const idl = json('source/drift.json');
const settle = idl.instructions.find(row => row.name === 'settlePnl');
// Git object IDs use SHA-1; this check binds the files to the pinned source commit.
const gitSha1 = bytes => crypto.createHash('sha1')
  .update(Buffer.from(`blob ${bytes.length}\0`)).update(bytes).digest('hex');
requireFact(gitSha1(source) === '1862893e79b9e34f7ee2f3df08e5a2ccd641eedd', 'lib.rs Git blob differs');
requireFact(gitSha1(read('source/drift.json')) === '7232b2925ffcee1f65ad85a0fddbf830add64f12', 'IDL Git blob differs');
requireFact(settle && settle.accounts.map(row => row.name).join(',') ===
  'state,user,authority,spotMarketVault', 'pinned interface differs');
requireFact(block.parentSlot === parent && block.previousBlockhash === predecessor.blockhash,
  'predecessor block identity differs');
requireFact(transaction.slot === slot && transaction.meta.err === null, 'target outcome differs');
const index = block.transactions.findIndex(row => row.transaction.signatures[0] === signature);
requireFact(index === 73, 'target block index differs');
const target = block.transactions[index];
requireFact(target.version === 0 && target.meta.err === null, 'target block result differs');
requireFact(JSON.stringify(target.transaction.message) === JSON.stringify(transaction.transaction.message),
  'target message differs between receipts');
const message = target.transaction.message;
requireFact(message.addressTableLookups.length === 0 &&
  target.meta.loadedAddresses.writable.length === 0 &&
  target.meta.loadedAddresses.readonly.length === 0, 'LUT state differs');
const driftInstructions = message.instructions.filter(ix =>
  message.accountKeys[ix.programIdIndex] === drift);
requireFact(driftInstructions.length === 1 && message.instructions.length === 3,
  'instruction count differs');
requireFact(message.instructions.slice(0, 2).every(ix =>
  message.accountKeys[ix.programIdIndex] === 'ComputeBudget111111111111111111111111111111'),
  'companion instruction differs');
const instruction = driftInstructions[0];
const instructionBytes = decode58(instruction.data);
const discriminator = sha(Buffer.from('global:settle_pnl')).slice(0, 16);
requireFact(instructionBytes.toString('hex') === discriminator + '0300',
  'settlePnl instruction bytes differ');
const roles = instruction.accounts.map(i => message.accountKeys[i]);
requireFact(roles.length === 14 && roles[1] === message.accountKeys[1] &&
  roles[9] === spot, 'settlePnl account roles differ');
const inputSet = new Set(roles);
const outputs = writableKeys(target).filter(address => inputSet.has(address));
requireFact(outputs.length === 4 && outputs.includes(spot), 'target writable set differs');
const earlier = [];
const later = [];
for (let i = 0; i < block.transactions.length; i++) {
  if (i === index) continue;
  const row = block.transactions[i];
  const hits = writableKeys(row).filter(address =>
    i < index ? inputSet.has(address) : outputs.includes(address));
  if (hits.length) (i < index ? earlier : later).push({
    index: i, signature: row.transaction.signatures[0],
    succeeded: row.meta.err === null, addresses: hits,
  });
}
requireFact(earlier.length === 0 && later.length === 4 &&
  later.every(row => row.succeeded && row.addresses.length === 1 && row.addresses[0] === spot),
  'closure result differs');
const types = new Map([['User', roles[1]], ['SpotMarket', spot], ['PerpMarket', roles[12]]]);
const changed = [];
for (const address of roles) {
  const before = archive(parent, address);
  if (!outputs.includes(address)) continue;
  const after = archive(slot, address);
  const type = [...types].find(([, value]) => value === address)?.[0] || null;
  if (type) {
    const disc = sha(Buffer.from(`account:${type}`)).slice(0, 16);
    requireFact(before.data.subarray(0, 8).toString('hex') === disc &&
      after.data.subarray(0, 8).toString('hex') === disc, `${type} discriminator differs`);
  }
  changed.push({ address, type, owner: before.owner,
    preDataSha256: before.dataSha256, postDataSha256: after.dataSha256,
    preLamports: before.lamports, postLamports: after.lamports,
    rawChanged: before.dataSha256 !== after.dataSha256 || before.lamports !== after.lamports,
    blockFinalAttributableToTarget: !later.some(row => row.addresses.includes(address)),
  });
}
requireFact(changed.filter(row => row.owner === drift && row.rawChanged).length === 3,
  'Drift owned change count differs');
requireFact(target.meta.innerInstructions.length === 0 &&
  target.meta.preTokenBalances.length === 0 && target.meta.postTokenBalances.length === 0,
  'token flow or CPI evidence differs');
const programReceipt = json('raw/accounts/409941999-program.json').result;
const programDataReceipt = json('raw/accounts/409941999-programdata-header.json').result;
requireFact(programReceipt.context.slot === parent && programDataReceipt.context.slot === parent &&
  programReceipt.value.owner === 'BPFLoaderUpgradeab1e11111111111111111111111' &&
  programReceipt.value.executable && programDataReceipt.value.space === 6673114,
  'historical program header differs');
const programHeader = Buffer.from(programReceipt.value.data[0], 'base64');
const dataHeader = Buffer.from(programDataReceipt.value.data[0], 'base64');
requireFact(programHeader.readUInt32LE(0) === 2 && dataHeader.readUInt32LE(0) === 3,
  'upgradeable loader state differs');
requireFact(programHeader.subarray(4, 36).equals(decode58(programData)),
  'ProgramData address differs');
const report = {
  outcome: 'U13-B', signature, slot, parentSlot: parent,
  blockhash: block.blockhash, blockSha256: sha(blockBytes),
  transactionCount: block.transactions.length, targetIndex: index,
  messageVersion: target.version, staticKeys: message.accountKeys.length,
  loadedKeys: 0, lutCount: 0,
  instructionDataHex: instructionBytes.toString('hex'), marketIndex: instructionBytes.readUInt16LE(8),
  accountRoles: roles, writableOutputs: outputs,
  validator: { succeeded: target.meta.err === null, fee: target.meta.fee,
    computeUnits: target.meta.computeUnitsConsumed, innerInstructionGroups: target.meta.innerInstructions.length,
    preTokenBalanceRows: target.meta.preTokenBalances.length,
    postTokenBalanceRows: target.meta.postTokenBalances.length },
  historicalProgram: { owner: programReceipt.value.owner,
    programDataAddress: programData,
    programDataSpace: programDataReceipt.value.space,
    lastUpgradeSlot: Number(dataHeader.readBigUInt64LE(4)) },
  changed, earlierWritableInputOverlaps: earlier, laterWritableOutputOverlaps: later,
  blocker: 'successful later writes to a changed validation output require a transaction-index state boundary or multi-transaction closure',
};
process.stdout.write(JSON.stringify(report, null, 2) + '\n');
