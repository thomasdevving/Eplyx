import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prepareVendorAssets } from './vendor.mjs';
import { writeRuntimeConfig } from './runtime-config.mjs';
import { technicalOverviewDocument } from './technical-overview-document.mjs';

await prepareVendorAssets();
const apiUrl = await writeRuntimeConfig(fileURLToPath(new URL('./public/', import.meta.url)));
globalThis.EPLYX_API_URL = apiUrl;

const root = fileURLToPath(new URL('.', import.meta.url));
const port = Number(process.env.PORT || 4173);
const types = { '.html':'text/html; charset=utf-8', '.js':'text/javascript; charset=utf-8', '.css':'text/css; charset=utf-8', '.svg':'image/svg+xml', '.json':'application/json', '.woff2':'font/woff2', '.txt':'text/plain; charset=utf-8', '.pdf':'application/pdf' };

createServer(async (request, response) => {
  const rawPath = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
  if (rawPath.replace(/\/+$/, '') === '/technical-overview') {
    const body = await technicalOverviewDocument(await readFile(join(root, 'index.html'), 'utf8'));
    response.writeHead(200, { 'Content-Type': types['.html'], 'Cache-Control': 'no-store' });
    return response.end(body);
  }
  const requested = normalize(rawPath).replace(/^(\.\.(\/|\\|$))+/, '');
  let file = join(root, requested === '/technical-overview.pdf' ? 'public/technical-overview.pdf' : requested);
  try { if ((await stat(file)).isDirectory()) file = join(file, 'index.html'); }
  catch { file = join(root, 'index.html'); }
  try {
    const body = await readFile(file);
    response.writeHead(200, { 'Content-Type': types[extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
    response.end(body);
  } catch {
    response.writeHead(404); response.end('Not found');
  }
}).listen(port, '127.0.0.1', () => console.log(`Eplyx frontend: http://localhost:${port}${apiUrl ? ` \u2192 API ${apiUrl}` : ' (API from the local default)'}`));
