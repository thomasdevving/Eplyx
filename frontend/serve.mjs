// Serve the built frontend where a managed host can reach it.
//
// Deliberately separate from `dev-server.mjs`, which binds 127.0.0.1 because a
// development server has no business being reachable from a network. This one
// binds every interface and follows the platform's PORT, which is what makes it
// a host rather than a convenience — and the difference is the whole reason it
// is a second file instead of a flag on the first.
//
// It serves static bytes and nothing else: no proxy to the API, no secret, no
// server-side rendering. The browser talks to Eplyx directly with the
// credential a person pasted into their own tab, so nothing here ever holds one.

import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../dist/', import.meta.url));
const port = Number(process.env.PORT || 8080);
const types = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.json': 'application/json; charset=utf-8',
  '.png': 'image/png',
  '.woff2': 'font/woff2',
};

// `runtime-config.js` carries the API base URL and is rewritten per deployment,
// so it must never be cached. Everything else is content the app ships with.
const neverCache = new Set(['/public/runtime-config.js']);

createServer(async (request, response) => {
  const url = new URL(request.url, 'http://localhost');
  if (url.pathname === '/healthz') {
    response.writeHead(200, { 'Content-Type': 'application/json' });
    return response.end('{"status":"ok"}');
  }
  const requested = normalize(decodeURIComponent(url.pathname)).replace(/^(\.\.(\/|\\|$))+/, '');
  let file = join(root, requested);
  try {
    if ((await stat(file)).isDirectory()) file = join(file, 'index.html');
  } catch {
    // Client-side routing: /projects and /runs/{id} are app routes, not files.
    // A path that names an extension is asking for an asset, though, and
    // answering that with index.html and a 200 hands the browser HTML to parse
    // as JavaScript — a missing file then surfaces as a syntax error in a file
    // that was never served. Say 404 and mean it.
    if (extname(requested)) {
      response.writeHead(404, { 'Content-Type': 'text/plain' });
      return response.end('Not found');
    }
    file = join(root, 'index.html');
  }
  try {
    const body = await readFile(file);
    response.writeHead(200, {
      'Content-Type': types[extname(file)] || 'application/octet-stream',
      'Cache-Control': neverCache.has(requested) ? 'no-store' : 'public, max-age=300',
      'X-Content-Type-Options': 'nosniff',
    });
    response.end(body);
  } catch {
    response.writeHead(404, { 'Content-Type': 'text/plain' });
    response.end('Not found');
  }
}).listen(port, '0.0.0.0', () => console.log(`Eplyx frontend listening on 0.0.0.0:${port}`));
