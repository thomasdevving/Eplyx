#!/usr/bin/env node
// Independent reproduction of the G1 Squads identities, outside the Rust
// implementation. Node built-ins only.
//
// From docs/examples/phase-g1-squads-binding/ it recomputes:
//   1. the canonical message hash, by decoding the stored VaultTransaction
//      account bytes by hand (Anchor discriminator + Borsh);
//   2. the same hash again, by re-encoding each binding's JSON message view;
//   3. every change_spec_id, from the documented identity tuple;
//   4. every binding_id, from the documented sealing tuple.
// Any disagreement exits non-zero.

import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

const DIR = new URL('../docs/examples/phase-g1-squads-binding/', import.meta.url).pathname;
const MESSAGE_DOMAIN = 'eplyx-squads-v4-vault-message-v1';
const VAULT_TRANSACTION_DISCRIMINATOR = [168, 250, 162, 100, 81, 14, 162, 207];

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const read = name => JSON.parse(readFileSync(join(DIR, name), 'utf8'));
let failures = 0;
const check = (condition, message) => {
  if (condition) console.log(`ok   ${message}`);
  else {
    failures += 1;
    console.log(`FAIL ${message}`);
  }
};

const messageHash = messageBytes =>
  sha256(Buffer.concat([Buffer.from(MESSAGE_DOMAIN), Buffer.from([0]), messageBytes]));

// ---- 1. the stored account, decoded by hand ------------------------------
function sliceMessage(data) {
  let at = 0;
  const take = n => {
    if (at + n > data.length) throw new Error('truncated');
    const out = data.subarray(at, at + n);
    at += n;
    return out;
  };
  const u32 = () => take(4).readUInt32LE(0);
  const u8 = () => take(1)[0];
  const discriminator = [...take(8)];
  if (discriminator.join() !== VAULT_TRANSACTION_DISCRIMINATOR.join()) throw new Error('not a VaultTransaction');
  take(32 + 32 + 8 + 1 + 1 + 1); // multisig, creator, index, bump, vault_index, vault_bump
  take(u32()); // ephemeral_signer_bumps
  const start = at;
  take(3); // num_signers, num_writable_signers, num_writable_non_signers
  take(32 * u32()); // account_keys
  for (let i = u32(); i > 0; i--) {
    u8();
    take(u32());
    take(u32());
  }
  for (let i = u32(); i > 0; i--) {
    take(32);
    take(u32());
    take(u32());
  }
  return data.subarray(start, at);
}

// ---- 2. re-encode a binding's message view --------------------------------
const B58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function base58(text) {
  let value = 0n;
  for (const c of text) {
    const digit = B58.indexOf(c);
    if (digit < 0) throw new Error(`bad base58 ${text}`);
    value = value * 58n + BigInt(digit);
  }
  const bytes = [];
  while (value > 0n) {
    bytes.unshift(Number(value & 0xffn));
    value >>= 8n;
  }
  for (const c of text) {
    if (c !== '1') break;
    bytes.unshift(0);
  }
  if (bytes.length !== 32) throw new Error(`${text} is not 32 bytes`);
  return Buffer.from(bytes);
}
const u32le = n => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n);
  return b;
};
const vec = bytes => Buffer.concat([u32le(bytes.length), Buffer.from(bytes)]);
function encodeMessage(view) {
  return Buffer.concat([
    Buffer.from([view.num_signers, view.num_writable_signers, view.num_writable_non_signers]),
    u32le(view.account_keys.length),
    ...view.account_keys.map(base58),
    u32le(view.instructions.length),
    ...view.instructions.map(ix => Buffer.concat([Buffer.from([ix.program_id_index]), vec(ix.account_indexes), vec(Buffer.from(ix.data, 'hex'))])),
    u32le(view.address_table_lookups.length),
    ...view.address_table_lookups.map(l => Buffer.concat([base58(l.account_key), vec(l.writable_indexes), vec(l.readonly_indexes)])),
  ]);
}

// `node scripts/verify-squads-binding.mjs FILE...` checks sealed bindings
// written anywhere (G1.1 mainnet evidence): id, message view → hash, slot.
const files = process.argv.slice(2);
if (files.length) {
  for (const file of files) {
    const binding = JSON.parse(readFileSync(file, 'utf8'));
    const { binding_id: stated, ...body } = binding;
    check(sha256(JSON.stringify(['eplyx-governance-binding-v1', body])) === stated, `${file} binding_id`);
    const view = binding.observation.message;
    if (view) check(messageHash(encodeMessage(view)) === binding.observation.delivery.message_sha256, `${file} message ${binding.observation.delivery.message_sha256.slice(0, 16)}… re-encodes`);
    check(binding.statement.includes(`slot ${binding.observation.slot}`), `${file} statement names slot ${binding.observation.slot}`);
  }
  if (failures) process.exit(1);
  console.log('\nevery binding reproduces independently');
  process.exit(0);
}

const fixture = read('vault-transaction.json');
const stored = sliceMessage(Buffer.from(fixture.data_base64, 'base64'));
const storedHash = messageHash(stored);
check(storedHash === fixture.message_sha256, `stored account message hash ${storedHash.slice(0, 16)}…`);
check(fixture.delivery.message_sha256 === storedHash, 'fixture delivery names the stored message');

// ---- 3. change spec identities --------------------------------------------
const changeSpecId = spec =>
  sha256(JSON.stringify(['eplyx-change-spec-v1', spec.schema_version, spec.change, spec.activation ?? null]));
const analysed = read('analysed-change-spec.json');
const bound = read('bound-change-spec.json');
for (const [name, spec] of [['analysed', analysed], ['bound', bound]]) {
  check(changeSpecId(spec) === spec.change_spec_id, `${name} change_spec_id ${spec.change_spec_id.slice(0, 16)}…`);
}
check(analysed.change.delivery === undefined, 'the analysed spec carries no delivery');
check(bound.change.delivery?.message_sha256 === storedHash, 'the bound spec names the stored message');
check(analysed.change_spec_id !== bound.change_spec_id, 'binding a delivery moves the identity');

// ---- 4. binding identities ------------------------------------------------
for (const name of readdirSync(DIR).filter(n => n.startsWith('binding-')).sort()) {
  const binding = read(name);
  const { binding_id: stated, ...body } = binding;
  check(sha256(JSON.stringify(['eplyx-governance-binding-v1', body])) === stated, `${name} binding_id`);
  check(binding.analysed_change_spec_id === bound.change_spec_id, `${name} is about the bound spec`);
  const view = binding.observation.message;
  if (view) {
    const hash = messageHash(encodeMessage(view));
    check(hash === binding.observation.delivery.message_sha256, `${name} message view re-encodes to its hash`);
    const sameMessage = hash === storedHash;
    check(sameMessage === (binding.outcome !== 'unsupported_proposal'), `${name} ${sameMessage ? 'shares' : 'does not share'} the stored message`);
  }
  check(typeof binding.observation.slot === 'number' && binding.statement.includes(`slot ${binding.observation.slot}`), `${name} statement names its slot`);
}

if (failures) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall G1 identities reproduce independently');
