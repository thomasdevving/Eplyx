import { cp, mkdir, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { prepareVendorAssets } from './vendor.mjs';
import { writeRuntimeConfig } from './runtime-config.mjs';

await prepareVendorAssets();
await writeRuntimeConfig(fileURLToPath(new URL('./public/', import.meta.url)));

const root = fileURLToPath(new URL('..', import.meta.url));
const out = join(root, 'dist');
await rm(out, { recursive: true, force: true });
await mkdir(out, { recursive: true });
await cp(join(root, 'frontend/index.html'), join(out, 'index.html'));
await cp(join(root, 'frontend/src'), join(out, 'src'), { recursive: true });
await cp(join(root, 'frontend/public'), join(out, 'public'), { recursive: true });
console.log('Built frontend to dist/');
