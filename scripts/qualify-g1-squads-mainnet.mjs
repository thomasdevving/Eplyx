#!/usr/bin/env node
// Phase G1.1: real-mainnet qualification support for the Squads binding.
//
// An INDEPENDENT decoder: hand-written Borsh over the layouts pinned in
// Squads-Protocol/v4 @ af94153f, ed25519 PDA derivation in BigInt, loader-v3
// states by hand. It imports nothing from Eplyx. Qualification claims come
// from the `eplyx` binary; this script discovers candidates, measures the
// population, and cross-checks what the binary reports.
//
//   node scripts/qualify-g1-squads-mainnet.mjs census  --out FILE
//   node scripts/qualify-g1-squads-mainnet.mjs sample  --n 300 --seed 1 --out FILE
//   node scripts/qualify-g1-squads-mainnet.mjs witness --transaction ADDR --out FILE
//
// The endpoint comes from EPLYX_GOVERNANCE_RPC_URL. Only its origin host is
// ever written out; a credential-bearing URL is never printed or stored.

import { createHash } from 'node:crypto';
import { writeFileSync } from 'node:fs';

const RPC = process.env.EPLYX_GOVERNANCE_RPC_URL;
if (!RPC) throw new Error('EPLYX_GOVERNANCE_RPC_URL is required');
const PROVIDER = new URL(RPC).origin.replace(/\/\/[^@]*@/, '//');

export const SQUADS = 'SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf';
const LOADER = 'BPFLoaderUpgradeab1e11111111111111111111111';
const RENT = 'SysvarRent111111111111111111111111111111111';
const CLOCK = 'SysvarC1ock11111111111111111111111111111111';
const DISC = {
  Multisig: [224, 116, 121, 186, 68, 161, 79, 236],
  VaultTransaction: [168, 250, 162, 100, 81, 14, 162, 207],
  Proposal: [26, 94, 189, 187, 116, 136, 53, 33],
};
const STATUS = ['draft', 'active', 'rejected', 'approved', 'executing', 'executed', 'cancelled'];
const LOADER_IX = ['InitializeBuffer', 'Write', 'DeployWithMaxDataLen', 'Upgrade', 'SetAuthority', 'Close', 'ExtendProgram', 'SetAuthorityChecked', 'Migrate', 'ExtendProgramChecked'];

// ---------------------------------------------------------------- transport
let requests = 0;
export async function rpc(method, params, attempt = 0) {
  requests += 1;
  const response = await fetch(RPC, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) });
  if (response.status === 429 && attempt < 6) {
    await new Promise(r => setTimeout(r, 1500 * 2 ** attempt));
    return rpc(method, params, attempt + 1);
  }
  const body = await response.json();
  if (body.error) throw new Error(`${method}: ${JSON.stringify(body.error)}`);
  return body.result;
}

// ------------------------------------------------------------------- base58
const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
export function b58encode(bytes) {
  let n = BigInt('0x' + (Buffer.from(bytes).toString('hex') || '0'));
  let s = '';
  while (n > 0n) { s = B58[Number(n % 58n)] + s; n /= 58n; }
  for (const b of bytes) { if (b) break; s = '1' + s; }
  return s;
}
export function b58decode(text) {
  let n = 0n;
  for (const c of text) { const d = B58.indexOf(c); if (d < 0) throw new Error(`bad base58 ${text}`); n = n * 58n + BigInt(d); }
  const out = [];
  while (n > 0n) { out.unshift(Number(n & 0xffn)); n >>= 8n; }
  for (const c of text) { if (c !== '1') break; out.unshift(0); }
  return Buffer.from(out);
}

