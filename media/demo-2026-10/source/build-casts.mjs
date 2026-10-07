// Prepare recorded casts for the film: the byte stream is kept exactly; only
// idle gaps longer than MAX_GAP are shortened (like asciinema's idle limit),
// so a 3-second RPC wait does not become 3 seconds of a static screen.
import { readFileSync, writeFileSync, readdirSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
const here = dirname(fileURLToPath(import.meta.url));
const src = join(here, '../assets/casts'), out = join(here, '../film/casts');
mkdirSync(out, { recursive: true });
const MAX_GAP = 0.45;
for (const f of readdirSync(src).filter(f => f.endsWith('.cast'))) {
  const lines = readFileSync(join(src, f), 'utf8').trim().split('\n');
  const header = JSON.parse(lines[0]);
  let last = 0, shift = 0;
  const events = lines.slice(1).map(l => JSON.parse(l)).map(([t, k, d]) => {
    const gap = t - last; last = t;
    if (gap > MAX_GAP) shift += gap - MAX_GAP;
    return [+(t - shift).toFixed(4), k, d];
  });
  writeFileSync(join(out, f.replace('.cast', '.json')), JSON.stringify({ width: header.width, height: header.height, events }));
  console.log(f, 'original', last.toFixed(1) + 's', '→', events.at(-1)[0].toFixed(1) + 's');
}
