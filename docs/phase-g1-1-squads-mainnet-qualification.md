# Phase G1.1 — real-mainnet Squads qualification

## Outcome: G1.1-A — REAL MAINNET MATCH

The production G1 path (`eplyx governance squads acquire | bind | verify`, the
built binary, no simulation) read three real Squads V4 program-upgrade
proposals on mainnet. It returned `matched` for all three. Two were re-verified
at later slots and cross-checked field by field against an independent decoder,
and every live negative control failed closed.

The contract held on real state, with three findings. None of them required
broadening G1:

1. **A normal-tooling shape G1 rejects.** 281 of 4,788 single-instruction
   upgrade proposals (5.9%, across 68 of 533 multisigs) name the vault itself
   as the spill account. They are reported, not admitted (§20).
2. **A historical Proposal layout.** Squads inserted `ProposalStatus::Executing`
   at variant 4 on 2023-06-23. Proposals last written before that deployment
   use the old numbering. G1 fails closed on one of those encodings, but the
   other is byte-identical to a current status and cannot be told apart (§20).
3. **A provider behaviour, fixed.** A load-balanced public endpoint once
   answered the consistency read with JSON-RPC `-32016` from a node behind the
   first read. G1 turned that into a spurious `unverifiable`. The read now waits
   for such a node, up to five times, and still never accepts an older view.
   This is the only code change in G1.1.

Provider: `https://api.mainnet-beta.solana.com` (public, no credentials). It
was configured explicitly as `EPLYX_GOVERNANCE_RPC_URL` / `SOLANA_RPC_URL`.
All reads used `finalized` commitment.

## 2. Bounded search population

Discovery is RPC-native and reads current accounts. No indexer was used.

| Population | Definition | Size |
| --- | --- | ---: |
| All live VaultTransactions | `getProgramAccounts(Squads, memcmp disc)` | 515,528 |
| **A — loader census** | every live VaultTransaction whose *static* keys contain `BPFLoaderUpgradeable` at index 0–31, with 0–2 ephemeral signer bumps (96 `memcmp` queries at the pinned offsets) | **6,429** |
| **B — usage sample** | seeded uniform sample (LCG seed 11) of all live VaultTransactions | 400 |

Population A covered slots 450,317,601–450,318,311 in 334 requests. Its limits:
it cannot see a loader referenced only through a lookup table (Squads allows a
LUT-loaded program index), a loader past static index 31, a transaction with 3
or more ephemeral bumps, or any VaultTransaction account already closed.

Tools: `scripts/qualify-g1-squads-mainnet.mjs census | sample | witness |
crosscheck`. Results: `docs/examples/phase-g1-1-squads-mainnet/`.

## 3. Shape distribution

**A — loader-bearing (6,429):**

| Class | Count |
| --- | ---: |
| exact G1 shape (one canonical Upgrade) | **4,466** |
| other loader instruction (SetAuthority 889, Close 153, ExtendProgram 35, empty data 30, …) | 1,130 |
| upgrade, other account shape | 319 |
| LUT-backed | 237 |
| multiple instructions including the loader | 217 |
| single non-loader instruction | 57 |
| Upgrade with non-canonical data (`03000000a0860100`, `03000000d0070000`) | 3 |

Of the 4,788 single-instruction upgrade proposals, **93.3%** have the exact
supported shape. The 319 others:

| Reason | Count |
| --- | ---: |
| spill = vault (`[1,2,3,0,5,6,0]`) | 281 |
| spill = buffer (`[1,2,3,3,5,6,0]`) | 18 |
| wrong account count | 8 |
| extra signer, or authority not the vault | 8 |
| ProgramData not the loader derivation | 3 |
| wrong sysvars | 1 |

The exact-shape proposals break down by status as: executed 4,050, active 138,
approved 131, cancelled 71, rejected 68, no proposal 3, legacy/unreadable
status 5. Of the 269 Active or Approved ones, **67 are bindable now** (buffer
live, buffer and upgrade authority both the vault). 184 have a buffer that no
longer exists, and 16 have an authority that isn't the vault.

**B — all Squads usage (400 sampled):** LUT-backed 222 (55.5%), single
non-loader instruction 90, multiple instructions 76, ephemeral signers 8,
**exact G1 upgrade 4 (1.0%)**. G1 is narrow against Squads usage as a whole,
which is mostly transfers and DeFi. It covers the great majority of what
program-upgrade proposals actually look like.