// ------------------------------------------------------ ed25519 on-curve, PDA
const P = 2n ** 255n - 19n;
const D = (-121665n * modpow(121666n, P - 2n)) % P;
function modpow(b, e, m = P) { let r = 1n; b %= m; if (b < 0n) b += m; while (e > 0n) { if (e & 1n) r = (r * b) % m; b = (b * b) % m; e >>= 1n; } return r; }
function onCurve(bytes) {
  const y = BigInt('0x' + Buffer.from(bytes).reverse().toString('hex')) & ((1n << 255n) - 1n);
  if (y >= P) return false;
  const y2 = (y * y) % P;
  const u = (y2 - 1n + P) % P;
  const v = (D * y2 + 1n) % P;
  const x2 = (u * modpow(v, P - 2n)) % P;
  if (x2 === 0n) return true;
  return modpow(x2, (P - 1n) / 2n) === 1n; // Euler: x² is a square
}
export function createPda(seeds, program) {
  const h = createHash('sha256');
  for (const s of seeds) h.update(s);
  h.update(b58decode(program));
  h.update(Buffer.from('ProgramDerivedAddress'));
  const out = h.digest();
  return onCurve(out) ? null : b58encode(out);
}
export function findPda(seeds, program) {
  for (let bump = 255; bump >= 0; bump--) {
    const address = createPda([...seeds, Buffer.from([bump])], program);
    if (address) return { address, bump };
  }
  throw new Error('no PDA');
}
const u8 = n => Buffer.from([n]);
const u64 = n => { const b = Buffer.alloc(8); b.writeBigUInt64LE(BigInt(n)); return b; };
export const pdas = {
  multisig: createKey => findPda([Buffer.from('multisig'), Buffer.from('multisig'), b58decode(createKey)], SQUADS),
  vault: (multisig, index) => findPda([Buffer.from('multisig'), b58decode(multisig), Buffer.from('vault'), u8(index)], SQUADS),
  transaction: (multisig, index) => findPda([Buffer.from('multisig'), b58decode(multisig), Buffer.from('transaction'), u64(index)], SQUADS),
  proposal: (multisig, index) => findPda([Buffer.from('multisig'), b58decode(multisig), Buffer.from('transaction'), u64(index), Buffer.from('proposal')], SQUADS),
  programdata: program => findPda([b58decode(program)], LOADER),
};

// -------------------------------------------------------------------- borsh
class Reader {
  constructor(buf) { this.buf = buf; this.at = 0; }
  take(n) { if (this.at + n > this.buf.length) throw new Error('truncated'); const s = this.buf.subarray(this.at, this.at + n); this.at += n; return s; }
  u8() { return this.take(1)[0]; }
  u16() { return this.take(2).readUInt16LE(0); }
  u32() { return this.take(4).readUInt32LE(0); }
  u64() { return this.take(8).readBigUInt64LE(0); }
  i64() { return this.take(8).readBigInt64LE(0); }
  key() { return b58encode(this.take(32)); }
  vec(f) { const n = this.u32(); const out = []; for (let i = 0; i < n; i++) out.push(f()); return out; }
  bytes() { return Buffer.from(this.take(this.u32())); }
  rest() { return this.buf.subarray(this.at); }
}
function anchor(data, name) {
  if (data.length < 8 || !DISC[name].every((b, i) => data[i] === b)) throw new Error(`not a ${name}`);
  return new Reader(data.subarray(8));
}
const zeroTail = r => r.rest().every(b => b === 0);

export function decodeMultisig(data) {
  const r = anchor(data, 'Multisig');
  const m = {
    create_key: r.key(), config_authority: r.key(), threshold: r.u16(), time_lock: r.u32(),
    transaction_index: r.u64(), stale_transaction_index: r.u64(),
    rent_collector: r.u8() ? r.key() : null, bump: r.u8(),
    members: r.vec(() => ({ key: r.key(), permissions: r.u8() })),
  };
  m.zero_tail = zeroTail(r);
  return m;
}
export function decodeVaultTransaction(data) {
  const r = anchor(data, 'VaultTransaction');
  const t = { multisig: r.key(), creator: r.key(), index: r.u64(), bump: r.u8(), vault_index: r.u8(), vault_bump: r.u8(), ephemeral_signer_bumps: [...r.bytes()] };
  const start = 8 + r.at;
  t.message = {
    num_signers: r.u8(), num_writable_signers: r.u8(), num_writable_non_signers: r.u8(),
    account_keys: r.vec(() => r.key()),
    instructions: r.vec(() => ({ program_id_index: r.u8(), account_indexes: [...r.bytes()], data: r.bytes().toString('hex') })),
    address_table_lookups: r.vec(() => ({ account_key: r.key(), writable_indexes: [...r.bytes()], readonly_indexes: [...r.bytes()] })),
  };
  t.message_bytes = Buffer.from(data.subarray(start, 8 + r.at));
  t.zero_tail = zeroTail(r);
  return t;
}
export function decodeProposal(data) {
  const r = anchor(data, 'Proposal');
  const p = { multisig: r.key(), transaction_index: r.u64() };
  const variant = r.u8();
  p.status = STATUS[variant] ?? `unknown(${variant})`;
  p.status_timestamp = variant === 4 ? null : r.i64();
  p.bump = r.u8();
  p.approved = r.vec(() => r.key()); p.rejected = r.vec(() => r.key()); p.cancelled = r.vec(() => r.key());
  p.zero_tail = zeroTail(r);
  return p;
}
export const messageHash = bytes => createHash('sha256').update(Buffer.concat([Buffer.from('eplyx-squads-v4-vault-message-v1'), Buffer.from([0]), bytes])).digest('hex');

