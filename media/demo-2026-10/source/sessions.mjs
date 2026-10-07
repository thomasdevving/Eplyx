// The terminal sessions in the film, in recording order. Each command is typed
// into a real bash and really executed; outputs and exit codes are whatever the
// built binaries returned. `expect` makes the recording fail if a command's exit
// status is not the one the film describes.
//
//   node sessions.mjs [name ...]      (records into ../assets/casts/)
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, '../../..');
const seed = JSON.parse(readFileSync('/srv/eplyx-demo/seed.json', 'utf8'));
const API = seed.base, P = seed.upgradeProject, T = seed.transitionsProject;
const bin = [join(repo, 'target/release')];
const D = '/home/demo';
const TOKEN = (() => { try { return (readFileSync(`${D}/ci-token`, 'utf8').match(/eplyx_[A-Za-z0-9_]+/) || [''])[0]; } catch { return ''; } })();
const SIG = '3omP6iKrk9jcFfjURFo76biHedNpXVJcK16jBxX3AUyqTQ4zr3ZHW5kpdue18TpVvBhjhkybEFUkhd4ivw4TaN2n';
const T22 = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';

export const sessions = {
  upgrade: { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 100, rows: 30, steps: [
    { cmd: 'eplyx bundle verify --bundle .eplyx/bundle', expect: 0, after: 1.2 },
    { cmd: 'eplyx ci check --bundle .eplyx/bundle --candidate .eplyx/bundle/binaries/current.so', expect: 0, after: 1.2 },
    { cmd: 'eplyx ci check --bundle .eplyx/bundle --candidate target/deploy/fixture_stake_pool_v2.so', expect: 1, after: 1.5 },
  ] },
  expectations: { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 100, rows: 30, steps: [
    { cmd: "grep -v '^#' .eplyx/expected-changes.toml | grep -v '^$'", expect: 0, after: 1 },
    { cmd: 'eplyx ci check --bundle .eplyx/bundle --candidate target/deploy/fixture_stake_pool_v2.so --expectations .eplyx/expected-changes.toml', expect: 1, after: 1.5 },
  ] },
  operator: { cwd: `${D}`, prompt: 'operator@eplyx', cols: 100, rows: 26, env: { EPLYX_DATA_DIR: '/srv/eplyx-demo/volume' }, steps: [
    { cmd: 'eplyx-server admin list-projects', expect: 0, after: 1 },
    { cmd: `eplyx-server admin register-bundle --project ${P} --path stake-pool-program/.eplyx/bundle`, expect: 0, after: 1 },
    { cmd: `eplyx-server admin activate-bundle --project ${P} --bundle {{grab:bndl_[A-Z0-9]+}}`, expect: 0, after: 1 },
    { cmd: `eplyx-server admin create-token --project ${P} --label github-actions > ci-token`, expect: 0, after: 1 },
  ] },
  hosted: { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 100, rows: 30, steps: [
    { cmd: 'export EPLYX_TOKEN="$(cat ~/ci-token | grep -o "eplyx_[A-Za-z0-9_]*" | head -1)"', expect: 0, after: .3 },
    { cmd: `scripts/eplyx-submit.sh --api ${API} --project ${P} --candidate target/deploy/fixture_stake_pool_v2.so --expectations .eplyx/expected-changes.toml --summary summary.md`, expect: 1, timeout: 300, after: 1.5 },
  ] },
  api: { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 100, rows: 30, env: { EPLYX_TOKEN: TOKEN }, steps: [
    { cmd: `curl -s -H "authorization: Bearer $EPLYX_TOKEN" ${API}/v1/projects/${P}/setup | jq -r '.steps[] | "\\(.status)\\t\\(.title)"'`, expect: 0, after: 1 },
    { cmd: `curl -s -H "authorization: Bearer $EPLYX_TOKEN" ${API}/v1/projects/${P}/runs | jq '.runs[0] | {run_id, status, exit_code, candidate_sha256}'`, expect: 0, after: 1.4 },
  ] },
  governance: { cwd: `${D}/squads`, prompt: '~/squads', cols: 100, rows: 30, steps: [
    { cmd: 'eplyx governance squads verify --change-spec bound.json --rpc-url https://api.mainnet-beta.solana.com', expect: 0, timeout: 120, after: 1.8 },
  ] },
  history: { cwd: `${D}/history`, prompt: '~/history', cols: 100, rows: 30, speed: 1.6,
    env: { SOLANA_RPC_URL: 'https://solana-mainnet.g.alchemy.com/v2/docs-demo', SOLANA_RPC_ORIGIN: 'https://www.alchemy.com' }, steps: [
    { cmd: `eplyx versions upgrades --program ${T22} --start-slot 420000000 --end-slot 430000000 --output .`, expect: 0, timeout: 180, after: 1 },
    { cmd: `eplyx historical acquire --signature ${SIG} --program ${T22} --output .`, expect: 0, timeout: 300, after: 1, speed: 3 },
    { cmd: `eplyx versions resolve --program ${T22} --slot 427147035 --output . --out v2.so`, expect: 0, timeout: 180, after: 1 },
    { cmd: 'eplyx compare --corpus corpus.json --v1 token-2022-mainnet-v1.so --v2 v2.so | head -16', expect: 0, after: 1.2 },
    { cmd: 'eplyx compare --corpus corpus.json --v1 token-2022-mainnet-v1.so --v2 ~/candidates/fixture_token2022_v2.so | sed -n 10,22p', expect: 0, after: 1.5 },
  ] },
  migration: { cwd: `${D}/token-migration`, prompt: '~/token-migration', cols: 100, rows: 30, steps: [
    { cmd: 'eplyx doctor', expect: 0, after: 1 },
    { cmd: 'eplyx migration analyse', expect: 0, after: 1.2 },
    { cmd: 'cp ~/builds/migration-deadline-defect.so target/deploy/migration.so', expect: 0, after: .4 },
    { cmd: 'eplyx migration search', expect: 0, after: 1.2 },
    { cmd: 'eplyx migration reproduce $(basename .eplyx/counterexamples/cx_* .json)', expect: 0, after: 1 },
    { cmd: "RUN=$(eplyx runs --json | jq -r '.runs | max_by(.timestamp).run_id')", expect: 0, after: .3 },
    { cmd: 'eplyx migration gate --run $RUN --policy strict', expect: 1, after: 1 },
    { cmd: 'eplyx migration plan --run $RUN --out unsigned.json', expect: 0, after: 1.2 },
  ] },
  lifecycle: { cwd: `${D}/lifecycle`, prompt: '~/lifecycle', cols: 100, rows: 32, steps: [
    { cmd: 'eplyx change lifecycle --scenario inputs/scenario.json --out change.json', expect: 0, after: .8 },
    { cmd: 'eplyx lifecycle analyse --snapshot inputs/snapshot.json --scenario inputs/scenario.json --change-spec change.json --at 2026-09-28T00:00:00Z --record ~/token-migration', expect: 0, after: 1.6 },
  ] },
  dashboard: { cwd: `${D}/token-migration`, prompt: '~/token-migration', cols: 100, rows: 12, steps: [
    { cmd: 'eplyx dashboard --no-open --port 4185', hold: 3 },
  ] },
  fee: { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 100, rows: 30, steps: [
    { cmd: 'eplyx parameter analyse --change proposals/deposit-fee-1pct.json --bundle .eplyx/bundle --record-id mainnet-spl-stake-pool-151010f709e113e7 --out fee-report.json', expect: 0, after: 1 },
    { cmd: "jq '.findings[0], .proposed.reconciliation.manager_fee_account_credit_raw' fee-report.json", expect: 0, after: 1 },
    { cmd: 'eplyx parameter reproduce --change proposals/deposit-fee-1pct.json --report fee-report.json', expect: 0, after: 1.4 },
  ] },
  interaction: { cwd: `${D}/stake-pool-program`, prompt: '~/stake-pool-program', cols: 100, rows: 30, steps: [
    { cmd: "eplyx interaction analyse --upgrade proposals/config-upgrade.json --parameter proposals/deposit-fee-1pct.json --bundle .eplyx/bundle --record-id mainnet-spl-stake-pool-151010f709e113e7 --candidate target/deploy/fixture_stake_pool_config_v2.so --out interaction | jq '{status, fee_effect_on_v1: .recipient_effects.parameter_v1.value, fee_effect_on_v2: .recipient_effects.parameter_v2.value, interaction: .recipient_effects.interaction.value}'", expect: 0, after: 1.6, speed: 2.2 },
  ] },
  sync: { cwd: `${D}/token-migration`, prompt: '~/token-migration', cols: 100, rows: 24, env: { EPLYX_CONFIG_DIR: `${D}/.config/eplyx` }, steps: [
    { cmd: `eplyx login --server ${API} --no-open`, expect: 0, timeout: 120, after: .8 },
    { cmd: `eplyx link --project ${T}`, expect: 0, after: .8 },
    { cmd: 'eplyx sync', expect: 0, timeout: 120, after: 1.5 },
  ] },
  ops: { cwd: `${D}`, prompt: 'operator@eplyx', cols: 100, rows: 26, env: { EPLYX_DATA_DIR: '/srv/eplyx-demo/volume' }, steps: [
    { cmd: 'eplyx-server admin list-projects', expect: 0, after: 1 },
    { cmd: "eplyx-server admin ops | jq -c '.workers, .queue, .outcomes, .execution'", expect: 0, after: 1.6 },
  ] },
  synthetic: { cwd: repo, prompt: '~/eplyx', cols: 100, rows: 30, steps: [
    { cmd: 'eplyx compare --no-minimize | head -27', expect: 0, after: 1.6 },
  ] },
};

