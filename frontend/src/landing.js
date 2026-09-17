import { IntroAnimation } from './intro.js';
import { EplyxCoreScene } from './core-scene.js';
import { Header, Footer } from './shell.js';

// A sparse set of stars that blink over the dense static field: left, top,
// size, period and offset. Real twinkling is uneven, so the periods are chosen
// not to fall into step with each other.
const twinkles = [
  [5, 21, 2.6, 5.3, 0], [11, 52, 1.6, 4.1, 1.9], [17, 13, 2.1, 6.4, 3.3], [9, 67, 1.4, 3.6, .8],
  [23, 34, 1.8, 7.1, 2.4], [29, 59, 2.4, 4.7, 5.2], [35, 17, 1.5, 5.9, 1.2], [41, 44, 2.2, 3.9, 4.4],
  [21, 77, 1.7, 6.7, 2.9], [47, 26, 1.4, 4.3, .4], [53, 54, 2.5, 7.4, 6.1], [33, 70, 1.9, 5.1, 3.8],
  [59, 12, 1.6, 6.2, 1.5], [65, 37, 2.3, 4.9, 5.7], [44, 65, 1.5, 3.4, 2.1], [71, 23, 2, 6.9, 4.8],
];

const flow = [
  ['01', 'Production inputs', 'Real production state + proposed change + historical action.'],
  ['02', 'Deterministic replay', 'Execute the same action from the same pinned historical state.'],
  ['03', 'Baseline vs candidate', 'Compare two program versions with identical inputs.'],
  ['04', 'Semantic / economic diff', 'Measure changes in outcomes, balances and user quantities.'],
  ['05', 'CI / review decision', 'Fail on undeclared or out-of-bounds changes.'],
];

const proofs = [
  ['01', 'Change surface', 'What can change, and who can change it?', 'Today: program binary changes. Governance, authority and parameter analysis are future layers.', 'Current + future'],
  ['02', 'Execution impact', 'What happens on real production state?', 'Replay validated historical interactions against baseline and candidate binaries from the same pre-state.', 'Live'],
  ['03', 'Economic consequence', 'Who is affected and by how much?', 'Measure user output, fees and protocol balances in the tested Stake Pool corpus. Other economic semantics depend on protocol coverage.', 'Live · corpus scoped'],
  ['04', 'Exit & control', 'Can users still exit, and who controls the rules?', 'WithdrawSol replay can expose an execution failure today. Broader exitability, liquidation and authority analysis are future layers.', 'Expanding / future layer'],
];

const roles = [
  ['01', 'Protocol developers', 'Will this PR change production behavior?', 'Run candidate binaries against validated historical production interactions before deployment.', ['CI pass/fail', 'Changed interactions', 'Regressions', 'Expected-change review'], 'Integration tests against mainnet reality.', '⟨/⟩'],
  ['02', 'Security engineers', 'Why did it change, and can I reproduce it?', 'Inspect execution evidence instead of relying on a generic risk score.', ['Historical transaction', 'Pre/post state', 'CPI graph', 'Binary hashes', 'Counterexample'], 'Trace the result back to the exact execution.', '⌘'],
  ['03', 'Protocol risk teams', 'Who is affected, and how much value is exposed?', 'Translate measured output and balance changes into economic consequences for the tested states.', ['Affected states', 'Economic deltas', 'Protocol balances'], 'Entity-wide exposure, liquidation and exit-path analysis are future layers.', '↗'],
  ['04', 'Governance / multisig operators', 'What will this proposal do before we execute it?', 'Evaluate candidate upgrades against production-derived state. Governance and privileged-change analysis belong to the broader platform direction.', ['Proposed upgrade', 'Tested binary hash', 'Economic consequences', 'Execution evidence'], 'Governance, authority and exitability analysis · planned.', '⋈'],
];

