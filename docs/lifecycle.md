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
