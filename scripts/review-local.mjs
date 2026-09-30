// Explicit loopback setup. Preserves its private receipt/storage; stops only its own processes.
import assert from 'node:assert/strict';
import {mkdtemp,mkdir,writeFile,readFile,readdir,copyFile,open} from 'node:fs/promises';
import {join,resolve} from 'node:path';
import {tmpdir,userInfo} from 'node:os';
import {spawn,execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {randomBytes,createHash} from 'node:crypto';
import {reviewTarget,smoke} from './review-smoke.mjs';
const exec=promisify(execFile);
const repo=resolve(process.argv[2]||'');
assert.ok(process.argv[2],'Pass the clean review worktree explicitly');
assert.equal((await exec('git',['status','--porcelain','--untracked-files=normal'],{cwd:repo})).stdout,'','Build/test only a clean snapshot');
const commit=(await exec('git',['rev-parse','HEAD'],{cwd:repo})).stdout.trim();
const root=await mkdtemp(join(tmpdir(),'eplyx-review-'));await mkdir(join(root,'volume'),{mode:0o700});
console.log(`Private review receipt/storage: ${root}`);
const binary=join(repo,'target/debug/eplyx-server'),cli=join(repo,'target/debug/eplyx');
const hash=b=>createHash('sha256').update(b).digest('hex');
const candidate=resolve(process.env.EPLYX_REVIEW_CANDIDATE||join(repo,'artifacts/fixture_stake_pool_config_v2.so'));
assert.equal(hash(await readFile(candidate)),'a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d');
assert.equal((await readFile(candidate)).length,133992);
const version=JSON.parse((await exec(cli,['version','--json'],{env:{},cwd:root})).stdout);assert.equal(version.commit,commit);
const identities={commit,branch:(await exec('git',['branch','--show-current'],{cwd:repo})).stdout.trim(),cli:version,server_sha256:hash(await readFile(binary)),worker_sha256:hash(await readFile(binary)),cli_sha256:hash(await readFile(cli)),lock_sha256:hash(await readFile(join(repo,'Cargo.lock'))),frontend_source:commit,image:null,candidate_sha256:hash(await readFile(candidate)),data_mount:'private host filesystem directory; no container volume tested'};
await writeFile(join(root,'build-identities.json'),JSON.stringify(identities,null,2)+'\n',{mode:0o600});
const password=randomBytes(24).toString('hex'),operator=randomBytes(32).toString('hex'),signupCode=randomBytes(16).toString('hex');
const passfile=join(root,'pg-password');await writeFile(passfile,password,{mode:0o600});
const pg=join(root,'postgres'),port=process.env.EPLYX_REVIEW_LOCAL_PORT||'4491',pgPort=process.env.EPLYX_REVIEW_PG_PORT||'55491';
const base=`http://127.0.0.1:${port}`;
const pgEnv={PATH:process.env.PATH};
await exec('initdb',['-D',pg,'-U',userInfo().username,'-A','scram-sha-256','--pwfile',passfile],{env:pgEnv});
await exec('pg_ctl',['-D',pg,'-l',join(root,'postgres.log'),'-o',`-h 127.0.0.1 -p ${pgPort} -k ${root}`,'-w','start'],{env:pgEnv});
const serviceEnv={EPLYX_DATA_DIR:join(root,'volume'),EPLYX_BIND:`127.0.0.1:${port}`,EPLYX_PUBLIC_URL:base,EPLYX_ALLOWED_ORIGINS:base,EPLYX_OPERATOR_TOKEN:operator,EPLYX_SIGNUP_CODE:signupCode,EPLYX_MAX_CONCURRENT_RUNS:'1',EPLYX_DATABASE_URL:`postgresql://${userInfo().username}:${password}@127.0.0.1:${pgPort}/postgres`};
let service;
const start=async()=>{
 const log=await open(join(root,'service.log'),'a',0o600);
 service=spawn(binary,['serve'],{cwd:root,env:serviceEnv,stdio:['ignore',log.fd,log.fd]});await log.close();
 for(let i=0;i<100;i++){if(service.exitCode!==null)throw new Error('Review server exited; inspect private service.log');try{if((await fetch(base+'/ready')).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}
 throw new Error('Review readiness timeout');
};
const stop=async()=>{if(service?.exitCode===null){service.kill('SIGTERM');await new Promise(r=>service.once('exit',r));}};
const post=async(path,data,{cookie='',admin=false}={})=>{
 const form=data instanceof FormData;
 const r=await fetch(base+path,{method:'POST',headers:{origin:base,...(form?{}:{'content-type':'application/json'}),...(cookie?{cookie}:{}),...(admin?{authorization:`Bearer ${operator}`}:{})},body:form?data:JSON.stringify(data)});
 assert.ok(r.ok(),`Bootstrap ${path}: ${r.status()}`);
 return {body:await r.json(),cookie:r.headers.get('set-cookie')?.split(';')[0]||cookie};
};
try {
 await start();
 const owner=await post('/v1/auth/signup',{email:`owner-${Date.now()}@example.invalid`,name:'Step 11A review owner',password,signup_code:signupCode});
 const credentials={email:`reviewer-${Date.now()}@example.invalid`,password:randomBytes(24).toString('hex')};
 await post('/v1/auth/signup',{...credentials,name:'Step 11A test reviewer',signup_code:signupCode});
 const workspace=owner.body.workspace_id;
 const project=(await post(`/v1/workspaces/${workspace}/projects`,{name:'Step 11A isolated Stake Pool review',program_id:'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy'},{cookie:owner.cookie})).body.project.id;
 await post(`/v1/workspaces/${workspace}/members`,{email:credentials.email},{cookie:owner.cookie});
 const otherWs=(await post('/v1/workspaces',{name:'Step 11A authorization control'},{cookie:owner.cookie})).body.workspace.id;
 const deniedProject=(await post(`/v1/workspaces/${otherWs}/projects`,{name:'Step 11A inaccessible control',program_id:'SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy'},{cookie:owner.cookie})).body.project.id;
 const bundle=new FormData();
 const add=async(dir,prefix='')=>{for(const entry of await readdir(dir,{withFileTypes:true})){const name=prefix+entry.name;if(entry.isDirectory())await add(join(dir,entry.name),name+'/');else bundle.append(name,new Blob([await readFile(join(dir,entry.name))]),name);}};
 await add(join(repo,'deploy/bundle'));
 const registered=(await post(`/v1/projects/${project}/bundles`,bundle,{admin:true})).body;
 await post(`/v1/projects/${project}/bundles/${registered.bundle_id}/activate`,{},{admin:true});
 const upload=new FormData();upload.append('candidate',new Blob([await readFile(candidate)]),'candidate.so');upload.append('change_spec',new Blob([await readFile(join(repo,'docs/examples/stake-pool-config-upgrade-change.json'))]),'change.json');
 const parent=(await post(`/v1/projects/${project}/checks`,upload,{admin:true})).body.run_id;
 let parentResult;
 for(let i=0;i<180;i++){const r=await fetch(`${base}/v1/runs/${parent}`,{headers:{cookie:owner.cookie}});assert.ok(r.ok());parentResult=await r.json();if(!['queued','running'].includes(parentResult.status))break;await new Promise(r=>setTimeout(r,1000));}
 assert.equal(parentResult.report_available,true,'Retained parent must come from the real worker');
 const selection={url:base,selection:'isolated-review',mode:'local',source_commit:commit};
 await writeFile(join(root,'target.json'),JSON.stringify(selection,null,2)+'\n',{mode:0o600});
 const target=await reviewTarget(join(root,'target.json'));
 const options={target,credentials,project,parent,deniedProject,proposalPath:join(repo,'docs/examples/stake-pool-parameter-change.json')};
 await writeFile(join(root,'bootstrap.json'),JSON.stringify({workspace,project,deniedProject,parent,parent_status:parentResult.status,bundle:registered},null,2)+'\n',{mode:0o600});
 const first=await smoke({...options,out:join(root,'before-restart')});
 // The exact same PostgreSQL cluster and filesystem are reused, with no bootstrap.
 await stop();await start();
 const second=await smoke({...options,out:join(root,'after-restart'),existingRun:first.child});
 assert.equal(second.report_sha256,first.report_sha256);assert.equal(second.checks.download_sha256,first.checks.download_sha256);
 const parentAfter=await fetch(`${base}/v1/runs/${parent}`,{headers:{cookie:owner.cookie}});assert.ok(parentAfter.ok());assert.equal((await parentAfter.json()).bundle_sha256,parentResult.bundle_sha256);
 // Fresh standalone directory: only the executable and the downloaded artifact.
 const standalone=join(root,'standalone');await mkdir(standalone,{mode:0o700});await copyFile(cli,join(standalone,'eplyx'));
 await exec('python3',['-c',`import tarfile,pathlib,sys
archive,dest=sys.argv[1:];root=pathlib.Path(dest)
with tarfile.open(archive) as t:
 members=t.getmembers();assert len(members)<=40;assert sum(m.size for m in members)<=128*1024*1024
 for m in members:
  p=pathlib.PurePosixPath(m.name);assert (m.isfile() or m.isdir()) and p.parts[0]=='interaction' and not p.is_absolute() and '..' not in p.parts and '\\\\' not in m.name
 t.extractall(root,filter='data')`,join(root,'after-restart/interaction.tar'),standalone]);
 const policy='(version 1)(allow default)(deny network*)';
 // Negative control proves the policy denies even loopback network requests.
 let denied=false;try{await exec('/usr/bin/sandbox-exec',['-p',policy,'/usr/bin/curl','--max-time','2',base+'/health'],{env:{},cwd:standalone});}catch{denied=true;}assert.ok(denied,'Network-denial control must fail');
 const offline={};
 for(const operation of ['verify','reproduce']) {
  const {stdout}=await exec('/usr/bin/sandbox-exec',['-p',policy,join(standalone,'eplyx'),'interaction',operation,'--artifact',join(standalone,'interaction'),'--format','json'],{cwd:standalone,env:{},timeout:300000,maxBuffer:4*1024*1024});
  offline[operation]=JSON.parse(stdout);assert.equal(offline[operation].report_sha256,first.report_sha256);assert.equal(offline[operation].analysis_input_sha256,first.analysis_input_sha256);
 }
 assert.equal(offline.verify.vm_execution_requested,false);assert.equal(offline.reproduce.vm_execution_requested,true);
 const receipt={status:'locally_verified_remote_not_validated',identities,bootstrap:{workspace,project,deniedProject,parent,parent_status:parentResult.status,bundle:registered},browser:first,restart:{completed_result:'passed',project_baseline_parent_child_artifact:'survived same-storage service restart',queued_running_recovery:'not exercised'},offline,network:'macOS sandbox-exec deny network* inherited by CLI child; loopback curl denial control',remote:'No positively confirmed accessible isolated environment; no push/deployment',production:'untouched'};
 await writeFile(join(root,'validation-receipt.json'),JSON.stringify(receipt,null,2)+'\n',{mode:0o600});
 console.log(`Validation receipt: ${join(root,'validation-receipt.json')}`);
} finally {await stop();await exec('pg_ctl',['-D',pg,'-m','fast','-w','stop'],{env:pgEnv});}
