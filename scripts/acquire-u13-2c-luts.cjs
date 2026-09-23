// Exact execution-slot LUT receipts for the existing universal v0 proof.
// The endpoint remains in the environment and is never written to artifacts.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../docs/examples/phase-u13-2c-sequence');
const rpc = process.env.SOLANA_RPC_URL;
if (!rpc) throw Error('SOLANA_RPC_URL is required');
const slot = 409942000;
const addresses = [
  'EiWSskK5HXnBTptiS5DH6gpAJRVNQ3cAhTKBGaiaysAb',
  'Fpys8GRa5RBWfyeN7AaDUwFGD1zkDCA4z3t4CJLV8dfL',
];
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
async function main() {
  fs.mkdirSync(path.join(root, 'raw'), { recursive: true });
  if (fs.existsSync(path.join(root, 'lut-acquisition.json')))
    throw Error('LUT acquisition already exists');
  const receipts = [];
  for (const [index, address] of addresses.entries()) {
    const params = [address, { encoding: 'base64', commitment: 'finalized', slot }];
    const response = await fetch(rpc, { method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', id: index + 1,
        method: 'getAccountInfo', params }),
      signal: AbortSignal.timeout(30000) });
    const raw = Buffer.from(await response.arrayBuffer());
    const parsed = JSON.parse(raw);
    if (response.status !== 200 || parsed.error || parsed.result?.context?.slot !== slot || !parsed.result.value)
      throw Error(`historical LUT receipt invalid for ${address}: HTTP ${response.status}`);
    const name = `account-${slot}-${address}.json`;
    fs.writeFileSync(path.join(root, 'raw', name), raw);
    const data = Buffer.from(parsed.result.value.data[0], 'base64');
    if (parsed.result.value.data[1] !== 'base64' || data.length !== parsed.result.value.space)
      throw Error(`LUT account length invalid for ${address}`);
    receipts.push({ address, name, contextSlot: slot, bytes: raw.length,
      responseSha256: sha(raw), accountDataSha256: sha(data) });
  }
  const manifest = { schema: 'U13_2CLutAcquisitionV1',
    providerHost: new URL(rpc).host, slot, receipts };
  fs.writeFileSync(path.join(root, 'lut-acquisition.json'), JSON.stringify(manifest, null, 2) + '\n');
  process.stdout.write(JSON.stringify({ receipts: receipts.length, slot }) + '\n');
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
