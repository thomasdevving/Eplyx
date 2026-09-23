// Named adversarial checks for the frozen U13.1 graph and generic rollback cases.
const fs = require('node:fs');
const path = require('node:path');
const base = path.resolve(__dirname, '../docs/examples/phase-u13-1-causal-closure');
const closure = JSON.parse(fs.readFileSync(path.join(base, 'closure.json')));
const block = JSON.parse(fs.readFileSync(path.resolve(__dirname,
  '../docs/examples/phase-u13-drift-witness/raw/block-409942000.json'))).result;
const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function assert(ok, description) { if (!ok) throw Error(description); }
function decode58(value) {
  let n = 0n;
  for (const char of value) n = n * 58n + BigInt(alphabet.indexOf(char));
  let hex = n.toString(16); if (hex.length % 2) hex = '0' + hex;
  return Buffer.concat([Buffer.alloc(value.match(/^1*/)[0].length),
    n ? Buffer.from(hex, 'hex') : Buffer.alloc(0)]);
}
const rows = block.transactions.map((tx, index) => {
  const message = tx.transaction.message;
  const staticKeys = message.accountKeys;
  const loaded = tx.meta.loadedAddresses;
  const keys = staticKeys.concat(loaded.writable, loaded.readonly);
  const h = message.header;
  const declared = staticKeys.filter((_, i) => i < h.numRequiredSignatures
    ? i < h.numRequiredSignatures - h.numReadonlySignedAccounts
    : i < staticKeys.length - h.numReadonlyUnsignedAccounts).concat(loaded.writable);
  let nonce = null;
  const first = message.instructions[0];
  if (first && keys[first.programIdIndex] === '11111111111111111111111111111111'
    && decode58(first.data).equals(Buffer.from([4, 0, 0, 0]))) {
    nonce = keys[first.accounts[0]];
  }
  const success = tx.meta.err === null;
  return { index, success, keys, declared,
    committed: success ? declared : [staticKeys[0], nonce].filter(Boolean) };
});
const writers = account => rows.filter(r => r.committed.includes(account)).map(r => r.index);
const cases = [];
function check(name, run) {
  run(); cases.push({ name, passed: true });
}
function validateFrozen(selected, proposal = closure) {
  const selectedSet = new Set(selected);
  assert(selectedSet.has(73), 'distinguished target omitted');
  for (const account of proposal.terminalStateFrontier) {
    const last = writers(account).at(-1);
    assert(last !== undefined && selectedSet.has(last),
      `terminal output lacks its last writer: ${account}`);
  }
  for (const row of proposal.transactions.filter(r => selectedSet.has(r.index))) {
    for (const source of row.inputSources) {
      const preceding = writers(source.account).filter(i => i < row.index).at(-1);
      assert((preceding ?? null) === source.transactionIndex,
        `input predecessor differs at ${row.index}: ${source.account}`);
      if (preceding !== undefined) assert(selectedSet.has(preceding),
        `predecessor omitted at ${row.index}: ${source.account}`);
    }
  }
  for (const census of proposal.accountWriterCensus) {
    assert(JSON.stringify(census.committedWriterIndexes) === JSON.stringify(writers(census.account)),
      `writer census differs: ${census.account}`);
  }
}
check('frozen_sequence_valid', () => validateFrozen(closure.transactionIndexes));
for (const index of [428, 431, 438, 1245]) {
  check(`omit_${index}_rejected`, () => {
    try { validateFrozen(closure.transactionIndexes.filter(i => i !== index)); }
    catch { return; }
    throw Error(`omitting ${index} was accepted`);
  });
}
check('ignore_successful_spot_writer_rejected', () => {
  const altered = structuredClone(closure);
  const spot = altered.accountWriterCensus.find(r => r.account === altered.target.contaminatedOutput);
  spot.committedWriterIndexes = spot.committedWriterIndexes.filter(i => i !== 428);
  try { validateFrozen(altered.transactionIndexes, altered); }
  catch { return; }
  throw Error('censored successful writer was accepted');
});

// Small synthetic schedules exercise cases absent from the frozen five-tx slice.
function syntheticClosure(schedule, target) {
  const selected = new Set([target]);
  const pending = [target];
  const committed = row => row.success ? row.declared
    : [row.feePayer, row.nonce].filter(Boolean);
  while (pending.length) {
    const consumer = pending.pop();
    for (const account of schedule[consumer].inputs) {
      let predecessor = null;
      for (let i = consumer - 1; i >= 0; i--) {
        if (committed(schedule[i]).includes(account)) { predecessor = i; break; }
      }
      if (predecessor !== null && !selected.has(predecessor)) {
        selected.add(predecessor); pending.push(predecessor);
      }
    }
  }
  return [...selected].sort((a, b) => a - b);
}
const writer = { success: true, inputs: ['P', 'X'], declared: ['P', 'X'], feePayer: 'P' };
const reader = { success: true, inputs: ['Q', 'X'], declared: ['Q'], feePayer: 'Q' };
check('earlier_writer_of_readonly_input_required', () => {
  const required = syntheticClosure([writer, reader], 1);
  assert(JSON.stringify(required) === '[0,1]' && required.some(i => ![1].includes(i)),
    'readonly input predecessor omitted');
});
const failed = { success: false, inputs: ['P', 'X'], declared: ['P', 'X'],
  feePayer: 'P', nonce: null };
check('failed_declared_writer_not_committed', () => {
  assert(JSON.stringify(syntheticClosure([failed, reader], 1)) === '[1]',
    'failed nonpersistent writer treated as committed');
});
check('failed_fee_payer_reuse_required', () => {
  const reuse = { ...reader, inputs: ['Q', 'P'] };
  assert(JSON.stringify(syntheticClosure([failed, reuse], 1)) === '[0,1]',
    'failed fee payer mutation ignored');
});
check('failed_nonce_reuse_required', () => {
  const nonce = { ...failed, nonce: 'N', declared: ['P', 'X', 'N'] };
  const reuse = { ...reader, inputs: ['Q', 'N'] };
  assert(JSON.stringify(syntheticClosure([nonce, reuse], 1)) === '[0,1]',
    'failed durable nonce mutation ignored');
});
check('failed_fee_payer_omission_rejected', () => {
  const reuse = { ...reader, inputs: ['Q', 'P'] };
  const required = syntheticClosure([failed, reuse], 1);
  const proposal = [1];
  assert(required.some(index => !proposal.includes(index)),
    'fee-payer omission not detected');
});
const result = { schema: 'U13_1MinimalityChecksV1', cases };
fs.writeFileSync(path.join(base, 'minimality.json'), JSON.stringify(result, null, 2) + '\n');
console.log(JSON.stringify({ passed: cases.length, names: cases.map(c => c.name) }));
