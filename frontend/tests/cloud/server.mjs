import { observationRPC } from './observation-rpc.mjs';
// Real MAIN service and CLI over test-only Postgres and a temporary registry.
// All run bytes and seed credentials stay in scratch storage.
import { cp, mkdir, mkdtemp, rm, writeFile, readFile, readdir } from 'node:fs/promises';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
const [port] = process.argv.slice(2);
const database = process.env.EPLYX_CLOUD_TEST_DATABASE_URL;
if (!database) throw new Error('EPLYX_CLOUD_TEST_DATABASE_URL is required; cloud tests never skip');
const admin=new URL(database);
if (!['127.0.0.1','localhost','[::1]'].includes(admin.hostname)) throw new Error('Cloud browser tests require a loopback test database');
const repo = fileURLToPath(new URL('../../../', import.meta.url));
const exe = name => join(repo,'target/debug',process.platform === 'win32' ? `${name}.exe` : name);
const base = `http://127.0.0.1:${port}`;
const databaseName=`eplyx_browser_${process.pid}_${Date.now()}`;
const pgEnv={PATH:process.env.PATH,PGHOST:admin.hostname,PGPORT:admin.port||'5432',PGUSER:decodeURIComponent(admin.username),PGPASSWORD:decodeURIComponent(admin.password),PGDATABASE:admin.pathname.slice(1)};
const sql=async statement=>{try{await promisify(execFile)('psql',['--no-psqlrc','-v','ON_ERROR_STOP=1','-c',statement],{env:pgEnv});}catch{throw new Error('Test database setup or cleanup failed');}};
await sql(`CREATE DATABASE ${databaseName}`);
const ownDatabase=new URL(database);ownDatabase.pathname=`/${databaseName}`;
const scratch = process.env.EPLYX_CLOUD_BROWSER_DIR || join(tmpdir(),'eplyx-cloud-browser-results');
await mkdir(scratch,{ recursive:true });
await rm(join(scratch,'cloud-seed.json'),{ force:true });
const root = await mkdtemp(join(tmpdir(),'eplyx-cloud-browser-'));
const observation=await observationRPC(repo);
const catalogue=JSON.parse(await readFile(join(repo,'fixtures/catalogue/provenance.json')));
await promisify(execFile)(exe('eplyx-server'),['admin','import-catalogue','--capture',join(repo,'fixtures/catalogue',catalogue.current_version+'.json')],{env:{EPLYX_DATA_DIR:join(root,'volume')}});
const operator=crypto.randomUUID();
const server = spawn(exe('eplyx-server'),[],{ stdio:'inherit',env:{ EPLYX_OPERATOR_TOKEN:operator,EPLYX_MIGRATION_CANDIDATE:join(repo,'artifacts/eplyx_token_migration.so'),EPLYX_OBSERVATION_RPC_URL:observation.url,EPLYX_DATA_DIR:join(root,'volume'),EPLYX_DATABASE_URL:ownDatabase.href,EPLYX_PUBLIC_URL:base,EPLYX_BIND:`127.0.0.1:${port}` } });
let stopping=false;
const stop=async () => { if(stopping)return;stopping=true;server.kill();if(server.exitCode===null)await new Promise(resolve=>server.once('exit',resolve));await observation.stop();await sql(`DROP DATABASE IF EXISTS ${databaseName} WITH (FORCE)`);await rm(root,{recursive:true,force:true});process.exit(0); };
process.on('SIGINT',stop);process.on('SIGTERM',stop);
server.on('exit',code=>{if(!stopping)process.exit(code??1);});
let ready=false;
for(let i=0;i<100;i++){try{if((await fetch(`${base}/ready`)).ok){ready=true;break;}}catch{} await new Promise(r=>setTimeout(r,100));}
if(!ready)throw new Error('Test service did not become ready');
const stamp=Date.now();const email=`alice+${stamp}@example.com`,password='correct horse battery';
const post=async(path,body,cookie='')=>{
 const response=await fetch(base+path,{method:'POST',headers:{'content-type':'application/json',origin:base,...(cookie?{cookie}:{})},body:JSON.stringify(body)});
 if(!response.ok)throw new Error(`seed request failed: ${path} (${response.status})`);
 return {body:await response.json(),cookie:response.headers.get('set-cookie')?.split(';')[0]??cookie};
};
const {body:account,cookie}=await post('/v1/auth/signup',{email,password,name:'Alice'});
const {body:{project}}=await post(`/v1/workspaces/${account.workspace_id}/projects`,{name:`transition-acceptance-${stamp}`},cookie);
const {body:ci}=await post(`/v1/projects/${project.id}/tokens`,{label:'browser seed'},cookie);
const local=join(root,'local');await cp(join(repo,'fixtures/dashboard/transition-acceptance'),local,{recursive:true});
for(const child of ['runs','counterexamples','reproductions','cache'])await mkdir(join(local,'.eplyx',child),{recursive:true});
await promisify(execFile)(exe('eplyx'),['sync'],{cwd:local,env:{EPLYX_TOKEN:ci.token,EPLYX_PROJECT_ID:project.id,EPLYX_URL:base,EPLYX_CONFIG_DIR:join(root,'home')}});
const {body:{project:analysisProject}}=await post(`/v1/workspaces/${account.workspace_id}/projects`,{name:`current-analysis-${stamp}`},cookie);
// A real upgrade through the unchanged upload/worker contract, in the same
// history as the current analyses. These bytes are MAIN's existing fixture.
const {body:{project:upgradeProject}}=await post(`/v1/workspaces/${account.workspace_id}/projects`,{name:`upgrade-history-${stamp}`,program_id:'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy'},cookie);
const bundle=new FormData();
async function addBundle(dir,prefix=''){for(const entry of await readdir(dir,{withFileTypes:true})){const name=prefix+entry.name;if(entry.isDirectory())await addBundle(join(dir,entry.name),name+'/');else bundle.append(name,new Blob([await readFile(join(dir,entry.name))]),name);}}
await addBundle(join(repo,'deploy/bundle'));
const sendOperator=async(path,body)=>{const response=await fetch(base+path,{method:'POST',headers:{authorization:`Bearer ${operator}`,...(body instanceof FormData?{}:{'content-type':'application/json'})},body:body instanceof FormData?body:JSON.stringify(body)});const result=await response.json();if(!response.ok)throw new Error(`Upgrade seed failed (${response.status})`);return result;};
const registered=await sendOperator(`/v1/projects/${upgradeProject.id}/bundles`,bundle);
await sendOperator(`/v1/projects/${upgradeProject.id}/bundles/${registered.bundle_id}/activate`,{});
const check=new FormData();check.append('candidate',new Blob([await readFile(join(repo,'artifacts/fixture_stake_pool_v2.so'))]),'candidate.so');
const upgrade=await sendOperator(`/v1/projects/${upgradeProject.id}/checks`,check);
for(let n=0;n<600;n++){const response=await fetch(`${base}/v1/runs/${upgrade.run_id}`,{headers:{cookie}});const run=await response.json();if(run.report_available)break;if(n===599)throw new Error('Upgrade seed did not finish');await new Promise(r=>setTimeout(r,100));}
await writeFile(join(scratch,'cloud-seed.json'),JSON.stringify({base,email,password,project:project.id,analysisProject:analysisProject.id,upgradeProject:upgradeProject.id,upgradeRun:upgrade.run_id,mint:observation.mint,source:observation.source,owner:observation.owner}),{mode:0o600});
console.log('Seeded test workspace');
