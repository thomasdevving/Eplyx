# Phase U17.2 — Phoenix historical replay qualification

## Decision

**U17.2-D — historical runtime evidence gap.** Exact account state, the full
transaction block, and every invoked BPF binary were acquired for the Phoenix
transaction at slot `450026714`, index `95`. The backend-known feature
inventory requires 349 exact-slot account receipts; only 62 were acquired
before the public Alchemy Account Archive demo repeatedly returned HTTP 429.
No archive credential is configured in this worktree. The remaining **287
feature identities** are named in the retained `runtime-partial.json`.
Consequently there is no qualified feature-set identity or historical
`RuntimeProfile`, and **no execution replay was attempted**. No schema-2
observation, corpus, bundle, CI result, or Phoenix adapter was created.

The [partial evidence archive](examples/phase-u17-2-phoenix-evidence/README.md)
retains raw provider response bodies, manifests, and offline audits. Its
compressed SHA-256 is
`dffad5526dbfa4240d2f6cb1628488825e970773b891da72304026defb7af684`.
The archive's exact-slot `getAccountInfo` semantics are documented by
[Alchemy](https://www.alchemy.com/docs/solana/account-archive). These are
provider observations, not cryptographic validator attestations.

## Parent checkpoint frontier and absence

The frozen `LegacyV2` message has 18 ordered static keys, no LUT. All were
queried at parent slot `450026713`, with complete raw data where present.
The three upgradeable ProgramData accounts below are additional implicit
loader inputs. Runtime sysvars are a separate input set. The exact receipt,
owner, lamports, rent epoch, executable bit, space, and data SHA-256 of every
account are in `acquisition.json` inside the archive.

| Index | Address | Parent classification |
| ---: | --- | --- |
| 0 | `8LzcWniAwV2pa2q3itGmQcWGgbJjU4CVk3qQ9K2tDYKq` | Present; writable signer and fee payer |
| 1 | `2tbbNnesHrUJkwaMLMEEDXpjeLb6o7xbieGy7UWhutwA` | Present; writable SPL Token account |
| 2 | `2vfxhTAgQYJTojyRELrUEdcP2Frshb4L43rYooR1sZtJ` | Present; writable SPL Token account |
| 3 | `6BVnCYbEQZXkDZuQyaKTQMPs4HjpKcd7poMxKnRNEX32` | Present; writable 445,536-byte Phoenix market |
| 4 | `7tmGVJHYZZyqgqiWNPWQ86p1YTnx4ezXUhW5CWgkdRGn` | Present; writable SeatManager-owned account |
| 5 | `BAmjhnnpAGCL1uPND16JJxTzUQhEhis6P9h7LTv4wop8` | Present; writable Phoenix seat |
| 6 | `DNkx5wNGDkLyDk37HtEdocxayyes9K7dWtg4XsFkuiUV` | Present; writable SPL Token vault |
| 7 | `Fnum6HcWtVFtDo21fKJZJZDRKT49Mj8EcxDvnHg1M9ws` | Present; writable SPL Token account |
| 8 | `FpMeYdAtUP6ePNRiiqn29NTQ1MYRbXYF8wBdb3Ki7qEq` | Present; writable system account |
| 9 | `11111111111111111111111111111111` | Present; readonly native System program |
| 10 | `3NZ9JMVBmGAqocybic2c7LQCJScmgsAZ6vQqTDzcqmJh` | Present; readonly SPL Token mint |
| 11 | `7aDTsspkQNGKmrexAN7FLx9oxU3iPczSSvHNggyuqYkR` | **Proven absent**; readonly Phoenix log authority |
| 12 | `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL` | Present; readonly legacy BPF ATA program |
| 13 | `ComputeBudget111111111111111111111111111111` | Present; readonly native ComputeBudget program |
| 14 | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` | Present; readonly SPL Token mint |
| 15 | `PhoeNiXZ8ByJGLkxNfZRnkUfjvmuYqLR89jjFHGqdXY` | Present; readonly upgradeable Phoenix Program |
| 16 | `PSMxQbAoDWDbvd9ezQJgARyq6R9L5kJAasaLDVcZwf1` | Present; readonly upgradeable SeatManager Program |
| 17 | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` | Present; readonly upgradeable SPL Token Program |

The additional parent ProgramData addresses are
`2myyNegEA6pjAHmmEsJC6JdYhW51gwxQW7ZCTWvwaKTk` (Phoenix),
`3vDphnMGqw28P4Vn4SGj2x4BDbHDziGskcJeQt5LyxAH` (SeatManager), and
`3gvYRKWyXRR9xKWe1ZjPhLY5ZJRN7KDB4rFZFGoJfFk2` (SPL Token).
The only absent message key is readonly and remains an absence proof; no
account was fabricated or assumed created. The two ATA targets already exist
at PRE.

## Executables and historical ELF

The complete outer-instruction and validator CPI census invokes ComputeBudget,
SeatManager, ATA, Phoenix, and SPL Token. System is a native program supplied
as an instruction account. The Phoenix log CPI invokes Phoenix again, not a
sixth BPF program. Program accounts were read at the exact parent slot; every
ProgramData account was read at both parent and target slots. Upgradeable pointers, loader owners,
deployment slots, authority fields, complete length, 45-byte header, ELF
offset, and deterministic slice coverage were checked. The complete parent
and target ProgramData images are byte-identical for each program.

| Program | Historical ELF SHA-256 | ProgramData length | Deploy slot |
| --- | --- | ---: | ---: |
| ATA, legacy BPF | `6804554e69fd3a58caa191dc4a58f4c67223d30ca28ab8987f39fc18d2f7374d` | 105,032-byte Program account | — |
| SeatManager | `f10dd903a4a53c10d894b34c7876feefd23b41e04d8ba481f10deda66f946345` | 3,000,045 | 244,933,787 |
| Phoenix | `e3d76e5f479989823847fd77b4f96a9c8ce4255dafefcacf920a99643e15420f` | 5,000,045 | 231,433,470 |
| SPL Token | `8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697` | 108,645 | 419,472,000 |

The exact historical Phoenix ELF equals the retained **current** ELF hash from
U17. This equality follows from complete exact-slot account bytes; the old
deploy-slot timestamp alone was never used as the proof. No source-to-ELF
build equivalence is claimed.

## Runtime and blockhash status

Exact target-slot Clock, Rent, and EpochSchedule accounts were acquired.
The archive returned null for RecentBlockhashes and SlotHashes, consistent
with its documented per-slot-sysvar exclusion. Their materiality has **not**
been tested by a qualified replay. The 349 backend-known feature identities
come from the pinned U13.2B universe; 62 target-slot receipts were retained,
287 remain missing after repeated HTTP 429. The later U13 feature-set hash
cannot be reused at this slot. Historical `RuntimeProfile` identity and
feature-set identity are therefore **unavailable**. Native program behavior,
fee model, and ComputeBudget compatibility also remain execution questions.

The first outer instruction is ComputeBudget, not System
`AdvanceNonceAccount`: this is an **ordinary recent-blockhash** transaction.
Its message blockhash is `8hjQjz5znBmGoJiaq1X4QBVTzSbtfZ5PSdxXg67YGECG`;
the independently retained target block reports previous blockhash
`5LejCsFV9CWSRv4D2e1zCMPkkkGjpNeUorRM5YiLAzyb` for the runtime
environment. No nonce machinery was imported.

## Closure, terminal evidence, and companion boundary

The complete finalized `getBlock` response contains 1,116 transactions and
has SHA-256 `c45f20982728b1935ab764ec48b9c7b14501ac3cf99ca8daeaf057ae5475d058`.
Its parent slot, blockhash, and all ordered signatures agree with the retained
account-mode block. Its tx95 message and validator envelope agree with the
frozen `getTransaction` response, apart from the RPC representation of an
empty rewards field. Recomputed access sets find zero earlier possible
writers to any of the 18 message keys, zero later possible writers to any of
the nine writable keys, and zero slot writers to the three ProgramData
addresses. This supports the **proposed** parent and terminal checkpoint
placement. No schema-2 contract-2 closure proof was created or run.

All nine writable accounts have exact target-slot terminal receipts, with
complete owner, lamports, executable, rent epoch, and data. Their validator
post-lamports match 9/9; parent lamports match 18/18; four pre and four post
SPL Token amounts match raw account bytes 8/8. Across the two archive
boundaries, data changes in the trader base token account, Phoenix market,
and base vault; the other six writable data images are unchanged. These are
archive-to-validator metadata checks, **not local execution comparisons**.

The complete replay target would include all six outer instructions. Before
Phoenix outer instruction 5, SeatManager outer instruction 2 names the market
and seat, so neither is proven untouched at the instruction boundary. ATA
companions name the trader base and quote token accounts. Neither Phoenix
vault is named by an earlier outer instruction, and the validator reports no
CPI group for the companions; both vaults are untouched by those earlier
instructions under the retained instruction/CPI envelope. This does not
establish an order state or token fill.

## Replay, CI, mutations, and next gate

The retained validator envelope reports success, fee 40,001 lamports, 39,955
compute units, 30 log lines, one inner group with two instructions, and
return data. No local execution comparison, full account reconciliation,
SlotHashes/RecentBlockhashes materiality test, mutation campaign, schema-2
observation, corpus/bundle identity, or CI check exists for U17.2. The
**intended** successful fidelity route remains
`CheckpointedExecutionV1` contract 2, not `CompleteExecutionV2` or contract
3; no profile was selected in a productized observation.

To resume U17.2, supply an exact-slot Account Archive credential through
`SOLANA_ARCHIVE_RPC_URL` and acquire the **287 named feature receipts** at
slot `450026714`. Then construct and validate the historical feature set and
runtime profile, test per-slot-sysvar materiality, execute the complete
legacy transaction, reconcile all nine complete terminal snapshots and the
validator envelope, and only then build a replay-only `none@0` corpus/bundle
and run the requested mutations. **U17.3 is not ready.**
