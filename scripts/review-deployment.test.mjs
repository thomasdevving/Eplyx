import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtempSync, existsSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
const guard = new URL('../deploy/review/expire.sh', import.meta.url).pathname;
const run = (expiry, args) => spawnSync('/bin/sh', [guard, ...args], {
  env: { PATH: process.env.PATH, EPLYX_REVIEW_EXPIRES_UNIX: expiry }, encoding: 'utf8', timeout: 6000,
});
test('missing deadline fails closed; expired restart cannot execute a command', () => {
  const dir=mkdtempSync(join(tmpdir(),'eplyx-expiry-test-'));
  try {
    const file=join(dir,'started');
    assert.equal(run('', ['touch',file]).status,64);
    assert.equal(run('invalid', ['touch',file]).status,64);
    assert.equal(run(String(Math.floor(Date.now()/1000)-1), ['touch',file]).status,0);
    assert.equal(existsSync(file),false);
  } finally { rmSync(dir,{recursive:true,force:true}); }
});
test('absolute deadline stops a running service and cannot be renewed by restart', () => {
  const expiry=String(Math.floor(Date.now()/1000)+2);
  const started=Date.now();
  const stopped=run(expiry,['sleep','30']);
  assert.equal(stopped.status,124,stopped.stderr);
  assert.ok(Date.now()-started<4500);
  const restarted=run(expiry,['sleep','30']);
  assert.equal(restarted.status,0,restarted.stderr);
});
test('review image contains matching CLI and retained embedded CLI fixture', () => {
  const docker=readFileSync(new URL('../deploy/review/Dockerfile',import.meta.url),'utf8');
  assert.ok(docker.includes('--locked --release -p eplyx-server -p eplyx-engine --bin eplyx-server --bin eplyx'));
  assert.ok(docker.includes('/build/target/release/eplyx /usr/local/bin/eplyx'));
  const ignore=readFileSync(new URL('../deploy/review/Dockerfile.dockerignore',import.meta.url),'utf8');
  assert.ok(ignore.includes('!examples/migrations/minimal/fixtures/world.json'));
  assert.ok(ignore.includes('media/'));
});
