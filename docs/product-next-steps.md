# Suggested next product steps

These are engineering priorities based on the current implementation, not claims
of shipped functionality or validated market demand. The workflow selector and
CLI documentation improve discovery; the next steps should reduce the distance
between choosing a question and getting a useful, reproducible answer.

| Priority | Opportunity | Concrete next increment | Success signal |
| --- | --- | --- | --- |
| 1 | A complete first analysis | Ship a small, licensed, self-contained offline example with every required input and a known control/regression pair. Add versioned CLI release binaries with checksums and platform-specific installation instructions. | A new user reaches the expected report from a clean machine without locating missing captures or building the analysis engine. |
| 2 | Explain readiness before submission | Use authenticated server capability/readiness responses to show project access, active bundle, supported analysis kinds and missing provider/mechanism configuration. Keep provider secrets and filesystem paths out of the browser. | The user knows what can run and what is missing before filling in a form. Errors name an actionable next step rather than sending users between Projects and Workspace. |
| 3 | Guided web inputs over the existing engine | Add browser flows for prepared migration and lifecycle submissions, with file/schema validation, a preview of the exact proposal and an asynchronous run link. Retain the existing input, size and identity checks. | The same inputs reach equivalent canonical engine results through the CLI and the web flow; unsupported inputs are explained before expensive execution. |
| 4 | A useful protocol-team CI pilot | Onboard a small number of teams around one supported upgrade workflow. Prepare and review their evidence bundles, integrate the existing submission client and track how reports affect real reviews. | Teams use the check repeatedly; they can explain a failing result and distinguish candidate regressions from missing evidence. Prioritise observed friction and gaps over protocol count. |
| 5 | A complete governance review trail | Make it easy to retain the pre-execution Squads binding, inspect the associated impact report, re-check freshness and append a later deployment attestation. | A reviewer can follow one proposal from candidate identity to execution evidence without treating a dated match as permanent or an Executed status as byte-level proof. |
| 6 | Expand coverage from concrete requests | Select a requested protocol/action shape, qualify historical state/runtime evidence, add bounded semantics and publish its precise supported/unsupported cases. | A previously blocked user question becomes reproducibly answerable. A new adapter name alone is not a coverage milestone. |

Keep one engine and one authoritative analytical record path. Website guidance,
CLI commands and CI integrations should expose those capabilities with different
levels of assistance. Do not add a second browser evaluator or infer availability
from marketing labels.

Continuous monitoring can later schedule bounded acquisition and analysis, append
new evidence and notify reviewers. It should follow trustworthy freshness and
coverage handling; it is not part of the current public offering. Live-provider
qualification remains separate, explicitly authorised work.
