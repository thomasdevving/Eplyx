// Generate new presentation inputs with MAIN's built binary; no provider or
// credentials are inherited. Existing output directories are refused.
import { cp, mkdir, mkdtemp, readFile, readdir, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const repo=fileURLToPath(new URL('..',import.meta.url));
const out=process.argv[2];if(!out)throw new Error('Pass a new fixture output directory.');
const work=await mkdtemp(join(tmpdir(),'eplyx-dashboard-build-'));
const binary=join(repo,'target/debug',process.platform==='win32'?'eplyx.exe':'eplyx');
async function project(name){
 const path=join(work,name);await cp(join(repo,'examples/migrations/minimal'),path,{recursive:true});
 await mkdir(join(path,'target/deploy'),{recursive:true});
 const config=await readFile(join(path,'eplyx.toml'),'utf8');await writeFile(join(path,'eplyx.toml'),config.replace('name = "minimal-migration"',`name = "${name}"`));return path;
}
function cli(path,args,allowed=[0]){
 const run=spawnSync(binary,args,{cwd:path,env:{},encoding:'utf8',maxBuffer:32*1024*1024});
 if(!allowed.includes(run.status))throw new Error(`Fixture command failed (${run.status}): ${args.slice(0,2).join(' ')}\n${run.stdout}\n${run.stderr}`);
 return JSON.parse(run.stdout);
}
const install=(path,defect=false)=>cp(join(repo,`artifacts/eplyx_token_migration${defect?'_defect_deadline_inclusive':''}.so`),join(path,'target/deploy/migration.so'));
async function setReserve(path,amount){const file=join(path,'migration.json');const spec=JSON.parse(await readFile(file));spec.destination_funding.reserve.funded_raw=amount;await writeFile(file,JSON.stringify(spec,null,2)+'\n');}
async function analyse(path,search=false){const r=cli(path,['migration',search?'search':'analyse','--format','json'],[0,1]);return r.run_id;}
async function reproduce(path,run,dimension){
 const base=join(path,'.eplyx/counterexamples');
 for(const file of await readdir(base)){
  const c=JSON.parse(await readFile(join(base,file)));
  if(c.parent_run===run && c.counterexample.kind==='MigrationDerived' && (!dimension||c.counterexample.dimension===dimension)){
   cli(path,['migration','reproduce',c.id,'--format','json']);return c.id;
  }
 }
 // The enum uses an internal serde tag; locate the typed dimension independent
 // of spelling of that tag. No outcome is synthesized.
 for(const file of await readdir(base)){
  const c=JSON.parse(await readFile(join(base,file)));
  if(c.parent_run===run && c.counterexample.dimension && (!dimension||c.counterexample.dimension===dimension)){
   cli(path,['migration','reproduce',c.id,'--format','json']);return c.id;
  }
 }
 throw new Error('Required derived counterexample missing.');
}
const transition=await project('transition-acceptance');await install(transition);
// A smaller synthetic population keeps dashboard quantities readable.
// The original minimal recipe is never modified.
const smallPath=join(transition,'fixtures/world.json'),small=JSON.parse(await readFile(smallPath));
for(const account of small.tokenAccounts){account.amount=(BigInt(account.amount)/1000000n).toString();if(account.delegate)account.delegate.amount=(BigInt(account.delegate.amount)/1000000n).toString();}
await writeFile(smallPath,JSON.stringify(small,null,2)+'\n');await setReserve(transition,'2000000');
const healthyNoSearch=await analyse(transition);
await install(transition,true);await setReserve(transition,'1000000');const underfundedEarly=await analyse(transition,true);
await install(transition);await setReserve(transition,'2000000');const healthy=await analyse(transition,true);
await install(transition,true);await setReserve(transition,'1000000');const underfunded=await analyse(transition,true);
const derived=await reproduce(transition,underfunded,"DeadlineBoundary");
await reproduce(transition,underfunded); // two actual immutable replay records
const migration=await project('token-migration');await install(migration);
const reference=await analyse(migration,true);await install(migration,true);const defect=await analyse(migration,true);
const deadline=await reproduce(migration,defect,'DeadlineBoundary');
const second=await project('second-asset');await install(second);
const recipeFile=join(second,'fixtures/world.json');const recipe=JSON.parse(await readFile(recipeFile));recipe.id='dashboard-second-asset';await writeFile(recipeFile,JSON.stringify(recipe,null,2)+'\n');
const addresses=cli(second,['migration','fixture','fixtures/world.json','--format','json']).addresses;
const address=label=>addresses.find(x=>x.label===label).address;
const specFile=join(second,'migration.json'),spec=JSON.parse(await readFile(specFile));spec.source.mint=address('source');spec.destination.mint=address('destination');spec.authorities.expected.source_mint_authority.address=address('issuer');await writeFile(specFile,JSON.stringify(spec,null,2)+'\n');
const secondRun=await analyse(second,true);
// Build the example first: cargo build -p eplyx-engine --example dashboard_records.
// Its provider is a synthetic in-memory implementation with no network client.
const inputs=join(work,'analytical-inputs'), analytical=join(work,'analytical-kinds');
const generated=spawnSync(join(repo,'target/debug/examples/dashboard_records'),[inputs],{env:{},encoding:'utf8'});
if(generated.status!==0)throw new Error(generated.stderr);
await mkdir(analytical);
cli(analytical,['lifecycle','analyse','--snapshot',join(inputs,'snapshot.json'),'--scenario',join(inputs,'scenario.json'),'--at','2026-09-28T00:00:00Z','--record',analytical,'--format','json']);
cli(analytical,['observe','replay','--input',join(inputs,'current.capture.json'),'--record',analytical,'--format','json']);
const projection=spawnSync(process.execPath,[join(repo,'scripts/dashboard-fixture.mjs'),resolve(out),`${transition}=transition-acceptance`,`${migration}=token-migration`,`${second}=second-asset`,`${analytical}=analytical-kinds`],{env:{},encoding:'utf8'});
if(projection.status!==0)throw new Error(projection.stderr);process.stdout.write(projection.stdout);
const ids={healthyNoSearch,underfundedEarly,healthy,underfunded,derived,reference,defect,deadline,secondRun};
await writeFile(join(out,'ids.json'),JSON.stringify(ids,null,2)+'\n',{flag:'wx'});
const rust=Object.entries(ids).map(([key,id])=>`pub const ${key.replace(/[A-Z]/g,x=>'_'+x).toUpperCase()}: &str = "${id}";`).join('\n');
await writeFile(join(out,'ids.rs'),`// Generated fixture identities. Economic expectations remain in the T0 contract.\n${rust}\n`,{flag:'wx'});
console.log('Generated reference, deficient-funding, deadline-defect and second-asset records. No live provider used.');
