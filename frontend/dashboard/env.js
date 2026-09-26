// Where this dashboard lives. The local dashboard serves at the root with its
// API under /api. A hosted Eplyx workspace serves the same modules under a
// project path, pointed at synced results; `cloud` changes wording only.
const data = document.documentElement.dataset;
export const BASE = data.base || '';
export const API = data.api || '/api';
export const CLOUD = data.cloud === '1';
export const DEMO = data.demo === '1';
export const PROJECT = data.project || '';
// The public Eplyx site. The hosted workspace logo returns there; the local
// dashboard logo stays on this machine.
export const MAIN_SITE_URL = '';
