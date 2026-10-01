import { Header, Footer, workspaceHref } from './shell.js';

const workflows = [
  {
    id: 'upgrades', name: 'Program upgrade', hint: 'A new program build',
    question: 'What changes if I deploy this build?',
    when: 'Use this when you have a compiled Solana program upgrade to compare with a validated baseline.',
    inputs: 'A candidate .so and a validated bundle for the target program. The web form also needs access to a hosted project with an active bundle. Expected-change declarations are optional.',
    result: 'A before/after report, execution checks, measured effects and a gate result within the bundle’s coverage.',
    boundary: 'Selected protocols and replay paths only. A passing check is not approval to deploy.',
    web: { label: 'Open upgrade form', href: '/analyse', internal: true, note: 'For a one-off check in an already configured hosted project.' },
    cli: 'upgrades', cliNote: 'For local builds and offline checks. Use CI to repeat the same gate for each candidate.',
    technical: 'The bundle pins the baseline, retained interactions and dependencies. The candidate is built outside Eplyx; the engine verifies baseline replay before comparing results.',
  },
  {
    id: 'migration', name: 'Token migration', hint: 'A replacement mechanism',
    question: 'Does this migration account for the holders and funds?',
    when: 'Use this when you have a proposed token migration mechanism and want to rehearse it before execution.',
    inputs: 'Migration terms, a captured or synthetic world, a compatible SBF candidate and local project configuration. Creating templates alone is not enough to run the analysis.',
    result: 'Holder coverage, funding and fee checks, balance reconciliation, gate findings and saved counterexamples when found.',
    boundary: 'Results apply to the supplied world and mechanism. They do not establish issuer authorisation or signing possession.',
    web: { label: 'Open workspace', href: workspaceHref(), note: 'In a configured project, choose Analyse → Token migration. The guided form accepts a prepared candidate, state descriptor and its fixture or captured-world file within the supported migration shape. Pinned program captures still require the CLI/API.' },
    cli: 'migration', cliNote: 'Start here for a full local rehearsal, saved-run search and offline reproduction.',
    technical: 'The candidate executes in a local VM. The migration gate, invariant results and exact accounting remain tied to the recorded inputs; search coverage is reported separately.',
  },
  {
    id: 'lifecycle', name: 'Lifecycle terms', hint: 'A deadline or policy change',
    question: 'Who is affected when these terms change?',
    when: 'Use this for declared policy changes, effective times, deadlines or a proposed replacement mint.',
    inputs: 'A retained snapshot, an explicit scenario and an evaluation time. The prepared browser form needs a snapshot; a workspace observation scenario uses its selected observation and retained checks.',
    result: 'Consequences of the declared terms, with unknown eligibility and untested execution kept visible.',
    boundary: 'Changing policy time does not refresh account state. Declared terms are not proof of an official transition.',
    web: { label: 'Open workspace', href: workspaceHref(), note: 'In a configured project, choose Analyse → Lifecycle change to upload a prepared snapshot and declare the supported policy transition. Proposed scenario is a separate route from a retained observation.' },
    cli: 'lifecycle', cliNote: 'Use your own snapshot and scenario files, compare times and retain the output locally.',
    technical: 'The lifecycle ChangeSpec identifies the scenario. Policy findings, observed protocol state and candidate execution are separate assurance dimensions.',
  },
  {
    id: 'paths', name: 'Current token path', hint: 'Transfer, exit or withdrawal',
    question: 'Can this exact account use this path?',
    when: 'Use this for a selected mint and public owner when you need evidence for one specific token path.',
    inputs: 'An exact token account, path terms and recorded state and code. The hosted flow acquires a new observation through an operator-configured provider.',
    result: 'Path-specific availability, execution and reconciliation, or an explicit reason why the path could not be evaluated.',
    boundary: 'A successful Transfer does not prove market exit. New observations start with untested paths. Fresh live-provider validation remains work ahead.',
    web: { label: 'Open workspace', href: workspaceHref(), note: 'For mint/owner inspection, Transfer and bounded Meteora market-exit checks when the observation service is configured.' },
    cli: 'paths', cliNote: 'Replay saved captures and run exact path probes. Liquidity-withdrawal checks currently use the CLI.',
    technical: 'Provider acquisition and offline VM execution are distinct operations. Public account ownership does not demonstrate control of a signing key.',
  },
  {
    id: 'parameters', name: 'Protocol fee change', hint: 'A fee without a new build',
    question: 'How does this fee change affect a retained action?',
    when: 'Use this for either of the two supported fee operations: active Token-2022 transfer-fee bps or a retained Stake Pool SOL deposit fee.',
    inputs: 'An exact parameter ChangeSpec and eligible retained evidence. Token-2022 uses a capture or typed transfer input; Stake Pool uses a qualified historical bundle and an explicit DepositSol record.',
    result: 'Independent baseline/proposed executions, reconciled raw token effects, or a precise unsupported or unavailable reason.',
    boundary: 'Token-2022 derives the active fee field; Stake Pool simulates SetFee with an assumed manager signer. Neither establishes authority, activation or coverage of other holders.',
    web: { label: 'Open workspace', href: workspaceHref(), note: 'Open an eligible retained Token-2022 transfer run and choose Parameter Change. Stake Pool parameter submission uses the authenticated API or CLI; there is no standalone guided fee form.' },
    cli: 'parameters', cliNote: 'Analyse and reproduce one exact case. Local amount search and selected case sets require a current compatible CLI build and their own retained inputs.',
    technical: 'For code and fee together, a retained upgrade run can offer Compare code and fee. This bounded Stake Pool experiment compares four action cells under qualified code, with independent configuration executions and fixed Clock.',
  },
  {
    id: 'governance', name: 'Squads upgrade proposal', hint: 'Bind a proposal to evidence',
    question: 'Does this proposal match the upgrade that was analysed?',
    when: 'Use this to bind or re-check a supported Squads V4 upgrade proposal, or attest its deployment after execution.',
    inputs: 'An analysed upgrade ChangeSpec, a multisig and transaction index, and read-only RPC access. Deployment attestation also needs the sealed pre-execution binding and exact candidate bytes.',
    result: 'A dated proposal match or mismatch, or a deployment attestation with unsupported and unverifiable outcomes made explicit.',
    boundary: 'One supported loader-v3 program upgrade only. Eplyx does not sign, approve or execute the proposal.',
    web: { label: 'Open project reports', href: '/projects', internal: true, note: 'Review existing governance evidence in a project’s run report. New checks use the CLI or the configured hosted API.' },
    cli: 'governance', cliNote: 'Acquire, bind and verify the proposal; attest deployment only when the required prior evidence exists.',
    technical: 'G1 binds the proposal and buffer to the analysed candidate. G2 separately attributes execution and deployed bytes; proposal status alone is not deployment evidence.',
  },
];

