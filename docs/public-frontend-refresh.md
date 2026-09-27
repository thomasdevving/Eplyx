# Public frontend refresh

The public site now describes the integrated Eplyx implementation instead of only
the original upgrade demo. It adds `/cli` with source-build instructions, copyable
commands and the input requirements for each workflow. No prebuilt binary or
installer is advertised. The existing upgrade form, project console, demo report
and analytical report contracts remain in place.

The owner authorised pushing the completed integration and this frontend change
to `main`. Work used an isolated checkout of the pushed `t-token-migration` branch
at `a3eac9d`, because the original local checkout was not accessible to this
session. No source repository, historical evidence, provider configuration or
hosted infrastructure was edited.

## Product claims

| Public description | Implementation and boundary |
| --- | --- |
| Program upgrades | MAIN retained-state comparison, semantic findings and expectation gate; historical execution requires supported replay paths and fidelity |
| Token migrations | T3–T5 local VM execution, holder coverage, reconciliation, gates, search and reproduction; no signing authorisation inferred |
| Lifecycle changes | T6 declared policy over retained state; unknown eligibility and independent assurance paths remain explicit |
| Current paths | T7 bounded Transfer, Meteora exit and exact liquidity withdrawal; hosted acquisition requires configuration and was validated with mock providers during migration |
| Squads governance | G1/G2 read-only binding and attestation for the supported single loader-v3 upgrade; broader governance remains future work |
| Local and hosted use | T8 dashboard and T9 durable runs, workspace identity and exact-byte sync; hosted features depend on operator configuration |
| Next validation | Bounded read-only mainnet validation of the migrated current-state flow; not claimed as completed |
| Future scope | Partial/claim-based migrations, local-validator plan validation and wider protocol/governance coverage; no dates or current-coverage claim |

The repeated role, proof, expectation and coverage marketing sections were
consolidated into the current capabilities, method, one labelled recorded demo,
CLI setup and roadmap. The blocking intro and content-fade dependency were
removed. The hero keeps MAIN's artwork with corrected capability labels; fonts
are the existing licensed local assets. Workspace links use the configured API
origin, so they do not accidentally enter the static site's route fallback.

## Verification

All results below were observed on this frontend change with pnpm 11.24.0 and
Node 22.23.1:

- `pnpm install --frozen-lockfile`: passed. An initial offline attempt reported a
  missing cached package; the normal frozen installation succeeded without a
  lockfile change.
- `make programs`: passed with the pinned SBF toolchains. The lending baseline
  remained `b307faa27e09fc74fee9e8087263f249bef565a6998f49fd5e562d2932bf1641`.
- `pnpm check:frontend`: passed, including the existing report, console, identity,
  impact, governance, dashboard, catalogue and cloud presentation checks.
- `pnpm verify:report`: passed after supplying the Rust toolchain on PATH and
  building the local test programs. The initial invocation failed because Cargo
  was absent from PATH. Final values remained 141 fixtures, 89 identical,
  52 changed, 11 critical and $6,182,370 collateral represented.
- `pnpm verify:governance`: passed, including independent G1/G1.1 identities.
- `pnpm build`: passed; browser checks served `dist` through the production static
  server, including direct route reloads.
- `pnpm exec playwright test --config playwright.public.config.js`, with
  `EPLYX_CHROME` set: all nine tests passed on the final layout. Five widths
  (1440, 1024, 834, 390 and 320) covered navigation, direct anchors, visible content,
  transitions and horizontal overflow. Other cases covered exact copying and
  clipboard-denial fallback, keyboard menu dismissal, persistent presentation
  mode, configured workspace links and existing demo/upgrade routes.
- Desktop and mobile screenshots were inspected. A narrow nested CLI column was
  corrected and the complete public browser suite passed again. Tests requested
  no external resources; the public fonts loaded locally.
- The freshly built CLI accepted nineteen help/version invocations covering the
  displayed command paths. A disposable `init --migration --fixture` invocation
  created the documented three template files. No provider acquisition, login,
  sync upload or cluster transaction was invoked by these checks.
- Six linked product guides resolve in the checkout. Public renders and the diff
  were checked for malformed output, machine paths and whitespace errors.

The full Rust and Postgres-backed suites were not repeated for this presentation
change. Their completed T10 results remain in
[the integration verification](phase-t10-verification.json). Browser exercises of
the new public routes do not claim a production database, observation provider or
migration candidate is configured. No new release packaging, infrastructure or
provider access was performed. A successful Git push is recorded separately from
any downstream deployment result.
