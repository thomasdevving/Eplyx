// Offline, protocol-neutral conservative closure planner over the frozen U13 block.
// A successful writable declaration is a possible persistent write. A failed
// transaction commits only the fee-payer and first-instruction durable nonce.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const input = path.resolve(__dirname, '../docs/examples/phase-u13-drift-witness/raw/block-409942000.json');
const targetIndex = 73;
const expectedSignature = '2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK';
const spot = '6gMq3mRCKf8aP3ttTyYhuijVZ2LGi14oDsBbkgubfLB3';
const drift = 'dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH';
const expectedBlockSha = 'e6e3065b6792dbdebe5ace25333af607cb13e57bc03784ca9697bc132a1af304';
const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const unique = values => [...new Set(values)];
function assert(ok, description) { if (!ok) throw Error(description); }
function decode58(value) {
  let n = 0n;
  for (const char of value) {
    const digit = alphabet.indexOf(char);
    assert(digit >= 0, 'invalid base58');
    n = n * 58n + BigInt(digit);
  }
  let hex = n.toString(16);
  if (hex.length % 2) hex = '0' + hex;
  return Buffer.concat([Buffer.alloc(value.match(/^1*/)[0].length),
    n ? Buffer.from(hex, 'hex') : Buffer.alloc(0)]);
}
function encode58(bytes) {
  let n = BigInt('0x' + Buffer.from(bytes).toString('hex'));
  let result = '';
  while (n) { result = alphabet[Number(n % 58n)] + result; n /= 58n; }
  return '1'.repeat(Buffer.from(bytes).findIndex(byte => byte !== 0) === -1
    ? bytes.length : Buffer.from(bytes).findIndex(byte => byte !== 0)) + result;
}

const raw = fs.readFileSync(input);
assert(sha(raw) === expectedBlockSha, 'frozen block hash differs');
const block = JSON.parse(raw).result;
const programReceipt = JSON.parse(fs.readFileSync(path.resolve(__dirname,
  '../docs/examples/phase-u13-drift-witness/raw/accounts/409941999-program.json'))).result;
const programBytes = Buffer.from(programReceipt.value.data[0], 'base64');
assert(programReceipt.context.slot === 409941999 && programBytes.readUInt32LE(0) === 2,
  'frozen historical Program account differs');
const driftProgramData = encode58(programBytes.subarray(4, 36));
const programDataByProgramId = new Map([[drift, driftProgramData]]);
assert(block.parentSlot === 409941999 && block.transactions.length === 1382,
  'frozen block identity differs');

function transaction(row, index) {
  const message = row.transaction.message;
  const staticKeys = message.accountKeys;
  const loaded = row.meta.loadedAddresses || { writable: [], readonly: [] };
  const resolvedKeys = staticKeys.concat(loaded.writable, loaded.readonly);
  const { numRequiredSignatures: signed, numReadonlySignedAccounts: signedReadOnly,
    numReadonlyUnsignedAccounts: unsignedReadOnly } = message.header;
  const writable = unique(staticKeys.filter((_, i) => i < signed
    ? i < signed - signedReadOnly : i < staticKeys.length - unsignedReadOnly)
    .concat(loaded.writable));
  const programs = unique(message.instructions.map(ix => resolvedKeys[ix.programIdIndex])
    .concat((row.meta.innerInstructions || []).flatMap(group =>
      group.instructions.map(ix => resolvedKeys[ix.programIdIndex]))));
  assert(programs.every(Boolean), `program index missing at ${index}`);
  const first = message.instructions[0];
  let nonce = null;
  if (first && resolvedKeys[first.programIdIndex] === '11111111111111111111111111111111'
    && decode58(first.data).equals(Buffer.from([4, 0, 0, 0]))) {
    assert(first.accounts.length === 3, `nonce shape differs at ${index}`);
    nonce = resolvedKeys[first.accounts[0]];
  }
  const success = row.meta.err === null;
  const feePayer = staticKeys[0];
  assert(feePayer && writable.includes(feePayer), `fee payer not writable at ${index}`);
  const committedWrites = success ? writable : unique([feePayer, nonce].filter(Boolean));
  const lookups = (message.addressTableLookups || []).map(lut => lut.accountKey);
  assert(resolvedKeys.every(Boolean) && lookups.every(Boolean), `key missing at ${index}`);
  const implicitStateInputs = unique(lookups.concat(programs
    .flatMap(program => programDataByProgramId.has(program)
      ? [programDataByProgramId.get(program)] : [])));
  return { index, signature: row.transaction.signatures[0], success,
    fee: row.meta.fee, feePayer, durableNonceAccount: nonce,
    messageVersion: row.version, accountReads: unique(resolvedKeys),
    implicitStateInputs, requiredStateInputs: unique(resolvedKeys.concat(implicitStateInputs)),
    declaredWritableAccounts: writable, committedWrites,
    programIds: programs, lutReferences: lookups,
    innerInstructionGroups: (row.meta.innerInstructions || []).length,
    preTokenBalanceRows: (row.meta.preTokenBalances || []).length,
    postTokenBalanceRows: (row.meta.postTokenBalances || []).length,
    loadedWritableCount: loaded.writable.length,
    loadedReadonlyCount: loaded.readonly.length };
}
const rows = block.transactions.map(transaction);
assert(rows[targetIndex].signature === expectedSignature && rows[targetIndex].success,
  'target transaction differs');
