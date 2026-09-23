const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '../docs/examples/phase-u13-1-causal-closure');
const manifest = path.join(root, 'checksums.sha256');
function files(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    const full = path.join(directory, entry.name);
    return entry.isDirectory() ? files(full) : [full];
  });
}
const body = files(root).filter(file => file !== manifest).sort()
  .map(file => `${crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex')}  ${path.relative(root, file).replaceAll('\\', '/')}`)
  .join('\n') + '\n';
if (process.argv.includes('--write')) fs.writeFileSync(manifest, body);
else if (fs.readFileSync(manifest, 'utf8') !== body) throw Error('U13.1 artifact manifest differs');
console.log(JSON.stringify({ files: body.trimEnd().split('\n').length,
  mode: process.argv.includes('--write') ? 'written' : 'verified' }));
