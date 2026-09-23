#!/usr/bin/env node
// Read-only frozen product controls; writes only the new U13.2B receipt.
const fs = require('node:fs');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');
const root = 'docs/examples/phase-u13-2b-feature-universe';
const cli = 'target/debug/eplyx.exe';
const u10 = 'target/debug/examples/qualify_u10_historical_replay.exe';
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const candidate = bundle => `${bundle}/binaries/current.so`;
const verify = bundle => ['bundle', 'verify', '--bundle', bundle, '--format', 'json'];
const ci = bundle => ['ci', 'check', '--bundle', bundle, '--candidate', candidate(bundle), '--format', 'json'];
const cases = [
  ['u10', u10, [], 0, '12635f15e3fb2ffae466cccf7985c2edfaefaffb39b55d28a094892e07864edd'],
  ['u11_verify', cli, verify('docs/examples/phase-u11-checkpointed-bundle'), 0, 'a1a05576abb0ee215080890c6d413263205c888217e7a9eb85627707783b27b9'],
  ['u11_ci', cli, ci('docs/examples/phase-u11-checkpointed-bundle'), 2, 'd2c79c7d00abc8df7ae729f063d43edde089754256b731372ff471c3747c358e'],
  ['u11_2_ci', cli, ci('docs/examples/phase-u11-2-checkpointed-bundle'), 2, '0114c76d03fc8c294d309e0fecb4e7d554f4c5b0dd71ed8aaf973c7cb69476c7'],
  ['u12_ci', cli, ci('docs/examples/phase-u12-orca-semantic-bundle'), 0, 'e4fa879f568de5eb7301e34fa06a536513a2b574f424a7bb9af9f53f185e74e0'],
  ['u12_1_verify', cli, verify('docs/examples/phase-u12-1-semantic-binding-bundle'), 0, '0ea4d7a9fddf1e11e12176b6d45d4f6fd94e1eadd09b2f1fdd25db61eef58e64'],
  ['u12_1_ci', cli, ci('docs/examples/phase-u12-1-semantic-binding-bundle'), 0, '07ce6389c6b2b0105e1886db6c15648eece8a586c2fdba93f054bc1b5762e04a'],
  ['kamino_verify', cli, verify('docs/examples/phase-u4-kamino-bundle'), 0, '529355d96e95bf81c44debd50a007f3624612009cffcf5cee47cc631d05e3b18'],
  ['kamino_ci', cli, ci('docs/examples/phase-u4-kamino-bundle'), 0, 'b7f33953ca35bcdc7949a52a9ea5263739e1be20a1c00ddc1d0d7c89f1a8fe44'],
];
const stake = JSON.parse(fs.readFileSync('docs/examples/phase-u3-before/controls.json')).cases;
for (const row of stake) {
  const name = row.case;
  const args = name === 'verify' ? ['bundle', 'verify', '--bundle', 'deploy/bundle'] :
    ['ci', 'check', '--bundle', 'deploy/bundle', '--candidate',
      ['regression', 'bounded'].includes(name) ? 'artifacts/fixture_stake_pool_v2.so' :
        'deploy/bundle/binaries/current.so', '--format', 'json'];
  if (['bounded', 'stale', 'unevaluable'].includes(name)) args.push('--expectations', `docs/pilot/expected-changes.${name}.toml`);
  cases.push([`stake_${name}`, cli, args, row.exit_code,
    ['regression', 'bounded'].includes(name) ? null : row.stdout_sha256]);
}
const results = cases.map(([name, executable, args, expectedExit, expectedSha256]) => {
  const run = spawnSync(executable, args, { encoding: null, maxBuffer: 32 * 1024 * 1024,
    env: Object.fromEntries(Object.entries(process.env).filter(([key]) =>
      !/RPC|API_KEY|ARCHIVE/i.test(key))) });
  const stdout = run.stdout || Buffer.alloc(0);
  const result = { name, exit: run.status, expectedExit, stdoutSha256: sha(stdout), expectedSha256,
    matchesExit: run.status === expectedExit,
    matchesFrozenBytes: expectedSha256 === null ? null : sha(stdout) === expectedSha256,
    error: run.error?.message || null };
  console.log(JSON.stringify(result));
  return result;
});
const report = { schema: 'U13_2BCompatibilityControlsV1', results,
  allRequiredMatched: results.every(r => r.matchesExit && r.matchesFrozenBytes !== false) };
fs.writeFileSync(`${root}/compatibility.json`, `${JSON.stringify(report, null, 2)}\n`);
if (!report.allRequiredMatched) process.exitCode = 2;
