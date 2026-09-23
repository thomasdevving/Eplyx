// Offline input qualification for U13.2A. No replay observation or semantic decode.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../docs/examples/phase-u13-2a-runtime');
const u13 = path.resolve(__dirname, '../docs/examples/phase-u13-drift-witness');
const u131 = path.resolve(__dirname, '../docs/examples/phase-u13-1-causal-closure');
const slot = 409942000, parent = slot - 1;
const drift = 'dRiftyHA39MWEi3m9aunc5MzRF1JYuBsbn6VPcn33UH';
const pdata = '7dLgmtcTavcguNoynVimF9ZNVb13FvhXVRfj2HyrDGaP';
const compute = 'ComputeBudget111111111111111111111111111111';
const sha = b => crypto.createHash('sha256').update(b).digest('hex');
function check(ok, message) { if (!ok) throw Error(message); }
function json(file) { return JSON.parse(fs.readFileSync(file)); }
const alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function b58(bytes) {
  let n = BigInt('0x' + (bytes.toString('hex') || '0')), result = '';
  while (n) { result = alphabet[Number(n % 58n)] + result; n /= 58n; }
  let zeros = 0;
  while (zeros < bytes.length && !bytes[zeros]) zeros++;
  return '1'.repeat(zeros) + result;
}
function un58(value) {
  let n = 0n;
  for (const ch of value) {
    const d = alphabet.indexOf(ch); check(d >= 0, 'invalid base58');
    n = n * 58n + BigInt(d);
  }
  let h = n.toString(16); if (h.length % 2) h = `0${h}`;
  return Buffer.concat([Buffer.alloc(value.match(/^1*/)[0].length),
    n ? Buffer.from(h, 'hex') : Buffer.alloc(0)]);
}
function rawAccount(file, expectedSlot, allowSlice = false) {
  const raw = fs.readFileSync(file), response = JSON.parse(raw);
  check(response.result?.context?.slot === expectedSlot, `${file}: context slot`);
  const value = response.result.value;
  if (!value) return { rawSha256: sha(raw), value: null, data: null };
  check(value.data[1] === 'base64', `${file}: account encoding`);
  const data = Buffer.from(value.data[0], 'base64');
  check(allowSlice ? data.length <= value.space : data.length === value.space,
    `${file}: account is incomplete`);
  return { rawSha256: sha(raw), value, data };
}
const acquisition = json(path.join(root, 'acquisition.json'));
check(acquisition.providerHost === 'solana-mainnet.g.alchemy.com'
  && acquisition.requestedSlot === slot && acquisition.receipts.length === 316,
  'runtime acquisition manifest identity');
const byName = new Map();
for (const receipt of acquisition.receipts) {
  const file = path.join(root, 'raw', receipt.name);
  const raw = fs.readFileSync(file);
  check(raw.length === receipt.responseBytes && sha(raw) === receipt.responseSha256
    && receipt.params[0] === receipt.address && receipt.params[1].slot === slot
    && receipt.contextSlot === slot && receipt.httpStatus === 200 && !receipt.rpcErrorCode,
  `${receipt.name}: raw receipt mismatch`);
  const actual = rawAccount(file, slot);
  check(!!actual.value === receipt.present
    && (actual.data ? sha(actual.data) : null) === receipt.dataSha256,
  `${receipt.name}: decoded receipt mismatch`);
  byName.set(receipt.name, { ...actual, receipt });
}
const archive = path.join(u13, 'raw', 'accounts');
const program = rawAccount(path.join(archive, '409941999-program.json'), parent);
const header = rawAccount(path.join(archive, '409941999-programdata-header.json'), parent, true);
check(program.value.owner === 'BPFLoaderUpgradeab1e11111111111111111111111'
  && program.value.executable && program.data.length === 36
  && program.data.readUInt32LE(0) === 2, 'upgradeable Program account');
check(b58(program.data.subarray(4, 36)) === pdata, 'ProgramData pointer differs');
check(header.value.owner === program.value.owner && !header.value.executable
  && header.data.length === 64, 'ProgramData header slice metadata');
