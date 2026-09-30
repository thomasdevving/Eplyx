/* Second edit: real moving browser recordings, condensed waits, and stepwise
   explanations. The recorded interface and its results are never reconstructed. */
const canvas=document.querySelector('#film'),ctx=canvas.getContext('2d',{alpha:false});
const W=1920,H=1080,DURATION=128,imgs={},videos={};
const scenes=[
 {at:0,end:6,kind:'hero',chapter:'THE WEEK IN EPLYX'},
 {at:6,end:9,kind:'hook',chapter:'35 COMMITS · THREE CONNECTED WORKFLOWS'},
 {at:9,end:19,kind:'replay',chapter:'01 / HOW HISTORICAL REPLAY WORKS'},
 {at:19,end:25,kind:'protocols',chapter:'01 / PROTOCOL ANALYSIS'},
 {at:25,end:35,kind:'upgrade',chapter:'02 / REVIEW AN UPGRADE'},
 {at:35,end:43,kind:'governance',chapter:'02 / CONNECT IT TO GOVERNANCE'},
 {at:43,end:52,kind:'migration',chapter:'03 / REHEARSE A MIGRATION'},
 {at:52,end:60,kind:'stress',chapter:'03 / INSPECT THE FAILURE'},
 {at:60,end:67,kind:'witness',chapter:'03 / REPRODUCE IT'},
 {at:67,end:74,kind:'compare',chapter:'03 / COMPARE TWO RUNS'},
 {at:74,end:86,kind:'observe',chapter:'04 / CAPTURE CURRENT STATE'},
 {at:86,end:96,kind:'path',chapter:'04 / CHECK ONE EXACT PATH'},
 {at:96,end:105,kind:'candidate',chapter:'04 / TEST THE CANDIDATE'},
 {at:105,end:116,kind:'lifecycle',chapter:'04 / EVALUATE THE LIFECYCLE'},
 {at:116,end:124,kind:'mosaic',chapter:'ONE PROGRAM · MULTIPLE WAYS TO WORK'},
 {at:124,end:128,kind:'end',chapter:'EPLYX'}
];
window.filmDuration=DURATION;window.filmScenes=scenes;
const names=['hero','dashboard','stress','witness','compare','upgrade','observe','path','candidate','lifecycle'];
function event(name,label){const rec=window.MOTION_LOG.find(x=>x.name===name);const point=rec?.events.find(x=>x.label===label);if(!point)throw Error('No recorded event '+name+' / '+label);return point.time;}
function warp(u,points){for(let i=1;i<points.length;i++){if(u<=points[i][0])return mix(points[i-1][1],points[i][1],clamp((u-points[i-1][0])/(points[i][0]-points[i-1][0])));}return points.at(-1)[1];}
const maxTime=n=>Math.max(0,(videos[n]?.duration||0)-.12);
function clipTime(name,u,seconds,start=.15,end=maxTime(name)){return mix(start,end,clamp(u/seconds));}
function sources(s,u){
 switch(s.kind){
 case 'hero':return [['hero',clipTime('hero',u,6,.85,6.8)]];
 case 'migration':return [['dashboard',clipTime('dashboard',u,9,.85)]];
 case 'upgrade':return [['upgrade',clipTime('upgrade',u,10,1.0)]];
 case 'stress':return [['stress',clipTime('stress',u,8,1.3)]];
 case 'witness':return [['witness',clipTime('witness',u,7,1.0)]];
 case 'compare':return [['compare',clipTime('compare',u,7,1.0)]];
 case 'observe':return [['observe',clipTime('observe',u,12,.75)]];
 case 'path':return [['path',warp(u,[[0,event('path','focus one transfer path')],[3.7,event('path','queue offline transfer')+.1],[5.8,event('path','actual transfer result')+.1],[10,maxTime('path')]])]];
 case 'candidate':return [['candidate',warp(u,[[0,event('candidate','candidate terms')],[3.0,event('candidate','queue candidate execution')+.1],[5.4,event('candidate','candidate result loaded')+.1],[9,maxTime('candidate')]])]];
 case 'lifecycle':return [['lifecycle',warp(u,[[0,event('lifecycle','declare lifecycle scenario')],[3.2,event('lifecycle','queue lifecycle evaluation')+.1],[6.3,event('lifecycle','separate assurance results loaded')+.1],[11,maxTime('lifecycle')]])]];
 case 'mosaic':return [['dashboard',clipTime('dashboard',u,8,1.8)],['upgrade',clipTime('upgrade',u,8,3.8)],['lifecycle',clipTime('lifecycle',u,8,event('lifecycle','queue lifecycle evaluation'))]];
 default:return [];
 }
}
async function seekVideo(name,time){const v=videos[name];time=clamp(time,.001,maxTime(name));if(Math.abs(v.currentTime-time)<.009&&v.readyState>=2)return;
 await new Promise((resolve,reject)=>{const timer=setTimeout(()=>{v.removeEventListener('seeked',done);reject(Error('Video seek timeout: '+name));},10000);const done=()=>{clearTimeout(timer);resolve();};v.addEventListener('seeked',done,{once:true});v.currentTime=time;});
}
window.prepareFrame=async function(t){t=clamp(t,0,DURATION-.001);const s=scenes.find(s=>t>=s.at&&t<s.end);await Promise.all(sources(s,t-s.at).map(([name,time])=>seekVideo(name,time)));};
function footage(name,x,y,w,h,{label='RECORDED FRONTEND',crop=null,bar=true,alpha=1}={}){
 const v=videos[name],bh=bar?34:0;ctx.save();ctx.globalAlpha*=alpha;
 ctx.shadowColor='#0009';ctx.shadowBlur=38;ctx.shadowOffsetY=12;box(x,y,w,h+bh,C.ink,15,'#bda0f64a');ctx.shadowColor='transparent';
 ctx.save();round(x+1,y+bh,w-2,h,bar?[0,0,14,14]:14);ctx.clip();let [sx,sy,sw,sh]=crop||[0,0,v.videoWidth,v.videoHeight];const scale=Math.max(w/sw,h/sh),nw=w/scale,nh=h/scale;sx+=(sw-nw)/2;ctx.drawImage(v,sx,sy,nw,nh,x,y+bh,w,h);ctx.restore();
 if(bar){for(let i=0;i<3;i++){ctx.fillStyle='#88649f';ctx.beginPath();ctx.arc(x+18+15*i,y+17,3.5,0,7);ctx.fill();}text(label,x+77,y+8,15,C.muted,500);}
 ctx.restore();
}
function topTitle(kicker,title){text(kicker,64,143,20,C.violet,600);text(title,64,184,61,C.white,500,'Manrope');}
function nativeTitle(title){text(title,64,142,54,C.white,500,'Manrope');}
function callout(u,index,title,body,meaning){
 const x=1440;tag('0'+index,x,281,{size:22});lines(title,x,351,42,400,C.white,500,1.12);
 let yy=lines(body,x,481,27,388,C.lav,400,1.38);rule(x,yy+32,391);
 text('WHAT THIS MEANS',x,yy+66,17,C.violet,600);lines(meaning,x,yy+105,26,391,C.muted,400,1.34);
}
function guide(rows,active,boundary){
 const x=1434;
 rows.forEach(([title,body],i)=>{const y=290+i*171,on=i===active;box(x,y,424,150,on?'#492168':'#181020',13,on?'#b68af9':'#8556ad38');text(String(i+1).padStart(2,'0'),x+18,y+22,19,on?C.lav:C.violet,600);text(title,x+60,y+17,29,on?C.white:C.lav,500,'Manrope');lines(body,x+20,y+65,24,383,on?C.lav:C.muted,400,1.3);});
 rule(x,835,424);text('READ THE RESULT',x,861,17,C.violet,600);lines(boundary,x,902,25,415,C.lav,400,1.3);
}
function stageRail(items,active,y=955){
 const gap=18,w=(1792-gap*(items.length-1))/items.length;
 items.forEach((s,i)=>{const x=64+i*(w+gap);box(x,y,w,51,i===active?'#6732b0':'#1b1128',9,i===active?'#bc94ff':'#7b4f9838');text(String(i+1).padStart(2,'0'),x+16,y+16,17,i===active?C.white:C.violet,600);text(s,x+52,y+12,22,i===active?C.white:C.muted,500);});
}
function replay(u){
 const n=u<3?0:u<6.5?1:2;topTitle('HISTORICAL REPLAY','Same transaction. Same state. Two builds.');
 const nodes=[['01','Pin the evidence','Transaction, accounts,\nprogram bytes, runtime'],['02','Execute twice','Run the baseline and\ncandidate from identical state'],['03','Compare outcomes','Logs, CPIs, account state\nand economic measurements']];
 nodes.forEach(([num,title,body],i)=>{let x=64+i*450;reveal(u,i*.16,()=>{box(x,328,410,254,i===n?'#432064':'#181020',18,i===n?'#b891ff':'#b598ef3b');text(num,x+26,352,25,C.violet,600);lines(title,x+26,402,33,356);lines(body,x+26,469,23,356,C.lav,400,1.3);if(i<2)text('→',x+419,435,28,C.violet);});});
 reveal(u,.5,()=>screenshot('orca',1450,325,405,590,{label:'Actual Orca report',crop:[140,140,1870,1070],py:0,fit:'contain'}));
 if(n===0){text('Hash checks bind every input to the saved evidence.',68,650,35,C.lav,500);text('Legacy and native v0 share the execution route.',68,713,27,C.muted);}
 if(n===1){box(66,630,1287,222,'#0c0912',16,'#9c68ff42');text('$ eplyx ci check --bundle … --candidate …',94,655,26,C.violet,400,'monospace');text('Replay/proof: matched',94,709,32,C.green,500,'monospace');text('Fresh offline Orca execution · checkpoint proof contract 2',94,768,24,C.muted);}
 if(n===2){text('Checkpoint replay can reconstruct a causal sequence.',68,638,30,C.lav);['73','428','431','438','1245'].forEach((id,i)=>{const x=70+i*242;box(x,712,194,65,'#3f2067',12,'#9c68ff88');text(id,x+97,729,27,C.lav,500,'DM','center');if(i<4)text('→',x+207,728,25,C.violet);});text('Derived boundary stays explicit; it is not labeled an observed transaction boundary.',68,815,24,C.muted);}
 stageRail(['Verify inputs','Replay baseline + candidate','Inspect the differences'],n);
}
function protocols(u){
 topTitle('PROTOCOL ANALYSIS','Turn execution differences into named quantities.');
 const n=u<3?0:1;
 screenshot('orca-impact',64,329,879,447,{label:'Actual Orca SwapV2 impact component',fit:'contain',active:n===0});
 screenshot('drift-impact',977,329,879,447,{label:'Actual Drift settlePnl impact component',fit:'contain',active:n===1});
 tag('SwapV2 → token flows',80,823,{size:24});tag('settlePnl → PnL settled',995,823,{size:24});
 caption(n===0?'Orca measures user input and vault flows from account-state changes.':'Drift evaluates the bounded settlement. Unsupported quantities stay unevaluated.');
 text('Shared semantic / onboarding contracts · Phoenix: envelope qualified; full replay still unqualified.',67,906,18,C.muted);
}
function upgrade(u){
 nativeTitle('Open the run. See what the candidate changed.');const n=u<3.8?0:u<6.5?1:2;
 footage('upgrade',64,253,1310,737,{label:'ACTUAL HOSTED UPGRADE · CONTROLLED STAKE POOL FIXTURE'});
 const info=[['Open saved evidence','The workspace loads the completed upgrade report.','The durable run keeps its inputs and results.'],['Check the identity','ChangeSpec binds the target and the exact candidate hash.','This report is about those precise program bytes.'],['Read the impact','This candidate makes tested historical interactions fail.','Execution, economic coverage and unexplained changes remain separate.']][n];
 guide([['Open the run','Load its retained inputs and result.'],['Check identity','ChangeSpec binds the exact candidate bytes.'],['Inspect impact','This candidate breaks tested historical interactions.']],n,'Execution, economics and unexplained changes stay separate.');
}
function governance(u){
 topTitle('SQUADS V4','Does the proposal contain the code you analysed?');const n=u<4?0:1;
 const labels=['Analysed candidate','Squads proposal','Buffer bytes','Executed deployment'];
 labels.forEach((label,i)=>{let x=65+i*455;box(x,320,420,94,i===3?'#21142d':'#371955',14,i===3?'#7b5f913f':'#bc94ff65');text(label,x+24,352,27,C.lav,500);if(i<3)text('→',x+428,352,26,C.violet);});
 screenshot('governance',65,466,1790,271,{label:'ARCHIVED MAINNET EVIDENCE · PRODUCTION GOVERNANCE COMPONENT',crop:[110,62,1940,325],bar:false});
 lines(n===0?'G1 compares the proposal and buffer to the analysed ChangeSpec.':'G2 searches for execution, then checks attributable deployed bytes.',67,789,37,1740,C.white);
 caption(n===0?'Three real mainnet proposals matched at their observed slots.':'This saved proposal is still active: no deployment match is claimed.');
}
function migration(u){
 topTitle('TOKEN MIGRATION V1','Run the rehearsal. Open the recorded result.');
 terminal(64,337,607,539,u,{compact:false});
 footage('dashboard',703,337,1153,648,{label:'ACTUAL LOCAL DASHBOARD LOAD + NAVIGATION · DEFECT FIXTURE'});
 const phase=u<3?0:u<6?1:2;
 const notes=['The CLI executes the candidate locally over the declared state.','The dashboard loads saved runs; opening it performs no new analysis.','Technical mode reveals exact quantities and hashes without changing the result.'];
 caption(notes[phase]);
}
function stress(u){
 nativeTitle('Find the failing case, then read why it blocks.');const n=u<4.3?0:1;
 text('Bounded search preserves account identity, authority checks and state-drift evidence.',65,215,22,C.muted);
 footage('stress',64,253,1310,737,{label:'ACTUAL STRESS MATRIX → DEPLOYMENT GATE · SYNTHETIC FIXTURE'});
 guide([['Inspect the case','At the deadline, rejection was required.'],['Follow the failure','The defect candidate succeeds instead.'],['Read the gate','The recorded rule violation blocks release.']],n===0?1:2,'19 / 20 cases behaved as specified. This fixture exposes the exception.');
}
function witness(u){
 nativeTitle('A failure becomes a witness you can replay.');const n=u<3.2?0:1;
 footage('witness',64,253,1310,737,{label:'ACTUAL COUNTEREXAMPLE → COPY REPRODUCE COMMAND'});
 guide([['Inspect','Expected rejection; recorded success.'],['Copy the command','Use the exact saved witness ID.'],['Replay offline','Append a new reproduction to history.']],n===0?0:1,'The witness keeps its provenance. This example is derived from a fixture.');
}
function compare(u){
 nativeTitle('Select two runs. Inspect what actually changed.');const n=u<3.4?0:1;
 footage('compare',64,253,1310,737,{label:'ACTUAL HISTORY SELECTION → COMPARISON'});
 guide([['Select two runs','Choose reference and candidate.'],['Compare','Inspect funding, bytes and readiness.'],['Check the scope','Search domains must be comparable.']],n===0?0:2,'A missing finding alone does not prove the defect was fixed.');
}
function observe(u){
 nativeTitle('Start with a public token and owner address.');const n=u<3?0:u<7.1?1:2;
 footage('observe',64,253,1310,737,{label:'ACTUAL FORM LOAD → INPUT → OBSERVATION · SYNTHETIC PROVIDER'});
 const infos=[['Watch it load','The authenticated workspace loads the current-state form.','No wallet connection or private key is required.'],['Choose the scope','Enter the mint, select a public owner, then click Observe.','Eplyx acquires a bounded read-only observation.'],['Read the saved state','The observed accounts load into the page and offline analysis is queued.','An observation alone does not prove a transfer or migration can execute.']];
 guide([['Choose inputs','Mint + public owner. No wallet connection.'],['Capture','Read accounts; retain the observation.'],['Inspect','State loads. Offline analysis is queued.']],n,'Observation is not proof that a transfer or migration can execute.');
}
function path(u){
 nativeTitle('Choose one exact path and execute it locally.');const n=u<3.7?0:u<5.8?1:2;
 text('Bounded paths: Transfer · Meteora market exit · PositionV2 principal withdrawal',65,215,22,C.muted);
 footage('path',64,253,1310,737,{label:'ACTUAL TRANSFER CHECK · QUEUED WAIT CONDENSED · SYNTHETIC PROVIDER'});
 const infos=[['Set the transaction','Choose Transfer, the exact amount and recipient token account.','Only this source, mint, amount and destination are checked.'],['Capture, then execute','Eplyx captures the bounded final state and queues offline execution.','The page shows the real queued job while it runs.'],['Read the scope','The result records local execution and that no funds moved.','Holder signing possession remains “Cannot be judged.”']];
 guide([['Define the path','Transfer + exact amount + recipient.'],['Run locally','Capture final state. Queue execution.'],['Read the result','Execution performed. Funds moved: No.']],n,'Signing possession still cannot be judged. Only this path was checked.');
}
function candidate(u){
 nativeTitle('Test proposed migration terms against one account.');const n=u<3?0:u<5.4?1:2;
 footage('candidate',64,253,1310,737,{label:'ACTUAL CANDIDATE CHECK · QUEUED WAIT CONDENSED · SYNTHETIC PROVIDER'});
 const infos=[['Declare the terms','Set the replacement mint and amount; ratio, fees and reserve are explicit.','The reserve is a proposed local overlay.'],['Keep the input fixed','The service retains immutable inputs and queues the candidate check.','The tested account is not swapped out to obtain success.'],['Interpret the result','This exact candidate check passes for its tested account and amount.','That is not population readiness or an official transition.']];
 guide([['Declare terms','Replacement mint, amount, ratio and reserve.'],['Execute','Retain inputs; run the candidate locally.'],['Interpret','The exact account check passed.']],n,'This does not prove population readiness or an official transition.');
}
function lifecycle(u){
 nativeTitle('Add dates and policy. Evaluate separate assurances.');const n=u<3.2?0:u<6.3?1:2;
 footage('lifecycle',64,253,1310,737,{label:'ACTUAL SCENARIO EVALUATION · HYPOTHETICAL POLICY · WAIT CONDENSED'});
 const infos=[['Declare the scenario','Bind a saved candidate plan, successor, effective date and deadline.','Proposed policy stays distinct from observed state.'],['Evaluate offline','The lifecycle job reads the retained evidence and declared terms.','Transfer, market exit and principal withdrawal keep separate proof.'],['Read each answer','Prepared, Ready, Incomplete and Not evaluated answer different questions.','A ready candidate does not establish readiness for the whole population.']];
 guide([['Declare policy','Saved plan + effective date + deadline.'],['Evaluate','Keep proposed policy and observed state distinct.'],['Read separately','Prepared ≠ ready for every holder.']],n,'Each path keeps its own proof. Incomplete and unevaluated remain visible.');
}
function mosaic(u){
 topTitle('FOUR CONNECTED VIEWS OF THE WORKFLOW','Execute locally. Inspect evidence. Share the result.');
 const xs=[64,979],ys=[327,677],w=877,h=284,active=Math.min(3,Math.floor(u/2));
 [[xs[0],ys[0]],[xs[1],ys[0]],[xs[0],ys[1]],[xs[1],ys[1]]].forEach(([x,y],i)=>{if(active===i){ctx.save();ctx.shadowColor=C.violet;ctx.shadowBlur=22;box(x-3,y-3,w+6,h+39,'#6d31f2',17);ctx.restore();}});
 terminal(xs[0],ys[0],w,h+34,u,{compact:true});
 footage('dashboard',xs[1],ys[0],w,h,{label:'02 / LOCAL DASHBOARD',crop:[200,30,1400,650]});
 footage('upgrade',xs[0],ys[1],w,h,{label:'03 / HOSTED WORKSPACE',crop:[210,30,1390,650]});
 footage('lifecycle',xs[1],ys[1],w,h,{label:'04 / LIFECYCLE & CURRENT PATHS',crop:[210,30,1390,650]});
 const words=['Analyse → search → gate → unsigned plan','Inspect runs → stress → copy a reproduction','Device-code login → project access → exact-byte sync','Observe → check one path → evaluate a scenario'];
 text(words[active],1856,267,23,C.lav,500,'DM','right');
}
function render(t){
 t=clamp(t,0,DURATION-.001);const s=scenes.find(s=>t>=s.at&&t<s.end),u=t-s.at;ctx.globalAlpha=1;ctx.setTransform(1,0,0,1,0,0);background(t);
 if(s.kind!=='hero'&&s.kind!=='end')brand(t,s.chapter);
 if(s.kind==='hero'){
  footage('hero',0,0,W,H,{bar:false});
  const g=ctx.createLinearGradient(0,865,0,1080);g.addColorStop(0,'#0b071200');g.addColorStop(1,'#0b0712d9');ctx.fillStyle=g;ctx.fillRect(0,865,W,215);
  text('THE ACTUAL EPLYX INTRO',64,1004,22,C.lav,500);text('21–27 SEPTEMBER 2026',1856,1004,22,C.lav,500,'DM','right');
 }else if(s.kind==='hook'){
  reveal(u,0,()=>{lines('What happens\nwhen you change\nan onchain system?',68,217,98,1710);});
  ['REPLAY HISTORY','COMPARE THE UPGRADE','REHEARSE THE TRANSITION'].forEach((v,i)=>reveal(u,.25+i*.15,()=>tag(v,70+i*589,797,{size:23})));
 }else if(s.kind==='end'){
  glow(1000,470,860,.45);ctx.drawImage(imgs.logo,896,173,128,128);text('Eplyx',960,336,43,C.white,600,'Manrope','center');text('Know what changes.',960,463,91,C.white,500,'Manrope','center');text('Before you deploy.',960,580,91,C.lav,500,'Manrope','center');text('Replay · Governance · Migration · Lifecycle',960,804,29,C.muted,500,'DM','center');
 }else ({replay,protocols,upgrade,governance,migration,stress,witness,compare,observe,path,candidate,lifecycle,mosaic}[s.kind])(u);
 if(u<.22&&s.at>0){ctx.globalAlpha=(1-u/.22)*.3;ctx.fillStyle=C.violet;ctx.fillRect(0,0,W,H);ctx.globalAlpha=1;}
 if(t>127.5){ctx.fillStyle=`rgba(11,7,18,${(t-127.5)*2})`;ctx.fillRect(0,0,W,H);}return true;
}
window.renderFrame=render;
window.filmReady=(async()=>{
 for(const name of ['orca','orca-impact','drift-impact','governance']){const im=new Image();im.src='assets/'+name+'.png';await im.decode();imgs[name]=im;}
 imgs.logo=new Image();imgs.logo.src='assets/logo.svg';await imgs.logo.decode();
 for(const name of names){const v=document.createElement('video');v.muted=true;v.playsInline=true;v.preload='auto';v.src='assets/motion/'+name+'.mp4';v.style.display='none';document.body.append(v);await new Promise((res,rej)=>{v.addEventListener('loadeddata',res,{once:true});v.addEventListener('error',()=>rej(Error('Cannot load '+name)),{once:true});});videos[name]=v;}
 await document.fonts.load('500 40px Manrope');await document.fonts.load('400 30px DM');await document.fonts.ready;await prepareFrame(0);render(0);
})();
let playing=false,position=0,start=0,busy=false;const music=document.querySelector('#music'),play=document.querySelector('#play'),seek=document.querySelector('#seek');
const formatTime=t=>Math.floor(t/60)+':'+String(Math.floor(t%60)).padStart(2,'0');
async function tick(now){if(!playing)return;position=Math.min(DURATION,(now-start)/1000);if(!busy){busy=true;await prepareFrame(position);render(position);busy=false;seek.value=position;document.querySelector('#time').textContent=formatTime(position)+' / 2:08';}if(position>=DURATION){playing=false;music.pause();play.textContent='Replay';}else requestAnimationFrame(tick);}
play.onclick=async()=>{await filmReady;if(playing){playing=false;music.pause();play.textContent='Play';}else{if(position>=DURATION)position=0;start=performance.now()-position*1000;playing=true;music.currentTime=position;music.play().catch(()=>{});play.textContent='Pause';requestAnimationFrame(tick);}};
seek.oninput=async()=>{position=+seek.value;await prepareFrame(position);render(position);music.currentTime=position;start=performance.now()-position*1000;document.querySelector('#time').textContent=formatTime(position)+' / 2:08';};
if(new URLSearchParams(location.search).has('export'))document.body.classList.add('export');
