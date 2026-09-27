// Exercise the production build and SPA fallback with no external API.
import { spawn } from 'node:child_process';
const server = spawn(process.execPath, ['frontend/serve.mjs'], { stdio: 'inherit', env: { PORT: '4193' } });
for (const signal of ['SIGTERM', 'SIGINT']) process.on(signal, () => server.kill(signal));
server.on('exit', code => process.exit(code ?? 0));