// ------------------------------------------------------------------- loader
export function loaderState(data) {
  const tag = data.readUInt32LE(0);
  if (tag === 1) return { state: 'buffer', authority: data[4] === 1 ? b58encode(data.subarray(5, 37)) : null, bytes: data.subarray(37) };
  if (tag === 2) return { state: 'program', programdata: b58encode(data.subarray(4, 36)) };
  if (tag === 3) return { state: 'programdata', deploy_slot: data.readBigUInt64LE(4), authority: data[12] === 1 ? b58encode(data.subarray(13, 45)) : null, bytes: data.subarray(45) };
  return { state: `tag ${tag}` };
}

/** The G1 contract, restated independently. Returns [class, detail]. */
export function classify(tx) {
  const m = tx.message;
  const loaderIdx = m.account_keys.indexOf(LOADER);
  const loaderIxs = m.instructions.filter(ix => ix.program_id_index === loaderIdx);
  const loaderName = hex => { const d = Buffer.from(hex, 'hex'); return d.length >= 4 ? LOADER_IX[d.readUInt32LE(0)] ?? `?${d.readUInt32LE(0)}` : `empty(${d.length})`; };
  const names = m.instructions.map(ix => ix.program_id_index === loaderIdx ? `loader:${loaderName(ix.data)}` : `program:${m.account_keys[ix.program_id_index] ?? `#${ix.program_id_index}`}`);
  const vault = pdas.vault(tx.multisig, tx.vault_index).address;
  if (m.address_table_lookups.length) return ['lut_backed', names];
  if (tx.ephemeral_signer_bumps.length) return ['ephemeral_signers', names];
  if (m.instructions.length !== 1) return [loaderIxs.length ? 'multiple_instructions_with_loader' : 'multiple_instructions', names];
  const ix = m.instructions[0];
  if (ix.program_id_index !== loaderIdx) return ['non_upgrade_action', names];
  const data = Buffer.from(ix.data, 'hex');
  const upgrade = data.length >= 4 && data.readUInt32LE(0) === 3;
  if (!upgrade) return ['other_loader_instruction', names];
  const hex = data.toString('hex');
  if (!(hex === '03000000' || hex === '0300000001')) return ['upgrade_noncanonical_data', [hex]];
  const idx = ix.account_indexes;
  const reasons = [];
  if (m.num_signers !== 1 || m.account_keys[0] !== vault) reasons.push('signer_shape');
  if (idx.length !== 7) reasons.push('account_count');
  else {
    if (new Set(idx).size !== 7) reasons.push(`duplicate_indexes ${JSON.stringify(idx)}`);
    const writable = i => i < m.num_writable_signers || (i >= m.num_signers && i - m.num_signers < m.num_writable_non_signers);
    for (const p of [0, 1, 2, 3]) if (!writable(idx[p]) || idx[p] < m.num_signers) reasons.push(`position ${p} not a writable non-signer`);
    if (m.account_keys[idx[4]] !== RENT) reasons.push('rent');
    if (m.account_keys[idx[5]] !== CLOCK) reasons.push('clock');
    if (idx[6] !== 0) reasons.push('authority not index 0');
    const used = new Set([...idx, loaderIdx]);
    if (used.size !== m.account_keys.length) reasons.push('unreferenced keys');
    if (pdas.programdata(m.account_keys[idx[1]]).address !== m.account_keys[idx[0]]) reasons.push('programdata not derived');
  }
  if (new Set(m.account_keys).size !== m.account_keys.length) reasons.push('duplicate keys');
  return reasons.length ? ['upgrade_other_shape', reasons] : ['exact_supported_upgrade', names];
}

async function accounts(keys, slice = null) {
  const out = [];
  for (let i = 0; i < keys.length; i += 100) {
    const config = { encoding: 'base64', commitment: 'finalized' };
    if (slice) config.dataSlice = slice;
    const res = await rpc('getMultipleAccounts', [keys.slice(i, i + 100), config]);
    out.push(...res.value.map((v, j) => ({ address: keys[i + j], slot: res.context.slot, account: v && { ...v, data: Buffer.from(v.data[0], 'base64') } })));
  }
  return out;
}

/** Status, buffer liveness and authorities for one decoded transaction. */
async function enrich(rows) {
  const proposalKeys = rows.map(r => pdas.proposal(r.tx.multisig, r.tx.index).address);
  const proposals = await accounts(proposalKeys);
  rows.forEach((r, i) => {
    const p = proposals[i].account;
    r.proposal = proposalKeys[i];
    try { r.status = p ? decodeProposal(p.data).status : 'no_proposal'; } catch (e) { r.status = `unreadable: ${e.message}`; }
  });
  const upgrades = rows.filter(r => r.class === 'exact_supported_upgrade' || r.class === 'upgrade_other_shape');
  const targetKeys = upgrades.flatMap(r => {
    const m = r.tx.message; const ix = m.instructions.find(x => m.account_keys[x.program_id_index] === LOADER);
    r.upgrade = { programdata: m.account_keys[ix.account_indexes[0]], program: m.account_keys[ix.account_indexes[1]], buffer: m.account_keys[ix.account_indexes[2]], spill: m.account_keys[ix.account_indexes[3]] };
    return [r.upgrade.programdata, r.upgrade.buffer];
  });
  // Headers only: ProgramData and buffers run to megabytes, and a public
  // endpoint's data allowance is not spent on bytes a census does not hash.
  const targets = await accounts(targetKeys, { offset: 0, length: 45 });
  upgrades.forEach((r, i) => {
    const pd = targets[2 * i].account; const buf = targets[2 * i + 1].account;
    const vault = pdas.vault(r.tx.multisig, r.tx.vault_index).address;
    r.upgrade.upgrade_authority = pd?.owner === LOADER ? loaderState(pd.data).authority : 'unreadable';
    r.upgrade.upgrade_authority_is_vault = r.upgrade.upgrade_authority === vault;
    if (!buf) { r.upgrade.buffer_state = 'absent'; return; }
    const s = buf.owner === LOADER ? loaderState(buf.data) : { state: `owner ${buf.owner}` };
    r.upgrade.buffer_state = s.state;
    if (s.state === 'buffer') {
      r.upgrade.buffer_authority = s.authority;
      r.upgrade.buffer_authority_is_vault = s.authority === vault;
      r.upgrade.buffer_len = buf.space - 37;
    }
  });
}

function decodeRow(address, account) {
  const row = { transaction: address };
  try {
    row.tx = decodeVaultTransaction(account.data);
    [row.class, row.detail] = classify(row.tx);
  } catch (e) { row.class = 'malformed'; row.detail = [e.message]; }
  return row;
}
/** Population files: summary fields pretty, one row per line. */
function writePopulation(path, out) {
  const { rows, ...head } = out;
  const text = JSON.stringify(head, null, 2).replace(/\n}$/, ',\n  "rows": [\n' + rows.map(r => '    ' + JSON.stringify(r)).join(',\n') + '\n  ]\n}\n');
  writeFileSync(path, text);
}
const summary = rows => rows.reduce((acc, r) => { const k = r.class; acc[k] = (acc[k] ?? 0) + 1; return acc; }, {});
const statusByClass = rows => rows.reduce((acc, r) => { const k = `${r.class}/${r.status}`; acc[k] = (acc[k] ?? 0) + 1; return acc; }, {});
const strip = r => ({ transaction: r.transaction, proposal: r.proposal, multisig: r.tx?.multisig, index: r.tx ? Number(r.tx.index) : null, vault_index: r.tx?.vault_index, class: r.class, detail: r.detail, status: r.status, upgrade: r.upgrade, message_sha256: r.tx ? messageHash(r.tx.message_bytes) : null });