const prior = json(path.join(u131, 'acquisition.json'));
const chunks = prior.programData.chunks.map(c => {
  const file = path.join(u131, 'raw', c.receipt);
  const raw = fs.readFileSync(file), parsed = JSON.parse(raw);
  const receipt = prior.receipts.find(r => r.name === c.receipt);
  check(receipt && raw.length === receipt.bytes && sha(raw) === receipt.sha256
    && parsed.result.context.slot === parent
    && parsed.result.value.space === prior.programData.space
    && parsed.result.value.owner === program.value.owner,
  `${c.receipt}: ProgramData receipt differs`);
  const data = Buffer.from(parsed.result.value.data[0], 'base64');
  check(data.length === c.length && sha(data) === c.dataSha256,
    `${c.receipt}: ProgramData content differs`);
  return data;
});
const all = Buffer.concat(chunks);
check(all.length === prior.programData.space
  && sha(all) === prior.programData.accountSha256
  && all.subarray(0, 64).equals(header.data), 'assembled ProgramData differs');
check(all.readUInt32LE(0) === 3, 'ProgramData discriminant');
const deploymentSlot = Number(all.readBigUInt64LE(4));
check(deploymentSlot === 409872290 && deploymentSlot < slot,
  'ProgramData deployment slot');
check(all[12] === 0 || all[12] === 1, 'ProgramData authority option');
const authority = all[12] ? b58(all.subarray(13, 45)) : null;
const elf = all.subarray(45);
check(elf.subarray(0, 4).equals(Buffer.from([127, 69, 76, 70]))
  && sha(elf) === prior.programData.elfSha256,
  'historical ELF differs');
fs.writeFileSync(path.join(root, 'historical-drift.so'), elf);
const blockRaw = fs.readFileSync(path.join(u13, 'raw', 'block-409942000.json'));
const block = JSON.parse(blockRaw).result;
const target = block.transactions[73];
check(target.transaction.signatures[0] ===
  '2BD3UJFPUPJbxoJMAntwyZrzLjzKv3yeuQERTu2rPRZLf4H8Wdjihsh6xChwC4Vxruxy9qyrrknmnvi7UJqCHRSK'
  && target.version === 0 && target.meta.err === null,
  'target transaction identity or result');
const msg = target.transaction.message;
check(msg.accountKeys.length === 16 && !msg.addressTableLookups.length
  && !target.meta.loadedAddresses.writable.length
  && !target.meta.loadedAddresses.readonly.length
  && target.meta.innerInstructions.length === 0, 'execution message envelope');
const programs = msg.instructions.map(ix => msg.accountKeys[ix.programIdIndex]);
check(JSON.stringify([...new Set(programs)]) === JSON.stringify([compute, drift]),
  'outer executable program census');
check(msg.instructions[0].programIdIndex === msg.instructions[1].programIdIndex
  && msg.instructions[2].programIdIndex === 5, 'instruction ordering');
const budgets = msg.instructions.slice(0, 2).map(ix => un58(ix.data));
check(budgets[0].length === 5 && budgets[0][0] === 2
  && budgets[1].length === 9 && budgets[1][0] === 3,
  'ComputeBudget instruction shape');
const cuLimit = budgets[0].readUInt32LE(1);
const microLamports = Number(budgets[1].readBigUInt64LE(1));
const priorityFee = Math.ceil(cuLimit * microLamports / 1000000);
check(target.meta.fee === 5000 + priorityFee, 'fee arithmetic');
const systemId = '11111111111111111111111111111111';
const durableNonce = msg.instructions[0].programIdIndex === msg.accountKeys.indexOf(systemId)
  && un58(msg.instructions[0].data).subarray(0, 4).equals(Buffer.from([4, 0, 0, 0]));
