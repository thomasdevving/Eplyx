import {test,after} from 'node:test';
import assert from 'node:assert/strict';
import {mkdtemp,writeFile,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {reviewTarget} from './review-smoke.mjs';
const root=await mkdtemp(join(tmpdir(),'eplyx-review-target-'));
let n=0;
async function target(value){const path=join(root,String(n++));await writeFile(path,JSON.stringify(value));return reviewTarget(path);}
test('review guard refuses missing target and implicit local selection',async()=>{
 await assert.rejects(reviewTarget(undefined),/explicitly/);
 await assert.rejects(target({url:'http://127.0.0.1:4491'}),/non-production/);
});
test('review guard requires exact origin and disallows embedded credentials',async()=>{
 await assert.rejects(target({url:'https://review.example.invalid/p/foo',selection:'isolated-review'}));
 await assert.rejects(target({url:'https://user:secret@review.example.invalid',selection:'isolated-review'}));
});
test('remote target requires HTTPS and attested independent storage',async()=>{
 await assert.rejects(target({url:'http://review.example.invalid',selection:'isolated-review'}));
 await assert.rejects(target({url:'https://review.example.invalid',selection:'isolated-review'}),/Missing verified target/);
 const remote={url:'https://review.example.invalid',selection:'isolated-review',account_id:'a',project_id:'p',environment_id:'e',service_id:'s',identity_storage_id:'pg',analytical_storage_id:'volume'};
 await assert.rejects(target(remote),/Owner must verify/);
 assert.equal((await target({...remote,owner_confirmed_isolation:true})).url,remote.url);
});
test('explicit loopback review target is accepted',async()=>{
 assert.equal((await target({url:'http://127.0.0.1:4491',selection:'isolated-review'})).selection,'isolated-review');
});
// Only this test's newly allocated scratch data is disposable.
after(()=>rm(root,{recursive:true,force:true}));
