# Suggested next product steps

These are engineering priorities based on the current implementation, not claims
of shipped functionality or validated market demand. The workflow selector and
CLI documentation improve discovery; the next steps should reduce the distance
between choosing a question and getting a useful, reproducible answer.

The first-analysis foundation is complete in the reviewed source: the repository contains one
clean-checkout canonical offline example, its candidate build and identity are
protected on `macos-15`, and repository CI verifies the bundle, control,
regression and explicit-input portability. A separate release workflow now
builds, smoke-tests and packages a versioned `aarch64-apple-darwin` CLI archive
with a SHA-256 checksum; availability depends on actually published matching releases; this local review does not verify release publication. Broader platform builds, reproducible CLI-build proof, signing,
notarization, package-manager distribution and lower-friction installation
remain future work.

| Priority | Opportunity | Concrete next increment | Success signal |
| --- | --- | --- | --- |
| 1 | Lower-friction installation | Publish the first owner-approved macOS arm64 release, then evaluate signing, notarization and package-manager delivery from observed user friction. Keep other platforms unsupported until independently verified. | A macOS arm64 user verifies and runs the published CLI without compiling the workspace; no unsupported target is implied. |
| 2 | Improve recovery from missing readiness | Server-authored capability guidance is implemented. Make its next actions easier to follow and validate operator setup with users; retain project access, bundle and provider/mechanism boundaries. | The user knows what can run and what is missing before filling in a form. Errors name an actionable next step rather than sending users between Projects and Workspace. |
| 3 | Validate the prepared browser workflows | Migration and lifecycle forms, Token-2022 fee preview and bounded code/fee interaction are implemented. Test onboarding with prepared user inputs and improve the instructions where users get stuck. | The same inputs reach equivalent canonical engine results through the CLI and the web flow; unsupported inputs are explained before expensive execution. |
| 4 | A useful protocol-team CI pilot | Onboard a small number of teams around one supported upgrade workflow. Prepare and review their evidence bundles, integrate the existing submission client and track how reports affect real reviews. | Teams use the check repeatedly; they can explain a failing result and distinguish candidate regressions from missing evidence. Prioritise observed friction and gaps over protocol count. |
| 5 | Validate the implemented governance review trail | The retained paginated trail is implemented. Exercise the full reviewer journey with concrete proposals and improve freshness explanations and evidence navigation. | A reviewer can follow one proposal from candidate identity to execution evidence without treating a dated match as permanent or an Executed status as byte-level proof. |
| 6 | Expand coverage from concrete requests | Select a requested protocol/action shape, qualify historical state/runtime evidence, add bounded semantics and publish its precise supported/unsupported cases. | A previously blocked user question becomes reproducibly answerable. A new adapter name alone is not a coverage milestone. |

Keep one engine and one authoritative analytical record path. Website guidance,
CLI commands and CI integrations should expose those capabilities with different
levels of assistance. Do not add a second browser evaluator or infer availability
from marketing labels.

Continuous monitoring can later schedule bounded acquisition and analysis, append
new evidence and notify reviewers. It should follow trustworthy freshness and
coverage handling; it is not part of the current public offering. Live-provider
qualification remains separate, explicitly authorised work.
