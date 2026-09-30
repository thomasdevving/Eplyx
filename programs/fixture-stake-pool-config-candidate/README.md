# Stake Pool config candidate — Step 10B

**Constructed test fixture. Not an upstream release. Not intended for deployment.
Qualified only for the stated test boundary.** This separate fixture makes the
retained code/configuration experiment executable; it does not extend production
candidate admission or implement an interaction analyzer.

The dispatcher admits only exact official `SetFee(SolDeposit(Fee))` and ungated,
ten-account `DepositSol` messages. Deposit requires zero SOL referral percentage,
the declared program/pool/PDA/reserve/mint/manager-fee/legacy Token relationships,
a signing System-owned lamport source and initialized token roles. Gated deposit,
nonzero referral, extra/missing accounts, other instructions and malformed wire
encodings reject. Recipient, manager and referral roles may alias; real mints
accumulate, while reconciliation measures each account once.

Full typed state/instruction definitions and checked deposit/ceiling-fee helpers
come from pinned `spl-stake-pool=2.0.3`. Its compatible SBF graph uses
`solana-program=2.3.0`, `spl-token=8.0.0`, `solana-system-interface=1.0.0` and
`borsh=1.8.1` (the version already resolved by the host graph, satisfying the
official crate's requirement). The fixture has its own workspace/lockfile; main
workspace and original fixture dependencies are unchanged. Attribution and the
Apache-2.0 license are in `NOTICE` and `LICENSE`.

SetFee uses full-layout roundtrip validation, checks manager key/signature and
official exact fraction validation, and serializes only the complete typed
prefix. Every trailing byte is preserved. `0/0` succeeds; `1/0` and `2/1` reject.
The instruction reads no manager owner/data/balance. Actual SBF controls establish
independence for the tested rent-valid, non-executable envelopes; generic VM
admission still applies. Manager signing is assumed, not proof of key possession.

Build on the tested macOS 26.6.2 / Darwin arm64 boundary with cargo-build-sbf
4.4.0, cached platform-tools v1.54 / rustc 1.89.0-dev and SBF arch v0:

```sh
# Put the host Rust toolchain and cargo-build-sbf on PATH first.
scripts/build-stake-pool-config-candidate.sh
EPLYX_STAKE_CONFIG_QUALIFICATION_OUT=/private/tmp/eplyx-step10b-qualification \
  cargo test -p eplyx-engine --test stake_pool_config_candidate --offline -- --nocapture
```

For this machine the builder lives outside PATH; the equivalent invocation was:

```sh
export PATH=/Users/thomasnguyen/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH
EPLYX_SBF_BUILDER=/private/tmp/eplyx-cargo/bin/cargo-build-sbf \
  scripts/build-stake-pool-config-candidate.sh
```

`EPLYX_SBF_TOOLS` can locate the cached v1.54 `platform-tools` directory. The
script verifies its version commitments, stages the intended source/manifest/
lock/notice/license in a fresh tree, forces a fresh target/output, and builds
offline without installing tools or modifying rustup. Dependency/tool caches
are reused. An optional first argument selects the output directory. The script
rejects output other than SHA-256
`a664f74b73dedc713f16934829b25f9a0c0c3a06c6ce21f03fdc7869ae5b555d`,
133992 bytes, before copying `fixture_stake_pool_config_v2.so` and its build
receipt. Two independent clean trees reproduced identical ELF and build receipts;
fresh network installation and other operating systems were not tested.

The qualification test generates byte-bearing `qualification.json` and compact
`qualification-summary.json`. It independently runs K1/K2 and all four fresh
action VMs, verifies config preservation and actual action deltas, repeats the
experiment offline, and rejects changed ELF or tampered/resealed evidence.
K2's own pool supplies R11; manager/payer state and config fees are excluded.
The tracked compact receipt is
[`docs/examples/stake-pool-config-candidate-qualification.json`](../../docs/examples/stake-pool-config-candidate-qualification.json).

Historical V1 alone has Matched historical fidelity. The constructed V2 produces
the same measured fee effect (`-7609851` raw recipient tokens), with interaction
`0`. This is valid for the retained case; it is not universal compatibility,
validator equivalence, a loader Upgrade, an installed-program world or a rollout
ordering/safety claim. Inconsistent synthetic ledgers that fail reconciliation
remain qualitative evidence and never become numeric zero.
