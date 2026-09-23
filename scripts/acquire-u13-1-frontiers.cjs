// Acquire only the U13.1 account boundaries missing from the frozen U13 witness.
// The RPC URL is environment-only; receipts never contain it or headers.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../docs/examples/phase-u13-1-causal-closure');
const prior = path.resolve(__dirname, '../docs/examples/phase-u13-drift-witness/raw/accounts');
const closure = JSON.parse(fs.readFileSync(path.join(root, 'closure.json')));
const rawDir = path.join(root, 'raw');
const rpc = process.env.SOLANA_RPC_URL;
if (!rpc) throw Error('SOLANA_RPC_URL is required');
const host = new URL(rpc).host;
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const programData = '7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP';
const parent = 409941999;
const slot = 409942000;
const chunkSize = 1048576;
fs.mkdirSync(rawDir, { recursive: true });
if (fs.existsSync(path.join(root, 'acquisition.json')) && !process.argv.includes('--force'))
  throw Error('frozen acquisition exists; pass --force to reacquire explicitly');
const receipts = [];
let rpcId = 1;

async function call(method, params, name, requestedSlot) {
  const request = JSON.stringify({ jsonrpc: '2.0', id: rpcId++, method, params });
  const response = await fetch(rpc, { method: 'POST',
    headers: { 'content-type': 'application/json' }, body: request,
    signal: AbortSignal.timeout(30000) });
  const raw = Buffer.from(await response.arrayBuffer());
  let parsed;
  try { parsed = JSON.parse(raw); } catch { throw Error(`non-JSON ${method}: HTTP ${response.status}`); }
  if (response.status !== 200 || parsed.error) {
    throw Error(`${method} failed: HTTP ${response.status}, RPC ${parsed.error?.code ?? 'none'}`);
  }
  if (requestedSlot !== undefined && parsed.result?.context?.slot !== requestedSlot) {
    throw Error(`${name}: historical context slot differs`);
  }
  fs.writeFileSync(path.join(rawDir, name), raw);
  receipts.push({ name, method, params, requestedSlot: requestedSlot ?? null,
    contextSlot: parsed.result?.context?.slot ?? null, httpStatus: response.status,
    bytes: raw.length, sha256: sha(raw) });
  return parsed.result;
}
function accountFacts(result, address, name, expectedLength = null) {
  const value = result.value;
  if (value === null) return { address, present: false, dataLength: null };
  if (value.data?.[1] !== 'base64') throw Error(`${name}: encoding differs`);
  const data = Buffer.from(value.data[0], 'base64');
  if (expectedLength !== null && data.length !== expectedLength)
    throw Error(`${name}: returned data length differs`);
  if (expectedLength === null && data.length !== value.space)
    throw Error(`${name}: response is not a full account`);
  return { address, present: true, owner: value.owner,
    lamports: value.lamports, executable: value.executable,
    rentEpoch: value.rentEpoch, space: value.space, dataLength: data.length,
    dataSha256: sha(data), data };
}
async function main() {
  const genesis = await call('getGenesisHash', [], 'genesis.json');
  if (genesis !== '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d')
    throw Error('RPC is not Solana mainnet');
  const parentRows = [], terminalRows = [];
  for (const [boundary, addresses, output] of [
    [parent, closure.parentStateFrontier, parentRows],
    [slot, closure.terminalStateFrontier, terminalRows],
  ]) {
    for (const address of addresses) {
      if (address === 'ComputeBudget111111111111111111111111111111'
        || address === programData) continue;
      if (fs.existsSync(path.join(prior, `${boundary}-${address}.json`))) continue;
      if (address === 'dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH'
        && boundary === parent) continue; // frozen program.json
      const name = `account-${boundary}-${address}.json`;
      const result = await call('getAccountInfo', [address,
        { encoding: 'base64', commitment: 'finalized', slot: boundary }], name, boundary);
      const facts = accountFacts(result, address, name);
      delete facts.data;
      output.push({ boundary, ...facts, receipt: name });
    }
  }
  const frozenHeader = JSON.parse(fs.readFileSync(path.join(prior,
    '409941999-programdata-header.json'))).result.value;
  const expectedHeader = Buffer.from(frozenHeader.data[0], 'base64');
  const size = frozenHeader.space;
  const chunks = [];
  const assembled = [];
  for (let offset = 0; offset < size; offset += chunkSize) {
    const length = Math.min(chunkSize, size - offset);
    const name = `programdata-${parent}-${offset}-${length}.json`;
    const result = await call('getAccountInfo', [programData,
      { encoding: 'base64', commitment: 'finalized', slot: parent,
        dataSlice: { offset, length } }], name, parent);
    const facts = accountFacts(result, programData, name, length);
    if (!facts.present || facts.space !== size || facts.owner !== frozenHeader.owner)
      throw Error(`${name}: ProgramData metadata differs`);
    chunks.push({ offset, length, dataSha256: facts.dataSha256, receipt: name });
    assembled.push(facts.data);
  }
  const programDataBytes = Buffer.concat(assembled);
  if (programDataBytes.length !== size ||
    !programDataBytes.subarray(0, expectedHeader.length).equals(expectedHeader))
    throw Error('assembled ProgramData differs from frozen header');
  const elf = programDataBytes.subarray(45);
  if (!elf.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46])))
    throw Error('historical ProgramData has no ELF magic');
  const summary = { schema: 'U13_1MissingFrontierAcquisitionV1',
    providerHost: host, genesis, parentSlot: parent, slot,
    parentAccounts: parentRows, terminalAccounts: terminalRows,
    programData: { address: programData, space: size,
      accountSha256: sha(programDataBytes), elfLength: elf.length,
      elfSha256: sha(elf), chunks }, receipts };
  fs.writeFileSync(path.join(root, 'acquisition.json'), JSON.stringify(summary, null, 2) + '\n');
  console.log(JSON.stringify({ parentAcquired: parentRows.length,
    terminalAcquired: terminalRows.length, programDataChunks: chunks.length,
    elfBytes: elf.length, receiptCount: receipts.length }));
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
