// Archive the browser recordings compactly and restore them for re-rendering.
//
//   node pack-clips.mjs pack   <clipsDir>  → ../assets/web/<clip>.mp4 + <clip>.json
//   node pack-clips.mjs unpack <clipsDir>  ← extracts frames back with their timestamps
//
// Packing keeps every frame at its recorded timestamp (variable frame rate) in a
// high-quality H.264 file; the JSON keeps the pointer/mark log and the frame times.
import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const here = dirname(fileURLToPath(import.meta.url));
const store = join(here, '../assets/web');
const [, , mode, dir] = process.argv;
const CLIPS = ['hero-full', 'start', 'demo', 'console', 'hosted', 'dashboard', 'dashboard2', 'dashboard3', 'current', 'current2', 'device', 'workspace'];
const run = (cmd, args) => { const r = spawnSync(cmd, args, { stdio: ['ignore', 'inherit', 'inherit'] }); if (r.status !== 0) throw new Error(cmd + ' failed'); };
if (mode === 'pack') {
  mkdirSync(store, { recursive: true });
  for (const c of CLIPS) {
    const meta = JSON.parse(readFileSync(join(dir, c, 'clip.json'), 'utf8'));
    const list = meta.frames.map((f, i) => {
      const next = meta.frames[i + 1]?.t ?? f.t + 1 / 30;
      return `file '${join(dir, c, f.file)}'\nduration ${Math.max(0.001, next - f.t).toFixed(4)}`;
    }).join('\n') + `\nfile '${join(dir, c, meta.frames.at(-1).file)}'\n`;
    const listFile = join(dir, c, 'frames.txt'); writeFileSync(listFile, list);
    run('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'concat', '-safe', '0', '-i', listFile, '-fps_mode', 'vfr', '-c:v', 'libx264', '-preset', 'slow', '-crf', '14', '-pix_fmt', 'yuv444p', '-movflags', '+faststart', join(store, c + '.mp4')]);
    writeFileSync(join(store, c + '.json'), JSON.stringify({ ...meta, frames: meta.frames.map(f => ({ t: f.t })) }));
    console.log('packed', c, meta.frames.length, 'frames');
  }
} else if (mode === 'unpack') {
  for (const c of CLIPS) {
    const meta = JSON.parse(readFileSync(join(store, c + '.json'), 'utf8'));
    const out = join(dir, c); rmSync(out, { recursive: true, force: true }); mkdirSync(out, { recursive: true });
    run('ffmpeg', ['-loglevel', 'error', '-i', join(store, c + '.mp4'), '-fps_mode', 'passthrough', '-q:v', '2', join(out, '%05d.jpg')]);
    const files = readdirSync(out).filter(f => f.endsWith('.jpg')).sort();
    const frames = meta.frames.slice(0, files.length).map((f, i) => ({ t: f.t, file: files[i] }));
    writeFileSync(join(out, 'clip.json'), JSON.stringify({ ...meta, frames }));
    console.log('unpacked', c, frames.length, 'frames');
  }
} else throw new Error('usage: pack-clips.mjs pack|unpack <clipsDir>');
