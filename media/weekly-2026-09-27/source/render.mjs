import {chromium} from '@playwright/test';
import {createServer} from 'node:http';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,extname} from 'node:path';
import {spawn} from 'node:child_process';
import {once} from 'node:events';

const root=resolve('media/weekly-2026-09-27');
const ffmpeg=process.env.FFMPEG||'/private/tmp/eplyx-video-tools/imageio_ffmpeg/binaries/ffmpeg-macos-aarch64-v7.1';
const chrome=process.env.EPLYX_CHROME||'/Users/thomasnguyen/Library/Caches/ms-playwright/chromium-1228/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const cli=JSON.parse(await readFile(resolve(root,'assets/cli-recording.json')));
const orca=await readFile(resolve(root,'assets/orca-cli.txt'),'utf8');
await writeFile(resolve(root,'assets/recorded-data.js'),'window.RECORDED='+JSON.stringify({cli,orca})+';\n');
const mime={'.html':'text/html','.js':'text/javascript','.png':'image/png','.svg':'image/svg+xml','.woff2':'font/woff2','.wav':'audio/wav','.mp4':'video/mp4'};
const server=createServer(async(req,res)=>{try{let path=decodeURIComponent(new URL(req.url,'http://localhost').pathname);const file=resolve(root,'.'+path);if(!file.startsWith(root+'/'))throw Error('Path');res.setHeader('Content-Type',mime[extname(file)]||'application/octet-stream');res.end(await readFile(file));}catch{res.statusCode=404;res.end('Not found');}});
await new Promise(r=>server.listen(4414,'127.0.0.1',r));
const browser=await chromium.launch({executablePath:chrome,headless:true});
const page=await browser.newPage({viewport:{width:1920,height:1080},deviceScaleFactor:1});
page.on('pageerror',e=>console.error('PAGE ERROR',e));
await page.goto('http://127.0.0.1:4414/movie.html?export');await page.evaluate(()=>window.filmReady);
const shots=[3,11,20,31,42,55,63,70,75,83,93,103,110,116,126,132,137];
await mkdir(resolve(root,'output/qa'),{recursive:true});
for(const t of shots){await page.evaluate(t=>renderFrame(t),t);await page.screenshot({path:resolve(root,`output/qa/frame-${String(t).padStart(3,'0')}.jpg`),type:'jpeg',quality:88});}
console.log('Rendered '+shots.length+' review frames');
if(process.argv.includes('--stills')){await browser.close();server.close();process.exit(0);}
const fps=30,total=140*fps;
const args=['-y','-hide_banner','-loglevel','warning','-f','image2pipe','-framerate',String(fps),'-vcodec','mjpeg','-i','pipe:0','-i',resolve(root,'assets/soundtrack.wav'),'-i',resolve(root,'output/eplyx-weekly-2026-09-27.srt'),'-i',resolve(root,'output/chapters.ffmeta'),'-map','0:v','-map','1:a','-map','2:s','-map_metadata','3','-map_chapters','3','-c:v','libx264','-preset','fast','-crf','18','-pix_fmt','yuv420p','-c:a','aac','-b:a','192k','-ar','48000','-c:s','mov_text','-metadata:s:s:0','language=eng','-disposition:s:0','0','-movflags','+faststart','-t','140',resolve(root,'output/eplyx-weekly-2026-09-27.mp4')];
const enc=spawn(ffmpeg,args,{stdio:['pipe','inherit','inherit']});
const done=once(enc,'exit');
const start=Date.now();
for(let f=0;f<total;f++){
 const data=await page.evaluate(t=>{renderFrame(t);return document.querySelector('#film').toDataURL('image/jpeg',.95).split(',')[1];},f/fps);
 if(!enc.stdin.write(Buffer.from(data,'base64')))await once(enc.stdin,'drain');
 if(f%300===0)console.log(`${(f/fps).toFixed(0)} / 140 seconds · ${((Date.now()-start)/1000).toFixed(0)}s rendering`);
}
enc.stdin.end();const [code]=await done;await browser.close();server.close();if(code!==0)throw Error('FFmpeg failed: '+code);console.log('MP4 complete');