// ------------------------------------------------------------------ commands
const args = Object.fromEntries(process.argv.slice(3).reduce((a, x, i, all) => (x.startsWith('--') ? [...a, [x.slice(2), all[i + 1]]] : a), []));
const command = process.argv[2];
const slot = async () => rpc('getSlot', [{ commitment: 'finalized' }]);

if (command === 'census') {
  // Every live VaultTransaction whose static keys include the loader at index
  // j (0..31), for 0..2 ephemeral signer bumps. Offsets per the pinned layout:
  // 8 disc + 32 + 32 + 8 + 3 = 83 → eph u32; message at 87+e; keys at 94+e.
  const started = await slot();
  const found = new Map();
  let queries = 0;
  for (let e = 0; e <= 2; e++) {
    for (let j = 0; j < 32; j++) {
      queries += 1;
      const res = await rpc('getProgramAccounts', [SQUADS, { encoding: 'base64', commitment: 'finalized', filters: [
        { memcmp: { offset: 0, bytes: b58encode(DISC.VaultTransaction) } },
        { memcmp: { offset: 83, bytes: b58encode(Buffer.from([e, 0, 0, 0])) } },
        { memcmp: { offset: 94 + e + 32 * j, bytes: LOADER } },
      ] }]);
      for (const a of res) found.set(a.pubkey, Buffer.from(a.account.data[0], 'base64'));
    }
  }
  const rows = [...found].map(([address, data]) => decodeRow(address, { data }));
  await enrich(rows.filter(r => r.tx));
  const out = { provider: PROVIDER, population: 'live Squads V4 VaultTransaction accounts whose static account_keys contain BPFLoaderUpgradeable at index 0..31, with 0..2 ephemeral signer bumps', started_slot: started, finished_slot: await slot(), gpa_queries: queries, requests, count: rows.length, classes: summary(rows), class_status: statusByClass(rows), rows: rows.map(strip) };
  writePopulation(args.out, out);
  console.log(JSON.stringify({ count: out.count, classes: out.classes, class_status: out.class_status }, null, 2));
} else if (command === 'sample') {
  // A seeded uniform sample of every live VaultTransaction account.
  const started = await slot();
  const all = await rpc('getProgramAccounts', [SQUADS, { encoding: 'base64', commitment: 'finalized', dataSlice: { offset: 0, length: 0 }, filters: [{ memcmp: { offset: 0, bytes: b58encode(DISC.VaultTransaction) } }] }]);
  const keys = all.map(a => a.pubkey).sort();
  let seed = Number(args.seed ?? 1);
  const rand = () => { seed = (seed * 1103515245 + 12345) % 2 ** 31; return seed / 2 ** 31; };
  const picked = new Set();
  const n = Number(args.n ?? 300);
  while (picked.size < Math.min(n, keys.length)) picked.add(keys[Math.floor(rand() * keys.length)]);
  const read = await accounts([...picked]);
  const rows = read.filter(r => r.account).map(r => decodeRow(r.address, r.account));
  await enrich(rows.filter(r => r.tx));
  const out = { provider: PROVIDER, population: `seeded uniform sample (LCG seed ${args.seed ?? 1}) of ${n} of all ${keys.length} live VaultTransaction accounts`, started_slot: started, total_live: keys.length, requests, count: rows.length, classes: summary(rows), class_status: statusByClass(rows), rows: rows.map(strip) };
  writePopulation(args.out, out);
  console.log(JSON.stringify({ total_live: out.total_live, count: out.count, classes: out.classes, class_status: out.class_status }, null, 2));
} else if (command === 'witness') {
  // Independent decode of one proposal, for comparison with Eplyx's binding.
  const [tx] = await accounts([args.transaction]);
  const t = decodeVaultTransaction(tx.account.data);
  const multisigPda = t.multisig;
  const derivedTx = pdas.transaction(t.multisig, t.index);
  const derivedProposal = pdas.proposal(t.multisig, t.index);
  const vault = pdas.vault(t.multisig, t.vault_index);
  const m = t.message; const ix = m.instructions[0];
  const up = { programdata: m.account_keys[ix.account_indexes[0]], program: m.account_keys[ix.account_indexes[1]], buffer: m.account_keys[ix.account_indexes[2]], spill: m.account_keys[ix.account_indexes[3]], rent: m.account_keys[ix.account_indexes[4]], clock: m.account_keys[ix.account_indexes[5]], authority: m.account_keys[ix.account_indexes[6]] };
  const read = await accounts([multisigPda, derivedProposal.address, up.program, up.programdata, up.buffer]);
  const [ms, pr, prog, pd, buf] = read.map(r => r.account);
  const multisig = decodeMultisig(ms.data);
  const proposal = decodeProposal(pr.data);
  const program = loaderState(prog.data); const programdata = loaderState(pd.data); const buffer = loaderState(buf.data);
  const bytes = buffer.bytes;
  let trailingZeros = 0; for (let i = bytes.length - 1; i >= 0 && bytes[i] === 0; i--) trailingZeros++;
  const out = {
    provider: PROVIDER, observed_slot: read[0].slot,
    transaction: { address: args.transaction, derived: derivedTx.address, derived_bump: derivedTx.bump, stored_bump: t.bump, index: Number(t.index), multisig: t.multisig, vault_index: t.vault_index, stored_vault_bump: t.vault_bump, ephemeral_signer_bumps: t.ephemeral_signer_bumps, zero_tail: t.zero_tail, creator: t.creator },
    multisig: { address: multisigPda, derived_from_create_key: pdas.multisig(multisig.create_key), stored_bump: multisig.bump, threshold: multisig.threshold, time_lock: multisig.time_lock, transaction_index: Number(multisig.transaction_index), stale_transaction_index: Number(multisig.stale_transaction_index), members: multisig.members.length, config_authority: multisig.config_authority, zero_tail: multisig.zero_tail },
    proposal: { address: derivedProposal.address, derived_bump: derivedProposal.bump, stored_bump: proposal.bump, multisig: proposal.multisig, transaction_index: Number(proposal.transaction_index), status: proposal.status, status_timestamp: proposal.status_timestamp == null ? null : Number(proposal.status_timestamp), approved: proposal.approved.length, rejected: proposal.rejected.length, cancelled: proposal.cancelled.length, zero_tail: proposal.zero_tail },
    vault: { address: vault.address, bump: vault.bump, signs_with_stored_bump: createPda([Buffer.from('multisig'), b58decode(t.multisig), Buffer.from('vault'), u8(t.vault_index), u8(t.vault_bump)], SQUADS) },
    message: { ...m, sha256: messageHash(t.message_bytes), encoded_len: t.message_bytes.length, class: classify(t) },
    upgrade: { ...up, programdata_derived: pdas.programdata(up.program).address },
    program: { owner: prog.owner, executable: prog.executable, state: program.state, programdata: program.programdata },
    programdata: { owner: pd.owner, account_len: pd.data.length, deploy_slot: Number(programdata.deploy_slot), upgrade_authority: programdata.authority, deployed_len: programdata.bytes.length, deployed_sha256: createHash('sha256').update(programdata.bytes).digest('hex') },
    buffer: { owner: buf.owner, state: buffer.state, authority: buffer.authority, account_len: buf.data.length, artifact_len: bytes.length, artifact_sha256: createHash('sha256').update(bytes).digest('hex'), elf_magic: bytes.subarray(0, 4).toString('hex') === '7f454c46', trailing_zero_bytes: trailingZeros, elf_section_header_end: elfEnd(bytes) },
    requests,
  };
  writeFileSync(args.out, JSON.stringify(out, (k, v) => typeof v === 'bigint' ? Number(v) : v, 2) + '\n');
  console.log(JSON.stringify(out, (k, v) => (typeof v === 'bigint' ? Number(v) : k === 'account_keys' || k === 'instructions' ? undefined : v), 2));
} else if (command === 'crosscheck') {
  // Field-by-field: Eplyx's sealed binding against this decoder's witness.
  const { readFileSync } = await import('node:fs');
  const b = JSON.parse(readFileSync(args.binding, 'utf8'));
  const w = JSON.parse(readFileSync(args.witness, 'utf8'));
  const o = b.observation; const d = o.delivery;
  const u32 = n => { const x = Buffer.alloc(4); x.writeUInt32LE(n); return x; };
  const vec = xs => Buffer.concat([u32(xs.length), Buffer.from(xs)]);
  const reencoded = Buffer.concat([
    Buffer.from([o.message.num_signers, o.message.num_writable_signers, o.message.num_writable_non_signers]),
    u32(o.message.account_keys.length), ...o.message.account_keys.map(b58decode),
    u32(o.message.instructions.length), ...o.message.instructions.map(ix => Buffer.concat([Buffer.from([ix.program_id_index]), vec(ix.account_indexes), vec(Buffer.from(ix.data, 'hex'))])),
    u32(o.message.address_table_lookups.length), ...o.message.address_table_lookups.map(l => Buffer.concat([b58decode(l.account_key), vec(l.writable_indexes), vec(l.readonly_indexes)])),
  ]);
  const { binding_id, ...body } = b;
  const same = (x, y) => JSON.stringify(x) === JSON.stringify(y);
  const checks = [
    ['binding_id recomputes', createHash('sha256').update(JSON.stringify(['eplyx-governance-binding-v1', body])).digest('hex') === binding_id],
    ['outcome', b.outcome === 'matched'],
    ['multisig', d.multisig === w.multisig.address && w.multisig.derived_from_create_key.address === w.multisig.address && w.multisig.derived_from_create_key.bump === w.multisig.stored_bump],
    ['transaction PDA + canonical bump', d.transaction === w.transaction.derived && w.transaction.derived === w.transaction.address && w.transaction.derived_bump === w.transaction.stored_bump],
    ['proposal PDA + canonical bump', d.proposal === w.proposal.address && w.proposal.derived_bump === w.proposal.stored_bump],
    ['vault PDA, stored bump signs as it', d.vault === w.vault.address && w.vault.signs_with_stored_bump === w.vault.address && d.vault_index === w.transaction.vault_index],
    ['transaction index', d.transaction_index === w.transaction.index && w.proposal.transaction_index === w.transaction.index],
    ['stored message, every field', same(o.message.account_keys, w.message.account_keys) && same(o.message.instructions, w.message.instructions) && same(o.message.address_table_lookups, w.message.address_table_lookups) && o.message.num_signers === w.message.num_signers && o.message.num_writable_signers === w.message.num_writable_signers && o.message.num_writable_non_signers === w.message.num_writable_non_signers],
    ['message hash: Eplyx == independent account decode', d.message_sha256 === w.message.sha256],
    ['message hash: Eplyx == re-encoded Eplyx view', d.message_sha256 === messageHash(reencoded)],
    ['proposal status + votes', o.proposal.status === w.proposal.status && o.proposal.approvals === w.proposal.approved && o.proposal.rejections === w.proposal.rejected && o.proposal.cancellations === w.proposal.cancelled],
    ['upgrade accounts', o.upgrade.program === w.upgrade.program && o.upgrade.programdata === w.upgrade.programdata && o.upgrade.buffer === w.upgrade.buffer && o.upgrade.spill === w.upgrade.spill && o.upgrade.authority === w.upgrade.authority],
    ['ProgramData derived from Program', w.upgrade.programdata === w.upgrade.programdata_derived],
    ['upgrade authority', o.current_program.upgrade_authority === w.programdata.upgrade_authority && w.programdata.upgrade_authority === w.vault.address],
    ['deployed executable', o.current_program.executable.sha256 === w.programdata.deployed_sha256 && o.current_program.executable.len === w.programdata.deployed_len],
    ['buffer authority', o.buffer.authority === w.buffer.authority && w.buffer.authority === w.vault.address],
    ['buffer artefact', o.buffer.artifact.sha256 === w.buffer.artifact_sha256 && o.buffer.artifact.len === w.buffer.artifact_len && b.expected.candidate.sha256 === w.buffer.artifact_sha256],
  ];
  let failed = 0;
  for (const [name, ok] of checks) { console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`); if (!ok) failed++; }
  console.log(`eplyx slot ${o.slot}, independent slot ${w.observed_slot}`);
  if (failed) process.exit(1);
} else {
  console.error('usage: census | sample | witness | crosscheck');
  process.exit(2);
}

/** Where an ELF64 file ends per its own header (end of section header table). */
function elfEnd(b) {
  if (b.length < 64 || b.subarray(0, 4).toString('hex') !== '7f454c46' || b[4] !== 2) return null;
  const shoff = Number(b.readBigUInt64LE(0x28)); const shentsize = b.readUInt16LE(0x3a); const shnum = b.readUInt16LE(0x3c);
  return shoff + shentsize * shnum;
}
