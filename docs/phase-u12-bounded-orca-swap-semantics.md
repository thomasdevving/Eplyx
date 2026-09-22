# Phase U12 — Bounded Orca SwapV2 Semantics

## Result

The frozen U11.2 native-v0 transaction retains its `CheckpointedExecutionV1` contract-2 replay proof. A new [semantic bundle](examples/phase-u12-orca-semantic-bundle) attaches `orca-whirlpool@1` to the unchanged observation and corpus. With the historical Orca binary as candidate, ordinary and empty-environment bundle verification exit 0; CI exits 0 with `replay_proof.status = matched`, four covered subjects, no findings, and no failures. The bundle ID is `69402834a044953035811db1bda95c1e0a4d8fe8f851285cdb5499d26c0b02b6`.

The [frozen replay-only bundle](examples/phase-u11-2-checkpointed-bundle) remains `none@0`. It still verifies and replays with matched contract-2 proof, `NoSemanticCoverage`, exit 2, and its original CI stdout hash. The generic bundle reader accepts a historically unadapted corpus manifest; CI selects the adapter recorded in the bundle. Neither path changes replay execution.

## Supported interaction and proof sources

The adapter recognizes one successful direct Orca `SwapV2` instruction in a native-v0 transaction: Anchor discriminator `2b04ed0b1ac91e62`, 43-byte arguments, exact input, 15 ordered accounts, SPL Token A / Token-2022 B, and no remaining accounts. It checks memo and signer roles, writable roles, distinct addresses, and that companion outer instructions cannot write the measured user input or either vault. The pinned [Orca source](https://github.com/orca-so/whirlpools/blob/408c945fef4c49ab70def4303377cfaf8f0f3c99/programs/whirlpool/src/instructions/v2/swap.rs) defines the account order and `a_to_b` argument. The frozen argument has `amount_specified_is_input = true` and `a_to_b = false`: user token B is input; vault B receives it; vault A sends token A.

| Subject | Source of truth | Frozen measured amount, base units |
| --- | --- | ---: |
| Transaction outcome | Baseline and candidate runtime execution result | Both succeeded |
| `user_input_spent` | Predecessor user B Token-2022 account less final user B balance | 31,217,749 |
| `vault_a_tokens_out` | Predecessor vault A SPL Token amount less final vault A amount | 1,153,767,771 |
| `vault_b_tokens_in` | Final vault B Token-2022 amount less predecessor vault B amount | 31,217,749 |

The token account mint, authority, initialized state, owner program, and amount are checked through the shared SPL Token and Token-2022 decoders. Decimal scales (A: 9, B: 8) come from retained validator token-balance metadata after matching mint and token program; inconsistent or missing scales withhold that subject. Malformed or wrong-owner accounts cannot become zero amounts. Direction chooses which user account is input and which vault flow is inbound. Candidate logs and Orca events do not supply economic values.

The transaction creates a temporary wrapped SOL user A account before the swap and closes it afterward. Its transaction-boundary token account is absent. Thus this adapter does **not** report `user_output_received`: the retained final account state cannot independently measure it, and the current adapter result contains no intermediate token transfer detail. The vault A outflow is reported on its own terms, without equating it to net user receipt.

## Protocol binding size

The Orca adapter has 410 physical lines. Counting nonblank, noncomment source lines by contiguous source spans gives 78 for instruction recognition, argument layout and shape guards; 105 for account-role binding and standard-token reading; 150 for subject and finding mapping; **0 for CLMM or other protocol math**. The remaining 49 lines are imports and trait plumbing. The `a_to_b` argument selects economic direction; the exact-input flag is required. Stated amount, threshold, and price limit are not treated as executed quantities. The existing `ProtocolAdapter` seam expresses this bounded slice. Its transaction-aware `named_findings` method can derive the net flows; its older transaction-free `summarize` method cannot report them, so it remains empty.

## Mutations and compatibility

Six named Orca tests pass. They cover wrong discriminator, missing or aliased roles, a companion writer, reversed direction with and without a complete input account, SPL Token versus Token-2022 ownership, malformed pre and candidate token accounts, account-state precedence over contradictory logs, increased and decreased user input spend, changed vault flow, and execution reversion. The replay-only bundle test proves the same transaction still executes under `none@0` and yields `NoSemanticCoverage`; the semantic bundle test proves matched replay and four-subject coverage.

Ordinary and empty-environment semantic bundle verify stdout is byte-identical, SHA-256 `b334c3380890fe5c8f6d4154966c292323c621fc20abdf826183d864a405608a`. Ordinary and empty-environment semantic CI stdout is byte-identical, SHA-256 `e4fa879f568de5eb7301e34fa06a536513a2b574f424a7bb9af9f53f185e74e0`.

The 423 engine unit tests and 22 focused universal/checkpoint tests pass. U10 ordinary and empty-environment replay retain stdout SHA-256 `12635f15e3fb2ffae466cccf7985c2edfaefaffb39b55d28a094892e07864edd`; all seven U10 mutations fail closed. Frozen U11 contract-1 verify/CI hashes remain `a1a05576abb0ee215080890c6d413263205c888217e7a9eb85627707783b27b9` / `d2c79c7d00abc8df7ae729f063d43edde089754256b731372ff471c3747c358e`. Frozen U11.2 contract-2 CI remains exit 2 with stdout SHA-256 `0114c76d03fc8c294d309e0fecb4e7d554f4c5b0dd71ed8aaf973c7cb69476c7`. Stake Pool exits remain `0/0/1/1/3/5`; all six outputs match the U11.2 frozen controls. Kamino verify and baseline CI exit 0 with their frozen output hashes. The generic replay, corpus, bundle, CI, and command code contains no Orca name, target address, SwapV2 name, tick term, or CLMM branch.

## Limits

This qualifies one successful direct SwapV2 shape, one runtime epoch, and one closure, not general Whirlpools behavior. It does not infer user output receipt, price impact, fee allocation, tick crossings, liquidity or LP economics, or slippage beyond the retained instruction arguments and measured account effects. Opposite-direction flow tests are controlled semantic states, not new historical replay qualifications. Source account order is pinned to the cited Orca source and corroborated by the frozen successful execution; this phase does not establish source-to-historical-bytecode equivalence.
