// Serve a copied dashboard fixture through the real `eplyx dashboard` binary.
//   node frontend/tests/dashboard/server.mjs <fixture> <port> [--empty]
import { cp, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const [fixture, port, flag] = process.argv.slice(2);
const repo = fileURLToPath(new URL('../../../', import.meta.url));
const root = await mkdtemp(join(tmpdir(), `eplyx-dashboard-${fixture}-`));
await cp(join(repo, 'fixtures/dashboard', fixture), root, { recursive: true });
if (flag === '--empty') { await rm(join(root, '.eplyx/runs'), { recursive: true, force: true }); await rm(join(root, '.eplyx/counterexamples'), { recursive: true, force: true }); }
for (const child of ['runs', 'counterexamples', 'cache']) await mkdir(join(root, '.eplyx', child), { recursive: true });
const binary = join(repo, 'target/debug', process.platform === 'win32' ? 'eplyx.exe' : 'eplyx');
const server = spawn(binary, ['--config', join(root, 'eplyx.toml'), 'dashboard', '--no-open', '--port', port], { stdio:'inherit', env:{ SOLANA_RPC_URL:'BROWSER-RPC-SECRET-NOT-AN-ENDPOINT' } });
const stop = async () => { server.kill(); await rm(root, { recursive: true, force: true }); process.exit(0); };
process.on('SIGINT', stop); process.on('SIGTERM', stop);
server.on('exit', code => process.exit(code ?? 1));
