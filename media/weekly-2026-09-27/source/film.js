/* Deterministic 1920×1080 motion composition. Product pixels are screenshots,
   not redesigned product UI. All explanatory graphics are editorial overlays. */
const canvas=document.querySelector('#film'),ctx=canvas.getContext('2d',{alpha:false});
const W=1920,H=1080,DURATION=140;
const C={ink:'#0b0712',panel:'#171020',purple:'#6d31f2',violet:'#9c68ff',lav:'#e7ddff',white:'#f7f5fa',muted:'#b1a4c4',green:'#87dcbd',red:'#fa909e'};
const scenes=[
 {at:0,end:8,kind:'hero',chapter:'THE WEEK IN EPLYX'},
 {at:8,end:14,kind:'agenda',chapter:'35 COMMITS · 21–27 SEPTEMBER 2026'},
 {at:14,end:26,kind:'replay',chapter:'01 / HISTORICAL REPLAY'},
 {at:26,end:36,kind:'protocols',chapter:'01 / PROTOCOL ANALYSIS'},
 {at:36,end:48,kind:'upgrade',chapter:'02 / UPGRADE ANALYSIS'},
 {at:48,end:60,kind:'governance',chapter:'02 / GOVERNANCE'},
 {at:60,end:66,kind:'transition',chapter:'03 / TOKEN MIGRATION & LIFECYCLE'},
 {at:66,end:78,kind:'migration',chapter:'03 / REHEARSE THE PROPOSAL'},
 {at:78,end:88,kind:'stress',chapter:'03 / FIND THE BOUNDARY'},
 {at:88,end:98,kind:'counterexample',chapter:'03 / REPRODUCE THE FAILURE'},
 {at:98,end:108,kind:'lifecycle',chapter:'03 / LIFECYCLE & CURRENT PATHS'},
 {at:108,end:120,kind:'current',chapter:'03 / THE HOSTED WORKFLOW'},
 {at:120,end:134,kind:'mosaic',chapter:'ONE PROGRAM. MULTIPLE WAYS TO WORK.'},
 {at:134,end:140,kind:'end',chapter:'EPLYX / SEPTEMBER 2026'}
];
window.filmScenes=scenes;window.filmDuration=DURATION;
const imageNames=['hero','transitions','dashboard','migration','migration-stress','migration-gate','counterexample','comparison','lifecycle','lifecycle-policy','observation','orca','orca-detail','orca-impact','drift','drift-detail','drift-impact','governance','hosted','upgrade','upgrade-impact','current-form','current-input','current-observed','path-result','candidate-result','scenario-result'];
const imgs={};
const clamp=(v,a=0,b=1)=>Math.max(a,Math.min(b,v));
const ease=v=>{v=clamp(v);return 1-Math.pow(1-v,3)};
const smooth=v=>{v=clamp(v);return v*v*(3-2*v)};
const mix=(a,b,p)=>a+(b-a)*p;
function round(x,y,w,h,r=18){ctx.beginPath();ctx.roundRect(x,y,w,h,r);}
function box(x,y,w,h,fill=C.panel,r=18,stroke){round(x,y,w,h,r);ctx.fillStyle=fill;ctx.fill();if(stroke){ctx.lineWidth=1.5;ctx.strokeStyle=stroke;ctx.stroke();}}
function text(s,x,y,size=30,color=C.white,weight=400,font='DM',align='left'){
 ctx.font=`${weight} ${size}px ${font}`;ctx.fillStyle=color;ctx.textAlign=align;ctx.textBaseline='top';ctx.fillText(s,x,y);ctx.textAlign='left';
}
function lines(s,x,y,size=70,width=600,color=C.white,weight=500,leading=1.13){
 ctx.font=`${weight} ${size}px Manrope`;let yy=y;
 for(const paragraph of s.split('\n')){let line='';for(const word of paragraph.split(' ')){const next=line?line+' '+word:word;if(ctx.measureText(next).width>width&&line){text(line,x,yy,size,color,weight,'Manrope');yy+=size*leading;line=word;}else line=next;}text(line,x,yy,size,color,weight,'Manrope');yy+=size*leading;}
 return yy;
}
function rule(x,y,w,color='#e7ddff22'){ctx.fillStyle=color;ctx.fillRect(x,y,w,1);}
function tag(s,x,y,{color=C.lav,fill='#9c68ff1c',size=21,pad=15}={}){
 ctx.font=`500 ${size}px DM`;const w=ctx.measureText(s).width+pad*2;box(x,y,w,size+22,fill,50,'#ad89fb38');text(s,x+pad,y+10,size,color,500);return w;
}
function glow(x,y,r,alpha=.18){const g=ctx.createRadialGradient(x,y,0,x,y,r);g.addColorStop(0,`rgba(137,63,255,${alpha})`);g.addColorStop(1,'rgba(109,49,242,0)');ctx.fillStyle=g;ctx.fillRect(x-r,y-r,r*2,r*2);}
function background(t){
 ctx.fillStyle=C.ink;ctx.fillRect(0,0,W,H);glow(1440+Math.sin(t*.12)*120,360,900,.25);glow(130,1050,660,.14);
 ctx.strokeStyle='#a687dd0b';ctx.lineWidth=1;for(let x=80;x<W;x+=100){ctx.beginPath();ctx.moveTo(x,0);ctx.lineTo(x,H);ctx.stroke();}for(let y=80;y<H;y+=100){ctx.beginPath();ctx.moveTo(0,y);ctx.lineTo(W,y);ctx.stroke();}
 for(let i=0;i<35;i++){const x=(i*307.2+30)%W,y=(i*137.5+t*(i%3+1)*2)%H;ctx.fillStyle=`rgba(211,190,255,${.10+.1*Math.sin(i+t*.4)})`;ctx.beginPath();ctx.arc(x,y,i%4===0?1.8:1,0,Math.PI*2);ctx.fill();}
}
function brand(t,chapter){
 ctx.drawImage(imgs.logo,66,40,40,40);text('Eplyx',119,40,30,C.white,600,'Manrope');
 text(chapter,1850,51,18,C.muted,500,'DM','right');
 rule(64,102,1792);ctx.fillStyle='#9c68ff';ctx.fillRect(64,102,1792*t/DURATION,2);
 text('21—27 SEP 2026',65,1027,17,C.muted,500);text('t-token-migration',1855,1027,17,C.muted,400,'DM','right');
}
function reveal(u,delay,fn){let p=ease((u-delay)/.75);if(!p)return;ctx.save();ctx.globalAlpha=p;ctx.translate(0,24*(1-p));fn();ctx.restore();}
function imageIn(name,x,y,w,h,{zoom=1,px=.5,py=.5,crop,fit='cover'}={}){
 const im=imgs[name];let [sx,sy,sw,sh]=crop??[0,0,im.width,im.height];
 if(fit==='contain'){const scale=Math.min(w/sw,h/sh);ctx.drawImage(im,sx,sy,sw,sh,x+(w-sw*scale)/2,y+(h-sh*scale)/2,sw*scale,sh*scale);return;}
 const scale=Math.max(w/sw,h/sh)*zoom,nw=w/scale,nh=h/scale;
 sx+=(sw-nw)*px;sy+=(sh-nh)*py;
 ctx.drawImage(im,sx,sy,nw,nh,x,y,w,h);
}
function screenshot(name,x,y,w,h,{label='Eplyx',zoom=1,px=.5,py=.5,crop,shadow=true,bar=true,active=false,fit='cover'}={}){
 ctx.save();if(shadow){ctx.shadowColor='#0000008c';ctx.shadowBlur=45;ctx.shadowOffsetY=20;box(x,y,w,h+ (bar?40:0),'#130c20',16);}ctx.shadowColor='transparent';
 const bh=bar?40:0;
 box(x,y,w,h+bh,'#171020',16,active?'#b48eff':'#b594f43a');
 ctx.save();round(x+1,y+bh,w-2,h-1,bar?[0,0,15,15]:15);ctx.clip();imageIn(name,x,y+bh,w,h,{zoom,px,py,crop,fit});ctx.restore();
 if(bar){for(let i=0;i<3;i++){ctx.fillStyle=['#79568d','#735388','#6b4d82'][i];ctx.beginPath();ctx.arc(x+21+i*16,y+20,4,0,7);ctx.fill();}text(label,x+86,y+10,16,C.muted);}
 ctx.restore();
}
function cursor(x,y,t){ctx.save();ctx.translate(x,y);ctx.fillStyle=C.white;ctx.strokeStyle='#160c24';ctx.lineWidth=2;ctx.beginPath();ctx.moveTo(0,0);ctx.lineTo(0,28);ctx.lineTo(8,20);ctx.lineTo(15,34);ctx.lineTo(21,30);ctx.lineTo(14,17);ctx.lineTo(26,17);ctx.closePath();ctx.fill();ctx.stroke();const p=(t%2)/2;if(p<.55){ctx.strokeStyle=`rgba(194,162,255,${1-p/.55})`;ctx.lineWidth=3;ctx.beginPath();ctx.arc(3,9,10+p*60,0,7);ctx.stroke();}ctx.restore();}
function caption(s,note=''){box(64,941,1792,63,'#0b0712e8',12,'#c6adf91b');text(s,88,958,27,C.white);if(note)text(note,1840,1027,16,C.muted,400,'DM','right');}
function smallPoints(list,x,y,u,{size=25,gap=58}={}){list.forEach((s,i)=>reveal(u,.7+i*.25,()=>{text('↗',x,y+i*gap,size,C.violet);text(s,x+36,y+i*gap,size,C.lav);}));}
function stat(n,label,x,y){text(n,x,y,64,C.lav,500,'Manrope');text(label,x,y+80,23,C.muted);}
function terminal(x,y,w,h,u,{compact=false}={}){
 box(x,y,w,h,'#0c0912',17,'#9c68ff55');text('CLI / RECORDED EXCERPT · REFERENCE FIXTURE',x+25,y+21,compact?15:18,C.muted,500);rule(x+24,y+59,w-48);
 const r=window.RECORDED.cli.find(r=>r.command==='eplyx migration analyse');
 const output=r.stdout.split('\n').filter(Boolean);let rows=['$ eplyx migration analyse','',...output.slice(1,7)];
 const font=compact?18:23,lh=compact?25:34;
 const show=Math.min(rows.length,Math.floor(u*3)+1);
 for(let i=0;i<show;i++){
  let s=rows[i];if(s.startsWith('Eplyx migration analysis'))s='Eplyx migration analysis · '+s.split(' · ')[1].slice(0,23)+'…';
  const maxChars=Math.floor((w-50)/(font*.6));
  if(s.length>maxChars)s=s.slice(0,maxChars-1)+'…';
  text(s,x+25,y+81+i*lh,font,i===0?C.violet:s.includes('Incomplete')?'#e5ba84':s.includes('Ready')?C.green:C.lav,400,'monospace');
 }
 if(u>3.8&&!compact){text('$ eplyx migration gate --run "$RUN" --policy strict',x+25,y+h-112,22,C.violet,400,'monospace');text('Failed · population: Incomplete',x+25,y+h-72,24,C.red,400,'monospace');}
}
function hero(s,u,t){
 const p=ease(u/1.2);ctx.save();ctx.globalAlpha=p;imageIn('hero',0,0,W,H,{zoom:1.0+.018*u/8,px:.48,py:0});ctx.restore();
 const g=ctx.createLinearGradient(0,760,0,H);g.addColorStop(0,'#0b071200');g.addColorStop(1,'#0b0712ee');ctx.fillStyle=g;ctx.fillRect(0,760,W,320);
 reveal(u,.5,()=>{tag('THE WEEK IN EPLYX',72,917,{size:22});text('21–27 September 2026',1850,931,25,C.lav,400,'DM','right');});
 ctx.fillStyle=C.violet;ctx.fillRect(0,H-4,W*clamp(u/8),4);
}
function agenda(s,u,t){
 reveal(u,0,()=>{text('BUILT THIS WEEK',72,160,24,C.violet,600);lines('More ways to understand\nonchain change.',66,217,92,1500);});
 const list=[['01','Replay & protocols','Historical execution → economic meaning'],['02','Upgrades & governance','Candidate bytes → proposal identity'],['03','Migration & lifecycle','Rehearse → inspect → reproduce']];
 list.forEach(([n,title,body],i)=>reveal(u,.5+i*.25,()=>{let x=68+i*600;box(x,520,566,297,'#1b1029',18,'#9c68ff44');text(n,x+28,546,25,C.violet);lines(title,x+28,611,43,480);text(body,x+28,741,23,C.muted);}));
 caption('35 commits. Three connected areas of the product.');
}
function replay(s,u,t){
 reveal(u,0,()=>{text('HISTORICAL REPLAY',70,163,23,C.violet,600);lines('Replay the\nactual history.',66,214,73,565);});
 smallPoints(['Legacy + native v0','Pinned state, code & runtime','Logs, CPIs & full account state'],72,438,u,{size:26,gap:59});
 reveal(u,.35,()=>screenshot('orca',650+24*(1-ease(u)),160,1206,754,{label:'Native Eplyx report · fresh offline Orca replay',zoom:1+.025*smooth(u/12),px:.38,py:0}));
 reveal(u,2,()=>{text('CAUSAL CHECKPOINT REPLAY',72,680,19,C.muted,500);const nodes=['73','428','431','438','1245'];nodes.forEach((n,i)=>{let x=74+i*107,active=u>3+i*.55;box(x,723,84,57,active?'#5b2aab':'#1d122d',10,active?'#ba93ff':'#4b355d');text(n,x+42,739,22,C.lav,500,'DM','center');if(i<4)text('→',x+89,738,20,C.violet);});text('Five transactions · derived boundary',72,809,22,C.muted);});
 caption('Verify the evidence, then execute offline. The proof keeps its boundary explicit.');
}
function protocols(s,u,t){
 reveal(u,0,()=>{text('PROTOCOL ANALYSIS',70,151,23,C.violet,600);lines('Execution becomes economic evidence.',67,197,64,1700);});
 reveal(u,.3,()=>{screenshot('orca-impact',67,340,878,445,{label:'Orca / SwapV2 · recorded engine report',fit:'contain'});tag('Token flows',90,831,{size:23});});
 reveal(u,.6,()=>{screenshot('drift-impact',973,340,878,445,{label:'Drift / settlePnl · archived engine report',fit:'contain'});tag('PnL settlement',996,831,{size:23});});
 caption(u<5?'Bounded Orca and Drift adapters measure the outcomes they can support.':'Shared semantic evaluation and onboarding contracts keep coverage explicit.');
 text('Phoenix: legacy envelope qualified; full historical replay remains unqualified.',73,909,19,C.muted);
}
function upgrade(s,u,t){
 reveal(u,0,()=>{text('UPGRADE ANALYSIS',70,161,23,C.violet,600);lines('One change.\nExact bytes.\nVisible impact.',67,211,70,546);});
 smallPoints(['ChangeSpec identity','Immutable candidate storage','Durable hosted runs'],72,526,u,{size:26,gap:59});
 reveal(u,.25,()=>screenshot(u<6?'upgrade':'upgrade-impact',625,158,1228,765,{label:'Actual hosted upgrade job · controlled Stake Pool fixture',zoom:1+.025*smooth(u/12),px:.3,py:0}));
 reveal(u,1,()=>tag('Execution  /  Economics  /  Unexplained',70,799,{size:19}));
 caption(u<6?'The candidate under test is bound to the proposed change.':'The impact view separates execution, economic coverage, and unexplained changes.');
}
function governance(s,u,t){
 reveal(u,0,()=>{text('SQUADS V4',70,155,23,C.violet,600);lines('Connect the analysis\nto the proposal.',67,205,80,1250);});
 reveal(u,.4,()=>{screenshot('governance',68,440,1783,265,{label:'Production governance component · archived mainnet evidence',crop:[110,62,1940,325],bar:false});});
 const steps=[['01','ChangeSpec'],['02','Squads proposal'],['03','Candidate buffer'],['04','Deployment attestation']];
 steps.forEach(([n,title],i)=>reveal(u,1+i*.2,()=>{const x=72+i*446;rule(x,788,400,i===3?'#a68cbd66':'#9c68ff');text(n,x,815,20,C.violet,600);text(title,x+47,811,26,C.lav);if(i<3)text('→',x+397,808,27,C.violet);}));
 caption(u<6?'Three real mainnet proposals matched at their observed slots.':'G2 checks execution and deployed bytes. This saved proposal is not executed yet.');
}
function transition(s,u,t){
 reveal(u,.25,()=>screenshot('transitions',1130,246,722,650,{label:'Actual public transition page',crop:[1110,205,1020,1050],py:0}));
 reveal(u,0,()=>{text('TOKEN MIGRATION · LIFECYCLE · PRODUCT FLOW',74,247,24,C.violet,600);lines('From a proposal\nto a reproducible\ndecision.',69,321,98,1040);});
 caption('Run locally. Inspect the result. Continue in a shared workspace.');
}
function migration(s,u,t){
 reveal(u,0,()=>{text('TOKEN MIGRATION V1',70,153,23,C.violet,600);lines('Rehearse before\nyou commit.',67,208,74,1300);});
 if(u<5){reveal(u,.3,()=>terminal(70,403,840,496,u));reveal(u,.55,()=>screenshot('dashboard',943,402,908,492,{label:'Local dashboard · deficient-reserve fixture',zoom:1.025,py:0}));}
 else{const q=ease((u-5)/.8);ctx.save();ctx.globalAlpha=q;screenshot('migration',70,402,1210,510,{label:'Actual migration run · deficient-reserve fixture',crop:[270,70,1890,805],py:0});ctx.restore();smallPoints(['Proposal & population','Sequential rehearsal','Invariants & gate','Evidence & replay'],1330,438,u-4,{size:26,gap:74});}
 caption(u<5?'A real CLI run: the mechanism is ready, while population evidence remains incomplete.':'Eight questions connect the proposed migration to its recorded evidence.');
}
function stress(s,u,t){
 reveal(u,0,()=>{text('STRESS & SEARCH',70,153,23,C.violet,600);lines('Make the edge\ncases visible.',67,208,75,557);});
 reveal(u,.4,()=>screenshot('migration-stress',657,155,1199,752,{label:'Actual stress results · derived synthetic cases',zoom:1.05,px:.65,py:0}));
 reveal(u,1,()=>{stat('19 / 20','cases behaved as specified',73,493);text('Deadline defect exposed',73,624,27,C.red,500);});
 smallPoints(['Bounded counterexample search','Authority & state-drift checks'],72,735,u,{size:23,gap:58});
 caption('Stress cases, observed search, and invariants each answer a different question.');
}
function counterexample(s,u,t){
 reveal(u,0,()=>{text('REPRODUCIBLE COUNTEREXAMPLES',70,153,23,C.violet,600);lines('Keep the failure.\nReplay the evidence.',67,208,73,1700);});
 reveal(u,.3,()=>screenshot(u<5?'counterexample':'comparison',70,403,1180,510,{label:u<5?'Saved counterexample · real reproduction history':'Actual run comparison · search equivalence stays visible',zoom:1.02,py:0}));
 reveal(u,.7,()=>{text('LOCAL HISTORY',1310,424,20,C.violet,600);lines('Saved bytes.\nImmutable runs.\nUnsigned plans.',1308,473,43,500);tag('Offline replay',1309,723,{size:24});});
 caption(u<5?'A saved witness can be reproduced without replacing the original record.':'A missing finding is only a fix when the comparison and exact retest support it.');
}
function lifecycle(s,u,t){
 reveal(u,0,()=>{text('LIFECYCLE & CURRENT PATHS',70,155,23,C.violet,600);lines('Policy, readiness\nand paths stay separate.',67,207,72,1740);});
 reveal(u,.3,()=>screenshot('lifecycle',69,413,1166,490,{label:'Actual lifecycle page · hypothetical declaration',crop:[250,50,1900,920],py:0}));
 smallPoints(['Declared policy & deadlines','Observed state & readiness','Transfer','Meteora market exit','PositionV2 principal withdrawal'],1288,414,u,{size:26,gap:83});
 caption('A successful check of one path does not establish success for another.');
}
function current(s,u,t){
 let n=u<3?0:u<6?1:u<9?2:3;
 const names=['current-input','current-observed','candidate-result','scenario-result'];
 const labels=['Observe current state','Inspect focused paths','Check a candidate','Evaluate a scenario'];
 reveal(u,0,()=>{text('ACTUAL HOSTED BROWSER FLOW',70,153,23,C.violet,600);lines('Inspect. Check. Evaluate.',67,205,76,1730);});
 screenshot(names[n],69,359,1280,558,{label:'Real browser operation · synthetic local provider',zoom:1.02,py:0});
 labels.forEach((l,i)=>{const y=392+i*111;box(1400,y,450,81,i===n?'#6330b1':'#1a1027',14,i===n?'#bc94ff':'#6b498344');text('0'+(i+1),1420,y+25,23,i===n?C.white:C.violet);text(l,1480,y+25,25,C.lav);});
 if(n===0)cursor(1230,825,u);
 caption(['Enter a token and wallet to create a bounded current-state observation.','The browser keeps each path and its execution status visible.','Run an exact candidate check and retain the recorded result.','Review the proposed lifecycle scenario with separate assurance results.'][n]);
}
function mosaic(s,u,t){
 reveal(u,0,()=>{text('ONE CONNECTED WORKFLOW',70,144,22,C.violet,600);lines('Your terminal. Your dashboard. Your workspace.',67,185,53,1810);});
 const gap=26,tw=878,th=302,left=68,right=972,top=313,bottom=667;
 let active=Math.min(3,Math.floor(u/3));
 const a=[left,top,tw,th],b=[right,top,tw,th],c=[left,bottom,tw,th],d=[right,bottom,tw,th];
 const positions=[a,b,c,d];
 positions.forEach(([x,y,w,h],i)=>{if(active===i){ctx.save();ctx.shadowColor='#9c68ff';ctx.shadowBlur=22;box(x-3,y-3,w+6,h+6,'#4c246b',15);ctx.restore();}});
 terminal(...a,u,{compact:true});
 screenshot(u>5?'counterexample':'dashboard',...b,{label:'02 / LOCAL DASHBOARD',crop:[225,40,1900,750],bar:true,shadow:false,py:0});
 screenshot('hosted',...c,{label:'03 / HOSTED WORKSPACE',crop:[220,30,1930,750],bar:true,shadow:false,py:0});
 screenshot(u>8?'scenario-result':'current-observed',...d,{label:'04 / CURRENT STATE & LIFECYCLE',crop:[260,20,1850,755],bar:true,shadow:false,py:0});
 tag('01 / CLI · OFFLINE EXECUTION',left+18,top-52,{size:18});
 const notes=['Analyse → search → gate → export an unsigned plan','Browse saved runs → inspect stress → reproduce a witness','Device-code login → project access → exact-byte sync','Observe → check one path → evaluate the declared scenario'];
 text(notes[active],1850,263,24,C.lav,500,'DM','right');
 // A moving pulse follows the four panes; no invented application state.
 const centers=[[945,465],[957,648],[950,823],[957,648]],p=(u%3)/3;
 ctx.strokeStyle='#ab78ff66';ctx.lineWidth=2;ctx.beginPath();ctx.moveTo(960,312);ctx.lineTo(960,1008);ctx.stroke();glow(960,330+650*p,80,.55);
}
function end(s,u,t){
 glow(1100,520,850,.35);ctx.save();ctx.strokeStyle='#a580e323';ctx.lineWidth=1.5;ctx.translate(960,540);ctx.rotate(-.2+u*.012);for(let i=0;i<3;i++){ctx.beginPath();ctx.ellipse(0,0,570+i*135,240+i*65,0,0,Math.PI*2);ctx.stroke();}ctx.restore();
 reveal(u,0,()=>{ctx.drawImage(imgs.logo,880,169,160,160);text('Eplyx',960,348,43,C.white,600,'Manrope','center');});
 reveal(u,.25,()=>{text('Know what changes.',960,463,91,C.white,500,'Manrope','center');text('Before you deploy.',960,579,91,C.lav,500,'Manrope','center');});
 reveal(u,.65,()=>{tag('REPLAY',651,780,{size:20});tag('GOVERNANCE',795,780,{size:20});tag('MIGRATION & LIFECYCLE',1006,780,{size:20});text('Built this week · 21–27 September 2026',960,893,27,C.muted,400,'DM','center');});
}
const painters={hero,agenda,replay,protocols,upgrade,governance,transition,migration,stress,counterexample,lifecycle,current,mosaic,end};
window.renderFrame=function(t){
 t=clamp(t,0,DURATION-.001);const s=scenes.find(s=>t>=s.at&&t<s.end);const u=t-s.at;
 ctx.globalAlpha=1;ctx.setTransform(1,0,0,1,0,0);background(t);if(s.kind!=='hero'&&s.kind!=='end')brand(t,s.chapter);painters[s.kind](s,u,t);
 // A restrained violet wipe separates chapters; continuous native-screen motion stays visible.
 if(u<.42&&s.at>0){let q=ease(u/.42);ctx.fillStyle='#6d31f2';ctx.globalAlpha=.55*(1-q);ctx.fillRect(W*q,0,W,H);ctx.globalAlpha=1;}
 if(t>139){ctx.fillStyle=`rgba(11,7,18,${smooth((t-139)/1)})`;ctx.fillRect(0,0,W,H);}
 return true;
};
window.filmReady=(async()=>{
 for(const name of imageNames){const im=new Image();im.src='assets/'+name+'.png';try{await im.decode();}catch(e){throw new Error('Cannot decode '+im.src+': '+e.message);}imgs[name]=im;}
 imgs.logo=new Image();imgs.logo.src='assets/logo.svg';await imgs.logo.decode();
 await document.fonts.load('500 40px Manrope');await document.fonts.load('400 30px DM');await document.fonts.ready;renderFrame(0);
})();
let playing=false,position=0,start=0;const music=document.querySelector('#music'),play=document.querySelector('#play'),seek=document.querySelector('#seek');
function clock(){return Math.floor(position/60)+':'+String(Math.floor(position%60)).padStart(2,'0');}
function tick(now){if(playing){position=Math.min(DURATION,(now-start)/1000);renderFrame(position);seek.value=position;document.querySelector('#time').textContent=clock()+' / 2:20';if(position>=DURATION){playing=false;play.textContent='Replay';music.pause();}else requestAnimationFrame(tick);}}
play.onclick=async()=>{await filmReady;if(playing){playing=false;music.pause();play.textContent='Play';}else{if(position>=DURATION)position=0;playing=true;start=performance.now()-position*1000;music.currentTime=position;music.play().catch(()=>{});play.textContent='Pause';requestAnimationFrame(tick);}};
seek.oninput=()=>{position=+seek.value;renderFrame(position);music.currentTime=position;start=performance.now()-position*1000;document.querySelector('#time').textContent=clock()+' / 2:20';};
if(new URLSearchParams(location.search).has('export'))document.body.classList.add('export');
