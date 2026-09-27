import { Header, Footer, workspaceHref } from './shell.js';

const REPO = 'https://github.com/thomasdevving/Eplyx';
export const CLI_COMMANDS = Object.freeze({
  install: 'git clone --branch main https://github.com/thomasdevving/Eplyx.git\ncd Eplyx\ncargo build --locked --release -p eplyx-engine\nexport PATH="$PWD/target/release:$PATH"\neplyx --help',
  init: 'mkdir my-migration\ncd my-migration\neplyx init --migration --fixture',
  migration: 'eplyx doctor\neplyx migration analyse\neplyx runs --json\neplyx dashboard --no-open',
  search: 'eplyx migration search --run RUN_ID\neplyx migration gate --run RUN_ID --policy strict\neplyx migration reproduce COUNTEREXAMPLE_ID',
  upgrade: 'eplyx ci check --bundle .eplyx/bundle \\\n  --candidate target/deploy/program.so --format json',
  lifecycle: 'eplyx change lifecycle --scenario scenario.json --out change.json\neplyx lifecycle analyse --snapshot snapshot.json --scenario scenario.json \\\n  --change-spec change.json --at 2031-01-01T00:00:00Z \\\n  --format json --out report.json',
  paths: 'eplyx observe replay --help\neplyx path capabilities --help\neplyx path probe --help\neplyx path probe-withdrawal --help',
  sync: 'eplyx login --server "$EPLYX_URL"\neplyx link --project PROJECT_ID\neplyx sync --dry-run\neplyx sync --latest\neplyx logout',
});
const escape = value => String(value).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

export function CommandBlock(id, label, command) {
  return `<div class="command-block"><div class="command-bar"><span>${escape(label)}</span><button type="button" data-copy-command="command-${id}" aria-label="Copy ${escape(label)} commands">Copy</button></div><pre tabindex="0" aria-label="${escape(label)} commands"><code id="command-${id}">${escape(command)}</code></pre><span class="command-feedback" role="status" aria-live="polite"></span></div>`;
}

const guide = (path, label) => `<a href="${REPO}/blob/main/docs/${path}" class="text-link">${label} ↗</a>`;

