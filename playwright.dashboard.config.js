import { defineConfig } from '@playwright/test';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
// The local dashboard is served by the built `eplyx` binary over copied
// fixtures. Set EPLYX_CHROME to a Chromium executable when Google Chrome is
// not installed.
const browser = process.env.EPLYX_CHROME ? { launchOptions:{ executablePath:process.env.EPLYX_CHROME } } : { channel:'chrome' };
const server = (fixture, port, flag = '') => ({ command:`node frontend/tests/dashboard/server.mjs ${fixture} ${port} ${flag}`.trim(), url:`http://127.0.0.1:${port}/api/project`, reuseExistingServer:false, timeout:60000 });
export default defineConfig({
 outputDir:join(tmpdir(),'eplyx-dashboard-browser-results'),
 testDir:'./frontend/tests/dashboard',
 testMatch:'**/*.spec.js',
 use:{ baseURL:'http://127.0.0.1:4185', reducedMotion:'reduce', ...browser },
 webServer:[server('transition-acceptance', 4185), server('second-asset', 4186), server('second-asset', 4187, '--empty'), server('token-migration', 4188), server('analytical-kinds', 4190), { command:'node frontend/tests/dashboard/public-server.mjs',url:'http://127.0.0.1:4189/token-transitions',reuseExistingServer:false,timeout:60000 }],
 reporter:'list',
});
