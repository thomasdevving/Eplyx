import { defineConfig } from '@playwright/test';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const browser = process.env.EPLYX_CHROME
  ? { launchOptions: { executablePath: process.env.EPLYX_CHROME } }
  : { channel: 'chrome' };
export default defineConfig({
  testDir: './frontend/tests/public',
  outputDir: join(tmpdir(), 'eplyx-public-browser-results'),
  use: { baseURL: 'http://127.0.0.1:4193', reducedMotion: 'reduce', ...browser },
  webServer: {
    command: 'node frontend/tests/public/server.mjs',
    url: 'http://127.0.0.1:4193/cli',
    reuseExistingServer: false,
    timeout: 60000,
  },
  reporter: 'list',
});
