import { defineConfig } from '@playwright/test';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
// The local dashboard is served by the built `eplyx` binary over copied
// fixtures. Set EPLYX_CHROME to a Chromium executable when Google Chrome is
// not installed.
const browser = process.env.EPLYX_CHROME ? { launchOptions:{ executablePath:process.env.EPLYX_CHROME } } : { channel:'chrome' };
export default defineConfig({
 outputDir:join(tmpdir(),'eplyx-cloud-browser-results','playwright'),
 testDir:'./frontend/tests/cloud',testMatch:'**/*.spec.js',
 use:{baseURL:'http://127.0.0.1:4390',reducedMotion:'reduce',...browser},
 webServer:[{command:'node frontend/tests/cloud/server.mjs 4390',url:'http://127.0.0.1:4390/ready',reuseExistingServer:false,timeout:60000}],
 workers:1,reporter:'list',
});
