// Shared drawing primitives copied from the first edit; no product UI is generated here.
const C={ink:'#0b0712',panel:'#171020',purple:'#6d31f2',violet:'#9c68ff',lav:'#e7ddff',white:'#f7f5fa',muted:'#b1a4c4',green:'#87dcbd',red:'#fa909e'};
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
 if(u>3.8&&!compact){text('$ eplyx migration gate --run "$RUN" --policy strict',x+25,y+h-112,w<700?16:22,C.violet,400,'monospace');text('Failed · population: Incomplete',x+25,y+h-72,24,C.red,400,'monospace');}
}