check(!durableNonce && !msg.accountKeys.includes(systemId), 'durable nonce unexpectedly present');
const sourceAccounts = [];
for (const key of msg.accountKeys) {
  if (key === compute) { sourceAccounts.push({ address: key, source: 'native_builtin' }); continue; }
  const file = key === drift ? path.join(archive, '409941999-program.json')
    : path.join(archive, `${parent}-${key}.json`);
  const account = rawAccount(file, parent);
  check(account.value !== null, `missing parent account ${key}`);
  sourceAccounts.push({ address: key, source: 'parent_archive', owner: account.value.owner,
    executable: account.value.executable, dataSha256: sha(account.data),
    responseSha256: account.rawSha256, bytes: account.data.length });
}
const features = acquisition.receipts.filter(r => r.kind === 'feature');
const earlier = json(path.resolve(__dirname, '../docs/examples/phase-u9-acquisition/acquisition.json'));
const laterById = new Map(earlier.feature_snapshot.accounts.map(r => [r.feature_id, r]));
const featureSummary = { inventoried: features.length, active: 0, inactivePresent: 0,
  prefundedNonFeature: 0, absent: 0, laterActivated: [], laterStillInactive: [] };
for (const item of features) {
  const after = laterById.get(item.address);
  check(after, 'feature missing from later census');
  if (!item.present) {
    check(after.activation_slot === null || after.activation_slot > slot,
      `${item.address}: active later feature absent at target`);
    featureSummary.absent++;
    if (after.activation_slot !== null) featureSummary.laterActivated.push({
      address: item.address, activationSlot: after.activation_slot, stateAtTarget: 'absent' });
    continue;
  }
  const account = byName.get(item.name);
  check(['Feature111111111111111111111111111111111111',
    '11111111111111111111111111111111'].includes(item.owner)
    && (account.data.length === 0 || account.data.length === 9)
    && (!account.data.length || [0, 1].includes(account.data[0])),
  `${item.address}: feature account layout`);
  if (item.owner !== 'Feature111111111111111111111111111111111111') {
    check(account.data.length === 0, `${item.address}: non-feature account has feature data`);
    featureSummary.prefundedNonFeature++;
  }
  const activation = account.data.length === 9 && account.data[0] === 1
    ? Number(account.data.readBigUInt64LE(1)) : null;
  if (activation === null) {
    check(after.activation_slot === null || after.activation_slot > slot,
      `${item.address}: inactive feature later state`);
    featureSummary.inactivePresent++;
    if (after.activation_slot !== null) featureSummary.laterActivated.push({
      address: item.address, activationSlot: after.activation_slot, stateAtTarget: 'inactive' });
    else featureSummary.laterStillInactive.push(item.address);
  } else {
    check(activation <= slot && activation === after.activation_slot,
      `${item.address}: feature activation differs from later census`);
    featureSummary.active++;
  }
}
check(featureSummary.active === 275 && featureSummary.inactivePresent === 4
  && featureSummary.prefundedNonFeature === 1 && featureSummary.absent === 32
  && featureSummary.laterActivated.length === 34,
  'target-slot feature census differs');
const clock = byName.get('sysvar-Clock.json');
const rent = byName.get('sysvar-Rent.json');
const epoch = byName.get('sysvar-EpochSchedule.json');
for (const row of [clock, rent, epoch])
  check(row.value.owner === 'Sysvar1111111111111111111111111111111111111',
    'historical sysvar owner');
check(clock.data.length === 40 && Number(clock.data.readBigUInt64LE(0)) === slot,
  'Clock slot differs');
check(rent.data.length === 17 && epoch.data.length === 33, 'sysvar layout differs');
const sysvars = { Clock: { dataSha256: sha(clock.data), slot,
    epoch: Number(clock.data.readBigUInt64LE(16)),
    unixTimestamp: Number(clock.data.readBigInt64LE(32)) },
  Rent: { dataSha256: sha(rent.data), lamportsPerByteYear: Number(rent.data.readBigUInt64LE(0)),
    exemptionThreshold: rent.data.readDoubleLE(8), burnPercent: rent.data[16] },
  EpochSchedule: { dataSha256: sha(epoch.data),
    slotsPerEpoch: Number(epoch.data.readBigUInt64LE(0)),
    leaderScheduleOffset: Number(epoch.data.readBigUInt64LE(8)), warmup: !!epoch.data[16] },
  RecentBlockhashes: { archivePresent: byName.get('sysvar-RecentBlockhashes.json').value !== null },
  SlotHashes: { archivePresent: byName.get('sysvar-SlotHashes.json').value !== null } };
