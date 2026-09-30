import {chromium} from '@playwright/test';
import {createServer} from 'node:http';
import {readFile,writeFile,mkdir,stat} from 'node:fs/promises';
import {createReadStream} from 'node:fs';
import {resolve,extname} from 'node:path';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
const root=resolve('media/weekly-2026-09-27');
const ffmpeg=process.env.FFMPEG||'/private/tmp/eplyx-video-tools/imageio_ffmpeg/binaries/ffmpeg-macos-aarch64-v7.1';
const chrome=process.env.EPLYX_CHROME||'/Users/thomasnguyen/Library/Caches/ms-playwright/chromium-1228/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
await writeFile(resolve(root,'assets/motion/capture-log.js'),'window.MOTION_LOG='+await readFile(resolve(root,'assets/motion/capture-log.json'),'utf8')+';');
const mime={'.html':'text/html','.js':'text/javascript','.png':'image/png','.jpg':'image/jpeg','.svg':'image/svg+xml','.woff2':'font/woff2','.wav':'audio/wav','.mp4':'video/mp4','.vtt':'text/vtt'};
const server=createServer(async(req,res)=>{
 try{const path=decodeURIComponent(new URL(req.url,'http://localhost').pathname);const file=resolve(root,'.'+path);if(!file.startsWith(root+'/'))throw Error('Path');const info=await stat(file);res.setHeader('Content-Type',mime[extname(file)]||'application/octet-stream');res.setHeader('Accept-Ranges','bytes');
 if(req.headers.range){const [,a,b]=req.headers.range.match(/bytes=(\d+)-(\d*)/)||[];const start=Number(a),end=b?Math.min(Number(b),info.size-1):info.size-1;if(!Number.isFinite(start)||start>end)throw Error('Range');res.writeHead(206,{'Content-Range':`bytes ${start}-${end}/${info.size}`,'Content-Length':end-start+1});createReadStream(file,{start,end}).pipe(res);}
 else{res.setHeader('Content-Length',info.size);createReadStream(file).pipe(res);}
 }catch{res.statusCode=404;res.end('Not found');}
});
await new Promise(r=>server.listen(4414,'127.0.0.1',r));
const browser=await chromium.launch({executablePath:chrome,headless:true});
try{
 const page=await browser.newPage({viewport:{width:1920,height:1080},deviceScaleFactor:1});page.on('pageerror',e=>console.error('PAGE ERROR',e));
 await page.goto('http://127.0.0.1:4414/movie-v2.html?export');await page.evaluate(()=>window.filmReady);
 const samples=[1,4.5,7.5,10.5,14,17.5,22,27,32,38,41,45,50,55,58,62,65,69,72,75,79,82,88,91,94,98,100,103,107,110,114,118,122,126];
 await mkdir(resolve(root,'output/qa-v2'),{recursive:true});
 for(const t of samples){await page.evaluate(async t=>{await prepareFrame(t);renderFrame(t);},t);await page.screenshot({path:resolve(root,`output/qa-v2/frame-${String(t).replace('.','-').padStart(5,'0')}.jpg`),type:'jpeg',quality:90});}
 console.log(`Rendered ${samples.length} full-resolution review frames`);
 if(!process.argv.includes('--stills')){
  const fps=30,duration=128,total=fps*duration;
  const args=['-y','-hide_banner','-loglevel','warning','-f','image2pipe','-framerate','30','-vcodec','mjpeg','-i','pipe:0','-i',resolve(root,'assets/soundtrack-v2.wav'),'-i',resolve(root,'output/eplyx-weekly-2026-09-27-v2.srt'),'-i',resolve(root,'output/chapters-v2.ffmeta'),'-map','0:v','-map','1:a','-map','2:s','-map_metadata','3','-map_chapters','3','-vf','scale=in_range=pc:out_range=tv:out_color_matrix=bt709','-c:v','libx264','-preset','fast','-crf','18','-pix_fmt','yuv420p','-color_range','tv','-colorspace','bt709','-color_primaries','bt709','-color_trc','bt709','-c:a','aac','-b:a','192k','-ar','48000','-c:s','mov_text','-metadata:s:s:0','language=eng','-disposition:s:0','0','-movflags','+faststart','-t','128',resolve(root,'output/eplyx-weekly-2026-09-27-v2.mp4')];
  const enc=spawn(ffmpeg,args,{stdio:['pipe','inherit','inherit']});const done=once(enc,'exit');let start=Date.now();
  for(let f=0;f<total;f++){
   const data=await page.evaluate(async t=>{await prepareFrame(t);renderFrame(t);return document.querySelector('#film').toDataURL('image/jpeg',.96).split(',')[1];},f/fps);
   if(!enc.stdin.write(Buffer.from(data,'base64')))await once(enc.stdin,'drain');
   if(f%300===0)console.log(`${f/fps} / 128 sec · ${((Date.now()-start)/1000).toFixed(0)}s rendering`);
  }
  enc.stdin.end();const [code]=await done;if(code!==0)throw Error('FFmpeg failed '+code);console.log('Revised MP4 complete');
 }
}finally{await browser.close();server.close();}
