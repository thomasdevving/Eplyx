import { DEMO_REPORT } from './demo.js';
import { IntroAnimation } from './intro.js';
import { EplyxCoreScene } from './core-scene.js';
import { Header, Footer, workspaceHref } from './shell.js';
import { CommandBlock, CLI_COMMANDS } from './cli.js';

const twinkles = [
  [5, 21, 2.6, 5.3, 0], [11, 52, 1.6, 4.1, 1.9], [17, 13, 2.1, 6.4, 3.3], [9, 67, 1.4, 3.6, .8],
  [23, 34, 1.8, 7.1, 2.4], [29, 59, 2.4, 4.7, 5.2], [35, 17, 1.5, 5.9, 1.2], [41, 44, 2.2, 3.9, 4.4],
  [21, 77, 1.7, 6.7, 2.9], [47, 26, 1.4, 4.3, .4], [53, 54, 2.5, 7.4, 6.1], [33, 70, 1.9, 5.1, 3.8],
  [59, 12, 1.6, 6.2, 1.5], [65, 37, 2.3, 4.9, 5.7], [44, 65, 1.5, 3.4, 2.1], [71, 23, 2, 6.9, 4.8],
];

const deposit = DEMO_REPORT.findings.find(finding => finding.fingerprint.includes('/deposit_sol/'));
const withdrawal = DEMO_REPORT.findings.find(finding => finding.fingerprint.includes('/withdraw_sol/'));