const selectedId = () => workflows.find(item => `#${item.id}` === location.hash)?.id ?? 'upgrades';

export function StartPage() {
  const selected = selectedId();
  return `<main id="main" class="inner-page start-page">${Header({ light: true })}
    <section class="guide-intro start-intro"><p class="eyebrow"><span></span> Try Eplyx · In development</p><h1>Start with your question.</h1><p>Choose what you want to check. Each workflow explains when to use it, what you need and whether to start in the browser or your terminal.</p>
      <div class="start-demo"><div><strong>Just exploring?</strong><p>Read a saved upgrade report. No account, installation or files needed.</p></div><a href="/runs/demo" data-link class="button button--light">Explore a demo <span>↗</span></a></div>
    </section>
    <section class="start-workflows" aria-label="Choose an analysis">
      <fieldset class="workflow-picker"><legend>What are you checking?</legend>${workflows.map(item => `<label><input type="radio" name="workflow" value="${item.id}" data-start-choice ${item.id === selected ? 'checked' : ''} aria-controls="start-${item.id}"><span><strong>${item.name}</strong><small>${item.hint}</small></span></label>`).join('')}</fieldset>
      <div class="workflow-panels">${workflows.map(item => `<section id="start-${item.id}" class="workflow-panel" aria-labelledby="title-${item.id}" ${item.id !== selected ? 'hidden' : ''}>
        <p class="eyebrow"><span></span> ${item.name}</p><h2 id="title-${item.id}">${item.question}</h2><p class="workflow-when">${item.when}</p>
        <dl class="workflow-requirements"><div><dt>Bring</dt><dd>${item.inputs}</dd></div><div><dt>You get</dt><dd>${item.result}</dd></div></dl>
        <p class="workflow-boundary">${item.boundary}</p>
        <div class="workflow-entrypoints"><article><h3>In the browser</h3><p>${item.web.note}</p><a href="${item.web.href}" ${item.web.internal ? 'data-link' : ''} class="button button--light">${item.web.label} <span>↗</span></a></article><article><h3>With the CLI</h3><p>${item.cliNote}</p><a href="/cli#${item.cli}" data-link class="text-link">CLI commands and inputs ↗</a></article></div>
        <p data-technical class="workflow-technical">${item.technical}</p>
      </section>`).join('')}</div>
    </section>
    <aside class="start-next"><div><h2>Preparing inputs or automating checks?</h2><p>Engineers can prepare evidence and run CI from the terminal. Reviewers can use the resulting report without installing the CLI.</p></div><div><a href="/cli#prepare" data-link class="text-link">Prepare a bundle ↗</a><a href="/cli#ci" data-link class="text-link">Add an upgrade gate to CI ↗</a><a href="/cli#install" data-link class="text-link">Install the CLI ↗</a></div></aside>
  </main>${Footer()}`;
}

export function attachStart() {
  document.querySelectorAll('[data-start-choice]').forEach(input => input.addEventListener('change', () => {
    if (!input.checked) return;
    document.querySelectorAll('.workflow-panel').forEach(panel => { panel.hidden = panel.id !== `start-${input.value}`; });
    history.replaceState({}, '', `/start#${input.value}`);
  }));
}
