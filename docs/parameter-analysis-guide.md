# Analyse a protocol fee change

Use this workflow to measure a supported fee proposal against one exact retained
action. It does not change the chain or prove authority to apply the proposal.
For general onboarding, see [getting started](getting-started.md).

## Choose the supported operation

| Operation | Evidence you supply | What executes | Browser entry |
| --- | --- | --- | --- |
| Active newer Token-2022 transfer-fee bps | Eligible retained transfer capture or complete typed input, plus exact mint expectations | The same TransferChecked twice, with only the active fee-bps field derived in the proposed mint | Parameter Change on an eligible authenticated retained transfer run |
| Stake Pool SOL deposit fee | Qualified historical bundle and explicit DepositSol record, plus exact pool expectations | Simulated SetFee with an assumed manager signer, then independent baseline/proposed deposits | CLI or authenticated parameter API; no standalone guided fee form |

The Token-2022 rate is an integer in `0..=10000` basis points. A pending newer
schedule cannot be activated by advancing time. Stake Pool fees are exact rational
fractions represented by canonical decimal strings. The retained program decides
whether a configuration is accepted; the browser does not calculate its result.
The [technical contract](protocol-parameter-change.md) defines the full schemas.

## Run the tracked Stake Pool example

Build or install the CLI first. From the repository root, with `eplyx` on `PATH`:

```sh
eplyx version --json
eplyx parameter analyse \
  --change docs/examples/stake-pool-parameter-change.json \
  --bundle deploy/bundle \
  --record-id mainnet-spl-stake-pool-151010f709e113e7 \
  --out stake-fee-report.json
eplyx parameter reproduce \
  --change docs/examples/stake-pool-parameter-change.json \
  --report stake-fee-report.json
```

The output path must be new. These commands emit JSON without `--format json`.
Add `--record /path/to/local-project` to analysis if you also want the report
indexed in an existing local project's dashboard. The report itself retains the
inputs needed for compatible offline reproduction.

For the qualified declaration `0/1000 → 1/100`, the retained reference measured
recipient credits of **760,985,008 → 753,375,157 raw pool tokens** and a separate
manager fee-account credit of **7,609,851**. The deposit is one historical record,
not an estimate for other depositors. Its fixed retained Clock includes epoch zero;
`Matched` historical fidelity is distinct from the controlled `Exact` contract.
The manager signer is a simulation assumption, not proof of key possession.

## Use your retained Token-2022 transfer

Prepare the ChangeSpec from the same retained mint evidence. Do not copy the
example's expectations onto another mint or later observation.

```sh
eplyx parameter analyse --change change.json \
  --capture transfer.capture.json --out parameter-report.json
eplyx parameter reproduce --change change.json --report parameter-report.json
```

`change.json` and `transfer.capture.json` are your already-prepared files, not
installation outputs. A complete typed `--input input.json` can replace
`--capture`; choose one source. The captured epoch, program bytes, account state
and original transfer stay fixed. This operation does not execute SetTransferFee.

Read the report status and reconciliation before reading economic findings:

| Result | Next step |
| --- | --- |
| Measured consequence | Inspect the exact raw credit/withheld differences and the recorded scope. |
| No observed consequence | Record that this case's measured outputs were equal; do not extend it to other cases. |
| Current-state mismatch | Check the proposal against its original evidence. A newer observation needs a new bound analysis. |
| Pending schedule or unsupported action | Use a supported active schedule/action; the engine does not change Clock or repair inputs. |
| Rejection, unavailable execution or failed reconciliation | Inspect the retained reason/stage. Missing or unreconciled values are not zero effects. |

A completed command or HTTP acceptance is not a safety verdict. Hosted jobs may
be queued or running before a report exists. Current-state and historical parent
routes require project access and server-verified eligibility.

## Compare code and fee together