export function LandingPage() {
  return `${IntroAnimation()}
    <main id="main">
      <section class="hero">
        ${Header()}
        <div class="hero__atmosphere" aria-hidden="true"><div class="hero__stars"></div><div class="hero__twinkle">${twinkles.map(([left, top, size, period, offset]) => `<i style="left:${left}%;top:${top}%;--s:${size}px;--t:${period}s;--d:${offset}s"></i>`).join('')}</div><div class="hero__mountains"></div></div>
        <div class="hero__wash"></div>
        <div class="hero__copy reveal">
          <p class="eyebrow"><span></span> Onchain change intelligence · In development</p>
          <h1>Know what <em>changes.</em><br>See who is affected.</h1>
          <p class="hero__lead ov-only">Eplyx is being built to explain onchain changes and their consequences. Try scoped upgrade checks, token migration rehearsals, lifecycle analyses and fee-change simulations.</p>
          <p class="hero__lead tech-only">Replay baseline and candidate programs over retained Solana state. Rehearse migrations in a local VM and evaluate declared lifecycle policies. Coverage is limited to supported protocols and paths; the broader product is still in development.</p>
          <div class="hero__actions">
            <a href="/start" data-link class="button button--primary">Try Eplyx <span>↗</span></a>
            <a href="#evidence" class="button button--text">Explore an example <span>↓</span></a>
          </div>

        </div>
        <div class="hero__visual reveal">${EplyxCoreScene()}</div>
        <div class="scroll-cue"><span></span> Scroll to inspect the system</div>
      </section>

      <section class="overview section" aria-labelledby="overview-title">
        <div class="overview__intro reveal">
          <div>
            <p class="eyebrow eyebrow--dark"><span></span> Eplyx, in brief</p>
            <h2 id="overview-title">Understand an onchain change before it reaches users.</h2>
          </div>
          <p>Eplyx is a developing set of tools for examining how proposed Solana changes affect recorded state. Today, it offers scoped program upgrade checks, token migration rehearsals, lifecycle analyses and two supported fee-change operations.</p>
        </div>
        <div class="overview__details reveal">
          <article>
            <span>01 / What it's used for</span>
            <h3>See the consequences of a proposed change.</h3>
            <p>Compare supported changes with recorded inputs, inspect affected interactions or holders, and trace findings to the evidence behind them.</p>
          </article>
          <article>
            <span>02 / Who it's for</span>
            <h3>Teams building and reviewing Solana protocols.</h3>
            <p>Protocol developers, security engineers, token teams, risk reviewers and governance operators can use the results to inform review before a change is approved or deployed.</p>
          </article>
        </div>
      </section>

      <section class="section product-section" id="product">
        <div class="section-heading reveal"><p class="eyebrow eyebrow--dark"><span></span> Current scope</p><h2>Tools you can try.<br>Coverage still growing.</h2><p>Eplyx is in development. These workflows cover selected protocols and scenarios; they are building blocks for the broader product.</p></div>
        <div class="product-grid reveal">
          <article><span class="product-index">01 / Program upgrades</span><h3>Compare the proposed build.</h3><p class="ov-only">See how a proposed program build changes the interactions you test, and whether those changes were expected.</p><p class="tech-only">Replay a baseline and candidate over the same retained state. Inspect execution differences, measured economic effects and whether changes match your declarations.</p><p class="product-boundary">Historical replay requires validated state and a supported instruction path. Protocol coverage is bounded.</p><a href="/analyse" data-link class="text-link">Analyse a program upgrade <span aria-hidden="true">↗</span></a></article>
          <article><span class="product-index">02 / Token migrations</span><h3>Account for the migration.</h3><p class="ov-only">Rehearse a token migration before execution. Check who is covered, what each holder receives and where the proposal fails.</p><p class="tech-only">Rehearse a proposed mechanism in a local VM. Inspect holder coverage, reserves, fees, authority requirements and balance reconciliation. Search and reproduce counterexamples.</p><p class="product-boundary">Findings apply to the recorded inputs. Passing a gate does not authorise execution or establish issuer entitlement.</p><a href="/token-transitions" data-link class="text-link">Explore token transitions <span aria-hidden="true">↗</span></a></article>
          <article><span class="product-index">03 / Lifecycle changes</span><h3>Evaluate the declared terms.</h3><p class="ov-only">Explore how a change in declared terms affects token holders. See what is known, uncertain or still untested.</p><p class="tech-only">Compare a policy before and after its effective time over one retained snapshot. Keep consequences, observed state and execution evidence separate.</p><p class="product-boundary">A notice or proposed replacement is a declaration. Unknown eligibility and untested paths remain visible.</p><a href="/cli#lifecycle" data-link class="text-link">Use the lifecycle CLI <span aria-hidden="true">↗</span></a></article>
        </div>
        <div class="capability-notes reveal"><p><strong>Parameter changes.</strong> Compare the active Token-2022 transfer-fee rate for one retained transfer, or simulate the SOL deposit fee for one retained Stake Pool deposit. A bounded code-and-fee interaction check is also available. <a href="/start#parameters" data-link>Choose a fee-change workflow ↗</a></p><p><strong>Current paths.</strong> Bounded Transfer and Meteora market-exit checks, plus exact liquidity-withdrawal checks through the CLI. Each path needs its own recorded execution and reconciliation.</p><p><strong>Governance binding.</strong> Read-only Squads V4 binding and attestation for the supported single loader-v3 program-upgrade proposal. Broader governance support remains future work.</p></div>
      </section>

      <section class="section section--ink workflow-section" id="how">
        <div class="section-heading section-heading--light reveal"><p class="eyebrow"><span></span> How it works</p><h2>Declare. Evaluate.<br>Inspect the evidence.</h2><p>Use the CLI locally, review retained results in a dashboard, or submit checks to a configured Eplyx service.</p></div>
        <ol class="workflow-steps reveal">
          <li><span>01</span><h3>Describe the change.</h3><p class="ov-only">Describe the proposed change and provide the inputs you want to evaluate.</p><p class="tech-only">Supply the proposal and exact inputs. Program checks use builds you provide; lifecycle analysis uses explicit policy terms.</p></li>
          <li><span>02</span><h3>Evaluate the recorded state.</h3><p class="ov-only">Test supported interactions locally, or compare declared terms over recorded state.</p><p class="tech-only">Executable checks run in an isolated local VM. Current-state acquisition is a separate read-only step; offline workers use retained inputs.</p></li>
          <li><span>03</span><h3>Review and reproduce.</h3><p class="ov-only">Inspect what changed, where the evidence is missing and how to reproduce a finding.</p><p class="tech-only">Follow findings to their inputs and measured outputs. Compare runs, inspect missing evidence and replay saved counterexamples.</p></li>
        </ol>
        <p class="workflow-note">Current captures describe observations across a slot range. Historical replay has a separate fidelity requirement. Neither a passing check nor a policy finding grants signing authority.</p>
      </section>

      <section class="section evidence-example" id="evidence">
        <div class="section-heading reveal"><p class="eyebrow eyebrow--dark"><span></span> Recorded example · controlled regression</p><h2>See a finding.<br>Follow its evidence.</h2><p>Ten validated historical production interactions from a Stake Pool corpus, replayed against a deliberately regressed candidate.</p></div>
        <div class="result-window reveal">
          <div class="window-bar"><span><i></i><i></i><i></i></span><b>eplyx / stake-pool / controlled regression demo</b><em>Exit code 1</em></div>
          <div class="result-summary">
            <div><span class="status-dot status-dot--fail"></span><p>Upgrade check</p><h3>Failed</h3></div>
            <div><p>Corpus</p><h3>${DEMO_REPORT.bundle.record_count}</h3><span>validated interactions</span></div>
            <div><p>Finding categories</p><h3>${DEMO_REPORT.findings.length}</h3><span>${deposit.observations.length} economic decrease · ${withdrawal.observations.length} reverts</span></div>
            <a href="/runs/demo" data-link>Open demo report <span>↗</span></a>
          </div>
          <div class="findings">
            <article class="finding finding--critical"><div><span>Critical</span><span>Unexpected</span></div><h3>WithdrawSol</h3><p>Transaction now reverts</p><dl data-technical><dt>Measured observations</dt><dd>${withdrawal.observations.length} / ${withdrawal.covered_observations}</dd><dt>Execution</dt><dd>Success → error</dd></dl></article>
            <article class="finding finding--high"><div><span>High</span><span>Unexpected</span></div><h3>DepositSol</h3><p>pool_tokens_received decreased</p><dl data-technical><dt>Affected observations</dt><dd>${deposit.observations.length} / ${deposit.covered_observations}</dd><dt>Maximum delta</dt><dd>${deposit.max_relative_delta_bps} bps</dd></dl></article>
          </div>
        </div>
        <p class="demo-note">This candidate implements DepositSol only, so WithdrawSol fails. These are saved historical results for a constructed fixture, not a live feed or a claim about every pool. Expected changes are reviewed against explicit declarations and bounds.</p>
      </section>

      <section class="section section--ink cli-preview" id="cli">
        <div class="section-heading section-heading--light reveal"><p class="eyebrow"><span></span> One CLI · local evidence</p><h2>Start from your terminal.</h2><p>Build Eplyx from source, inspect its commands and keep analytical records locally. The CLI guide covers migration, lifecycle, upgrade and fee checks, and workspace sync.</p><a href="/cli" data-link class="button button--light">Installation and commands <span>↗</span></a></div>
        <div class="reveal">${CommandBlock('install', 'Build from source', CLI_COMMANDS.install)}<p class="command-note">Requires Git, Rust stable and native build tools. The source build works independently of release publication. Check GitHub Releases for published macOS arm64 archives; no one-line installer is provided.</p></div>
      </section>

      <section class="section roadmap-section" id="roadmap">
        <div class="section-heading reveal"><p class="eyebrow eyebrow--dark"><span></span> Where Eplyx is going</p><h2>More changes.<br>The same evidence standard.</h2><p>The goal is broader change and consequence analysis across Solana. Protocol coverage, real-world validation and the wider governance and monitoring layers still need work.</p></div>
        <div class="roadmap-grid reveal">
          <article><span>Current foundation</span><h3>Local and hosted workflows.</h3><p>The CLI, local dashboard and hosted workflows are integrated within their tested scope. Hosted identity and current acquisition need operator configuration; this is not the finished product.</p><a href="${workspaceHref()}" class="text-link">Open workspace ↗</a></article>
          <article><span>Next validation</span><h3>Exercise current-state acquisition.</h3><p>Current-state acquisition was checked against frozen captures and mock providers. The next milestone is a bounded read-only mainnet validation with fresh evidence.</p></article>
          <article><span>Future scope</span><h3>Build the broader product.</h3><p>General governance, authority analysis, additional parameter operations, wider protocol coverage and continuous monitoring remain development goals. Partial and claim-based migrations and local-validator plan checks are also future work.</p></article>
        </div>
        <p class="scope-note">Eplyx analyses and reports. It does not submit transactions or establish possession of signing keys. Automated monitoring and general protocol coverage are not current features.</p>
      </section>
    </main>${Footer()}`;
}
