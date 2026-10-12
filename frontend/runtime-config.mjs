import { writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';

/**
 * Write the runtime configuration the app reads before it starts.
 *
 * The API base URL belongs to the deployment, not to the code and not to the
 * user. Generating it here means the same files serve a local server and a
 * deployed one, and that no component contains a hostname.
 */
export async function writeRuntimeConfig(directory) {
  await mkdir(directory, { recursive: true });
  const configured = (process.env.EPLYX_API_URL ?? '').trim();
  // Preserve an explicit same-origin choice, including in a local Pages
  // preview, where an empty/unconfigured value uses the developer API default.
  const url = configured === '/' ? '/' : configured.replace(/\/$/, '');
  await writeFile(
    join(directory, 'runtime-config.js'),
    `// Generated. Set EPLYX_API_URL when building or serving.\nglobalThis.EPLYX_API_URL = ${JSON.stringify(url)};\n`
  );
  return url;
}
