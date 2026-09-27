import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {captureCatalogue,SOURCE,MAX_BYTES} from '../../scripts/import-catalogue.mjs';
test('catalogue acquisition pins the first-party source and exact bytes without executing page code',async()=>{let called;const content='<script>throw new Error("never execute")</script>';const result=await captureCatalogue(async(url,options)=>{called={url,options};return new Response(content);});assert.equal(called.url,SOURCE);assert.equal(called.options.redirect,'error');assert.equal(result.content,content);assert.equal(result.content_sha256,createHash('sha256').update(content).digest('hex'));assert.equal(result.source_url,SOURCE);});
test('catalogue acquisition refuses failures, oversize responses and invalid UTF-8',async()=>{await assert.rejects(captureCatalogue(async()=>new Response('unavailable',{status:503})));await assert.rejects(captureCatalogue(async()=>new Response(new Uint8Array(MAX_BYTES+1))));await assert.rejects(captureCatalogue(async()=>new Response(new Uint8Array([255]))));});


test('saved capture import preserves the exact original envelope bytes',async()=>{
 const {readSavedCapture}=await import('../../scripts/import-catalogue.mjs');
 const {mkdtemp,writeFile,rm}=await import('node:fs/promises');const {tmpdir}=await import('node:os');const {join}=await import('node:path');
 const directory=await mkdtemp(join(tmpdir(),'eplyx-catalogue-bytes-'));
 try {const path=join(directory,'capture.json');const bytes=Buffer.from('  { "content": "saved public source", "schema": 1 }\n');await writeFile(path,bytes);assert.deepEqual(await readSavedCapture(path),bytes);await writeFile(path,'invalid');await assert.rejects(readSavedCapture(path));}
 finally {await rm(directory,{recursive:true,force:true});}
});
