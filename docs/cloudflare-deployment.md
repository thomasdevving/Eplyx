# Cloudflare website deployment

The public frontend is built from `thomasdevving/Eplyx` using Cloudflare Pages.
The production branch is `main`; pushes trigger an automatic build and deployment.
Only the generated `dist/` directory is published. The Rust API, database,
credentials and evidence bundles are not part of this static deployment.

## Build settings

| Setting | Value |
| --- | --- |
| Project | `eplyx` |
| Repository | `thomasdevving/Eplyx` |
| Production branch | `main` |
| Framework preset | None |
| Root directory | Repository root |
| Build command | `pnpm build:cloudflare` |
| Output directory | `dist` |
| Node version | `22.23.1`, pinned in `.node-version` |
| pnpm version | `11.24.0`, pinned in `package.json`; set `PNPM_VERSION=11.24.0` in Pages |
| Domains | `eplyx.dev`, `www.eplyx.dev` |

Connect the Cloudflare Workers and Pages GitHub app to this repository, then
create a Pages project with these settings. Add both custom domains through
Pages so Cloudflare provisions their DNS records and TLS certificates.

`frontend/build-cloudflare.mjs` preserves the existing public API URL:
`https://upgrade-impactreport-check-production.up.railway.app`.
Set `EPLYX_API_URL` as a Pages build variable to change it. Browser requests
require the API's `EPLYX_ALLOWED_ORIGINS` to include the public website origin;
static hosting does not change that API configuration. Workspace links open
the API's own origin.

## Routing and caching

The build writes explicit `_redirects` rewrites for application routes,
including project and report URLs. The bundled `404.html` prevents missing
assets and API paths from receiving the application shell with status 200.
`_headers` retains `nosniff` and `no-referrer`, prevents framing, and requires
asset revalidation. Runtime configuration uses `Cache-Control: no-store`.

## Local verification

```sh
pnpm check:frontend
pnpm test:public
pnpm preview:cloudflare
```

`EPLYX_CHROME` can select an installed Chromium binary for the browser suite.
After publishing, verify HTTPS on both domains, direct page loads, the demo
report, JavaScript and font assets, and a 404 for a missing asset.

The website notices record Cloudflare hosting without promising Netherlands-only
processing. Other unresolved operator details remain visible in the notices.

## Domain email

`contact@eplyx.dev` forwards through Cloudflare Email Routing to the verified
`eplyxcontact@gmail.com` inbox. The rule is enabled; Cloudflare manages the
required MX, SPF and DKIM DNS records. The catch-all is disabled. This is free
inbound forwarding, not an outbound mailbox.
