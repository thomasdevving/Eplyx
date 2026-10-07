# Eplyx documentation

New to Eplyx? [Choose a workflow](getting-started.md). To run your first meaningful
check without a provider or account, use the [offline Stake Pool example](../examples/stake-pool-upgrade/README.md).
The public `/runs/demo` page displays its retained regression report without
installation. Neither route covers every Solana program or account.

## Find the guide for your task

| I want to… | Read | What I need |
| --- | --- | --- |
| Install the CLI and run a first check | [Getting started](getting-started.md) | Rust/native build tools, or an actually published macOS arm64 archive; the example also needs its separately built SBF fixture. |
| Check a proposed program build | [Upgrade example](../examples/stake-pool-upgrade/README.md), [CI contract](phase-10-hosted-ci.md) | Compiled candidate and validated retained bundle. |
| Automate a hosted upgrade check | [Pilot onboarding](pilot-onboarding.md) | Configured service, project, active bundle and project token. |
| See what a project still needs, comment results on PRs, watch the queue, back up and restore | [Operations](operations.md) | Operator token for the queue view; `pg_dump`/`pg_restore` for backups. |
| Rehearse a token migration | [Token migration](token-migration.md) | Proposal, state and exact compatible candidate. Prepared browser files are supported within the documented shape. |
| Compare declared lifecycle terms | [Lifecycle](lifecycle.md) | Immutable snapshot and explicit policy/times. Execution remains a separate question. |
| Check a transfer, exit or withdrawal | [Current-state analysis](current-state-analysis.md) | Exact account/path inputs and retained state/code; new acquisition needs configured provider access. |
| Evaluate a supported fee proposal | [Fee walkthrough](parameter-analysis-guide.md) | Exact declaration and operation-specific retained input. |
| Compare code and fee effects together | [Interaction](upgrade-parameter-interaction.md) | Qualified Stake Pool candidate, two proposals and one retained deposit. |
| Rehearse upgrade/fee rollout order with an installed upgrade | [Rollout rehearsal](rollout-rehearsal.md) | Qualified rollout candidate, the two existing proposals and one retained deposit; signer assumptions are explicit. |
| Compare two migration transaction orders | [Migration order](migration-order-case.md) | Eligible retained migration and two selected source accounts. |
| Review a Squads upgrade | [Binding](phase-g1-squads-governance-binding.md), [deployment attestation](phase-g2-squads-deployment-attestation.md) | Supported proposal, exact candidate evidence and read-only RPC. |
| Review or share saved results | [Dashboard](dashboard.md), [cloud sync](cloud.md) | Saved local records; hosted sharing needs workspace access. Sync does not rerun analysis. |
| Prepare historical evidence | [Bundle qualification](bundle-qualification.md) | Declared scope and suitable historical providers; discovery alone is not replay evidence. |

## Understand the result

- **Retained state / capture:** recorded account and program evidence. It is dated;
  reading it does not refresh chain state.
- **ChangeSpec:** the exact proposed change, identified independently of display
  labels. The analysis additionally binds its inputs and runtime.
- **Bundle:** validated historical records with their baseline and dependencies.
  Its corpus is the set of interactions evaluated, not all protocol activity.
- **Raw units:** integer token amounts before decimal display. Preserve them as
  strings where the schema requires strings; do not estimate a USD value.
- **Gate passed:** the evaluated scope met the selected declarations/policy.
  It does not approve deployment or establish signing authority.
- **No observed consequence:** equal measured outputs for this exact evaluated
  case. Other accounts, amounts and times remain outside that result.
- **Unsupported / unavailable / not established:** the declared question could
  not be answered under the retained evidence or execution contract. It does not
  establish that an on-chain path is impossible or safe.
- **Verify versus reproduce:** verification checks retained bindings and
  consistency; reproduction executes the saved experiment again with a
  compatible runtime. Viewing a report performs neither acquisition nor execution.

## Source, service and development status

These guides describe the reviewed implementation. Hosted availability is decided
by the configured service's capabilities and each run's eligibility. They do not
attest the current contents or readiness of a remote deployment.

The fixed [Step 11A review](review-environment.md) is a pinned build. The current source includes the locally verified
`codex/analysis-integration` consolidation: amount search, selected parameter cases
and an upstream source-built candidate rehearsal. They have no hosted entry and
do not update the pinned review environment. Older binaries may lack them.
[Their walkthrough](parameter-analysis-guide.md#local-integration-extensions)
states the required build and private evidence prerequisites.

## Engineering and historical evidence

Files named `phase-*`, qualification JSON, design proposals and dated audits are
implementation/evidence records at their named revision. Read their date, source
identity and limitations before applying them to a new run. They are not the
installation guide, and an old "next step" is not a current feature inventory.
Historical evidence is preserved rather than rewritten to look current.

- [Architecture map](universal-replay-architecture.md)
- [Historical clean-checkout audit](clean-checkout-reproducibility-audit.md)
- [Product next steps](product-next-steps.md)
- [Public frontend record](public-frontend-refresh.md)
- [Website notices and open operator facts](website-legal.md)
- [Pinned STA archive and imported fixture requirements](../ARCHIVE.md)

If a check cannot run, first check `eplyx version --json`, command `--help`, input
availability and the report's exact reason. Missing private captures are an input
prerequisite; installation does not fabricate or download them automatically.
