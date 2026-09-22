# Phase U11.2 — Reconstructive Checkpoint Proof

## Result

The new `CheckpointedExecutionV1` observation uses proof contract 2. It passes the ordinary schema-2 path `ReplayObservationV2 → CorpusStore → bundle build → bundle verify → ci check` with no provider calls. The frozen U11 corpus and bundle remain unchanged and verifiable under their explicit historical contract 1.

| Identity | SHA-256 |
| --- | --- |
| Observation | `3c29a93121461bc22305bde8ec5c6ef89fb1da01c7427578ef52e86a8e5fe528` |
| Corpus | `fda1f235075defd77765b9876f12d67fa1751006b71d010afb771d3cd931e526` |
| Bundle | `22c2b97360a5d22e68074a051ab05f7716f918ee0b72f946c1643593d0d2071b` |

The corpus is at [phase-u11-2-checkpointed-corpus](examples/phase-u11-2-checkpointed-corpus); the bundle is at [phase-u11-2-checkpointed-bundle](examples/phase-u11-2-checkpointed-bundle). The new converter reads only retained repository evidence and writes to an empty output directory.

## Reconstructed facts

`DerivedAccountEvidenceV2` has two typed derivations. It parses the retained Yellowstone account event and independently recovers the address, event slot, owner, lamports, executable flag, rent epoch, and complete data. It rejects a transaction signature on either runtime event. For SlotHashes it also verifies the 512-entry layout and the predecessor-slot head. The resulting complete `AccountSnapshot` must equal the stored content.

For RecentBlockhashes the verifier parses the 150-entry post-slot event, the retained finalized `getBlock(448760809)` response, and its acquisition receipt. It checks the receipt's requested slot, finalized commitment, successful response and response hash, the tail block's parent relation, and the event's tail-block hash. It removes the post-slot head, appends the block response's `previousBlockhash`, carries the tail fee-per-signature, serializes the 150-entry pre-target image, and compares the complete account against stored content. The retained `RecentBlockhashes.pre-transaction.bin` is only the conversion's expected value; the resolver does not read it.

An ordinary archived `AccountObservation` cannot be captured or resolved at `BeforeTargetExecution`. That boundary is supported through the typed derived path in contract 2.

The closure verifier binds the account-mode block to the full retained block by identity, transaction count, and every signature in order. For each failed later validation-output overlap, it independently recovers the fee payer from the first resolved account key, combines static keys with loaded writable and readonly addresses, and recognizes a durable nonce only from a first System `AdvanceNonceAccount` instruction. It compares the result with both the failed-overlap annotation and the conflict census, including per-account nonce and fee-payer flags. All six frozen failed overlaps reconstruct an empty durable-nonce set. The implemented fee-payer and durable-nonce rollback rule is selected by a versioned profile bound to the retained runtime evidence and the pinned Agave source manifest, rollback source, and transaction processor source.

## Observed and derived evidence

The predecessor and terminal checkpoints, validator transaction and block responses, Yellowstone account events, tail-block response and acquisition receipt, historical program bytes, and pinned runtime/source files remain retained observations or source evidence. The pre-target RecentBlockhashes image, SlotHashes execution-boundary image, resolved runtime profile, transaction closure conclusion, and deterministic target execution are derived and rechecked. The conflict census corroborates failed-overlap facts; it does not choose the fee payer or nonce account.

## Verification and mutations

Ordinary and restricted-environment runs produced identical raw stdout hashes:

| Command | Exit | SHA-256 |
| --- | ---: | --- |
| Bundle verify | 0 | `dfaa71f7ce2ed5ad2030cdd9645e513cf2ed9c20a77320f317dbfac76b04bcb2` |
| CI check | 2 | `0114c76d03fc8c294d309e0fecb4e7d554f4c5b0dd71ed8aaf973c7cb69476c7` |

CI reports `replay_proof.status = matched`, `proof_contract_versions = [2]`, and `no_semantic_coverage`. Its exit 2 is the semantic coverage gate. Historical contract-1-only CI output omits the new field and remains byte-identical.

Named mutations reject changed stored RecentBlockhashes content with rebuilt hashes and identity, a rehashed tail predecessor, a rehashed Yellowstone event, a missing tail source, an incompatible derivation type, a direct archived before-target observation, and changed stored SlotHashes content. Closure mutations reject an omitted or fake failed overlap, a false nonce classification added to both annotation and census, a real nonce omitted in a synthetic full-transaction witness, a false fee payer, missing or changed rollback provenance, and missing full-block evidence. Mutations use a separate evidence-store copy; the frozen artifacts are untouched.

The 423 engine unit tests and 22 focused universal/checkpoint tests pass. U10 ordinary and restricted-environment replay remain byte-identical at `12635f15e3fb2ffae466cccf7985c2edfaefaffb39b55d28a094892e07864edd`; all seven U10 mutations fail closed. Frozen U11 bundle verify and CI retain hashes `a1a05576abb0ee215080890c6d413263205c888217e7a9eb85627707783b27b9` and `d2c79c7d00abc8df7ae729f063d43edde089754256b731372ff471c3747c358e`.

Stake Pool exits remain `0/0/1/1/3/5`. Verify, baseline, stale, and unevaluable stdout match their frozen hashes; regression and bounded retain the two known missing-historical-binary report differences. Kamino ordinary and restricted-environment verify/CI remain exit 0 with frozen hashes `529355d96e95bf81c44debd50a007f3624612009cffcf5cee47cc631d05e3b18` and `b7f33953ca35bcdc7949a52a9ea5263739e1be20a1c00ddc1d0d7c89f1a8fe44`. The core/product source scan finds zero protocol names, target signature/address, or requested instruction-family terms.

## Limits

Contract 1 remains available so historical U11 records can verify; it still checks source existence rather than reconstructing derived account content. A caller must distinguish contract 1 from contract 2 when interpreting a matched replay. Contract 2 is the reconstructive claim. The retained acquisition receipt and source manifest bind bytes and declared provenance but are not cryptographic attestations from a validator. The pinned rollback source is an Agave source revision supporting the implemented rule; the exact historical validator build remains unclaimed. This remains one successful native-v0 durable-nonce target, one runtime/feature epoch, four stable LUTs, and a one-transaction closure. Other epochs, failed targets, deactivating LUTs, and multi-transaction closures remain unqualified. No protocol semantics were added.