export function LandingPage() {
  return `${IntroAnimation()}
    <main id="main">
      <section class="hero">
        ${Header()}
        <div class="hero__atmosphere" aria-hidden="true"><div class="hero__stars"></div><div class="hero__twinkle">${twinkles.map(([left, top, size, period, offset]) => `<i style="left:${left}%;top:${top}%;--s:${size}px;--t:${period}s;--d:${offset}s"></i>`).join('')}</div><div class="hero__mountains"></div></div>
        <div class="hero__wash"></div>
        <div class="hero__copy reveal">
          <p class="eyebrow"><span></span> Onchain change intelligence</p>
          <h1>Know what <em>changes</em><br>before you deploy.</h1>
          <p class="hero__lead">Eplyx replays Solana upgrades against validated historical production state to show what changes, who is affected, and the economic consequences before mainnet.</p>
          <div class="hero__actions">
            <a href="/analyse" data-link class="button button--primary">Analyse <span>↗</span></a>
            <a href="#evidence" class="button button--text">See the evidence <span>↓</span></a>
          </div>

        </div>
        <div class="hero__visual reveal">${EplyxCoreScene()}</div>
        <div class="scroll-cue"><span></span> Scroll to inspect the system</div>
      </section>

      <section class="system section" id="how">
        <div class="section-heading reveal">
          <p class="eyebrow eyebrow--dark"><span></span> One reproducible system</p>
          <h2>From onchain change<br>to real consequences.</h2>
          <p>The same historical state. The same action. Two program versions. Every difference stays traceable to execution evidence.</p>
        </div>
        <div class="system-flow reveal">
          ${flow.map(([index, title, body], i) => `<article class="flow-step ${i === 1 ? 'flow-step--engine' : ''}"><span>${index}</span><h3>${title}</h3><p>${body}</p>${i < flow.length - 1 ? '<i>›</i>' : ''}</article>`).join('')}
        </div>
      </section>

      <section class="proofs section section--ink" id="product">
        <div class="section-heading section-heading--light reveal"><p class="eyebrow"><span></span> What Eplyx proves</p><h2>Understand the change.<br>Inspect the consequence.</h2><p>Execution gives a precise answer about tested state. The broader platform connects that answer to change, control and exitability.</p></div>
        <div class="proof-grid reveal">${proofs.map(([n, title, question, body, status]) => `<article><div><span>${n} / ${title}</span><small>${status}</small></div><h3>${question}</h3><p>${body}</p></article>`).join('')}</div>
      </section>

      <section class="result section">
        <div class="section-heading reveal">
          <p class="eyebrow eyebrow--dark"><span></span> A real regression, made legible</p>
          <h2>See the change,<br>not just the code diff.</h2>
          <p>Unexpected economic changes detected. Ten validated Stake Pool interactions expose the deliberately regressed demo candidate’s behavior.</p>
        </div>
        <div class="result-window reveal">
          <div class="window-bar"><span><i></i><i></i><i></i></span><b>eplyx / stake-pool / controlled regression demo</b><em>Exit code 1</em></div>
          <div class="result-summary">
            <div><span class="status-dot status-dot--fail"></span><p>Upgrade check</p><h3>Failed</h3></div>
            <div><p>Corpus</p><h3>10</h3><span>validated interactions</span></div>
            <div><p>Findings</p><h3>10</h3><span>4 critical · 1 high · 5 warning</span></div>
            <a href="/runs/demo" data-link>Inspect evidence <span>↗</span></a>
          </div>
          <div class="findings">
            <article class="finding finding--critical"><div><span>Critical</span><span>Unexpected</span></div><h3>WithdrawSol</h3><p>transaction now reverts</p><dl><dt>Affected</dt><dd>4 / 4 measurable observations</dd><dt>Execution</dt><dd>success → error</dd></dl></article>
            <article class="finding finding--high"><div><span>High</span><span>Unexpected</span></div><h3>DepositSol</h3><p>pool_tokens_received decreased</p><dl><dt>Affected</dt><dd>1 / 6 measurable observations</dd><dt>Maximum delta</dt><dd>21 bps</dd></dl></article>
          </div>
        </div>
        <p class="demo-note">Controlled demonstration: the candidate implements DepositSol only, so WithdrawSol fails. These results describe the tested fixture and corpus.</p>
      </section>

      <section class="expectations section">
        <div class="section-heading reveal"><p class="eyebrow eyebrow--dark"><span></span> Intent is explicit</p><h2>Intentional changes<br>should be explicit.</h2><p>Severity and expectedness are independent. An expected Critical change remains Critical. Eplyx does not hide impact simply because it was intentional.</p></div>
        <div class="expectation-grid reveal">
          <article class="expectation expectation--pass"><div><span>Critical</span><span>Expected</span><em>CI may pass</em></div><h3>WithdrawSol</h3><p>transaction now reverts</p><blockquote>“WithdrawSol intentionally removed in v3”</blockquote><dl><dt>Affected</dt><dd>4 / 4 measurable observations</dd><dt>Bounds</dt><dd>Within declared bounds</dd></dl></article>
          <div class="versus"><span></span><b>VS</b><span></span></div>
          <article class="expectation expectation--fail"><div><span>High</span><span>Unexpected</span><em>CI fails</em></div><h3>DepositSol</h3><p>pool_tokens_received decreased</p><blockquote>No matching declaration</blockquote><dl><dt>Affected</dt><dd>1 / 6 measurable observations</dd><dt>Maximum delta</dt><dd>−21 bps</dd><dt>Declaration</dt><dd>Not declared</dd></dl></article>
        </div>
      </section>

      <section class="evidence section section--violet" id="evidence">
        <div class="section-heading section-heading--light reveal">
          <p class="eyebrow"><span></span> Inspectable by construction</p>
          <h2>Evidence, not a score.</h2>
          <p>Every result traces back to reproducible execution evidence. Inspect the historical source, execution outputs and content-addressed provenance.</p>
        </div>
        <div class="evidence-console reveal">
          <aside><span>Observation</span><strong>DepositSol / historical observation</strong><span class="evidence-outline">Historical source<br>Execution<br>Economic quantities<br>Provenance</span><small>V1 fidelity <b>Matched</b></small></aside>
          <div class="evidence-body">
            <div class="observation-head"><div><span>Source</span><b>Mainnet</b></div><div><span>Slot</span><b>447454329</b></div><div><span>Action</span><b>DepositSol</b></div><div><span>Fidelity</span><b class="matched">Matched</b></div></div>
            <div class="comparison"><article><span>Baseline</span><h3>Transaction success</h3><p>pool_tokens_received</p><strong>Measured user output</strong></article><div><i>→</i><b>Economic diff</b></div><article><span>Candidate</span><h3>Transaction success</h3><p>pool_tokens_received</p><strong>Compared user output</strong></article></div>
            <div class="impact-line"><span>Named semantic difference</span><b>pool_tokens_received decreased</b><em>Unexpected</em></div>
            <div class="hashes"><span>baseline SHA <b>Program binary</b></span><span>candidate SHA <b>Tested binary</b></span><span>corpus SHA <b>Validated records</b></span><span>bundle SHA <b>Replay inputs</b></span></div>
          </div>
        </div>
        <div class="evidence-note"><span>Evidence structure preview · hash values belong to an individual run.</span><a href="/runs/demo" data-link>View demo report ↗</a></div>
      </section>

      <section class="roles section" id="roles">
        <div class="roles-intro reveal"><p class="eyebrow eyebrow--dark"><span></span> Built for protocol teams</p><h2>Who is<br>Eplyx for?</h2><p>One execution truth. Different views for the people responsible for protocol change.</p><div class="role-architecture"><span>CHANGE</span><i>↓</i><span>REPLAY</span><i>↓</i><strong>CONSEQUENCE</strong><i>↓</i><span>EXECUTION PROOF</span></div></div>
        <div class="role-list">${roles.map(([n, title, question, body, tags, note, icon]) => `<article class="role-row reveal"><div class="role-heading"><span class="role-icon" aria-hidden="true">${icon}</span><span>${title}</span><small>${n}</small></div><h3>“${question}”</h3><p>${body}</p><ul class="role-tags">${tags.map(tag => `<li>${tag}</li>`).join('')}</ul><small class="role-note">${note}</small></article>`).join('')}</div>
      </section>

      <section class="coverage section section--paper">
        <div class="coverage__copy reveal"><p class="eyebrow eyebrow--dark"><span></span> Coverage stays visible</p><h2>Coverage you<br>can inspect.</h2><p class="coverage-context">Example claim for a passing run</p><p class="quote">“No unexpected economic changes were detected across the tested validated historical corpus.”</p><p>A passing run makes a precise claim about what was tested. It does not turn incomplete history into false certainty.</p></div>
        <div class="coverage-panel reveal">
          <div class="coverage-total"><span>Validated historical corpus</span><strong>10</strong><em>observations</em></div>
          <div class="coverage-bars"><p><span>DepositSol</span><b>6</b></p><i><span style="width:60%"></span></i><p><span>WithdrawSol</span><b>4</b></p><i><span style="width:40%"></span></i></div>
          <div class="limitations"><span>Known limitations</span><ul><li>Jito-tipped DepositSol underrepresented</li><li>Failed historical originals unsupported</li><li>Account-creation paths unsupported</li><li>Real LUT-backed v0 messages unsupported</li></ul></div>
        </div>
      </section>

      <section class="vision section section--ink" id="vision">
        <div class="section-heading section-heading--light reveal"><p class="eyebrow"><span></span> The longer arc</p><h2>One engine for<br>every onchain change.</h2><p>Eplyx starts with program upgrades. The same evidence model can expand to other governed changes without presenting that future as today’s product.</p></div>
        <div class="vision-line reveal">
          <article class="active"><span>Today</span><i></i><h3>Program upgrades</h3><ul><li>Historical replay</li><li>Economic impact</li><li>Expected-change review</li><li>CI gating</li></ul><p>Available now</p></article>
          <article><span>Expanding</span><i></i><h3>Change & control</h3><ul><li>Governance proposals</li><li>Authority / privilege changes</li><li>Protocol parameters</li><li>Exitability</li></ul><p>Planned</p></article>
          <article><span>Longer term</span><i></i><h3>Continuous intelligence</h3><ul><li>Change monitoring</li><li>Post-deployment verification</li><li>Economic exposure alerts</li></ul><p>Vision</p></article>
        </div>
        <div class="vision-model reveal"><div><span>Changes</span><p>Who can change? · What can change?<br>When? · What changed?</p></div><i>→</i><strong>EPLYX</strong><i>→</i><div><span>Consequences</span><p>What happens? · Who is affected?<br>How much value? · Can users still exit?</p></div><i>→</i><b>Execution<br>proof</b></div>
        <div class="final-cta reveal"><div><h2>Understand the<br>consequence of change.</h2><p>Replay candidate upgrades against real production-derived state before they reach users.</p></div><div class="final-actions"><a href="/analyse" data-link class="button button--light">Analyse <span>↗</span></a><a href="#evidence" class="button button--text">View evidence <span>↑</span></a></div></div>
      </section>
    </main>${Footer()}`;
}