## 4–5. Selected witnesses

| | Primary | Second | Large |
| --- | --- | --- | --- |
| Multisig | `8fJvcwpRbEaTsRPcxWrrUHDPPstSnWSctQH7s4KYzgSy` | `74VMouGCdAZ7EsekxitF2DbfhoNxrfybfszqduANSXFQ` | `AxkJ8oH5aDu4ZRWfsujPtxdb6Vhq4gDehpoReBgrUUSm` |
| Transaction index | 1 | 1 | 16 |
| VaultTransaction | `EPSAEZxXWmDojSPvaom19d7jN1xgfMBMMyfooe2EhBt8` | `7tUHBDf22HWMHAsh8uzxdguH3vcsPRzz8ULMf7yrSTSG` | `AJxVeuszYo47GNqaauzepWu6eLDf4mhRqk1xHw9MPuMk` |
| Proposal | `AZrF6aoevkCgMjpLcP5SBLf1ktcmtfb8TJkX66c6Xzrp` | `HbbLdRTysKF4r1QvBo22wm9kqtMoG8SMxxMatb9oVB8G` | `85Uu4rWg57WwsSKE3FPEuEQmrS57ujzCh4wzhewgHQxq` |
| Vault (index 0) | `5UcYkuhhiVBxLGY764pDcw3Xaqx7L6x9Kerr3RbvTzJ5` | `Cekxm3iBGP1RP51nDsgjccJxUiRdGUk9pxxg83yDoKw5` | `5myNNmEmPm3UAnJ2ggLEpnTFb9t9Gk8369wKw6n3uAKx` |
| Status | Active (since 2026-09-04), 0/1 approvals, not stale | Active (since 2023-11-07), 1/2 approvals, not stale | Active |
| Bind slot | 450,318,819 | 450,319,467 | 450,319,753 |
| Message SHA-256 | `f3760817e9c10c55012ec22c19218e7989a0fb2a14372f96b08bb269fa65ee89` | `47a88a98b3cd3e182b80c2c2c194c4f115cb25af75a6e0aaf4d89bddc8be545a` | `4e97cb2c2d11e467d78904cdc86b2917dce1e5150015ea6f63f364bc2a6a5e85` |
| Target program | `6pppX8DZWkYBTpveVAteaHCzErphchSYMvyTtGhpySfm` | `ApwXwqGcQhbHdQiXtu1v4uY1eo4HQPh3ZTiznuwXfUut` | `PERPHjGBqRHArX4DySjwM6UJHiR3sWAatqfdBS2qQJu` |
| ProgramData | `GfAzhPuw7kT2mVn8DWdRX4ApD22vFdUFYcckwJ6SXBha` | `7w9DXzNo2KsRCdGnftQtUah2tFnLhqBgFM8eRXbkr3rS` | `C258LjtN8Q928yz2nJe5kwQycckGfWBSrhZBP2mTpLQt` |
| Buffer | `44HwysJhVGRQt6Yhm1EruDJ8VPXHYxLKcGv9xh4gbZgt` | `DWDPSMDP1B675zWF8VE5myFyU5XUxRC68HAPDPYLiAop` | `DuRQteB6CKmZy6oV8JBQgGDjRZJ5r6htSaoGbVH9bv3w` |
| Candidate | `b10c1c93…0761d`, 38,768 B | `fe2bc73d…43085`, 82,312 B | `3f3e9803…55bd1`, 2,586,336 B |
| Unbound ChangeSpec | `34e78668…6ed483` | `0a0a4a04…aa1107` | `e043d3a7…6fe54d` |
| Bound ChangeSpec | `cba0af68dc7376c33ab85610f1bbde30c58d6512aaf4a3f8d9c8949d95214536` | `80f5cf1a83c65c95ac1845769ecdf7a04e04cbacab9992690935aa6d55535839` | `0b1385ad…62ac4b` |

The primary witness is the most recent Active one: it is not stale and has a
small buffer. **Its buffer holds exactly the bytes already deployed**
(`b10c1c93…` in both), so it is a byte-identical re-upgrade. The binding is
still exact, and that is itself a useful thing for a reviewer to see. The
second witness proposes genuinely different code (deployed `761086bc…`, a
164,624-byte ProgramData tail, against buffer `fe2bc73d…`). The large witness
exercises a 10 MiB ProgramData plus a 2.5 MB buffer in one atomic read on the
public endpoint (4.9 s). Its candidate bytes are not committed.

