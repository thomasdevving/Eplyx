import { cp, writeFile } from 'node:fs/promises';

// Preserve the existing hosted service when publishing the public frontend.
// This public URL is not a credential. Override it explicitly for another API.
process.env.EPLYX_API_URL ??= 'https://upgrade-impactreport-check-production.up.railway.app';
await import('./build.mjs');

const output = new URL('../dist/', import.meta.url);
await cp(new URL('./cloudflare/_headers', import.meta.url), new URL('_headers', output));
await cp(new URL('./cloudflare/404.html', import.meta.url), new URL('404.html', output));

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