const laterSpotWriters = rows.filter(r => r.index > targetIndex && r.success
  && r.committedWrites.includes(spot)).map(r => r.index);
assert(JSON.stringify(laterSpotWriters) === JSON.stringify([428, 431, 438, 1245]),
  'later SpotMarket writers differ');

const writers = new Map();
for (const row of rows) for (const account of row.committedWrites) {
  if (!writers.has(account)) writers.set(account, []);
  writers.get(account).push(row.index);
}
function lastWriter(account, before) {
  const indexes = writers.get(account) || [];
  let low = 0, high = indexes.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (indexes[mid] < before) low = mid + 1;
    else high = mid;
  }
  return low ? indexes[low - 1] : null;
}

function plan() {
  const included = new Set();
  const reasons = new Map();
  const pending = [];
  const add = (index, reason) => {
    if (!reasons.has(index)) reasons.set(index, []);
    reasons.get(index).push(reason);
    if (!included.has(index)) { included.add(index); pending.push(index); }
  };
  add(targetIndex, { kind: 'distinguished_target' });
  for (const index of laterSpotWriters) add(index,
    { kind: 'later_successful_validation_output_writer', account: spot });
  while (pending.length) {
    const index = pending.pop();
    const row = rows[index];
    for (const account of row.requiredStateInputs) {
      const predecessor = lastWriter(account, index);
      if (predecessor !== null) add(predecessor,
        { kind: 'last_preceding_committed_writer', account, consumerIndex: index });
    }
  }
  const order = [...included].sort((a, b) => a - b);
  const dependencyEdges = [];
  const parentSeed = new Set();
  const terminal = new Set();
  const terminalContaminated = new Map();
  const depth = new Map();
  for (const index of order) {
    const row = rows[index];
    let maxDepth = 0;
    for (const account of row.requiredStateInputs) {
      const predecessor = lastWriter(account, index);
      if (predecessor === null) parentSeed.add(account);
      else {
        dependencyEdges.push({ from: predecessor, to: index, account });
        maxDepth = Math.max(maxDepth, (depth.get(predecessor) || 0) + 1);
      }
    }
    depth.set(index, maxDepth);
    for (const account of row.committedWrites) {
      const subsequent = (writers.get(account) || []).find(i => i > index);
      if (subsequent === undefined) terminal.add(account);
      else if (!included.has(subsequent)) terminalContaminated.set(account, subsequent);
    }
  }
  const includedRows = order.map(index => {
    const row = rows[index];
    const predecessors = unique(dependencyEdges.filter(edge => edge.to === index)
      .map(edge => edge.from)).sort((a, b) => a - b);
    const inputSources = row.requiredStateInputs.map(account => ({ account,
      source: lastWriter(account, index) === null ? 'parent_slot' : 'transaction',
      transactionIndex: lastWriter(account, index) }));
    return { ...row, inclusionReasons: reasons.get(index), predecessorIndexes: predecessors,
      inputSources, dependencyDepth: depth.get(index) };
  });
  return { order, includedRows, dependencyEdges, parentSeed: [...parentSeed].sort(),
    terminal: [...terminal].sort(),
    terminalContaminated: [...terminalContaminated].map(([account, nextWriter]) =>
      ({ account, nextWriter })).sort((a, b) => a.account.localeCompare(b.account)),
    maxDepth: Math.max(...depth.values()) };
}
const closure = plan();
const includedSet = new Set(closure.order);
const nextWriterOutside = closure.terminalContaminated;
const allAccounts = unique(closure.includedRows.flatMap(r => r.requiredStateInputs));
const programIds = unique(closure.includedRows.flatMap(r => r.programIds)).sort();
const lutReferences = unique(closure.includedRows.flatMap(r => r.lutReferences)).sort();
const failed = closure.includedRows.filter(r => !r.success);
const relevantAccounts = new Set(allAccounts);
const failedDeclaredOverlaps = rows.filter(r => !r.success
  && r.index >= targetIndex && r.index <= closure.order.at(-1)
  && r.declaredWritableAccounts.some(a => relevantAccounts.has(a)))
  .map(r => ({ index: r.index, signature: r.signature,
    declaredOverlaps: r.declaredWritableAccounts.filter(a => relevantAccounts.has(a)),
    persistentOverlaps: r.committedWrites.filter(a => relevantAccounts.has(a)),
    feePayer: r.feePayer, durableNonceAccount: r.durableNonceAccount }));