const names = process.argv.slice(2);
if (names.length) {
  const out = join(here, '../assets/casts'); mkdirSync(out, { recursive: true });
  const vars = JSON.parse((() => { try { return readFileSync('/srv/eplyx-demo/vars.json', 'utf8'); } catch { return '{}'; } })());
  for (const name of names) {
    const s = structuredClone(sessions[name]);
    if (!s) throw new Error('unknown session ' + name);
    for (const st of s.steps) if (st.cmd) st.cmd = st.cmd.replace(/\{\{(\w+)\}\}/g, (_, k) => { if (!vars[k]) throw new Error('missing var ' + k); return vars[k]; });
    const spec = { ...s, path: bin, home: D, title: name };
    // The recorder reads the real spec from scratch storage; the copy kept
    // next to the cast has credentials masked.
    const file = join('/srv/eplyx-demo', `${name}.session.json`);
    writeFileSync(file, JSON.stringify(spec, null, 1), { mode: 0o600 });
    const masked = { ...spec, env: Object.fromEntries(Object.entries(spec.env || {}).map(([k, v]) => [k, /TOKEN|SECRET|KEY/.test(k) ? '<redacted>' : v])) };
    writeFileSync(join(out, `${name}.session.json`), JSON.stringify(masked, null, 1));
    const r = spawnSync('python3', [join(here, 'pty_record.py'), file, join(out, `${name}.cast`)], { stdio: 'inherit' });
    if (r.status !== 0) { console.error(`session ${name} failed`); process.exit(1); }
  }
}
