// Terminal sessions for the week-of-4-October update. Same recorder as the
// product demo (../../demo-2026-10/source/pty_record.py): each command is typed
// into a real bash and executed; outputs and exit codes are what the binaries
// returned.
//
//   node sessions.mjs [name ...]      (records into ../assets/casts/)
import { spawnSync } from 'node:child_process';
import { writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, '../../..');
const recorder = join(here, '../../demo-2026-10/source/pty_record.py');
// A build of main after the week's last merge (323b9c5); EPLYX_BIN overrides.
const bin = [process.env.EPLYX_BIN || '/home/user/eplyx-rpc/target/release'];
const checkout = process.env.EPLYX_CHECKOUT || '/home/user/eplyx-rpc';
const D = '/home/demo';
const RECORD = 'mainnet-spl-stake-pool-151010f709e113e7';

export const sessions = {
  // The week, as git recorded it.
  'w-log': { cwd: repo, prompt: '~/eplyx', cols: 100, rows: 22, steps: [
    { cmd: 'git log --oneline --no-decorate --no-merges --since=2026-10-04 origin/main', expect: 0, after: 1.6 },
  ] },
  // Step 16A: a real loader Upgrade inside the retained Stake Pool world, in both orders.
  'w-rollout': { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 120, rows: 22, steps: [
    { cmd: `eplyx rollout analyse --upgrade proposals/rollout-upgrade.json --parameter proposals/deposit-fee-1pct.json --bundle .eplyx/bundle --record-id ${RECORD} --candidate target/deploy/fixture_stake_pool_rollout_v2.so --out rollout > rollout.json`, expect: 0, timeout: 300, after: .6, speed: 2.4 },
    { cmd: `jq -r '.scenarios[] | "\\(.name + " " * (17 - (.name | length)))\\(.steps | map("\\(.step) \\(.outcome)") | join("  →  "))"' rollout.json`, expect: 0, after: 1.2, speed: 2.4 },
    { cmd: "grep -E '^(Status|Under)' rollout/report.md | fold -s -w 118", expect: 0, after: 1.6, speed: 2 },
  ] },
  // The refactor: one 2,446-line API file became modules; main.rs shrank (as of the split commit).
  'w-modules': { cwd: checkout, prompt: '~/eplyx', cols: 100, rows: 14, steps: [
    { cmd: 'ls server/src/api/', expect: 0, after: .8 },
    { cmd: "git show --stat=90 --format='%h %s' 9c61e32 -- server/src/api.rs engine/src/main.rs", expect: 0, after: 1.4 },
  ] },
};

const names = process.argv.slice(2);
if (names.length) {
  const out = join(here, '../assets/casts'); mkdirSync(out, { recursive: true });
  for (const name of names) {
    const s = sessions[name];
    if (!s) throw new Error('unknown session ' + name);
    const spec = { ...s, path: s.path || bin, home: D, title: name };
    const file = join(out, `${name}.session.json`);
    writeFileSync(file, JSON.stringify(spec, null, 1));
    const r = spawnSync('python3', [recorder, file, join(out, `${name}.cast`)], { stdio: 'inherit' });
    if (r.status !== 0) { console.error(`session ${name} failed`); process.exit(1); }
  }
}
