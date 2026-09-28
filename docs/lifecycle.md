# Lifecycle analysis

A lifecycle ChangeSpec declares a policy transition over an immutable state
snapshot. Eplyx evaluates consequences and separate assurance questions without
turning issuer notices into chain proof or inventing a valuation.

```sh
eplyx change lifecycle --scenario scenario.json --out change.json
eplyx lifecycle analyse --snapshot snapshot.json --scenario scenario.json \
  --change-spec change.json --at 2031-01-01T00:00:00Z \
  --format json --out report.json
```

Inputs and dates are explicit. `--before` sets the comparison time; its default is
one nanosecond before the scenario's effective date. `--record PROJECT_DIRECTORY`
adds the exact result to an existing local project history. Outputs are exclusive,
so rerunning cannot replace historical evidence. These commands execute offline
in children with empty environments. Two evaluations of identical inputs produce
identical analytical bytes.

A public snapshot describes accounts at their recorded observations. Hypothetical
times change declared policy, not those account bytes. An issuer notice can produce
a scenario through the generic notice interface, with source identity retained;
it cannot establish issuer eligibility, official conversion, KYC or keys.

Keep these distinctions when reading an analysis:

- PreEvent is not Ready. Deadline passage alone does not establish Blocked.
- Ready, Blocked and Incomplete are policy findings within the declared question.
- Unsupported is an executor or evidence boundary, never proof that a path does
  not exist. Unknown eligibility stays unknown.
- Transfer, market exit, liquidity withdrawal, redemption and official transition
  have independent evidence. A pool vault does not establish LP ownership.
- Principal removal, fee collection and position closure remain separate facts.

`readiness`, `resolve-paths`, `evaluate-rollout`, `guard-rollout` and
`compare-scenarios` expose the assurance layer. Their inputs name exact saved
observations and executions. Guarded demonstrations write only explicit local
markers, never cluster transactions. Consult each subcommand's `--help` for its
input files. Archived SPACEX evidence is a reference fixture, not a default for
current tokens or newly observed accounts.

Hosted user-proposed scenarios use the same rules. They replay selected checks
against the exact observation and focused account, then display pre-event,
mobility, candidate execution and full-transition questions separately. Population
readiness is not inferred from a candidate check. See [current-state analysis](current-state-analysis.md),
[T6](phase-t6-lifecycle-kind.md) and [its reference identities](phase-t6-identity-reference.json).

## Guided prepared lifecycle flow

An authenticated workspace project exposes a focused browser entry at
`/p/{project_id}/analyse/lifecycle` when its server-authored
`lifecycle_change` capability can submit. The route prepares the same lifecycle
ChangeSpec, scenario, immutable snapshot and explicit comparison options used by
the existing hosted check endpoint. It does not add a browser evaluator or an
alternate lifecycle contract.

The supported browser shape deliberately remains small:

- the policy transition is `Active` to `TransitionRequired`;
- eligibility stays `unknown` and no ratio or conversion mechanism is declared;
- a successor and a post-effective deadline are optional declared terms;
- the scenario source is an explicit user-provided `ScenarioAssumption`, not
  issuer proof;
- the comparison is exactly one second before the effective time versus the
  effective time; and
- no execution or readiness checks are retained in this consequence-only job.

The preview separates three kinds of information. Declared terms are the user's
hypothetical policy and provenance. Observed/prepared input is the selected
snapshot file, uploaded without browser rewriting and shown with a SHA-256 of
its exact bytes. Derived lifecycle consequences do not exist until the server
has validated and bound the documents and the engine has evaluated them. In
Technical mode the preview displays the exact submitted ChangeSpec, scenario,
analysis options and snapshot-byte identity.

Submission uses `POST /v1/projects/{project_id}/checks` and the existing async
job and run route. The server remains authoritative for capability revalidation,
multipart parsing, scenario normalization, ChangeSpec-to-scenario binding,
snapshot validation, asset matching and all analysis results. Input rejection,
stale project readiness, authentication failure and job-creation failure leave
the browser draft intact. An accepted response is checked against the proposed
asset and optional successor before navigation to the returned run.
