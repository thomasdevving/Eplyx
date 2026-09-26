// A static public-site test server with no provider or deployment configuration.
import { spawn } from 'node:child_process';
const server=spawn(process.execPath,['frontend/dev-server.mjs'],{stdio:'inherit',env:{PORT:'4189'}});
for(const signal of ['SIGTERM','SIGINT'])process.on(signal,()=>server.kill(signal));
server.on('exit',code=>process.exit(code??0));
