import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prepareVendorAssets } from './vendor.mjs';
import { writeRuntimeConfig } from './runtime-config.mjs';

await prepareVendorAssets();
const apiUrl = await writeRuntimeConfig(fileURLToPath(new URL('./public/', import.meta.url)));

const root = fileURLToPath(new URL('.', import.meta.url));
const port = Number(process.env.PORT || 4173);
const types = { '.html':'text/html; charset=utf-8', '.js':'text/javascript; charset=utf-8', '.css':'text/css; charset=utf-8', '.svg':'image/svg+xml', '.json':'application/json', '.woff2':'font/woff2', '.txt':'text/plain; charset=utf-8' };

createServer(async (request, response) => {
  if (request.method !== 'GET' && request.method !== 'HEAD') {
    response.writeHead(405, { Allow: 'GET, HEAD' });
    return response.end('Method not allowed');
  }
  let requested;
  try {
    const rawPath = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
    if (rawPath.includes('\0')) throw new Error('Invalid path');
    requested = normalize(rawPath).replace(/^(\.\.(\/|\\|$))+/, '');
  } catch {
    response.writeHead(400, { 'Content-Type': 'text/plain; charset=utf-8' });
    return response.end('Invalid request path');
  }
  let file = join(root, requested);
  try { if ((await stat(file)).isDirectory()) file = join(file, 'index.html'); }
  catch {
    if (extname(requested)) {
      response.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' });
      return response.end('Not found');
    }
    file = join(root, 'index.html');
  }
  try {
    const body = await readFile(file);
    response.writeHead(200, { 'Content-Type': types[extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
    response.end(body);
  } catch {
    response.writeHead(404); response.end('Not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`Eplyx frontend: http://localhost:${port}${apiUrl ? ` \u2192 API ${apiUrl}` : ' (API from the local default)'}`));
