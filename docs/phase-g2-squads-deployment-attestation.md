# Phase G2 — Squads deployment attestation

G2 is a separate, sealed statement from G1. G1 `matched` says that a proposal,
buffer and analysed ChangeSpec agreed at a finalized observation slot. G2
requires that sealed G1 record, identifies the successful Squads
`vault_transaction_execute` and its loader-v3 `Upgrade` CPI, then compares
ProgramData only when it can attribute current bytes to that execution.

## Bounded contract

`eplyx governance squads attest --change-spec bound.json --binding bind.json
--artifacts STORE --rpc-url URL --format json --evidence-out attestation.json`
reads `STORE/programs/<candidate SHA-256>` through the P2 content-addressed
resolver. The hosted equivalent is `POST
/v1/projects/{p}/governance/squads/attest` with `change_spec_id` and
`binding_id`; the project must hold the candidate artefact. Both require a
finalized G1 `matched` binding that identifies the bound spec and candidate.
Neither signs nor executes anything.

The G2 outcome is `deployed_match`, `deployed_mismatch`, `superseded`,
`not_executed`, `unsupported`, or `unverifiable`. An `Executed` status is only a
navigation signal. G2 searches finalized signatures of the exact Proposal
PDA, fetches successful transactions, and requires the Squads V4 execute
discriminator with the sealed multisig, Proposal and VaultTransaction as its
first three accounts and the G1 stored-message keys as its remaining accounts.
The parent CPI group must contain the exact seven-account loader Upgrade from
the G1 message. A retained VaultTransaction message is re-hashed; if its
message is empty or the account is absent, the sealed G1 message commitment
is used. The signature search is bounded to ten pages of 1,000 entries.
The execute account order and message consumption behavior follow the pinned
[Squads V4 execution source](https://github.com/Squads-Protocol/v4/blob/af94153ff77a28b6effe46b9c94baaa93742b48c/programs/squads_multisig_program/src/instructions/vault_transaction_execute.rs).

Current ProgramData is decoded after its 45-byte loader header. Its executable
region must start with every candidate byte and all remaining allocated bytes
must be zero. The proof records account length, account digest, executable
length, candidate prefix hash, candidate length, padding length and padding
result. A shorter region, changed prefix or non-zero padding is a mismatch
only if state attribution is established. If ProgramData's deploy slot exceeds
the execution slot, the result is `superseded`; if it precedes it, the result
is `unverifiable`. For equal slots, the universal `screening::screen_later`
reads the complete finalized block. A later transaction that could write
Program or ProgramData prevents attribution. G2 records the execution even
when deployment bytes cannot be attested.

The `eplyx-squads-deployment-attestation-v1` record has a content hash over its
body. The hosted registry stores it under the bound change and validates the
hash on every read. New checks append records; they never rewrite a prior
match after a later upgrade. The analysis page displays deployment match,
mismatch and supersession in the existing Governance proposal card.

## Real mainnet inspection, 2026-09-25

Provider: `https://api.mainnet-beta.solana.com`, finalized. All addresses came
from the committed G1.1 census. These checks establish transaction and slot
evidence, **not a complete G2 attestation**: the census contains no sealed
pre-execution G1 `matched` binding or durable candidate artefact for these
already executed proposals. The three sealed G1.1 matches were all still
Active at finalized slot 450,382,760. Their status is not execution evidence.

The production G2 CLI did run against the committed primary G1 match after
re-acquiring its exact candidate into a temporary P2 store. It returned
`not_executed` at finalized slot 450,385,784, with no execution claim. Its
[sealed attestation](examples/phase-g2-squads-mainnet/active-not-executed.json)
has ID `92135e769176e02c1c49db66c527957eac675d8ea4871fb6231319208b50666e`.
This qualifies the real RPC and content-addressed artefact path before
execution, not the deployment comparison.

| Control | Exact executed proposal | Later upgrade |
| --- | --- | --- |
| Multisig / index | `Hrk3MiRHdu1jC6Kzwp4SPaEJ5HnFDnZHtotAwF8EkrtF` / 15 | `DdH1EJJrMWMMFvDj5Fc1BeTHpzg1DEZY72Jf6FkukQZP` / 14 |
| Proposal | `7fuhqx9Rfv4KudgngRaanS4ej39hN8GVaC4Lfgz8XCPb` | `CzPirmBwzrw2dAMYKXJVWxGfLxWroU44Gr2ypPJxDDKZ` |
| Execution signature | `3Nvd3MPxvNKDj6h5YVuaUd5xgjfJeByPJw1HdRu8QwdrEpSt7csgYaQ2qrXNifxwbGs7p9gGMt5Vq1JJRywgG2wV` | `3Ba1DfziBJr5uE6guvLZQBt8WTq7mCnsP6cuu2d4PWQ9QH5BHFFxAUyKdUffpwMcnVYxG6ASjJDy2PCn55oY5pAC` |
| Execution slot / block index | 290,383,111 / 1,002 | 273,512,066 / 765 |
| ProgramData | `HHMvUJJGNRamjj12fGPQYRj763P7AfptDkxwMrwE9Yvw` | `7AdH6wpQB8SbLrFsJbH8bjv5T4ek532fCYuSXSJwJQbS` |
| Current ProgramData deploy slot | 290,383,111 | 419,028,561 |

Both successful transactions contain a Squads execute instruction at top-level
index 3 and a loader Upgrade CPI at stack height 2, with the ProgramData,
Program, Buffer, spill and vault authority named in the census. The first
transaction's recent blockhash is
`8Q7gXpEpZM15vSPnxT6hfhWEjTnUp32nFxHm4FJm4tRu`. Its full finalized block
had 1,089 transactions, the execution at index 1,002, and no later writable
Program or ProgramData account. The live VaultTransaction still retained its
291-byte message; its domain-separated hash was
`1a6d6296d05493b51b70d3e9693def3723a94a17571b31fe20ec4d8f7bdff9a7`,
identical to the G1.1 census commitment. This qualifies the RPC execution and
same-slot evidence shape on mainnet. It does not reconstruct the consumed
Buffer's candidate bytes at the pre-execution G1 slot.

The second proposal demonstrates real later-slot supersession: current
ProgramData reports slot 419,028,561, later than its successful execution at
273,512,066. A current-byte comparison to that old proposal would be false
evidence. The production G2 `superseded` branch is covered by simulation, but
the complete branch cannot be run for this witness without a sealed G1 binding.

## Qualification status

The engine and hosted path are implemented, and simulated tests cover exact
execution, zero padding, prefix and padding mismatch, later-slot
supersession, same-slot later writers, wrong execution identity, Active status,
and seal tampering. The G2 acceptance condition of a **full real mainnet
deployed match** remains open because none of the committed pre-execution G1
bindings has executed. Squads governance should not yet be called complete for
this phase. A future executed proposal with a sealed G1 match and retained P2
candidate is needed for the final end-to-end qualification.
