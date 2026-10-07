import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createServer, request } from 'node:http';
import { cp, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

function get(port, path, method = 'GET') {
  return new Promise((resolve, reject) => {
    const req = request({ host: '127.0.0.1', port, path, method }, response => {
      let body = '';
      response.setEncoding('utf8').on('data', chunk => { body += chunk; });
      response.on('end', () => resolve({ status: response.statusCode, headers: response.headers, body }));
    });
    req.on('error', reject);
    req.setTimeout(3000, () => req.destroy(new Error('Static server request timed out')));
    req.end();
  });
}

for (const script of ['serve.mjs', 'dev-server.mjs']) {
  test(`${script}: malformed URLs cannot crash the server; routes and assets keep HTTP semantics`, async t => {
    const root = await mkdtemp(join(tmpdir(), 'eplyx-static-test-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    const frontend = join(root, 'frontend');
    await mkdir(frontend);
    await cp(new URL(`../${script}`, import.meta.url), join(frontend, script));
    // Asset preparation is outside the HTTP handler's contract.
    await writeFile(join(frontend, 'vendor.mjs'), 'export async function prepareVendorAssets() {}');
    await writeFile(join(frontend, 'runtime-config.mjs'), 'export async function writeRuntimeConfig() { return ""; }');
    for (const directory of [frontend, join(root, 'dist')]) {
      await mkdir(join(directory, 'public'), { recursive: true });
      await writeFile(join(directory, 'index.html'), '<!doctype html><title>Eplyx test</title>');
      await writeFile(join(directory, 'public/runtime-config.js'), 'globalThis.EPLYX_API_URL = "";');
    }
    const probe = createServer();
    await new Promise(resolve => probe.listen(0, '127.0.0.1', resolve));
    const port = probe.address().port;
    await new Promise(resolve => probe.close(resolve));
    const child = spawn(process.execPath, [join(frontend, script)], {
      env: { PORT: String(port) }, stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stderr = '';
    child.stderr.on('data', chunk => { stderr += chunk; });
    const exited = once(child, 'exit');
    t.after(async () => { child.kill(); await exited; });
    await Promise.race([
      once(child.stdout, 'data'),
      exited.then(() => { throw new Error(`Server failed to start: ${stderr}`); }),
    ]);

    for (const path of ['/%invalid', '/%E0%A4%A', '/%00']) {
      assert.equal((await get(port, path)).status, 400, path);
      assert.equal((await get(port, '/start')).status, 200, 'still serves after malformed input');
    }
    assert.equal((await get(port, '/missing.js')).status, 404);
    assert.equal((await get(port, '/public/runtime-config.js')).headers['cache-control'], 'no-store');
    const head = await get(port, '/start', 'HEAD');
    assert.equal(head.status, 200);
    assert.equal(head.body, '');
    const post = await get(port, '/start', 'POST');
    assert.equal(post.status, 405);
    assert.equal(post.headers.allow, 'GET, HEAD');
  });
}