export function CliPage() {
  return `<main id="main" class="inner-page cli-page">${Header({ light: true })}
    <section class="guide-intro"><p class="eyebrow"><span></span> Eplyx CLI</p><h1>From your terminal<br>to inspectable evidence.</h1><p>Run analyses locally, inspect saved results and reproduce findings. One <code>eplyx</code> binary covers upgrades, token migrations, lifecycle policies and supported current paths.</p><p data-technical>Local analytical records live in <code>.eplyx/</code>. Reproduction uses retained inputs; hosted sync transfers analytical documents without rerunning the engine. Required inputs and executable programs must be prepared separately.</p>
    <nav class="guide-nav" aria-label="CLI guide sections"><a href="#install">Install</a><a href="#migration">Migration</a><a href="#upgrades">Upgrades</a><a href="#lifecycle">Lifecycle</a><a href="#paths">Current paths</a><a href="#sync">Workspace sync</a></nav></section>

    <section class="guide-section" id="install"><div><span class="guide-number">01 / Install</span><h2>Build the CLI from source.</h2><p>The supported installation path here is a source build. Prebuilt Eplyx downloads and a one-line installer are not available yet.</p><p>Install Git, Rust stable with Cargo, and your platform’s native build tools first. Run these commands in the directory where you want the checkout.</p><p>The PATH command makes <code>eplyx</code> available in this terminal session. No account, wallet or provider is needed to build it or inspect command help.</p><a href="${REPO}" class="text-link">Browse the source ↗</a></div><div>${CommandBlock('install','Build from source',CLI_COMMANDS.install)}<p class="command-note">A source build compiles the analysis engine; it does not build your candidate program. Keep the checkout to retain the CLI path.</p></div></section>

    <section class="guide-section" id="migration"><div><span class="guide-number">02 / Token migration</span><h2>Prepare a proposal and state.</h2><p>Create editable templates in a new directory. With <code>--fixture</code>, the project uses synthetic state instead of acquiring current chain state.</p><p><strong>Before analysing:</strong> fill in <code>migration.json</code>, configure the fixture and place your SBF candidate at the path in <code>eplyx.toml</code>. <code>init</code> alone does not create a complete runnable migration.</p><p>Building the reference candidate requires the pinned Solana SBF toolchain. The archived minimal example also needs its exact captured-program fixture imported; it is not included in a fresh clone. Follow the guide for those prerequisites.</p>${guide('token-migration.md','Migration setup and fixture requirements')}</div><div>${CommandBlock('init','Create migration templates',CLI_COMMANDS.init)}${CommandBlock('migration','Analyse a prepared fixture project',CLI_COMMANDS.migration)}<p class="command-note">Doctor checks local configuration and inputs. The dashboard shows saved records on loopback; opening it does not start an analysis.</p></div></section>

    <section class="guide-section" id="reproduce"><div><span class="guide-number">03 / Inspect and reproduce</span><h2>Keep the finding reproducible.</h2><p>Search a saved migration, evaluate its gate and replay a counterexample offline. Use the IDs printed by your own run and search.</p><p>A strict gate can fail because required evidence is incomplete. A successful command does not establish issuer authorisation or signing possession.</p>${guide('dashboard.md','Local dashboard guide')}</div><div>${CommandBlock('search','Search and replay saved evidence',CLI_COMMANDS.search)}<p class="command-note"><code>RUN_ID</code> and <code>COUNTEREXAMPLE_ID</code> are placeholders. Reproduction needs a counterexample that your search actually saved.</p></div></section>

    <section class="guide-section" id="upgrades"><div><span class="guide-number">04 / Program upgrades</span><h2>Check a build against a bundle.</h2><p>Use a validated offline bundle containing the baseline, retained corpus and pinned dependencies. Build the candidate separately, then compare it against that bundle.</p><p>The gate reports expected, unexpected, exceeded, stale or unevaluable changes. Exit status belongs to the built binary, so invoke <code>eplyx</code> directly in CI.</p>${guide('phase-10-hosted-ci.md','Bundle preparation and CI guide')}<p><a href="/analyse" data-link class="text-link">Open the hosted upgrade form ↗</a></p></div><div>${CommandBlock('upgrade','Check a prepared upgrade bundle',CLI_COMMANDS.upgrade)}<p class="command-note">These paths must already contain your validated bundle and candidate. The command does not build code or acquire a corpus.</p></div></section>

    <section class="guide-section" id="lifecycle"><div><span class="guide-number">05 / Lifecycle policy</span><h2>Evaluate declared terms.</h2><p>Supply a retained snapshot and scenario, create the lifecycle ChangeSpec and choose an evaluation time. Both commands below use your own input files.</p><p>The date is an example. Policy time does not change the recorded account state. A proposed successor or deadline is not proof of an official transition.</p>${guide('lifecycle.md','Lifecycle inputs and assurance')}</div><div>${CommandBlock('lifecycle','Evaluate a lifecycle scenario',CLI_COMMANDS.lifecycle)}<p class="command-note">Output files must be new. Saved evidence is never overwritten. Use <code>--help</code> for comparison times and recording results in a local project.</p></div></section>

    <section class="guide-section" id="paths"><div><span class="guide-number">06 / Current paths</span><h2>Test one exact path.</h2><p>Replay retained observations and inspect bounded Transfer, Meteora market-exit and liquidity-withdrawal checks. Each command’s help specifies the required captures and inputs.</p><p>Observation acquisition is a separate read-only operation. Offline execution requires recorded state and code; a balance or owner address alone cannot prove an exit or possession of a key.</p>${guide('current-state-analysis.md','Current-state guide')}</div><div>${CommandBlock('paths','Inspect current-path commands',CLI_COMMANDS.paths)}<p class="command-note">These commands display help only. Current-state acquisition has been verified with frozen fixtures and mock providers; live-provider validation remains a separate milestone.</p></div></section>

    <section class="guide-section" id="sync"><div><span class="guide-number">07 / Optional hosted workspace</span><h2>Share retained analytical results.</h2><p>Use a service with workspace identity configured. Set <code>EPLYX_URL</code> to that service’s origin, approve the device code in your browser and replace <code>PROJECT_ID</code> with a project you can access.</p><p>Sync transfers exact analytical documents after a privacy check. It does not upload source, executable programs or captures, and it does not rerun the analysis.</p>${guide('cloud.md','Workspace setup and sync')}<p><a href="${workspaceHref()}" class="text-link">Open workspace ↗</a></p></div><div>${CommandBlock('sync','Link and sync a prepared local project',CLI_COMMANDS.sync)}<p class="command-note">Run inside a local project with saved results. Offline analysis does not require a hosted account. Availability depends on the service’s configuration.</p></div></section>
  </main>${Footer()}`;
}

export function attachCommandCopy() {
  document.querySelectorAll('[data-copy-command]').forEach(button => {
    button.addEventListener('click', async () => {
      const code = document.getElementById(button.dataset.copyCommand);
      const feedback = button.closest('.command-block').querySelector('.command-feedback');
      try {
        await navigator.clipboard.writeText(code.textContent);
        feedback.textContent = 'Commands copied.';
      } catch {
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(code);
        selection.removeAllRanges();
        selection.addRange(range);
        code.parentElement.focus();
        feedback.textContent = 'Copy unavailable. Commands selected; use your keyboard to copy.';
      }
    });
  });
}
