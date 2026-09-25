# Phase G1.1 — real-mainnet Squads evidence

Everything here was read from Solana mainnet
(`https://api.mainnet-beta.solana.com`, public, no credentials, `finalized`).
See `docs/phase-g1-1-squads-mainnet-qualification.md`.

| Path | What it is |
| --- | --- |
| `census.json` | Population A: every live VaultTransaction whose static keys contain the upgradeable loader (6,429), classified by `scripts/qualify-g1-squads-mainnet.mjs census` |
| `sample.json` | Population B: seeded uniform sample of 400 of 515,528 live VaultTransactions |
| `raw-accounts.json` | Raw Multisig / VaultTransaction / Proposal bytes for both witnesses, plus a legacy-enum Proposal, at one slot |
| `witness-primary/` | Squads #1 of `8fJvcw…`: `acquired-change-spec.json` and `store/` (from `eplyx governance squads acquire`), `bind.json` and `bound-change-spec.json` (from `bind`), `verify-*.json` (fresh re-verifies), `independent-decode.json` (independent decoder), `negative-controls/` |
| `witness-second/` | Squads #1 of `74VMou…`, a genuine code change |
| `witness-large/` | Squads #16 of `AxkJ8o…`, a 10 MiB ProgramData; candidate bytes not committed |

`bind.json`, `verify-*.json` and the negative-control records are sealed
`GovernanceBinding`s written by the `eplyx` binary. The candidate binaries in
`store/programs/<sha256>` are public mainnet buffer contents, stored in the
evidence-store layout `acquire` writes.

Offline re-checks:

```bash
cargo test -p eplyx-engine --test governance_mainnet_evidence
node scripts/verify-squads-binding.mjs docs/examples/phase-g1-1-squads-mainnet/witness-*/bind.json
EPLYX_GOVERNANCE_RPC_URL=https://api.mainnet-beta.solana.com \
  node scripts/qualify-g1-squads-mainnet.mjs crosscheck \
  --binding docs/examples/phase-g1-1-squads-mainnet/witness-primary/bind.json \
  --witness docs/examples/phase-g1-1-squads-mainnet/witness-primary/independent-decode.json
```

These are snapshots. The chain moves on: re-running `verify` today may
legitimately give another status, or `unverifiable` once a proposal executes.