## 6–8. Independent decoder, message hash and PDAs

`qualify-g1-squads-mainnet.mjs` decodes the same accounts with hand-written
Borsh over the pinned layouts, derives PDAs with its own BigInt ed25519
on-curve check, and reads loader states by hand. It imports nothing from Eplyx.
`crosscheck` compares it against Eplyx's sealed binding. All 17 checks pass for
both committed witnesses:

binding ID recomputes · outcome · multisig = PDA(create_key) with canonical
bump · VaultTransaction PDA + canonical bump · Proposal PDA + canonical bump ·
vault PDA, and the stored `vault_bump` signs as it · transaction and vault index
· **every field of the stored message** · message hash (Eplyx = independent
account decode) · message hash (Eplyx = JS re-encoding of Eplyx's view) ·
status and votes · all seven Upgrade accounts · ProgramData = derived(Program) ·
upgrade authority = vault · deployed executable · buffer authority = vault ·
buffer artefact = candidate.

`scripts/verify-squads-binding.mjs FILE…` (the existing independent verifier,
now accepting files) recomputes the binding ID and message hash of all 12
sealed mainnet records: 34/34 checks pass. The raw mainnet Multisig,
VaultTransaction and Proposal bytes are committed (`raw-accounts.json`).
`engine/tests/governance_mainnet_evidence.rs` decodes them through the
production decoder offline and re-verifies every sealed record.

## 9. Loader Upgrade on real state

All three witnesses: data `03000000` (the legacy encoding), account indexes
`[1,2,3,4,6,7,0]`. Real Squads tooling places the loader key at index 5,
**before** Rent and Clock, whereas G1's simulation put it last. Positional
decoding handles this unchanged. ProgramData equals the loader derivation of
Program, the Program account's ProgramData pointer agrees, and the current
upgrade authority is the vault in all three.

## 10. The buffer, and padding

| | Primary | Second | Large |
| --- | ---: | ---: | ---: |
| Buffer account length | 38,805 | 82,349 | 2,586,373 |
| Artefact (after 37-byte header) | 38,768 | 82,312 | 2,586,336 |
| ELF ends (section-header table end) | 38,768 | 82,312 | — |
| Trailing zero bytes | 15 | 15 | — |

All three buffers are owned by the loader, are in `Buffer` state, have the
vault as authority, and start with the ELF magic. **In each measured buffer the
artefact ends exactly where the ELF says it ends**: no allocation slack. The
15 trailing zeros belong to the ELF's own last section-header entry. So the
buffer artefact is byte-for-byte an ordinary built `.so`, and G1's definition
(every byte after the header, untrimmed) coincides with the file a developer
analyses. The simulation had predicted a padding risk; mainnet did not show one.
A buffer deliberately over-allocated (`write-buffer` with extra length) would
still be a different artefact, as specified.

## 11–13. Bind and live re-verification

| Step | Result | Slot | Message | Buffer | Status |
| --- | --- | ---: | --- | --- | --- |
| primary `acquire` | spec `34e78668…`, bytes stored at `store/programs/b10c1c93…` | — | — | 38,768 B | — |
| primary `bind` | **matched**, exit 0 | 450,318,819 | `f3760817…` | `b10c1c93…` | Active |
| primary `verify` #1 | **matched** | 450,318,858 | `f3760817…` | `b10c1c93…` | Active |
| primary `verify` #2 (30 s later) | **matched** | 450,318,970 | `f3760817…` | `b10c1c93…` | Active |
| primary `verify` #3 | **matched** | 450,319,541 | `f3760817…` | `b10c1c93…` | Active |
| second `bind` | **matched** | 450,319,467 | `47a88a98…` | `fe2bc73d…` | Active |
| second `verify` | **matched** | 450,319,541 | `47a88a98…` | `fe2bc73d…` | Active |
| large `bind` | **matched** | 450,319,753 | `4e97cb2c…` | `3f3e9803…` | Active |

Unbound ≠ bound IDs, the candidate SHA and the target are unchanged, and the
delivery message hash equals the real stored message hash. Every verify made
two new RPC reads, returned a later slot, and produced a new binding ID with
identical identities (§15).

This qualifies **governance binding** only. No impact analysis was run: there
is no replay bundle for these programs, and building one would be a protocol
phase (§17 of the brief).

## 14. Real-RPC negative controls

All ran against the primary proposal as it stands, with locally altered inputs:

| Control | Result | Exit |
| --- | --- | ---: |
| wrong `target.program_id` | `different_proposal` (`target_program_differs`) | 1 |
| wrong candidate SHA | `stale_artifact` (`candidate_differs`) | 1 |
| transaction index 2 (exists; its buffer is gone) | `unverifiable` (`proposal_reference_differs`, `buffer_missing`) | 2 |
| transaction index 3 (never created) | `unverifiable` (`proposal_reference_differs`, `account_missing`, `index_mismatch`) | 2 |
| wrong multisig | `unverifiable` (`proposal_reference_differs`, `account_missing` ×3) | 2 |

None defaulted to a match. The index-2 case is `unverifiable` rather than
`different_proposal`, by G1's precedence: missing evidence outranks a
reference mismatch. Both reasons are listed.

## 16. Hosted path

**Not exercised.** No legitimate Eplyx project or replay bundle exists for
`6pppX8…`, `ApwXwq…` or `PERPHj…`. Creating one only to make the endpoint
pass would invent a baseline, so it was not done. The hosted endpoint runs the
same `verify_squads_upgrade` over the same `HttpRpc`, and was qualified in G1
against the simulated chain. The CLI qualification is sufficient for G1.1-A.

## 20. Simulation vs mainnet

| | Simulation (G1) | Mainnet (G1.1) | Effect |
| --- | --- | --- | --- |
| Layouts, discriminators, PDAs, bumps | pinned | identical | none |
| Message key order | loader last | loader before sysvars | none (positional) |
| Spill | distinct key | distinct key in 93%; the vault in 5.9% of upgrade proposals | **reported, not admitted** |
| Buffer artefact | assumed possibly padded | exactly the ELF | none |
| Proposal status | current enum | pre-2023-06-23 proposals use the old enum | **reported** |
| RPC | one node | load-balanced; `-32016` on a lagging node | **fixed** (bounded wait) |
| Account size | small | 10 MiB ProgramData read atomically | none |

**Spill = vault.** Indexes `[1,2,3,0,5,6,0]`: the vault, already the writable
signer, receives the buffer's refund. The loader accepts this, and it is
arguably the safest spill there is. G1 refuses it because it requires seven
distinct keys. It is common enough to be one tool's deliberate choice (68
multisigs), but only 2 such proposals are live and bindable today. Admitting it
would be a narrow, well-defined relaxation: position 3 may be index 0. That
would be a G1.2 decision, not something to change silently here.

