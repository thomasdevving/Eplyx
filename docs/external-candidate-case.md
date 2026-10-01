# One external Stake Pool candidate — Step 13A

**Status: `public_upstream_rehearsal_evaluated`.** This is a public upstream
rehearsal, not a customer submission, deployment reproduction, endorsement or
vulnerability claim. No team-supplied candidate was found in the relevant project
artifacts. One unmodified upstream source revision completed ordinary upgrade
review across the entire existing ten-record bundle. The gate passed; measured
outcomes and economics were preserved. No Eplyx integration fix was needed.

The review question was: **Under the retained qualified account states and
supported actions, how does this exact upstream source-built executable behave
compared with the historical baseline?**

| Identity | Retained value |
| --- | --- |
| Development branch | `codex/external-candidate-case` |
| Base | `4c521f5815de2238ae0b921e7c34d2ead07cf5cd` |
| Origin | `public_upstream_release_rehearsal` |
| Program | `SPoo1Ku8WFXoNDMHPsrGSTSG1Y47rzgn41SLUNakuHy` |
| Official program release | [`program@v2.1.1`](https://github.com/solana-program/stake-pool/releases/tag/program%40v2.1.1), published 2026-09-09; retrieved 2026-10-01 |
| Full source commit | `3cda955e59a261f8c82f53f72560c6ea5a6647e8` |
| Locally built executable SHA-256 | `1113fbd035048c456d2768325e4e6d3699672070267e68b96c01abbc080fdb62` |
| Executable length | 293,200 bytes |
| Baseline ELF SHA-256 | `ec2dfefaa70d560754a0000f39bd2cabc192b895d36205b3c428f601b6e1d7e1` (1,080,464 bytes) |
| Bundle identity | `5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f` |
| Corpus identity | `a4b667316b972685743bf0724b5596b7fbb64dde46468e159f44d773894f69b1` |
| Bound ChangeSpec identity | `5b67c32be3f4c2438cfc7622925d7ed0611d1f8947fe48d4f07e66d01499de06` |

The base contains the existing ChangeSpec, bundle verification, historical fidelity,
candidate overlay, Stake Pool adapter v3 and deterministic report capabilities.
It does not need the separate completed parameter-search changes.

The earlier official `program@v2.1.0` asset was inspected first: its published raw
SHA-256 equals the baseline, so it was excluded before candidate execution as a
same-code control. No other revisions were tried. The selected candidate differs
from the baseline and both Eplyx-constructed Stake Pool candidate identities.

The selected release has no published executable asset or expected build hash.
Its annotated tag was resolved to the full commit, and the official source archive,
all source-file commitments, unchanged lockfile, Apache-2.0 license and build
documents are retained in the [case package](examples/external-candidate-case/case-manifest.json).
The executable was built locally through the upstream Makefile's `cargo build-sbf`
path using existing `cargo-build-sbf 4.4.0`, cached platform-tools v1.57,
`rustc 1.95.0-dev`, default SBPF v3, no additional features, and `--locked`.
The upstream manifest declares CLI 3.1.14 and host Rust 1.93.0; this local build
is not claimed to reproduce upstream CI. Exact commands, environment, compiler
commitments and two build-attempt logs are in the
[build manifest](examples/external-candidate-case/provenance/build-manifest.json).

The build ran with a cleared credential environment in a fresh source/target/cache
sandbox. Reads of private directories and the other worktrees were denied; writes
were confined to the build directory. One metadata-only sandbox correction allowed
Cargo to canonicalize parent directories. Source, program ID, lockfile, dependency
versions and behavior features were unchanged, verified again after building.
Source-to-local-binary provenance is established by that execution and retained
inputs. Publisher-produced bytes, deployed-byte equivalence, deployment chronology
and current mainnet code remain unverified. Raw ELF SHA-256, source commit,
ProgramData allocation/padding and tool-specific build hashes are distinct inputs;
none was substituted for another. The v2.1.0 `solana-verify` hash belongs to the
excluded release and was not used as this candidate's expected hash.

The original bundle is preserved, with a byte-identical self-contained copy in
`input/bundle`. Its ten records cover one ungated `DepositSol` and nine ungated
`WithdrawSol` actions at slots 447850493–447904974. Record IDs, exact Clock inputs,
original outcomes, commitments and acquisition/boundary assumptions are retained
in [historical-records.json](examples/external-candidate-case/provenance/historical-records.json).
All originals succeeded. The target and candidate load under the upgradeable
loader. Historical SPL Token (`8190d3f7…`, 108,600 bytes) and Stake (`3d2d39c5…`,
212,056 bytes) dependencies remain pinned; the manifest carries their full hashes.

Bundle verification passed. Before candidate interpretation, the baseline-as-control
run reproduced original success, fee, watched post-state and invocation graphs on
all ten records: **10/10 `Matched`, never `Exact`**. Analysis used the retained
macOS arm64 Eplyx executable at the base commit, LiteSVM 0.16.0, Agave feature-set
4.2.2, the existing fixed mainnet feature snapshot and unchanged signature/blockhash
checking policy. Every record retains epoch zero and its original slot/time.
This does not establish exact historical validator or epoch-sensitive equivalence.

Upstream instruction definitions and processors preserve discriminants 14/16,
u64 amount encoding, the supported ten/twelve account roles and optional-authority
contract. Its pool layout preserves the fields the existing adapter interprets;
the retained pools and base SPL Token mint/accounts satisfy that bounded contract.
The unchanged historical boundary checks and adapter decoded-field pairing remain
in force. Actual candidate execution provides the additional evidence: all ten
transactions succeeded with identical fees, watched account snapshots, post-state
hashes, measured economic fields and CPI shapes. The
[execution report](examples/external-candidate-case/reports/candidate-replay.json)
retains both executions, logs and account outputs for every record.

The [authoritative CI report](examples/external-candidate-case/reports/candidate-ci.json)
exits **0**, with no declarations, named findings or undeclarable changes. Qualified
subjects are deposit pool tokens received, withdrawal pool tokens debited/burned,
SOL received by the withdrawing user, and transaction execution for both actions.
Compute use alone decreased by 3.26–4.27% in this runtime; compute is separate from
the economic gate, and no production performance claim or cause is assigned.

No record was excluded from this case. Existing bundle exclusions still apply:
originally failed transactions, account creation, rent-sensitive below-minimum
credits and actual lookup-table resolution. Other actions, SetFee, parameter
interactions, search, epoch transitions, wider states and full-release safety were
not evaluated. Production frequency and denominators remain unknown.

To repeat independently on compatible **macOS arm64**, use the retained engine,
candidate and input copy. No compiler, upstream download, RPC or provider is needed
for analysis. From the case directory:

```sh
cd docs/examples/external-candidate-case
python3 rerun.py --out-dir /private/tmp/eplyx-step13a-reviewer-repeat
```

The output directory must be fresh. The case-specific script verifies artifact
SHA-256/lengths, requires a socket permission-denial probe, then uses
`/usr/bin/sandbox-exec` with `(deny network*)` for existing `bundle verify`, baseline
`compare`, bound `ci check` and candidate `compare` commands. It does not repair
inputs or reports. The actual invocations, exit codes and hashes are retained in
[repeat-receipt.json](examples/external-candidate-case/repeat/repeat-receipt.json).
All four fresh reports were byte-identical. Only this host/platform was tested;
cross-platform portability and fresh engine/source rebuilding are not claimed.

A real protocol team would supply its intended target and review question, exact
candidate bytes or pinned unmodified source/build recipe, provenance and license,
justified expectations, and any historical account/runtime evidence required for
its actual supported actions. No remaining prerequisite blocks this rehearsal.

All changes are this document and its local case package. The completed search
worktree (including uncommitted files), fixed review branch/image, canonical
fixtures and original historical evidence remained unchanged. No commits, pushes,
merges, tags, deployment changes, paid resources, uploads, team outreach or on-chain
transactions occurred. Work stops after this case.
