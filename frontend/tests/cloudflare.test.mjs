import { test } from 'node:test';
import assert from 'node:assert/strict';
import worker from '../cloudflare/worker.js';
import { writeRuntimeConfig } from '../runtime-config.mjs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const env = {
  EPLYX_UPSTREAM_ORIGIN: 'https://internal.example',
  EPLYX_PUBLIC_ORIGIN: 'https://eplyx.dev',
  ASSETS: { fetch: async () => new Response('static') },
};

test('explicit same-origin configuration also works in local Pages previews', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'eplyx-proxy-config-'));
  const previous = process.env.EPLYX_API_URL;
  t.after(async () => {
    if (previous === undefined) delete process.env.EPLYX_API_URL;
    else process.env.EPLYX_API_URL = previous;
    delete globalThis.EPLYX_API_URL;
    delete globalThis.location;
    await rm(directory, { recursive: true, force: true });
  });
  process.env.EPLYX_API_URL = '/';
  assert.equal(await writeRuntimeConfig(directory), '/');
  assert.match(await readFile(join(directory, 'runtime-config.js'), 'utf8'), /EPLYX_API_URL = "\/"/);
  globalThis.EPLYX_API_URL = '/';
  globalThis.location = { hostname: '127.0.0.1' };
  assert.equal((await import('../src/session.js?pages-preview')).API_BASE, '');
});

test('workspace proxy preserves requests, cookies, origin checks and response security', async t => {
  t.mock.method(globalThis, 'fetch', async (request, options) => {
    assert.equal(request.url, 'https://internal.example/v1/workspaces?next=%2Fp%2Fexample');
    assert.equal(request.method, 'POST');
    assert.equal(request.redirect, 'manual');
    assert.equal(request.headers.get('origin'), 'https://eplyx.dev');
    assert.equal(request.headers.get('cookie'), 'eplyx_session=test-only');
    assert.equal(request.headers.get('authorization'), 'Bearer test-only');
    assert.equal(await request.text(), '{"name":"Test"}');
    assert.equal(options.cf.cacheTtl, 0);
    return new Response('{"ok":true}', { headers: {
      'Set-Cookie': 'eplyx_session=new-test-only; HttpOnly; Secure; SameSite=Strict; Path=/',
      'Content-Security-Policy': "default-src 'self'",
    } });
  });
  const result = await worker.fetch(new Request('https://eplyx.dev/v1/workspaces?next=%2Fp%2Fexample', {
    method: 'POST', body: '{"name":"Test"}', headers: {
      origin: 'https://eplyx.dev', cookie: 'eplyx_session=test-only', authorization: 'Bearer test-only',
    },
  }), env);
  assert.equal(result.status, 200);
  assert.equal(result.headers.get('cache-control'), 'no-store');
  assert.match(result.headers.get('set-cookie'), /HttpOnly; Secure; SameSite=Strict/);
  assert.equal(result.headers.get('content-security-policy'), "default-src 'self'");
});

test('cross-site origins are preserved for the backend to reject', async t => {
  t.mock.method(globalThis, 'fetch', async request => {
    assert.equal(request.headers.get('origin'), 'https://untrusted.example');
    return Response.json({ error: 'cross-site request refused' }, { status: 403 });
  });
  const result = await worker.fetch(new Request('https://eplyx.dev/v1/auth/login', {
    method: 'POST', headers: { origin: 'https://untrusted.example' }, body: '{}',
  }), env);
  assert.equal(result.status, 403);
});

test('private pages canonicalize to the domain and singular workspace path', async t => {
  t.mock.method(globalThis, 'fetch', () => { throw new Error('Must not contact upstream'); });
  for (const [url, location] of [
    ['https://www.eplyx.dev/workspace', 'https://eplyx.dev/workspace'],
    ['https://preview.eplyx.pages.dev/login?next=%2Fp%2Ftest', 'https://eplyx.dev/login?next=%2Fp%2Ftest'],
    ['https://eplyx.dev/workspaces/', 'https://eplyx.dev/workspace'],
  ]) {
    const result = await worker.fetch(new Request(url), env);
    assert.equal(result.status, 308);
    assert.equal(result.headers.get('location'), location);
  }
});

test('marketing routes and similarly named paths stay static', async t => {
  t.mock.method(globalThis, 'fetch', () => { throw new Error('Must not contact upstream'); });
  for (const path of ['/', '/start', '/public/logo.svg', '/src/app.js', '/v10/projects', '/workspace-other']) {
    assert.equal(await (await worker.fetch(new Request(`https://eplyx.dev${path}`), env)).text(), 'static');
  }
});

test('redirects never send browser credentials to a followed upstream location', async t => {
  t.mock.method(globalThis, 'fetch', async request => {
    assert.equal(request.redirect, 'manual');
    return new Response(null, { status: 302, headers: { Location: 'https://internal.example/login?next=%2Fp%2Ftest' } });
  });
  const result = await worker.fetch(new Request('https://eplyx.dev/workspace'), env);
  assert.equal(result.status, 302);
  assert.equal(result.headers.get('location'), 'https://eplyx.dev/login?next=%2Fp%2Ftest');
});

test('upstream errors do not reveal the internal address', async t => {
  t.mock.method(globalThis, 'fetch', () => { throw new Error('https://internal.example failed'); });
  const result = await worker.fetch(new Request('https://eplyx.dev/v1/auth/me'), env);
  assert.equal(result.status, 502);
  assert.equal(result.headers.get('cache-control'), 'no-store');
  assert.doesNotMatch(await result.text(), /internal\.example/);
});