const featureSourcePath = path.resolve(__dirname,
  '../docs/examples/phase-u9-freeze/runtime-source/litesvm-0.16.0/src/features.rs');
const featureSource = fs.readFileSync(featureSourcePath);
const u9Runtime = json(path.resolve(__dirname,
  '../docs/examples/phase-u9-analysis/runtime-evidence.json'));
check(sha(featureSource) === u9Runtime.runtime_family.litesvm_features_source_sha256,
  'frozen LiteSVM feature source differs');
const backendFeatureEntries = [...featureSource.toString().matchAll(
  /agave_feature_set::([A-Za-z0-9_:]+)::ID,\s*([\d_]+)/g)]
  .map(match => ({ name: match[1], activationSlot: Number(match[2].replaceAll('_', '')) }));
check(backendFeatureEntries.length > 200, 'LiteSVM feature source parser incomplete');
const laterBackendFeatures = backendFeatureEntries.filter(row => row.activationSlot > slot);
check(laterBackendFeatures.some(row => row.name === 'replace_spl_token_with_p_token')
  && laterBackendFeatures.some(row => row.name === 'provide_instruction_data_offset_in_vm_r2'),
  'expected post-target LiteSVM features missing');
const backendSource = fs.readFileSync(path.resolve(__dirname,
  '../engine/src/universal/execution.rs'), 'utf8');
check(backendSource.includes('let mut svm = LiteSVM::new()')
  && !backendSource.includes('.with_feature_set('),
  'current universal backend feature configuration changed');
const audit = { schema: 'U13_2AExecutionInputAuditV1', slot, parentSlot: parent,
  providerHost: acquisition.providerHost, rawReceiptCount: acquisition.receipts.length,
  sourceBlockSha256: sha(blockRaw),
  historicalProgram: { programId: drift, loader: program.value.owner,
    programResponseSha256: program.rawSha256, programDataAddress: pdata,
    programDataHeaderResponseSha256: header.rawSha256,
    programDataAccountSha256: sha(all), programDataBytes: all.length,
    layout: { programDiscriminant: 2, programDataDiscriminant: 3, elfOffset: 45 },
    deploymentSlot, upgradeAuthority: authority,
    elfSha256: sha(elf), elfBytes: elf.length },
  envelope: { signature: target.transaction.signatures[0], index: 73,
    messageVersion: target.version, staticKeys: msg.accountKeys.length,
    loadedKeys: 0, lookupTables: 0, outerProgramIds: [...new Set(programs)],
    innerInstructionGroups: target.meta.innerInstructions.length,
    preTokenBalanceRows: target.meta.preTokenBalances.length,
    postTokenBalanceRows: target.meta.postTokenBalances.length,
    durableNonce, recentBlockhash: msg.recentBlockhash,
    bankEnvironmentBlockhash: block.previousBlockhash,
    computeUnitLimit: cuLimit, microLamportsPerUnit: microLamports,
    priorityFee, fee: target.meta.fee, computeUnitsConsumed: target.meta.computeUnitsConsumed },
  sourceAccounts, implicitProgramDataSeed: pdata,
  featureSummary, sysvars,
  backendFeatureGap: { litesvmSourceSha256: sha(featureSource),
    parsedBackendFeatures: backendFeatureEntries.length,
    backendFeaturesAfterTarget: laterBackendFeatures,
    backendInstantiatesMainnetDefault: true,
    backendCanInjectHistoricalFeatureSet: false } };
fs.writeFileSync(path.join(root, 'input-audit.json'), `${JSON.stringify(audit, null, 2)}\n`);
console.log(JSON.stringify({ elfSha256: audit.historicalProgram.elfSha256,
  deploymentSlot, upgradeAuthority: authority, targetSeeds: sourceAccounts.length,
  featureSummary: { active: featureSummary.active, inactivePresent: featureSummary.inactivePresent,
    absent: featureSummary.absent, laterActivated: featureSummary.laterActivated.length },
  backendFeaturesAfterTarget: laterBackendFeatures.length,
  sysvars, fee: target.meta.fee }, null, 2));
