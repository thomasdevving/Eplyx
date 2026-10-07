// Create the demo workspace on the local hosted service through its public
// API: one account, one upgrade project (SPL Stake Pool) and one token
// transitions project. Bundles, tokens, checks and syncs are performed later
// on camera through the operator CLI, the submit client and `eplyx sync`.
// Credentials stay in /srv/eplyx-demo (outside the repository).
import { writeFile } from 'node:fs/promises';
const base = process.env.EPLYX_URL || 'http://127.0.0.1:4390';
const email = 'alex@demo.eplyx.local', password = 'correct horse battery staple', name = 'Alex Demo';
async function post(path, body, cookie = '') {
  const r = await fetch(base + path, { method: 'POST', headers: { 'content-type': 'application/json', origin: base, ...(cookie ? { cookie } : {}) }, body: JSON.stringify(body) });
  const text = await r.text();
  if (!r.ok) throw new Error(`${path} ${r.status} ${text}`);
  return { body: JSON.parse(text), cookie: r.headers.get('set-cookie')?.split(';')[0] ?? cookie };
}
const { body: account, cookie } = await post('/v1/auth/signup', { email, password, name });
const { body: { project: upgrade } } = await post(`/v1/workspaces/${account.workspace_id}/projects`, { name: 'stake-pool-upgrades', program_id: 'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy' }, cookie);
const { body: { project: transitions } } = await post(`/v1/workspaces/${account.workspace_id}/projects`, { name: 'token-transitions' }, cookie);
const seed = { base, email, password, workspace: account.workspace_id, upgradeProject: upgrade.id, transitionsProject: transitions.id };
await writeFile('/srv/eplyx-demo/seed.json', JSON.stringify(seed, null, 2), { mode: 0o600 });
console.log(JSON.stringify({ ...seed, password: '(stored)' }, null, 2));