const report = {
  schema: 'U13_1ConservativeCausalFrontierV1',
  source: { path: 'docs/examples/phase-u13-drift-witness/raw/block-409942000.json',
    sha256: sha(raw), slot: 409942000, parentSlot: block.parentSlot,
    blockhash: block.blockhash, transactionCount: rows.length },
  target: { index: targetIndex, signature: expectedSignature,
    contaminatedOutput: spot, laterSuccessfulWriters: laterSpotWriters },
  policy: { successfulWrite: 'all_declared_writable_accounts_possible',
    failedWrite: 'fee_payer_and_first_instruction_durable_nonce_only',
    input: 'all_resolved_message_keys_including_readonly_plus_LUT_and_ProgramData',
    predecessor: 'last_prior_possible_committed_writer' },
  summary: { includedTransactions: closure.order.length,
    includedSuccessful: closure.includedRows.length - failed.length,
    includedFailed: failed.length, fractionOfBlock: closure.order.length / rows.length,
    uniqueAccounts: allAccounts.length, parentSeedAccounts: closure.parentSeed.length,
    terminalAccounts: closure.terminal.length,
    terminalContaminatedAccounts: nextWriterOutside.length,
    distinctProgramIds: programIds.length, distinctLuts: lutReferences.length,
    blockFailedTransactions: rows.filter(r => !r.success).length,
    failedDeclaredOverlapCount: failedDeclaredOverlaps.length,
    innerInstructionGroups: closure.includedRows.reduce((n, r) => n + r.innerInstructionGroups, 0),
    preTokenBalanceRows: closure.includedRows.reduce((n, r) => n + r.preTokenBalanceRows, 0),
    postTokenBalanceRows: closure.includedRows.reduce((n, r) => n + r.postTokenBalanceRows, 0),
    longestDependencyDepth: closure.maxDepth,
    earliestIndex: closure.order[0], latestIndex: closure.order.at(-1) },
  transactionIndexes: closure.order,
  transactions: closure.includedRows,
  dependencyEdges: closure.dependencyEdges,
  parentStateFrontier: closure.parentSeed,
  terminalStateFrontier: closure.terminal,
  accountWriterCensus: allAccounts.sort().map(account => ({ account,
    committedWriterIndexes: writers.get(account) || [] })),
  unanchoredTerminalOutputs: nextWriterOutside,
  distinctProgramIds: programIds,
  distinctLutReferences: lutReferences,
  failedTransactionIndexes: failed.map(r => r.index),
  failedDeclaredOverlaps,
};
const out = process.argv[2];
if (out) fs.writeFileSync(out, JSON.stringify(report, null, 2) + '\n');
else process.stdout.write(JSON.stringify(report.summary, null, 2) + '\n');
