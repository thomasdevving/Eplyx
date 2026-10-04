import { Header, Footer, SOURCE_URL } from './shell.js';

export const OVERVIEW_PATH = '/technical-overview';
export const OVERVIEW_PDF = '/technical-overview.pdf';
export const OVERVIEW_TITLE = 'Eplyx — Technical Overview';
export const OVERVIEW_DESCRIPTION = 'How Eplyx uses production-derived Solana simulation to evaluate program upgrades, parameter changes, migrations and other on-chain changes before they reach users.';

const capabilities = [
  ['Program upgrades', 'Compare a baseline and candidate over retained interactions.'],
  ['Protocol parameter changes', 'Evaluate Token-2022 transfer fees and Stake Pool SOL deposit fees.'],
  ['Token migrations', 'Rehearse a mechanism and reconcile source and destination balances.'],
  ['Lifecycle changes', 'Compare declared policy terms over the same retained snapshot.'],
  ['Migration ordering', 'Compare both execution orders against a shared reserve.'],
  ['Code × configuration interaction', 'Compare code and fee effects in a qualified four-cell experiment.'],
  ['Bounded edge-case search', 'Search a declared amount range or saved migration for matching cases.'],
];

const arrow = `<svg class="technical-arrow" viewBox="0 0 20 28" aria-hidden="true"><path d="M10 1v23m-5-5 5 5 5-5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
const outward = `<svg class="technical-icon" viewBox="0 0 20 20" aria-hidden="true"><path d="M5 15 15 5M5 5h10v10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>`;
const download = `<svg class="technical-icon" viewBox="0 0 20 20" aria-hidden="true"><path d="M10 3v10m-4-4 4 4 4-4M4 14v3h12v-3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

function Architecture() {
  return `<figure class="technical-architecture" aria-labelledby="architecture-title">
    <div class="technical-node"><h2 id="architecture-title">Real Solana data</h2><p>Retained accounts, transactions and program identities</p></div>
    ${arrow}
    <div class="technical-node"><h3>Reconstructed protocol environment</h3><p>Relevant state, code and runtime assumptions</p></div>
    <div class="technical-branch"><p>Same user actions</p><span aria-hidden="true"></span></div>
    <div class="technical-vms">
      <div class="technical-node technical-node--vm"><h3>Current behavior</h3><p>Isolated Solana VM</p></div>
      <div class="technical-node technical-node--vm"><h3>Proposed change</h3><p>Isolated Solana VM</p></div>
    </div>
    <div class="technical-merge" aria-hidden="true"></div>
    <div class="technical-node technical-node--result"><h3>Behavioral + economic differences</h3></div>
    ${arrow}
    <p class="technical-evidence">Reproducible evidence</p>
    <figcaption>Production-derived simulation for supported executable checks. Each result stays tied to its retained inputs and declared scope.</figcaption>
  </figure>`;
}

export function TechnicalOverviewPage() {
  return `<main id="main" class="inner-page technical-page">
    ${Header({ light: true })}
    <section class="technical-hero" aria-labelledby="technical-title">
      <div class="technical-intro">
        <p class="eyebrow"><span></span> Technical Overview</p>
        <h1 id="technical-title">Simulation-backed assurance for <span class="technical-no-break">on-chain</span> changes</h1>
        <p>Eplyx reconstructs relevant protocol environments from real Solana data. For supported executable checks, it compares current and proposed behavior side by side in isolated Solana VMs.</p>
        <p>The Technical Overview explains the simulation architecture, evidence model, supported change types and reproducibility model behind Eplyx.</p>
        <div class="technical-actions">
          <a href="${OVERVIEW_PDF}" target="_blank" rel="noopener" class="button button--primary">Read Technical Overview ${outward}</a>
          <a href="${SOURCE_URL}" target="_blank" rel="noopener" class="text-link">View on GitHub ${outward}</a>
        </div>
      </div>
      ${Architecture()}
    </section>
    <section class="section technical-capabilities" aria-labelledby="technical-capabilities-title">
      <h2 id="technical-capabilities-title">What Eplyx can evaluate</h2>
      <dl class="technical-capability-list">${capabilities.map(([name, copy]) => `<div><dt>${name}</dt><dd>${copy}</dd></div>`).join('')}</dl>
      <p class="scope-note">Coverage depends on the selected protocol, path and retained inputs. Lifecycle analysis evaluates declared terms; it does not establish executable migration readiness.</p>
    </section>
    <section class="technical-reading" aria-labelledby="technical-evidence-title">
      <div class="technical-reading__copy">
        <h2 id="technical-evidence-title">Results you can verify</h2>
        <p>Supported analyses retain relevant inputs, program identities, execution evidence and measured outcomes. Read-only verification checks identities and internal consistency; reproduction executes again from retained inputs in a compatible environment.</p>
        <p>Evidence gaps remain visible. A result applies to the evaluated cases and assumptions.</p>
      </div>
      <div class="technical-document" aria-labelledby="technical-document-title">
        <h2 id="technical-document-title">Read the full overview</h2>
        <p>Explore the architecture, worked examples and evidence contracts in the complete technical document.</p>
        <div class="technical-actions">
          <a href="${OVERVIEW_PDF}" target="_blank" rel="noopener" class="button button--light">Open PDF ${outward}</a>
          <a href="${OVERVIEW_PDF}" download="Eplyx_Technical_Overview.pdf" class="text-link">Download PDF ${download}</a>
        </div>
        <p class="technical-document__note">The PDF opens directly in your browser. You can also download it to read offline.</p>
      </div>
    </section>
  </main>${Footer()}`;
}
