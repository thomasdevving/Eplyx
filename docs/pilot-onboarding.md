# Getting Eplyx into your CI

For a protocol engineer with an existing Solana program. Budget under an hour
once we have prepared your first bundle.

You do not need to understand historical-state acquisition, corpus selection or
replay fidelity to do this. Those happen on our side, before your first pull
request.

## What you are getting

Every pull request that changes your program builds a candidate `.so`. Eplyx
replays a pinned set of **real historical mainnet transactions against your
program** — real accounts, real pool state, real amounts, at the slot they
happened — under both the deployed version and your candidate, and fails the
check if user-visible economics change in a way you did not declare.

## 1. We prepare your bundle

Send us your program ID. We build and verify the first validated corpus and
activate it for your project, then send you:

- a **project id** (e.g. `stake-pool`)
- a **CI token** — shown once, store it immediately

## 2. Add the token and project id to your repository

- `EPLYX_TOKEN` as a **repository secret**
- `EPLYX_PROJECT` and `EPLYX_URL` as **repository variables**

## 3. Add the client script

Copy `eplyx-check.sh` into `scripts/`. It uploads the candidate, fetches the
reports, writes the GitHub summary and returns Eplyx's gate code. It contains no
analysis logic.

## 4. Add the workflow

Copy `eplyx-upgrade-impact.yml` into `.github/workflows/`, and change the
candidate path to your program's artefact:

```yaml
run: ./scripts/eplyx-check.sh target/deploy/your_program.so
```

Your program is built in **your** runner. Eplyx never clones your repository and
never runs your build.

## 5. Create an empty expectation file

```toml
# .eplyx/expected-changes.toml
version = 1
```

Declaring nothing is the correct starting point. It means every behavioural
change is unexpected, which is what you want until you deliberately intend one.

## 6. Open a pull request

The check runs and posts a report to the job summary. A clean upgrade reports:

> No unexpected economic changes were detected across the tested validated
> historical corpus.

## 7. When a change is intentional, declare it — narrowly

Suppose you raise the deposit fee, and the check fails with:

```text
HIGH / UNEXPECTED
spl-stake-pool/deposit_sol/economic/pool_tokens_received/decreased
Affected: 5 of 6 measurable observations, 5 economic entities
Largest change: -21 bps
```

Declare exactly that, with bounds and a reason:

```toml
version = 1

[[change]]
protocol = "spl-stake-pool"
action   = "deposit_sol"
domain   = "economic"
subject  = "pool_tokens_received"
change   = "decreased"

max_delta_bps             = 25
max_affected_observations = 6

reason = "Approved deposit fee increase from 0.10% to 0.25% (governance #142)"
```

Push again and the finding becomes `HIGH / EXPECTED` and the check passes.

This file is **not an allowlist**. There is no field that turns a severity off,
no way to ignore an instruction, and no wildcard. A declaration names one exact
change; anything else your candidate does stays visible and still fails. If the
real impact turns out to be 40 bps, the bound catches it and the check fails as
`EXPECTED_BUT_EXCEEDED` rather than passing.

Keep the file in your repository, reviewed in the pull request alongside the
code. An intentional economic change should be attributable to the commit that
introduces it.

## Reading the exit codes

| Code | What happened | What to do |
|---:|---|---|
| 0 | passed | — |
| 1 | a change nobody declared, or one larger than declared | fix the code, or declare it narrowly |
| 2 | malformed configuration | fix `expected-changes.toml` |
| 3 | a declaration for behaviour that no longer happens | remove the stale declaration |
| 4 | bundle or baseline incompatibility | tell us — this is ours to fix |
| 5 | a declaration this corpus cannot judge | the corpus has no coverage for it; tell us |
| 70 | the Eplyx API was unreachable | a transport fault, never a verdict on your code |

Codes 3 and 5 fail on purpose. A stale declaration leaves standing permission
for behaviour that no longer exists, and an unjudgeable one means the analysis
was incomplete — neither should quietly pass.

## What this does not tell you

The report carries its own limitations, and they hold for passing runs too. For
the current stake-pool corpus, for example, Jito-tipped deposits are
under-represented, transactions that failed on mainnet are not replayed, and
account-creation paths are excluded.

Eplyx tests a pinned set of real historical interactions. It is not a proof that
an upgrade is safe, and it does not claim to cover all production activity.
