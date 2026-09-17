import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prepareVendorAssets } from './vendor.mjs';

await prepareVendorAssets();

const root = fileURLToPath(new URL('.', import.meta.url));
const port = Number(process.env.PORT || 4173);
const types = { '.html':'text/html; charset=utf-8', '.js':'text/javascript; charset=utf-8', '.css':'text/css; charset=utf-8', '.svg':'image/svg+xml' };

createServer(async (request, response) => {
  const rawPath = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
  const requested = normalize(rawPath).replace(/^(\.\.(\/|\\|$))+/, '');
  let file = join(root, requested);
  try { if ((await stat(file)).isDirectory()) file = join(file, 'index.html'); }
  catch { file = join(root, 'index.html'); }
  try {
    const body = await readFile(file);
    response.writeHead(200, { 'Content-Type': types[extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
    response.end(body);
  } catch {
    response.writeHead(404); response.end('Not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`Eplyx frontend: http://localhost:${port}`));
