#!/usr/bin/env node
// Operator-only catalogue acquisition. Parsing and publication stay in MAIN's
// server, whose immutable capture is authoritative. This script runs no code
// from the downloaded page and accepts no arbitrary source URL.
import { createHash } from 'node:crypto';
import { readFile, mkdtemp, writeFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawn } from 'node:child_process';
export const SOURCE='https://prestocks.com/products';
export const MAX_BYTES=2*1024*1024;
export async function captureCatalogue(fetcher=fetch){
 const response=await fetcher(SOURCE,{redirect:'error',signal:AbortSignal.timeout(20000),headers:{Accept:'text/html'}});
 if(!response.ok||!response.body)throw new Error('Catalogue source unavailable');
 const chunks=[];let length=0;for await(const chunk of response.body){length+=chunk.length;if(length>MAX_BYTES)throw new Error('Catalogue source exceeds byte bound');chunks.push(chunk);}
 const bytes=Buffer.concat(chunks);const content=new TextDecoder('utf-8',{fatal:true}).decode(bytes);
 return{source_url:SOURCE,retrieved_at:new Date().toISOString(),content_sha256:createHash('sha256').update(bytes).digest('hex'),content};
}
export async function readSavedCapture(path){
 if((await stat(path)).size>3*1024*1024)throw new Error('Saved catalogue capture exceeds byte bound');
 const bytes=await readFile(path);if(bytes.length>3*1024*1024)throw new Error('Saved catalogue capture exceeds byte bound');
 JSON.parse(new TextDecoder('utf-8',{fatal:true}).decode(bytes));return bytes;
}
async function main(){
 const args=process.argv.slice(2),options={};for(let i=0;i<args.length;i+=2){if(!['--capture','--server-binary','--data-dir'].includes(args[i])||!args[i+1]||options[args[i]])throw new Error('Usage: import-catalogue.mjs --server-binary PATH --data-dir PATH [--capture SAVED_JSON]');options[args[i]]=args[i+1];}
 if(!options['--server-binary']||!options['--data-dir'])throw new Error('Explicit server binary and data directory are required');
 const bytes=options['--capture']?await readSavedCapture(options['--capture']):Buffer.from(JSON.stringify(await captureCatalogue()));
 const scratch=await mkdtemp(join(tmpdir(),'eplyx-catalogue-'));try{const path=join(scratch,'capture.json');await writeFile(path,bytes,{flag:'wx',mode:0o600});
 const code=await new Promise((resolveExit,reject)=>{const child=spawn(resolve(options['--server-binary']),['admin','import-catalogue','--capture',path],{env:{EPLYX_DATA_DIR:resolve(options['--data-dir'])},stdio:'inherit'});child.once('error',reject);child.once('exit',code=>resolveExit(code??1));});process.exitCode=code;
 }finally{await rm(scratch,{recursive:true,force:true});}
}
if(process.argv[1]&&import.meta.url===pathToFileURL(resolve(process.argv[1])).href)main().catch(()=>{console.error('Catalogue import failed; the existing catalogue was retained.');process.exitCode=1;});
