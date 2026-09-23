#!/usr/bin/env node
// Exact-slot preflight only. Never prints the credential-bearing endpoint.
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const slot = 409942000;
const source = 'docs/examples/phase-u9-freeze/runtime-source/agave-feature-set-4.2.2/src/lib.rs';
const later = 'docs/examples/phase-u9-acquisition/acquisition.json';
const root = 'docs/examples/phase-u13-2b-feature-universe';
const endpoint = process.env.SOLANA_RPC_URL;
if (!endpoint) throw Error('SOLANA_RPC_URL must be configured');
const src = await readFile(source, 'utf8');
const inventory = JSON.parse(await readFile(later, 'utf8'));
const known = new Set(inventory.feature_snapshot.accounts.map(a => a.feature_id));
const missing = [...src.matchAll(/solana_pubkey::declare_id!\("([^"]+)"\)/g)]
  .map(m => m[1]).filter(id => !known.has(id));
const sha = data => createHash('sha256').update(data).digest('hex');
await mkdir(join(root, 'raw'), { recursive: true });

async function acquire(address) {
  const params = [address, { encoding: 'base64', commitment: 'finalized', slot }];
  const request = { jsonrpc: '2.0', id: 1, method: 'getAccountInfo', params };
  let body, status, error;
  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      const response = await fetch(endpoint, { method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(request), signal: AbortSignal.timeout(25000) });
      status = response.status;
      body = Buffer.from(await response.arrayBuffer());
      const parsed = JSON.parse(body.toString('utf8'));
      error = parsed.error || null;
      if (status === 200 && !error && parsed.result?.context?.slot === slot) break;
    } catch (e) { error = { message: e.message }; }
    if (attempt < 3) await new Promise(r => setTimeout(r, 400 * 2 ** attempt));
  }
  const name = `feature-${address}.json`;
  await writeFile(join(root, 'raw', name), body || Buffer.alloc(0));
  const parsed = body?.length ? JSON.parse(body.toString('utf8')) : null;
  const value = parsed?.result?.value || null;
  const data = value?.data?.[1] === 'base64' ? Buffer.from(value.data[0], 'base64') : null;
  return { name, address, method: request.method, params,
    httpStatus: status || null, contextSlot: parsed?.result?.context?.slot || null,
    rpcErrorCode: error?.code || null, rpcError: error?.message || null,
    responseBytes: body?.length || 0, responseSha256: sha(body || Buffer.alloc(0)),
    present: !!value, owner: value?.owner || null, executable: value?.executable ?? null,
    lamports: value?.lamports ?? null, space: value?.space ?? null,
    dataLength: data?.length ?? null, dataSha256: data ? sha(data) : null };
}
const receipts = [];
for (let at = 0; at < missing.length; at += 8) {
  receipts.push(...await Promise.all(missing.slice(at, at + 8).map(acquire)));
  console.log(JSON.stringify({ acquired: receipts.length, total: missing.length,
    errors: receipts.filter(r => r.httpStatus !== 200 || r.contextSlot !== slot || r.rpcError).length }));
}
const manifest = { schema: 'U13_2BMissingFeatureAcquisitionV1',
  providerHost: new URL(endpoint).host, requestedSlot: slot,
  sourceFeatureSet: source, sourceSha256: sha(src),
  laterInventory: later, missingCount: missing.length, receipts };
await writeFile(join(root, 'acquisition.json'), `${JSON.stringify(manifest, null, 2)}\n`);
if (receipts.some(r => r.httpStatus !== 200 || r.contextSlot !== slot || r.rpcError)) process.exitCode = 2;
