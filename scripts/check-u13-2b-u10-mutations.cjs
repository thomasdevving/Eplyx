#!/usr/bin/env node
const fs = require('node:fs');
const { spawnSync } = require('node:child_process');
const binary = 'target/debug/examples/qualify_u10_historical_replay.exe';
const cases = {
  'environment-blockhash': 'RuntimeConfigurationMismatch: durable nonce differs',
  'nonce-account': 'frozen seed snapshot identity differs',
  'recent-blockhashes': 'runtime sysvar snapshot differs from resolved profile',
  'feature-profile': 'unsupported historical runtime profile',
  'historical-bpf': 'frozen seed snapshot identity differs',
  'runtime-profile-identity': 'resolved runtime profile identity differs',
  's-checkpoint': 'checkpoint S snapshot identity differs',
};
const results = Object.entries(cases).map(([name, expected]) => {
  const run = spawnSync(binary, [name], { encoding: 'utf8' });
  return { name, exit: run.status, expectedFailure: expected,
    failClosed: run.status !== 0 && run.stderr.includes(expected) };
});
const report = { schema: 'U13_2BU10MutationControlsV1', results,
  allFailClosed: results.every(r => r.failClosed) };
fs.writeFileSync('docs/examples/phase-u13-2b-feature-universe/u10-mutations.json',
  `${JSON.stringify(report, null, 2)}\n`);
console.log(JSON.stringify({ allFailClosed: report.allFailClosed, cases: results.length }));
if (!report.allFailClosed) process.exitCode = 2;
