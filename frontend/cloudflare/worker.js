// Only the hosted workspace and API reach the backend. Public pages and assets
// remain on Pages. The upstream address is server-side configuration only.
const below = (path, root) => path === root || path.startsWith(`${root}/`);
const workspacePage = path => ['/workspace', '/workspaces', '/login', '/signup', '/device', '/p', '/demo']
  .some(root => below(path, root));
const hosted = path => workspacePage(path) || ['/v1', '/assets'].some(root => below(path, root))
  || path === '/health' || path === '/ready';

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (!hosted(url.pathname)) return env.ASSETS.fetch(request);

    const publicOrigin = env.EPLYX_PUBLIC_ORIGIN || 'https://eplyx.dev';
    const canonical = new URL(url.pathname + url.search, publicOrigin);
    if (below(url.pathname, '/workspaces')) {
      canonical.pathname = url.pathname.replace(/^\/workspaces(?=\/|$)/, '/workspace');
    }
    if (['/workspace/', '/login/', '/signup/', '/device/'].includes(canonical.pathname)) {
      canonical.pathname = canonical.pathname.slice(0, -1);
    }
    // Keep login cookies and cookie-write origin checks on one public origin.
    if (workspacePage(url.pathname) && canonical.href !== url.href) {
      return new Response(null, { status: 308, headers: { Location: canonical.href, 'Cache-Control': 'no-store' } });
    }

    try {
      const upstream = new URL(env.EPLYX_UPSTREAM_ORIGIN);
      if (upstream.username || upstream.password || upstream.pathname !== '/' || upstream.search || upstream.hash
        || !(upstream.protocol === 'https:' || (upstream.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(upstream.hostname)))) {
        throw new Error('Invalid upstream configuration');
      }
      const upstreamOrigin = upstream.origin;
      upstream.pathname = url.pathname;
      upstream.search = url.search;
      const headers = new Headers(request.headers);
      headers.delete('host');
      // Preserve Origin, cookies, authorization and the streamed request body.
      // Never follow an upstream redirect with a visitor's credentials.
      const forwarded = new Request(upstream, request);
      const response = await fetch(new Request(forwarded, { headers, redirect: 'manual' }), {
        cf: { cacheEverything: false, cacheTtl: 0 },
      });
      const result = new Response(response.body, response);
      result.headers.set('Cache-Control', 'no-store');
      const location = result.headers.get('Location');
      if (location) {
        const destination = new URL(location, upstream);
        if (destination.origin === upstreamOrigin) {
          result.headers.set('Location', new URL(destination.pathname + destination.search + destination.hash, publicOrigin).href);
        }
      }
      return result;
    } catch {
      // Do not expose internal hostnames or fetch error details to the browser.
      return Response.json({ error: 'Eplyx is temporarily unavailable. Please try again.' }, {
        status: 502, headers: { 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' },
      });
    }
  },
};
