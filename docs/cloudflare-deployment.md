# Cloudflare website deployment

The public frontend is built from `thomasdevving/Eplyx` using Cloudflare Pages.
The production branch is `main`; pushes trigger an automatic build and deployment.
The generated `dist/` directory includes public assets and a Pages Function that
proxies the hosted workspace. The Rust API, database, credentials and evidence
bundles remain on the existing backend; they are not uploaded as static assets.

The first production deployment was published on 8 October 2026 (Amsterdam time),
from commit `3c8965f`, with deployment ID
`49130259-cbbc-4c58-9d49-a48ef3f24e2e`. Both `https://eplyx.dev` and
`https://www.eplyx.dev` are active with SSL enabled. The Pages fallback address is
`https://eplyx.pages.dev`.

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

## Public workspace and API

All browser-facing entry points use `https://eplyx.dev`. The workspace opens at
`/workspace`, login/signup/device approval stay on this origin, and project pages
use `/p/:id`. The old `/workspaces` path redirects to `/workspace`. Workspace
pages on `www` or Pages preview domains redirect to the canonical apex domain so
login sessions use one origin.

`frontend/build-cloudflare.mjs` forces same-origin runtime configuration (`/`).
The internal backend address exists only in the server-side
`EPLYX_UPSTREAM_ORIGIN` binding in `wrangler.jsonc`. The worker forwards workspace
HTML/assets and `/v1/*` requests, streaming bodies and preserving cookies,
authorization and the original Origin header. It never follows backend redirects
with credentials and rewrites backend Location URLs to the public domain.

In Railway's `friendly-bravery` project, the `@upgrade-impact/report-check`
production service must use `EPLYX_PUBLIC_URL=https://eplyx.dev`. This setting
controls cookie-write origin validation and generated CLI/project URLs. Keep
`EPLYX_ALLOWED_ORIGINS` configured for the two Eplyx domains where needed by
bearer-authenticated requests. Session cookies remain HttpOnly, Secure and
SameSite=Strict; authentication and membership checks stay on the backend.

## Routing and caching

The build writes explicit `_redirects` rewrites for application routes,
including project and report URLs. The bundled `404.html` prevents missing
assets and API paths from receiving the application shell with status 200.
`_headers` retains `nosniff` and `no-referrer`, prevents framing, and requires
asset revalidation. Runtime configuration uses `Cache-Control: no-store`.
`_routes.json` limits Function invocations to workspace and API paths. Proxied
responses always use `no-store` and retain the backend security headers; private
workspace responses must never enter an asset cache.

## Local verification

```sh
pnpm check:frontend
pnpm test:frontend-runtime
pnpm test:public
pnpm preview:cloudflare
```

`EPLYX_CHROME` can select an installed Chromium binary for the browser suite.
After publishing, verify HTTPS on both domains, direct page loads, the demo
report, JavaScript and font assets, and a 404 for a missing asset. Also verify
`/workspace`, `/login`, same-origin `/v1/auth/me`, login/logout cookies, cross-site
write rejection and generated CLI/project links. Use an isolated local database
for account creation and session tests, never production test users.

Verified live on 8 October 2026 after deployment of `d749e9b`: workspace,
login, signup, device approval and workspace assets return 200 on `eplyx.dev`;
unauthenticated workspace/session APIs return JSON 401; the legacy workspace
path and `www` workspace redirect to `https://eplyx.dev/workspace`.
The runtime configuration uses `/`, private responses use `no-store`, and the
workspace displays `eplyx login --server https://eplyx.dev`. Empty login requests
from the public origin reach input validation (422), while a foreign origin is
rejected (403). Signup, login, session cookies, project navigation, logout and
CLI device URLs were also exercised against an isolated local database through
the real Pages runtime and Rust backend. No production test account was created.

The website notices record Cloudflare hosting without promising Netherlands-only
processing. Other unresolved operator details remain visible in the notices.

## Domain email

`contact@eplyx.dev` forwards through Cloudflare Email Routing to the verified
`eplyxcontact@gmail.com` inbox. The rule is enabled; Cloudflare manages the
required MX, SPF and DKIM DNS records. The catch-all is disabled. This is free
inbound forwarding, not an outbound mailbox.
