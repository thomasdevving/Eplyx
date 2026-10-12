// Render the film deterministically: every frame is produced by renderAt(t)
// in headless Chromium and piped into ffmpeg.
//
//   node render.mjs --stills 1,5.5,12        review frames → ../output/stills/
//   node render.mjs --range 30,41            one segment → ../output/segment.mp4
//   node render.mjs                          the whole film → ../output/eplyx-demo-video.mp4 (+ audio if present)
//
// Another film on the same runtime (a weekly update): EPLYX_FILM_EXTRA is its
// film folder, served at /x/ (its casts/ take precedence over this film's),
// EPLYX_SCENES its scene module (/x/scenes.js), EPLYX_OUT its output folder
// and EPLYX_VIDEO the file name of the full render.
import { chromium } from '@playwright/test';
import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { stat, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, join } from 'node:path';
import { spawn } from 'node:child_process';
import { once } from 'node:events';

const here = resolve(new URL('.', import.meta.url).pathname);
const root = resolve(here, '..');
const film = join(root, 'film');
const clips = process.env.EPLYX_CLIPS || '/tmp/claude-0/-home-user-Eplyx/72216ad1-bced-5eac-8781-c0ed5a8f02ef/scratchpad/clips';
const extra = process.env.EPLYX_FILM_EXTRA ? resolve(process.env.EPLYX_FILM_EXTRA) : null;
const out = process.env.EPLYX_OUT ? resolve(process.env.EPLYX_OUT) : join(root, 'output');
const scenesParam = process.env.EPLYX_SCENES ? '&scenes=' + encodeURIComponent(process.env.EPLYX_SCENES) : '';
const videoName = process.env.EPLYX_VIDEO || 'eplyx-demo-video.mp4';
const chrome = process.env.EPLYX_CHROME || '/opt/pw-browsers/chromium-1194/chrome-linux/chrome';
const FPS = 30;
const args = process.argv.slice(2);
const opt = k => { const i = args.indexOf('--' + k); return i >= 0 ? args[i + 1] : null; };

const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.jpg': 'image/jpeg', '.png': 'image/png', '.woff2': 'font/woff2', '.svg': 'image/svg+xml' };
const server = createServer(async (req, res) => {
  try {
    const p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    let [base, rel] = p.startsWith('/clips/') ? [clips, p.slice(7)] : extra && p.startsWith('/x/') ? [extra, p.slice(3)] : [film, p.slice(1) || 'index.html'];
    if (extra && p.startsWith('/casts/') && await stat(join(extra, p.slice(1))).then(() => true, () => false)) [base, rel] = [extra, p.slice(1)];
    const file = resolve(base, rel);
    if (!file.startsWith(base)) throw new Error('path');
    const info = await stat(file);
    res.writeHead(200, { 'content-type': mime[extname(file)] || 'application/octet-stream', 'content-length': info.size, 'cache-control': 'max-age=3600' });
    createReadStream(file).pipe(res);
  } catch { res.writeHead(404); res.end(); }
});
const port = Number(process.env.EPLYX_PORT || 4415);
await new Promise(r => server.listen(port, '127.0.0.1', r));
const browser = await chromium.launch({ executablePath: chrome, args: ['--disable-background-networking', '--hide-scrollbars', '--force-color-profile=srgb'] });
try {
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  page.on('pageerror', e => console.error('PAGE ERROR', e.message));
  page.on('console', m => { if (m.type() === 'error' || m.type() === 'warning') console.error('console', m.text()); });
  await page.goto(`http://127.0.0.1:${port}/index.html?export` + scenesParam);
  await page.waitForFunction(() => window.filmReady === true, null, { timeout: 120000 });
  const duration = await page.evaluate(() => window.filmDuration);
  await writeFile(join(out, 'timeline.json'), JSON.stringify(await page.evaluate(() => ({ duration: window.filmDuration, scenes: window.filmScenes, cues: window.audioCues(), ...(window.filmMusic ? { music: window.filmMusic } : {}) })), null, 1));
  const frame = async t => { await page.evaluate(t => window.renderAt(t), t); return page.screenshot({ type: 'jpeg', quality: 95, clip: { x: 0, y: 0, width: 1920, height: 1080 } }); };
  if (opt('stills')) {
    await mkdir(join(out, 'stills'), { recursive: true });
    for (const t of opt('stills').split(',').map(Number)) {
      await writeFile(join(out, 'stills', `t${t.toFixed(2).padStart(6, '0')}.jpg`), await frame(t));
      console.log('still', t);
    }
  } else {
    const [a, b] = opt('range') ? opt('range').split(',').map(Number) : [0, duration];
    const name = opt('range') ? 'segment.mp4' : videoName;
    const audio = !opt('range') && process.env.EPLYX_AUDIO;
    const ff = ['-y', '-hide_banner', '-loglevel', 'warning', '-f', 'image2pipe', '-framerate', String(FPS), '-c:v', 'mjpeg', '-i', 'pipe:0',
      ...(audio ? ['-i', audio] : []),
      '-map', '0:v', ...(audio ? ['-map', '1:a', '-c:a', 'aac', '-b:a', '256k', '-ar', '48000'] : []),
      '-vf', 'scale=in_range=pc:out_range=tv:out_color_matrix=bt709,format=yuv420p',
      '-c:v', 'libx264', '-preset', 'slow', '-crf', '16', '-tune', 'animation', '-profile:v', 'high', '-color_range', 'tv', '-colorspace', 'bt709', '-color_primaries', 'bt709', '-color_trc', 'bt709',
      '-movflags', '+faststart', '-t', String(b - a), join(out, name)];
    const enc = spawn('ffmpeg', ff, { stdio: ['pipe', 'inherit', 'inherit'] });
    const done = once(enc, 'exit');
    const total = Math.round((b - a) * FPS), start = Date.now();
    for (let f = 0; f < total; f++) {
      const buf = await frame(a + f / FPS);
      if (!enc.stdin.write(buf)) await once(enc.stdin, 'drain');
      if (f % 150 === 0) console.log(`${(a + f / FPS).toFixed(1)}s / ${b}s  ·  ${((Date.now() - start) / 1000).toFixed(0)}s elapsed`);
    }
    enc.stdin.end();
    const [code] = await done; if (code !== 0) throw new Error('ffmpeg failed ' + code);
    console.log('wrote', join(out, name));
  }
} finally { await browser.close(); server.close(); }
