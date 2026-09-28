// Hosted workspace project pages that have no local equivalent: the project
// summary with its linked local stores, and settings (CI tokens). Loaded only
// in cloud mode. Statuses shown are copied from synced engine output.
import { esc, pill, kv, panel, tile, commandLine, count, when, ago, ident, empty } from './ui.js';
import { DEMO, PROJECT } from './env.js';
import { fetchProjectCapabilities, projectCapabilitiesHTML, capabilityFailureHTML } from '/assets/capabilities.js';

async function call(method, path, body) {
 const response = await fetch(path, { method, credentials:'same-origin', headers:{ Accept:'application/json', ...(body ? { 'Content-Type':'application/json' } : {}) }, body:body ? JSON.stringify(body) : undefined });
 const data = await response.json().catch(() => ({}));
 if (!response.ok) throw new Error(data.error || `Request failed (${response.status})`);
 return data;
}

const WHAT = `<ul class="limits">
 <li><strong>Sync includes:</strong> for each run, the exact bytes of <code>metadata.json</code>, <code>report.json</code>, <code>bindings.json</code>, the ChangeSpec and state descriptor, and the search result, each with its SHA-256. It also includes saved counterexamples, reproduction records and the sizes of artifacts kept local.</li>
 <li><strong>Sync excludes:</strong> source code, candidate <code>.so</code> files, captures, <code>eplyx.toml</code>, environment variables, RPC URLs and credentials, local absolute paths, Solana keys and your Eplyx token.</li>
 <li>A synced run is immutable. Re-syncing identical content is a no-op; different content under the same run ID is rejected as a conflict. A search result can be attached once.</li>
 <li>Viewing a result never reruns observation or execution. Hosted checks are requested separately.</li>
</ul>`;

export async function projectPage({ project }) {
 const cloud = project.context?.cloud ?? {};
 const s = project.stats ?? {};
 const sources = s.run_sources ?? {};
 const links = cloud.links ?? [];
 const id = project.project?.id ?? PROJECT;
 const capabilityResult = DEMO ? null : await fetchProjectCapabilities(id, path => call('GET', path))
  .then(capabilities => ({ capabilities }))
  .catch(error => ({ error }));
 const html = `
 <div class="page-head"><h1>Project</h1><p class="muted">Private to the <strong>${esc(cloud.workspace?.name ?? 'workspace')}</strong> workspace${cloud.demo ? ' · also published read-only as this server’s public demo' : ''}. Counts include hosted and synced runs.</p></div>
 <div class="tiles">
  ${tile({ label:'Recorded runs', value:count(s.runs), sub:`${count(sources.local)} local · ${count(sources.ci)} CI${sources.not_recorded ? ` · ${count(sources.not_recorded)} source not recorded` : ''}` })}
  ${tile({ label:'Searches', value:count(s.searches) })}
  ${tile({ label:'Counterexamples', value:count(s.counterexamples_saved), sub:`${count(s.counterexample_kinds?.Observed)} observed · ${count(s.counterexample_kinds?.Derived)} derived` })}
  ${tile({ label:'Blocked releases', value:count(s.blocked), sub:`${count(s.warned)} warned · ${count(s.passed)} passed` })}
  ${tile({ label:'Reproductions', value:count(s.offline_reproductions ?? 0), sub:s.offline_reproductions ? `${count(s.reproductions_succeeded)} reproduced · ${count(s.reproductions_failed)} failed · last ${esc(ago(s.latest_reproduction))}` : 'none synced' })}
  ${tile({ label:'First → latest run', value:esc(s.first_run ? when(s.first_run) : '—'), sub:esc(s.latest_run ? `latest ${ago(s.latest_run)}` : '') })}
 </div>
 ${panel({ title:'Project', body:kv([
  ['Name', esc(project.project?.name ?? '—')],
  ['Cloud project ID', ident(project.project?.id, 'commit')],
  ['Workspace', esc(cloud.workspace?.name ?? '—')],
  ['Visibility', 'Private to workspace members'],
  ['Your role', esc(cloud.role ?? (cloud.demo ? 'public demo viewer' : '—'))],
  ['Created', esc(when(cloud.project?.created_at))],
  ['Branches in run history', (s.branches ?? []).map(b => `<code>${esc(b)}</code>`).join(' ') || '<span class="muted">none recorded</span>'],
 ]) })}
 ${capabilityResult ? panel({ title:'Analysis availability', body:capabilityResult.capabilities ? projectCapabilitiesHTML(capabilityResult.capabilities) : capabilityFailureHTML() }) : ''}
 ${cloud.demo && !links.length ? '' : panel({ title:'Linked local projects', body:links.length ? `<div class="table-wrap"><table class="table"><thead><tr><th>Local project ID</th><th>Linked by</th><th>Via</th><th>When</th></tr></thead><tbody>${links.map(l => `<tr><td><code>${esc(l.local_project_id)}</code></td><td>${esc(l.linked_by)}</td><td>${esc(l.linked_via === 'ci' ? 'CI token' : 'CLI')}</td><td>${esc(when(l.linked_at))}</td></tr>`).join('')}</tbody></table></div><p class="note">Each developer checkout and CI workspace has its own stable local project ID. Their runs share this cloud project.</p>` : empty('No local project is linked yet.', `eplyx link --project ${project.project?.id ?? ''}`) })}
 ${panel({ title:'What is synced?', body:WHAT })}`;
 return { title:'Project', html };
}

