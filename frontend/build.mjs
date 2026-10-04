import { cp, mkdir, rm, readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prepareVendorAssets } from './vendor.mjs';
import { writeRuntimeConfig } from './runtime-config.mjs';
import { technicalOverviewDocument } from './technical-overview-document.mjs';

await prepareVendorAssets();
globalThis.EPLYX_API_URL = await writeRuntimeConfig(fileURLToPath(new URL('./public/', import.meta.url)));

const root = fileURLToPath(new URL('..', import.meta.url));
const out = join(root, 'dist');
await rm(out, { recursive: true, force: true });
await mkdir(out, { recursive: true });
await cp(join(root, 'frontend/index.html'), join(out, 'index.html'));
await cp(join(root, 'frontend/src'), join(out, 'src'), { recursive: true });
await cp(join(root, 'frontend/public'), join(out, 'public'), { recursive: true });
await cp(join(root, 'frontend/public/technical-overview.pdf'), join(out, 'technical-overview.pdf'));
await mkdir(join(out, 'technical-overview'), { recursive: true });
await writeFile(join(out, 'technical-overview/index.html'), await technicalOverviewDocument(await readFile(join(root, 'frontend/index.html'), 'utf8')));
console.log('Built frontend to dist/');
