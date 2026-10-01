# Selected Token-2022 parameter cases

`eplyx parameter cases` evaluates two or three explicitly selected retained
transfers with distinct observed source accounts. It calls the existing parameter
analyzer, verifier and offline reproducer; it adds no fee calculation, scenario,
search dimension or hosted operation.

Development branch: `codex/parameter-case-set`, based on
`4c521f5815de2238ae0b921e7c34d2ead07cf5cd`. Search, external-candidate evidence
and the frozen review deployment are separate and untouched.

The unchanged [existing proposal](examples/parameter-case-set-change.json) is
parsed and validated through `ChangeSpec::parse`. Its ID is
`7f24680b92d65edec79f97175be9a81cb35dbf8670581af13519c8f444da3a11`:
active newer Token-2022 fee **50 → 200 bps**, expected mint data hash
`5031771a84320c80236f4453c9b7d554446acc23490a9e1abffdd141cfada219`,
schedule epoch **1032**, cap **18446744073709551615**.

## Selection and measured result

The existing private lifecycle capture manifest and execution index retain three
original-owner TransferChecked source accounts: groups 0, 4 and 5. All have the
same mint expectations and deployed Token-2022 ELF
`0999dbf708971e723b08d1caafc988826a59c6001ed6dc02260da07defbe1469`.
Their own retained Clocks are at epoch 1037. Owner/system envelopes, actual
destination evidence, balance and full transcript admission are checked by the
existing transfer builder; `parameter_change::mutate` checks the exact proposal
expectations and active schedule without changing the retained input.

Select the full retained transfers from the first two eligible capture groups
(0 and 4), before comparing results. The execution-index entries already bind
these exact amounts and fixture hashes. Group 5 is eligible but excluded because
two cases suffice. Groups 1–3 describe SecondaryMarketExit, outside this
original-owner TransferChecked case-set contract. The later current-transfer
capture repeats group 0's source and has a changed mint state with a pending
newer schedule; it cannot be substituted under this proposal.

Both selected cases use actual destination
`124XHuTYUNnCf2ABCNcCEdQFpHQ7Y6MnPe9NeouDB1az` and preserve its separately
captured balance and owner evidence. Values below are raw units from verified
single-case reports; differences are existing engine findings.

| Retained group / source | Amount / initial source balance | Baseline credit | Proposed credit | Credit difference | Destination withheld increment, baseline → proposed |
|---|---:|---:|---:|---:|---:|
| 0 / `741ZXYKzgjPZucRhPUQFK7kKAgP1ESJwimhumgGpuPHs` | 17,621 | 17,532 | 17,268 | -264 | 89 → 353 |
| 4 / `E6NHqVMSHrssPiKGgKnfxNqjoTnQXMoL5xDUvDBXiFdE` | 309,138 | 307,592 | 302,955 | -4,637 | 1,546 → 6,183 |

Group 0 retains observation `2026-09-18T07:39:15.727432Z`, Clock slot
448018365 and fixture `d99ec5e1c0c6aedf4d1a675ddaff9c5cbf7dab11b77c478ef045416c6185c264`.
Group 4 retains observation `2026-09-18T07:40:03.392352Z`, Clock slot
448018544 and fixture `a83e9a55c1e6fc2cd2d0e363e32ad9f7ec6cb8434ca269f32ce24618be70ef68`.
No Clock or account batch is shared or overwritten.

Two selected cases, two completed baseline/proposed pairs (four fresh VMs), two
reconciled pairs, two measured decreases, zero no-consequence or unavailable
cases. Verification executes no VM; a complete repeat performs two additional
pairs, recorded separately. These are per-invocation counts; qualification tests
also exercise repeat and negative controls.

Both selected transfers show a reconciled decrease. Their amounts, balances and
observation times differ, so the different magnitudes do not establish an effect
of account identity. Source accounts do not identify people. No sum, production
coverage, representativeness, population estimate, authority possession or
on-chain activation is established.

## Local commands and evidence package

Start from complete existing `parameter_change::Input` JSON files, preserving
each input's own fixture, amount and context:

```sh
eplyx parameter cases prepare --change change.json \
  --input retained-a.json retained-b.json --out selected-request
eplyx parameter cases analyse --manifest selected-request/manifest.json \
  --out selected-result
eplyx parameter cases verify --package selected-result
eplyx parameter cases reproduce --package selected-result
```

Outputs must be new. All commands use the existing empty-environment worker.
The strict schema-1 manifest binds `revision`, the validated `change_spec_id`,
the current parameter runtime, runner source, captured decoder revision, deployed
program/loader/ELF commitments and signer/runtime assumptions. Each case has
`case_<canonical input SHA-256>`, `input_sha256`, and a package-relative
`{path, sha256}` reference. Cases sort by their deterministic IDs. Unknown
fields, duplicate sources, missing/tampered references, incompatible execution
contracts and stale/unsupported inputs are rejected before paired execution.

`case_set_id` uses the existing canonical digest helper over proposal, ordered
input identities, semantics/revision and contract. Paths, file formatting and
optional case labels are excluded. `result_sha256` separately commits the
verified per-case rows, intact engine report identities, counts and limitations.
The verifier rebuilds that entire projection; merely recomputing a digest does
not legitimize edited values. Each economic value requires successful,
reconciled execution; unavailable values remain JSON null, including missing
findings. Equal verified quantities yield an exact zero difference.

The private package contains unchanged `change.json`, each complete
`input-N.json`, intact `report-N.json`, `manifest.json`, `summary.json` and a
readable `summary.md`. Inputs/reports embed the captured account and deployed
executable evidence. Report references correspond to summary rows in order.
Individual repeat uses the existing command:

```sh
eplyx parameter reproduce --change selected-result/change.json \
  --report selected-result/report-0.json
```

Package reproduction checks every reference/report and the saved projection
before executing. Its receipt retains each case's verification and reproduction
result; invalid evidence blocks replay explicitly. Actual replay failures do not
prevent subsequent valid cases from being attempted. Analytical rejection,
reconciliation failure and unavailable execution retain their original engine
statuses. Unexpected internal errors retain per-case `analysis-errors.json` and
available reports, return failure and produce no complete summary.

## Retained qualification and checks

The regression adapter reads the imported lifecycle snapshot and original
execution-index/result bytes, verifies their commitments, and constructs the
existing typed Input with the already retained amount/context. It asserts exact
equality of the original message and captured Clock. No evidence is synthesized
to qualify these cases. To export the private qualified package locally:

```sh
mkdir -p data
EPLYX_PARAMETER_CASE_QUALIFICATION_OUT="$PWD/data/parameter-case-set" \
  cargo test --locked --offline -p eplyx-engine --test parameter_cases \
  genuine_selected_cases_and_complete_offline_repeat
```

The local lifecycle fixtures must already be imported through the existing
fixture importer; this command does not acquire evidence. `data/` is ignored.
Only the [safe qualification commitments and summary](examples/parameter-case-set-qualification.json)
are public. Provider-bearing input/report transcripts remain private.

Focused tests cover two genuine sources, exact unchanged inputs/proposal,
distinct Clock/message evidence, duplicate/renamed accounts, wrong target/stale
state/unsupported/missing evidence, deterministic identities, cosmetic labels
and JSON formatting, tampered references/reports/projections, unavailable
quantities, separate selected-case counts and complete offline repeat after
removing original request/input paths. Existing parameter/current-transfer and
Token-2022 regressions, formatting and targeted Clippy are run separately.
Offline qualification is limited to the tested macOS arm64 host/runtime; no
global portability claim is made.