export async function settingsPage({ project }) {
 const cloud = project.context?.cloud ?? {};
 const id = project.project?.id ?? PROJECT;
 const owner = cloud.role === 'owner';
 const tokens = owner ? (await call('GET', `/v1/projects/${id}/tokens`)).tokens : [];
 const html = `
 <div class="page-head"><h1>Settings</h1><p class="muted">Set analysis options in <code>eplyx.toml</code> and link projects from the CLI.</p></div>
 ${panel({ title:'Local projects', body:`${commandLine('eplyx login', 'Sign in (browser approval)')}${commandLine(`eplyx link --project ${id}`, 'Link a local project')}${commandLine('eplyx sync', 'Sync completed runs')}${commandLine('eplyx sync --dry-run', 'Show exactly what would be synced')}` })}
 ${panel({ title:'CI tokens', body:owner ? `
  <p>A project CI token can submit checks, sync and read this project’s results. It cannot change the active bundle. Store it as the <code>EPLYX_TOKEN</code> secret of trusted workflows, next to <code>EPLYX_PROJECT_ID=${esc(id)}</code>. Fork pull requests must not receive it.</p>
  <form class="form-row" data-new-token><label class="field"><span>Label</span><input name="label" required maxlength="80" placeholder="github-actions main"></label><button class="button" type="submit">Create CI token</button></form>
  <div data-token-result></div>
  ${tokens.length ? `<div class="table-wrap"><table class="table"><thead><tr><th>Label</th><th>Created</th><th>Last used</th><th>Status</th><th></th></tr></thead><tbody>${tokens.map(t => `<tr><td>${esc(t.label)}</td><td>${esc(when(t.created_at))} <span class="muted">by ${esc(t.created_by)}</span></td><td>${t.last_used_at ? esc(ago(t.last_used_at)) : '<span class="muted">never</span>'}</td><td>${t.revoked_at ? pill('NotTested', 'revoked') : pill('Valid', 'active')}</td><td>${t.revoked_at ? '' : `<button type="button" class="button button--ghost" data-revoke="${esc(t.id)}">Revoke</button>`}</td></tr>`).join('')}</tbody></table></div>` : '<p class="muted">No CI tokens yet.</p>'}` : '<p class="muted">Only workspace owners create and revoke CI tokens.</p>' })}
 ${panel({ title:'CI workflow', body:`${commandLine('eplyx migration analyse')}${commandLine('eplyx sync --latest')}<p class="muted">With <code>EPLYX_TOKEN</code> and <code>EPLYX_PROJECT_ID</code> set, CI runs sync without <code>eplyx link</code>. A sync failure never changes the analysis gate result or exit code. See <code>examples/ci/eplyx-cloud-sync.yml</code>.</p>` })}`;
 return { title:'Settings', html, attach(root) {
  root.querySelector('[data-new-token]')?.addEventListener('submit', async event => {
   event.preventDefault();
   const slot = root.querySelector('[data-token-result]');
   try {
    const created = await call('POST', `/v1/projects/${id}/tokens`, { label:new FormData(event.target).get('label') });
    slot.innerHTML = `<p><strong>Copy this token now; it is shown once.</strong></p><code class="secret">${esc(created.token)}</code>${commandLine(`EPLYX_PROJECT_ID=${created.project_id}`, 'Project ID for CI')}`;
   } catch (error) { slot.innerHTML = `<p class="form-error">${esc(error.message)}</p>`; }
  });
  root.querySelectorAll('[data-revoke]').forEach(button => button.addEventListener('click', async () => {
   await call('DELETE', `/v1/projects/${id}/tokens/${button.dataset.revoke}`).catch(error => alert(error.message));
   window.dashboardNavigate(location.pathname);
  }));
 } };
}

// Fill the sidebar's project switcher with every project the viewer can see.
export async function attachSwitcher(app, project) {
 const select = app.querySelector('[data-project-switch]');
 if (!select) return;
 const { workspaces } = await call('GET', '/v1/workspaces');
 select.innerHTML = workspaces.map(ws => `<optgroup label="${esc(ws.name)}">${ws.projects.map(p => `<option value="${esc(p.id)}" ${p.id === project.project?.id ? 'selected' : ''}>${esc(p.name)}</option>`).join('')}</optgroup>`).join('');
 select.addEventListener('change', () => location.assign(`/p/${select.value}`));
}