**Legacy ProposalStatus.** Before commit `8416203` the enum was Draft, Active,
Rejected, Approved, Executed, Cancelled. Afterwards it is Draft, Active,
Rejected, Approved, **Executing**, Executed, Cancelled. A proposal not
rewritten since that deployment behaves as follows:

- an old `Executed {ts}` (variant 4 plus a timestamp) does not decode under the
  pinned layout. G1 returns `unverifiable`, which fails closed. There are 7 in
  the census, all from July 2023, and a committed example is pinned by
  `a_legacy_proposal_status_fails_closed`;
- an old `Cancelled {ts}` (variant 5 plus a timestamp) is byte-identical to a
  current `Executed {ts}`, so G1 would *label* it Executed. Status is never part
  of identity or the outcome, and such a proposal is terminal and from 2023.
  The status label is the only error, but it cannot be detected from the bytes.

## 21. Recommendation for G2

G1's identity model is qualified on mainnet. Build G2 as planned:

1. **Post-execution verification.** When a bound proposal becomes Executed, the
   buffer is gone (G1 correctly says `unverifiable`). Compare the executed
   transaction's resulting ProgramData, header stripped, with the candidate.
   Mainnet shows buffers are unpadded, but ProgramData is padded up to its
   allocated length. The comparison must therefore be "prefix equal and the
   rest zero", with the ProgramData length recorded. It must not be a whole-hash
   equality.
2. **Status-transition re-verification** (the bot/CI hook), now that a plain
   public RPC was shown to be enough, including for a 10 MiB program.
3. Decide **G1.2 spill = vault** explicitly, with a test on the committed
   census shape. Do not widen further (LUTs, multiple instructions) until
   demand appears. The census shows those are not how upgrades are proposed.
4. Whether to decode the pre-2023 ProposalStatus numbering with a version hint
   is optional. It only affects labels on terminal 2023 proposals.