The separate [interaction guide](upgrade-parameter-interaction.md) compares one
retained Stake Pool deposit under V1/V2 and current/proposed fee settings. Build
its exact config-capable fixture first; the DepositSol-only regression fixture
from the first-analysis example cannot substitute for it. The hosted entry is
**Compare code and fee** on an eligible retained upgrade run.

It distinguishes code, fee, combined and interaction effects. Zero interaction
does not mean zero fee effect. This experiment does not execute a real loader
upgrade, prove rollout order or qualify arbitrary candidate programs.

## Local integration extensions

**Build requirement:** current source includes amount search and selected cases,
consolidated from verified `codex/analysis-integration` commit
`8980e7d515616afabb33b9a125099c9b2f348467`. Build the current source and inspect
`eplyx version --json` and `eplyx parameter --help`; an older installed release may
not include them. These tools have no hosted form and do not update the fixed
Step 11A review deployment. The original qualification inputs remain private;
a fresh clone alone cannot repeat those reference runs. Their detailed contracts
are [amount search](parameter-edge-search.md), [selected cases](parameter-case-set.md)
and [source integration](analysis-integration.md).

### Search a bounded amount interval

Bring the same Token-2022 declaration, a verified successful reconciled parent
report, and a new `search.json` with this schema:

```json
{
  "schema_version": 1,
  "dimension": "transfer_amount_raw",
  "min_raw": "1",
  "max_raw": "10000",
  "predicate": { "kind": "recipient_loss_exceeds", "threshold_raw": "100" },
  "budget": { "max_evaluations": 64, "max_refinements": 16, "max_vm_calls": 128 }
}
```

The example interval is not suitable for every source balance. It must fit the
retained source amount. Only the hypothetical transfer amount varies; observed
accounts, code, Clock and fee proposal remain unchanged.

```sh
eplyx parameter search --change change.json --parent-report parameter-report.json \
  --spec search.json --out ./parameter-search --format json
eplyx parameter verify-search --artifact ./parameter-search --format json
eplyx parameter reproduce-search --artifact ./parameter-search --format json
eplyx parameter reproduce-witness --artifact ./parameter-search \
  --witness WITNESS_SHA256 --format json
```

If the parent binds an original capture, add `--capture transfer.capture.json`
with its exact original bytes to **search**. Replace `WITNESS_SHA256` with an
identity actually saved by that search. Output directories must be new.
Verification performs no VM execution. Full reproduction reruns the experiment;
witness reproduction reruns only the selected pair with a compatible build.

The reference `50 → 200 bps` run tested **64** amounts with **128 VM calls**,
found **25** matches, left **9,936** amounts untested and exhausted its budget.
The smallest **tested** match was **6,732 raw**, with recipient credits
**6,698 → 6,597** (loss 101). This is a user-declared condition, not a vulnerability
verdict or a certified global minimum.

### Evaluate two or three selected observed cases

Bring complete typed retained transfer inputs from distinct observed source
accounts, one exact proposal and compatible execution contracts. Preserve each
input's own amount, state and Clock. These are independent pairs, not sequential
actions in a merged account world.

```sh
eplyx parameter cases prepare --change change.json \
  --input retained-a.json retained-b.json --out selected-request
eplyx parameter cases analyse --manifest selected-request/manifest.json \
  --out selected-result
eplyx parameter cases verify --package selected-result
eplyx parameter cases reproduce --package selected-result
```

Prepare the manifest with the same build used for fresh analysis. The case
commands emit JSON without a format flag. Missing/tampered evidence, duplicate
sources or incompatible inputs are rejected. Verification performs no VMs; two
completed case pairs use four fresh VMs. Their magnitudes do not identify people,
establish a population effect or isolate an effect of account identity.

The integration also preserves an upstream **source-built** Stake Pool candidate
rehearsal over ten retained records: baseline 10/10 Matched, candidate 10/10
successful and gate exit 0 within that corpus. It is not a customer release,
proof of deployed-byte equivalence or general safety. Its exact retained candidate
and package are explicit private prerequisites. It does not qualify that candidate
for parameter or interaction analysis.
