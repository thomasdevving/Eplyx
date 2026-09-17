import { cp, mkdir } from 'node:fs/promises';
// Only the modules needed by the hero sculpture are exposed to the browser.
export const vendorFiles = new Map([
  ['three.module.js', new URL('../node_modules/three/build/three.module.js', import.meta.url)],
  ['three.core.js', new URL('../node_modules/three/build/three.core.js', import.meta.url)],
  ['SVGLoader.js', new URL('../node_modules/three/examples/jsm/loaders/SVGLoader.js', import.meta.url)],
  ['RoomEnvironment.js', new URL('../node_modules/three/examples/jsm/environments/RoomEnvironment.js', import.meta.url)],
  ['LICENSE', new URL('../node_modules/three/LICENSE', import.meta.url)],
]);

export async function prepareVendorAssets() {
  const directory = new URL('./public/vendor/', import.meta.url);
  await mkdir(directory, { recursive: true });
  for (const [name, source] of vendorFiles) await cp(source, new URL(name, directory));
}
