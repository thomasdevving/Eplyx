import { cp, writeFile } from 'node:fs/promises';

// Cloudflare proxies the workspace API on the website's own origin. Never
// publish the internal upstream address in browser configuration.
process.env.EPLYX_API_URL = '/';
await import('./build.mjs');

const output = new URL('../dist/', import.meta.url);
await cp(new URL('./cloudflare/_headers', import.meta.url), new URL('_headers', output));
await cp(new URL('./cloudflare/404.html', import.meta.url), new URL('404.html', output));
await cp(new URL('./cloudflare/worker.js', import.meta.url), new URL('_worker.js', output));
await writeFile(new URL('_routes.json', output), JSON.stringify({
  version: 1,
  include: ['/workspace', '/workspace/*', '/workspaces', '/workspaces/*',
    '/login', '/login/', '/signup', '/signup/', '/device', '/device/',
    '/p/*', '/demo', '/demo/*', '/assets/*', '/v1/*', '/health', '/ready'],
  exclude: [],
}, null, 2));

// Rewrite only application routes. Missing scripts, images and API endpoints
// must remain 404s instead of receiving HTML with a successful status.
const pages = [
  'start', 'cli', 'token-transitions', 'analyse', 'projects',
  'legal', 'privacy', 'cookies', 'terms', 'contact', 'licenses',
];
const routes = [...pages.map(page => `/${page}`), '/projects/:id', '/runs/:id'];
const redirects = routes.flatMap(route => [
  `${route}/ ${route} 301`,
  `${route} / 200`,
]);
await writeFile(new URL('_redirects', output), `${redirects.join('\n')}\n`);
console.log('Prepared Cloudflare routing and response headers.');
