#!/usr/bin/env node
// Isolated exact-slot historical runtime acquisition. Credentials stay in ENV.
import { createHash } from 'node:crypto';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

const slot = 409942000;
const root = 'docs/examples/phase-u13-2a-runtime';
const rawDir = join(root, 'raw');
const endpoint = process.env.SOLANA_RPC_URL;
if (!endpoint) throw Error('SOLANA_RPC_URL must be configured');
const host = new URL(endpoint).host;
const previous = JSON.parse(await readFile('docs/examples/phase-u9-acquisition/acquisition.json'));
const features = previous.feature_snapshot.accounts.map((row) => row.feature_id).sort();
const sysvars = {
  Clock: 'SysvarC1ock11111111111111111111111111111111',
  Rent: 'SysvarRent111111111111111111111111111111111',
  EpochSchedule: 'SysvarEpochSchedu1e111111111111111111111111',
  RecentBlockhashes: 'SysvarRecentB1ockHashes11111111111111111111',
  SlotHashes: 'SysvarS1otHashes111111111111111111111111111',
};
const work = [
  ...Object.entries(sysvars).map(([name, address]) => ({ name: `sysvar-${name}.json`, address, kind: 'sysvar' })),
  ...features.map((address) => ({ name: `feature-${address}.json`, address, kind: 'feature' })),
];
const sha = (data) => createHash('sha256').update(data).digest('hex');
await mkdir(rawDir, { recursive: true });
async function acquire(item) {
  const params = [item.address, { encoding: 'base64', commitment: 'finalized', slot }];
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
    if (attempt < 3) await new Promise((resolve) => setTimeout(resolve, 400 * 2 ** attempt));
  }
  if (!body) body = Buffer.alloc(0);
  await writeFile(join(rawDir, item.name), body);
  const parsed = body.length ? JSON.parse(body.toString('utf8')) : null;
  const value = parsed?.result?.value || null;
  const data = value?.data?.[1] === 'base64' ? Buffer.from(value.data[0], 'base64') : null;
  return { name: item.name, kind: item.kind, address: item.address,
    method: request.method, params, httpStatus: status || null,
    contextSlot: parsed?.result?.context?.slot || null,
    rpcErrorCode: error?.code || null, rpcError: error?.message || null,
    responseBytes: body.length, responseSha256: sha(body),
    present: !!value, owner: value?.owner || null, executable: value?.executable ?? null,
    lamports: value?.lamports ?? null, space: value?.space ?? null,
    dataLength: data?.length ?? null, dataSha256: data ? sha(data) : null };
}
const receipts = [];
for (let at = 0; at < work.length; at += 8) {
  const batch = await Promise.all(work.slice(at, at + 8).map(acquire));
  receipts.push(...batch);
  if ((at + 8) % 80 === 0 || at + 8 >= work.length) {
    console.log(JSON.stringify({ acquired: receipts.length, total: work.length,
      errors: receipts.filter((r) => r.httpStatus !== 200 || r.contextSlot !== slot || r.rpcError).length }));
  }
}
const manifest = { schema: 'U13_2ARuntimeAcquisitionV1', providerHost: host,
  requestedSlot: slot, sourceFeatureSnapshot: 'docs/examples/phase-u9-acquisition/acquisition.json',
  sourceFeatureCount: features.length, receipts };
await writeFile(join(root, 'acquisition.json'), `${JSON.stringify(manifest, null, 2)}\n`);
if (receipts.some((r) => r.httpStatus !== 200 || r.contextSlot !== slot || r.rpcError)) {
  process.exitCode = 2;
}
