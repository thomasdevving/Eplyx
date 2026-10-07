import { test } from 'node:test';
import assert from 'node:assert/strict';

test('operator credentials work for the page when browser storage is blocked', async () => {
  globalThis.sessionStorage = {
    getItem() { throw new Error('Storage blocked'); },
    setItem() { throw new Error('Storage blocked'); },
    removeItem() { throw new Error('Storage blocked'); },
  };
  const { setOperatorToken, operatorToken, isConnected, api } = await import('./src/session.js?blocked');
  setOperatorToken('temporary-token');
  assert.equal(operatorToken(), 'temporary-token');
  assert.equal(isConnected(), true);
  globalThis.fetch = async (_url, options) => {
    assert.equal(options.headers.Authorization, 'Bearer temporary-token');
    return new Response('{"projects":[]}', { status: 200 });
  };
  assert.deepEqual(await api('/v1/projects'), { projects: [] });
  setOperatorToken('');
  assert.equal(operatorToken(), '');
  assert.equal(isConnected(), false);
});

test('successful HTML or malformed JSON is reported as an invalid API response', async () => {
  const { api, ApiError } = await import('./src/session.js');
  for (const body of ['<!doctype html>', '{invalid']) {
    globalThis.fetch = async () => new Response(body, { status: 200 });
    await assert.rejects(api('/v1/projects'), error => error instanceof ApiError && /invalid response/i.test(error.message));
  }
  globalThis.fetch = async () => new Response(null, { status: 204 });
  assert.deepEqual(await api('/v1/example', { method: 'DELETE' }), {});
  globalThis.fetch = async () => new Response('Bad Gateway', { status: 502 });
  await assert.rejects(api('/v1/projects'), error => error instanceof ApiError && error.status === 502);
});
