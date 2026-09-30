import {cp,mkdir,mkdtemp,readFile,writeFile} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {resolve,join} from 'node:path';
import {tmpdir} from 'node:os';
const root=resolve('media/weekly-2026-09-27'),exe=resolve('target/debug/eplyx');
const project=await mkdtemp(join(tmpdir(),'eplyx-video-cli-'));
await cp('examples/migrations/minimal',project,{recursive:true});
await mkdir(join(project,'target/deploy'),{recursive:true});
await cp('artifacts/eplyx_token_migration.so',join(project,'target/deploy/migration.so'));
const records=[];
function run(args){
 const r=spawnSync(exe,args,{cwd:project,encoding:'utf8',env:{PATH:process.env.PATH},maxBuffer:10*1024*1024});
 const rec={command:'eplyx '+args.join(' '),exit:r.status,stdout:r.stdout,stderr:r.stderr};records.push(rec);console.log(rec.command+' -> '+rec.exit);return rec;
}
run(['doctor']);
run(['migration','analyse']);
const list=run(['runs','--json']);
await writeFile(join(root,'assets','cli-history.json'),list.stdout);
const data=JSON.parse(list.stdout);console.log(JSON.stringify(data).slice(0,600));
const runs=Array.isArray(data)?data:data.runs;
const id=runs[0].id??runs[0].run_id;
run(['migration','search','--run',id]);
run(['migration','gate','--run',id,'--policy','strict']);
run(['migration','plan','--run',id,'--out','unsigned.json']);
await writeFile(join(root,'assets','cli-recording.json'),JSON.stringify(records,null,2));
await writeFile(join(root,'assets','cli-transcript.txt'),records.map(r=>'$ '+r.command+'\n'+r.stdout+r.stderr+'\n[exit '+r.exit+']').join('\n\n'));
