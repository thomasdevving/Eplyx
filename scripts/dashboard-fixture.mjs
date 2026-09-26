// Exact small-document projection of freshly generated MAIN local stores.
// Refuses an existing output directory: historical fixtures are never rewritten.
// node scripts/dashboard-fixture.mjs <new-out> <project-dir>=<fixture-name> ...
import { mkdir, readFile, readdir, lstat, writeFile } from 'node:fs/promises';
import { join, basename } from 'node:path';
import { createHash } from 'node:crypto';
const [out,...sources]=process.argv.slice(2);
if (!out || !sources.length) throw new Error('usage: dashboard-fixture.mjs <new-out> <project-dir>=<fixture-name> ...');
await mkdir(out); // exclusive directory creation
const files=['metadata.json','result/report.json','result/report.md','result/bindings.json','input/change.json','input/state.json','search/migration-search.json'];
const digest=b=>createHash('sha256').update(b).digest('hex');
const rows=[];
async function copy(source,target,relative,required=false) {
 const meta=await lstat(source).catch(e=>{if(e.code==='ENOENT'&&!required)return null;throw e;});
 if(!meta)return;
 if(!meta.isFile()||meta.isSymbolicLink()||meta.size>16*1024*1024)throw new Error(`Invalid small artifact: ${relative}`);
 const bytes=await readFile(source);
 if(bytes.length!==meta.size)throw new Error('Artifact changed while copying');
 const text=bytes.toString('utf8');
 if(/(?:\/Users\/|\/home\/[^ /]+\/|\/private\/tmp\/|\/var\/folders\/|https?:\/\/[^\s"<>]*(?:api-key=|apikey=)|PRIVATE KEY-----)/i.test(text))throw new Error(`Private path or credential-shaped content refused: ${relative}`);
 await mkdir(join(target,'..'),{recursive:true});
 await writeFile(target,bytes,{flag:'wx'});rows.push({path:relative,bytes:bytes.length,sha256:digest(bytes)});
}
for(const source of sources){
 const split=source.lastIndexOf('=');if(split<1)throw new Error('A fixture name is required');
 const directory=source.slice(0,split),name=source.slice(split+1);
 if(!/^[a-z0-9-]+$/.test(name))throw new Error('Invalid fixture name');
 const store=join(directory,'.eplyx'),target=join(out,name,'.eplyx');
 for(const child of ['runs','counterexamples','reproductions','cache'])await mkdir(join(target,child),{recursive:true});
 await copy(join(store,'project.json'),join(target,'project.json'),`${name}/.eplyx/project.json`,true);
 const runs=(await readdir(join(store,'runs'))).filter(id=>/^run_[a-z0-9_]+$/.test(id)).sort();
 for(const id of runs)for(const file of files)await copy(join(store,'runs',id,file),join(target,'runs',id,file),`${name}/.eplyx/runs/${id}/${file}`,['metadata.json','result/report.json'].includes(file));
 for(const child of ['counterexamples','reproductions'])for(const file of await readdir(join(store,child))){
  if(!/^(cx|repro)_[a-z0-9_]+\.json$/.test(file))continue;
  const record=JSON.parse(await readFile(join(store,child,file),'utf8'));
  if(runs.includes(record.parent_run))await copy(join(store,child,file),join(target,child,file),`${name}/.eplyx/${child}/${file}`);
 }
}
await writeFile(join(out,'provenance.json'),JSON.stringify({schema_version:1,purpose:'Presentation fixture; exact small MAIN engine artifacts. Captures and candidate bytes stay outside this fixture, so it cannot be replayed.',files:rows},null,2)+'\n',{flag:'wx'});
await writeFile(join(out,'README.md'),'# Dashboard presentation records\n\nGenerated from MAIN with the built `eplyx` binary and projected by\n`scripts/dashboard-fixture.mjs`. Every retained analytical byte is exact.\nCandidate binaries and raw state captures are omitted; these records test\npresentation and cannot be replayed. See `provenance.json` for sizes and hashes.\nThese fixtures never substitute for the frozen T0 contract.\n',{flag:'wx'});
console.log(`Projected ${rows.length} exact small artifacts into ${basename(out)}.`);
