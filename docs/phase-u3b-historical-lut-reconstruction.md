# Phase U3B.2 — production historical v0 LUT reconstruction

**Decision A: HISTORICAL LUT RECONSTRUCTION PROVEN — strong A, 8/8.**
Historical archive evidence, cross-checked against frozen production transaction
address resolution under canonical ALT semantics, independently reconstructs all
eight frozen v0 messages. **8/8 are historically LUT proven, 8/8 advance past the
old LUT gate, and 0/8 remain unproven.** All eight then meet an unchanged admission
rejection. No Kamino execution, baseline fidelity or production replay eligibility
is established.

The control remains `d20c764916aa8ceb84bb8fbb69d1c1c88406c23c`. This continuation
belongs to the commit containing this document and its new proof artifacts.
All **344 U3A checksums** and both offline baseline rebuilds preserve fingerprint
`b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af` and policy `3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3`. The denominator remains seven successful
and one failed on-chain LUT transaction. The ninth legacy System-companion case
is unchanged and excluded from this experiment.

## Acquisition history and the corrected trust boundary

Attempt 1 is retained byte for byte under [phase-u3b-lut](examples/phase-u3b-lut/acquisition.json):
no archive configured, no requests, 0/8 proofs. Its original report and prose are
retained below as a historical snapshot.

Attempt 2 supplied the already documented public historical archive through
`SOLANA_ARCHIVE_RPC_URL` and its public Origin through
`SOLANA_ARCHIVE_RPC_ORIGIN`. The sandbox could not resolve the host
(`transport_exit_6`). Its genesis request failed before any table request;
[attempt-2 membership](examples/phase-u3b2-validation/attempt-2/acquisition.json)
retains all sixteen unattempted contexts. No previous directory was overwritten.

Attempt 3 used the same public configuration with authorized network access.
[Acquisition](examples/phase-u3b2-lut/acquisition.json) retains sixteen successful
exact-slot table responses and the raw mainnet genesis response. Provider identity
is `https://solana-mainnet.g.alchemy.com`; private credentials were neither read
nor used. The collector remains provider-neutral and receives configuration only
through the environment. It makes one bounded request per context, with no retry
chain or current-state fallback.

Layer 1 reuses Eplyx's existing historical-account qualification, documented in
[Phase 6](phase-6-historical-state.md), with its retained mainnet record and
provider source bound in [archive-validation.json](examples/phase-u3b2-lut/archive-validation.json).
Selecting the previously qualified endpoint is an explicit provider trust boundary;
a flag does not independently qualify an arbitrary provider. The separately
supplied divergent-LUT fixture is now **optional defense in depth**. None was
supplied, and its absence does not block acquisition.

Layer 2 remains mandatory for each frozen transaction: historical table bytes,
the pinned official codec and execution-slot rules independently resolve the
original lookup indexes. Only then are writable and readonly vectors compared
with frozen RPC metadata, requiring identical counts, pubkeys and order.
No frozen loaded address is used as a resolution input.

## Finalized end-of-S boundary

The final query is `getAccountInfo(pubkey, {encoding:base64, commitment:finalized,
slot:S})`, requiring `context.slot == S`. Eplyx's already qualified archive returns
the latest account write at or before finalized S. The provider's
[point-in-time documentation](https://www.alchemy.com/docs/solana/account-archive)
corroborates this boundary; an echoed slot by itself remains insufficient.

The pinned ALT interface **3.1.0** supplies `AddressLookupTable::deserialize`,
`get_active_addresses_len` and `lookup`. The official
[ALT processor](https://github.com/solana-program/address-lookup-table/blob/main/program/src/processor.rs)
appends addresses without moving the old prefix. Its first extension in a slot
records the old length; later extensions in that slot preserve the start index.
Official lookup at S excludes that same-slot suffix. Deactivation at S remains
usable. Earlier deactivation requires execution-bank SlotHashes, and closure
requires completed cooldown. Therefore a table usable earlier in the same bank
cannot close later in S. End-of-S bytes suffice for lookup-result equivalence
under these canonical rules; they do not prove transaction-position protocol
pre-state or historically pinned program ELF identity.

All acquired tables have the ALT owner, positive lamports, `executable=false`,
`last_extended_slot < S` and `deactivation_slot=u64::MAX`. Consequently **zero
SlotHashes contexts were required or fetched**. Warmup/deactivation controls
remain active and fail closed when evidence is required. Requested privileges
come only from the original header and lookup class; loaded keys are never
signers. Duplicate keys are rejected without deduplication. Runtime reserved-key
or program demotion is outside this experiment.

## Measured eight-transaction result

[targets.json](examples/phase-u3b2-lut/targets.json) is rederived from the frozen
captures: **8 transactions, 10 distinct LUT pubkeys, 16 exact table/slot contexts**.
It retains every signature, static field, descriptor/index vector, loaded count
and exact table pubkey. [final-report.json](examples/phase-u3b2-validation/final-report.json)
contains all requested/returned contexts, raw account/envelope hashes, owners,
metadata, independent writable/readonly vectors and exact next rejection text.

S1 acquisition; S2 execution-slot validity; S3 independent resolution;
S4 frozen RPC comparison; S5 full key vector; S6 compiled instruction identity;
S7 previous LUT gate; S8 measured next admission rejection. “Passed” denotes a
completed proof stage, never runtime replay success.

| Signature | Slot | Tables | Loaded W/R | On-chain success | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S8 |
|---|---:|---:|---:|---|---|---|---|---|---|---|---|---|
| `5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97` | 448195166 | 1 | 6/7 | yes | passed | passed | passed | passed | passed | passed | passed | unsupported HFn8Gn… companion |
| `3zswY7FXGLsVCVJTg8LawCQZp27xBza4M4Cp7mtBm2LxP97kn8eZrXPkCCJv3dt3qMb9fvRXKT3qZTXiucgR6y4Q` | 448195147 | 5 | 33/19 | yes | passed | passed | passed | passed | passed | passed | passed | multiple KLend actions |
| `47XZ8dNPUBxMTnJ3zrxeCKt37PRsckvhKzPSutdUp2NavU1UTTNYRd7rnvEP3pSmHQszrLW3cdB56zTw3kyrwZUf` | 448195003 | 5 | 37/18 | no | passed | passed | passed | passed | passed | passed | passed | original transaction failed |
| `37XJKQbB9fRX1R76cKKGSENNNA2RSY1WvjkCx2Q7joN2irKfTTZ5Z6bHgH4rfgDPJvhXf3peTe4mwxisfpSEmimu` | 448194959 | 1 | 20/10 | yes | passed | passed | passed | passed | passed | passed | passed | multiple KLend actions |
| `55EdbRGcpNETQZ2AerE4KuvKC1xKCFzHkFHxxN98gjz7DWzGCGbgXKSXW6q3SoABZBbvEsEKb6sVBt9eGThj2kvk` | 448194517 | 1 | 7/4 | yes | passed | passed | passed | passed | passed | passed | passed | unsupported HFn8Gn… companion |
| `5NKNdj527vBVpCGhNdeGwpj9LdQfpSCxSH3b8oaKkLe3EzKbZdrsc2fR7yCuCkYgt8EmYXFDwJidi15y8ZQNY43V` | 448194462 | 1 | 4/5 | yes | passed | passed | passed | passed | passed | passed | passed | unsupported HFn8Gn… companion |
| `49zi8hsUATfn3oM7x3bx8fBWULY9mN6DGHtFCVWRLcuPJFyBkP54XVdZGWbqVecwHzsSzFkVQdvLKPp8r8929Amx` | 448194435 | 1 | 7/7 | yes | passed | passed | passed | passed | passed | passed | passed | unsupported HFn8Gn… companion |
| `2dc94E8Mvh83c8MiecgRekd7Vj8YE6aexfVeCJK58K46LQcTRjpDomii6pYmwbdeYmdTM3bVbHw6rhFdtwRtSTnT` | 448194341 | 1 | 6/7 | yes | passed | passed | passed | passed | passed | passed | passed | unsupported ATA |

Each [transaction proof](examples/phase-u3b2-lut/stage-table.json) binds the frozen
fingerprint, raw transaction hash, native v0 message, historical evidence
identities, independently loaded vectors and full requested-privilege key vector.
Table diagnostics retain metadata, requested indexes and resolved pubkeys;
compiled diagnostics retain program/account indexes, resolved identities and raw
bytes. Native `VersionedMessage::V0` is preserved. The sealed `ProvenV0` still
requires reconstruction; serialized proof flags cannot authorize admission.

**BEFORE: 8/8 first rejected on LUT addresses. AFTER: N=8/8 historical LUT proofs,
M=8/8 advanced past that gate, 0/8 unproven.** N and M are equal. Ordinary admission
and ReplayRecord validation retain the old LUT rejection. Only the experimental
sealed-proof path reaches unchanged instruction admission.

The actual S8 distribution is **4 HFn8Gn… companion rejections, 1 ATA rejection,
2 one-action-contract rejections and 1 failed-original rejection**. The failed
55-loaded-address transaction remains in the denominator and proves its message
before its original failure is observed as the next admission blocker. These
results do not establish later lifecycle/state/runtime blockers.

## Offline reproduction and validation

```bash
./scripts/rebuild-kamino-u3b-lut-proofs.sh --evidence docs/examples/phase-u3b2-lut --verify
./scripts/rebuild-kamino-u3b-lut-proofs.sh --evidence docs/examples/phase-u3b2-lut --verify --reverse --workers 4
python3 scripts/test_kamino_u3b_luts.py
cargo test --offline -p eplyx-engine --test historical_lut
python3 scripts/mutate-kamino-u3b-luts.py --production --output /tmp/eplyx-u3b2-mutations.json
```

The rebuild strips RPC/archive/provider configuration and calls no transport.
Normal/reversed input, four independent processes, directory relocation and
reversed filesystem enumeration preserve every canonical byte.
[Determinism](examples/phase-u3b2-validation/determinism.json) records the hashes.
[The complete evidence inventory](examples/phase-u3b2-lut/inventory.sha256) covers
raw responses, qualification, acquisition membership, proofs and separate timing.

**544 normal tests passed**: 421 library, 4 historical, 27 LUT, 10 Kamino,
16 replay, 12 Token-2022, 24 acquisition and 30 U3A integrity tests.
[Production regressions](../engine/tests/historical_lut.rs) use actual acquired
bytes for a single table, five tables/52 loaded addresses and five tables/55
loaded addresses, checking official decode, slot-aware lookup, both exact vectors,
full key privileges and every compiled program/account/data identity. Wrong-byte
and RPC-without-evidence controls reject. A separately labeled visibility test
uses real table bytes in an **explicitly synthetic message/bank context** at the
extension boundary and ±1; it does not add a production proof. None of the actual
execution-slot ±1 values crosses an extension boundary.

**20/20 behavioral mutants were killed by named assertions**, including all
12 original faults and 8 additional evidence controls. Seven additional controls
use acquired frozen production evidence; the eighth uses the explicitly synthetic
visibility context with real bytes. No compiler failure is counted, every named
unmutated control passed, and isolated sources were restored. The core source
hash remains identical before/after. [Exact mutation results](examples/phase-u3b2-validation/mutations.json).
All six [product reports](examples/phase-u3b2-validation/reports/controls.json)
retain U2 byte hashes and exits 0/0/1/1/3/5. Clippy with warnings denied, Rust
formatting and `git diff --check` passed. Attempt 1 still rebuilds to its original
failure artifacts.

## Timing, storage and next phase

Attempt 3 made **17 requests: one genesis and sixteen account-state requests;
16/16 table requests succeeded, 0 failed**. Their measured sequential transport
wall-clock durations total **5.155 seconds**. Full
capture overhead was not instrumented for this attempt; local qualification,
decoding and file writes are excluded. The collector now measures full capture
time separately for future attempts. No pricing or wider latency estimate is made.
Normal offline rebuild took **0.685 s**;
reverse/four-worker rebuild took **1.178 s**.

The complete evidence directory is **2,052,941 bytes
in 36 files**. Six repeated contexts reuse ten identical historical
account byte states: **46,896 distinct account bytes, 75,424 referenced bytes and
28,528 duplicate referenced bytes**. Transaction artifacts total
**1,806,442 bytes**, including complete official
decoded tables and human-readable diagnostics. Direct retention is reasonable
for this bounded experiment; no content-addressed deduplication was added.
These are experimental audit storage figures, never ReplayRecord overhead.

[Secret scan](examples/phase-u3b2-validation/secret-scan.json) found no credential
fields or credential-bearing URLs. No credentials or environment dumps are
included. Historical truth remains conditional on the qualified archive and
canonical ALT semantics; no cryptographic ledger inclusion proof is claimed.

**Decision A proves historical v0 address reconstruction for these eight frozen
production transactions.** S8 favors a next phase addressing the measured
companion contract (5/8), with multiple-action scope (2/8) and failed-original
policy (1/8) retained as separate blockers. Define that phase from the measured
rejections. U3C, Kamino runtime execution, companion/lifecycle support, ReplayRecord
migration, contract v2, hosted changes and adapter #4 were not implemented.

## Attempt 1 — retained pre-acquisition snapshot

The following is the original U3B record. Its mandatory divergent-fixture rule
is superseded by the two-layer standard above; its acquisition failure evidence
and numerical results remain authentic historical results.

<!-- U3B.2 retained-attempt-1 -->
# Phase U3B — historical v0 LUT reconstruction

**Decision B: PARTIAL LUT RECONSTRUCTION — SOME FROZEN TRANSACTIONS REMAIN UNPROVABLE.**
The generic reconstruction infrastructure is implemented; **0/8 real historical
proofs are established** because no historical account archive is configured.
This is an acquisition stop condition, not evidence that canonical LUT
reconstruction failed. U3B production acceptance remains outstanding.

The frozen U3A control is commit `d20c764`; sample fingerprint
`b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af` and policy
`3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3` were verified
offline before implementation. All six product reports match U2 byte hashes;
Token-2022 controls passed 12/12 and Kamino controls passed 10/10.

## Engineering model recorded before implementation

The ALT owner is `AddressLookupTab1e1111111111111111111111111`. Use the pinned
`solana-address-lookup-table-interface` 3.1.0 `AddressLookupTable::deserialize`,
`LookupTableMeta`, `get_active_addresses_len` and `lookup`. The official codec
handles the state discriminant, 56-byte metadata area and trailing 32-byte
addresses. Eplyx must not implement account byte offsets itself.

At execution bank slot S, an active table exposes all addresses when
`S > last_extended_slot`. When `S == last_extended_slot`, only the prefix before
`last_extended_slot_start_index` is usable. Each extension appends addresses;
the first extension in a slot records the old length and later extensions in
that slot preserve that start index. Existing address positions never change.
Reject metadata with extension or deactivation slots after S: those bytes cannot
be an end-of-S historical snapshot.

`deactivation_slot == u64::MAX` is active. Deactivation at S remains usable at S.
For an earlier deactivation, usability depends on membership in the execution
bank's SlotHashes, using the official implementation. A missing historical
SlotHashes snapshot fails closed; wall-clock cooldown estimates are insufficient.
SlotHashes is bank context (recent ancestor slots), not a list inferred from
transaction slots. Validate raw sysvar bytes with an official codec and retain
its historical context and owner.

The proposed supported archive boundary is **finalized end-of-execution-slot S**,
not S-1. This is sufficient for *lookup-result equivalence*, even if a table was
extended after this transaction within S: applying warmup to end-of-S bytes
excludes the entire suffix appended in S, while the old prefix is unchanged.
Same-slot deactivation also preserves usability. Closure is allowed only after
cooldown, so cannot remove a table usable earlier in the same bank context.
An absent/closed end-of-S table is rejected conservatively. Authority changes
do not affect lookup results. This argument does not prove exact transaction-
position account bytes or protocol pre-state. It relies on canonical ALT program
semantics and an archive independently validated to return end-of-S state;
an echoed `context.slot` alone does not establish that provider capability.

Writable and readonly indexes resolve separately, in descriptor order and index
order. Concatenate static keys, all loaded writable keys, then all loaded
readonly keys. Loaded keys are never signers. Requested writability comes from
the header and lookup vectors; runtime program/reserved-key demotion is a
separate pinned-runtime concern. Record requested privileges; reserve runtime
demotion for execution integration.
Do not infer privileges from balance changes or account ownership.

The lock validator rejects duplicate keys (`AccountLoadedTwice`), including
static/loaded, repeated indexes and overlaps across tables. Do not deduplicate.
The official v0 message sanitizer validates header, the 256-key bound, every
compiled index and that top-level program IDs are static. Instruction account
positions, repeated instruction references and raw data are preserved.

The lockfile resolves LiteSVM 0.16.0, solana-message 4.4.1 and
solana-transaction 4.1.6 (the manifest version requirements are lower bounds).
LiteSVM natively accepts VersionedTransaction/VersionedMessage::V0, loads ALT
accounts through its AccountsDb, and uses Clock/SlotHashes and the same official
ALT lookup. Eplyx currently rebuilds legacy messages. U3B returns an
explicit native v0 message in a parallel experimental proof path; durable replay
integration and ReplayRecord migration remain later work.

Message equivalence includes version, header, recent blockhash, static keys,
lookup descriptors, independently loaded vectors, full key sequence, compiled
program/account indexes, resolved account identities, requested privileges and
instruction bytes. Signatures are retained as transaction identity but are not
cryptographically reverified by this reconstruction experiment.

Primary references: [official ALT processor](https://github.com/solana-program/address-lookup-table/blob/main/program/src/processor.rs),
[ALT interface 3.1.0](https://docs.rs/solana-address-lookup-table-interface/3.1.0/solana_address_lookup_table_interface/state/index.html),
[v0 loaded message](https://docs.rs/solana-message/4.4.1/solana_message/v0/struct.LoadedMessage.html),
[LiteSVM 0.16.0](https://docs.rs/litesvm/0.16.0/litesvm/struct.LiteSVM.html).
The local Cargo registry source and lockfile are the implementation controls.


## Acquisition and proof design

Generic infrastructure lives in [`engine/src/message.rs`](../engine/src/message.rs),
not the Kamino adapter. `FrozenV0` carries the original compiled native v0 message
in parallel with the unchanged normalized transaction. No normalized JSON fields,
ReplayRecord schema, ActionIds, semantic subjects or production bundle changed.
The existing adapters' message gate moved to the trait's default `accept`; their
instruction contracts retain identical checks and order. An experimental
`accept_reconstructed_message` takes only a sealed `ProvenV0`, so a deserialized
proof flag cannot bypass the gate. Ordinary admission and ReplayRecord validation
continue to reject LUT-backed messages. No Kamino runtime execution is attempted.

`HistoricalAccountEvidence` preserves an entire raw envelope (base64 in the core
API), account-data SHA-256, envelope SHA-256, table pubkey, requested/returned/
execution slots, genesis, provider scheme/host, visibility and archive-validation
artifact hash. Its identity binds all of these. The standalone collector stores
exact transport response bytes in separate files; the existing `RpcProvider`
helper stores a canonical envelope because that abstraction returns decoded
results. These are different preservation boundaries, explicitly documented in
source. Neither acquisition path falls back to current state.

The acquisition command is provider-neutral and uses the existing exact-slot
`getAccountInfo` convention. It verifies the archive's raw `getGenesisHash`
response against the frozen mainnet genesis. It first validates a LUT at two
historically divergent slots against independently supplied account-byte hashes,
with retained archive-semantics and independent-fixture references. Merely
receiving two different snapshots does not validate end-of-slot semantics.
The offline rebuild rechecks this validation and every retained response hash.
An archive that labels last-write slot instead of requested bank slot needs an
explicit, separately validated integration; this implementation rejects it.

One capture directory is one provider attempt. It refuses overwriting; a second
provider/version needs a separate directory. Request/cache membership is keyed
by pubkey **and execution-slot/visibility context**, with provider provenance in
the containing capture. There is one bounded request per context, with no hidden
retry chain. Every safe response, including non-JSON/error bodies, is retained.
A response echoing credentials is withheld with explicit failure membership and
its hash; such a response can never become evidence. No private credentials were
read. No network acquisition request was made in this run.

The collector invokes the official decoder to determine whether an earlier
`deactivation_slot` requires historical SlotHashes, acquiring that sysvar only
for those execution slots. The core uses official wincode/SlotHashes decoding,
validates sysvar owner and bank context, then applies official ALT status/lookup.
There is **no custom ALT account parsing**. RPC JSON field parsing for the v0
compiled message is the only new format conversion.

`LutResolutionProof` retains original native v0 fields, table metadata/full
address arrays, lookup indexes, independently resolved vectors, all evidence
hashes and contexts, the complete requested-privilege key vector and comparison
results. Revalidating a serialized proof reconstructs it from raw evidence and
compares every field. Canonical identity contains no timestamp/path. The
transaction-scoped artifact additionally binds the frozen U3A fingerprint and
exact raw transaction capture hash. No signatures are cryptographically checked,
no runtime ELF/bank-feature identity is acquired, and no ledger-root inclusion
proof is asserted. Historical truth remains conditional on validated archive
provenance and canonical ALT semantics.

## Eight-transaction stage table

S1 = historical table acquisition; S2 = execution-slot validity; S3 = independent
resolution; S4 = exact RPC comparison; S5 = full key vector; S6 = compiled
instruction identity; S7 = previous LUT gate; S8 = next old-engine rejection.
`—` means not attempted, never success. Exact raw-derived identities and message
fields are in [targets.json](examples/phase-u3b-lut/targets.json), with individual
artifacts under [proofs/](examples/phase-u3b-lut/stage-table.json).

| Signature | Execution slot | Tables / loaded | On-chain success | S1 | S2 | S3 | S4 | S5 | S6 | S7 | S8 / next rejection |
|---|---:|---:|---|---|---|---|---|---|---|---|---|
| `5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97` | 448195166 | 1 / 13 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |
| `3zswY7FXGLsVCVJTg8LawCQZp27xBza4M4Cp7mtBm2LxP97kn8eZrXPkCCJv3dt3qMb9fvRXKT3qZTXiucgR6y4Q` | 448195147 | 5 / 52 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |
| `47XZ8dNPUBxMTnJ3zrxeCKt37PRsckvhKzPSutdUp2NavU1UTTNYRd7rnvEP3pSmHQszrLW3cdB56zTw3kyrwZUf` | 448195003 | 5 / 55 | no | unavailable | — | — | — | — | — | — | — / unmeasured |
| `37XJKQbB9fRX1R76cKKGSENNNA2RSY1WvjkCx2Q7joN2irKfTTZ5Z6bHgH4rfgDPJvhXf3peTe4mwxisfpSEmimu` | 448194959 | 1 / 30 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |
| `55EdbRGcpNETQZ2AerE4KuvKC1xKCFzHkFHxxN98gjz7DWzGCGbgXKSXW6q3SoABZBbvEsEKb6sVBt9eGThj2kvk` | 448194517 | 1 / 11 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |
| `5NKNdj527vBVpCGhNdeGwpj9LdQfpSCxSH3b8oaKkLe3EzKbZdrsc2fR7yCuCkYgt8EmYXFDwJidi15y8ZQNY43V` | 448194462 | 1 / 9 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |
| `49zi8hsUATfn3oM7x3bx8fBWULY9mN6DGHtFCVWRLcuPJFyBkP54XVdZGWbqVecwHzsSzFkVQdvLKPp8r8929Amx` | 448194435 | 1 / 14 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |
| `2dc94E8Mvh83c8MiecgRekd7Vj8YE6aexfVeCJK58K46LQcTRjpDomii6pYmwbdeYmdTM3bVbHw6rhFdtwRtSTnT` | 448194341 | 1 / 13 | yes | unavailable | — | — | — | — | — | — | — / unmeasured |

**BEFORE: 8/8 first rejected on loaded LUT addresses. AFTER: 0/8 have exact
historical proof and pass that blocker; 8/8 remain unproven at acquisition.**
All failures are `historical_account_archive_not_configured`, recorded for all
16 table/slot contexts in [acquisition.json](examples/phase-u3b-lut/acquisition.json).
There are 10 unique LUT accounts. Historical table hashes, independent production
RPC matches, production full-key/instruction matches and next blockers are
**unavailable**, not inferred. The ninth legacy System-companion case is excluded
from this experiment and retains its old admission result.

The failed on-chain transaction remains in the eight-target denominator. If its
LUT proof is later established, unchanged admission checks may reject its failure
status first; this run does not measure that later blocker. Likewise no companion,
action-count, shape or lifecycle diagnosis is substituted for an observed blocker.

## Offline rebuild and acquisition commands

```bash
./scripts/rebuild-kamino-u3b-lut-proofs.sh --verify
./scripts/rebuild-kamino-u3b-lut-proofs.sh --verify --reverse --workers 4
./scripts/rebuild-kamino-u3b-lut-proofs.sh --output /tmp/eplyx-u3b-derived
python3 scripts/test_kamino_u3b_luts.py
cargo test --offline -p eplyx-engine --test historical_lut
python3 scripts/mutate-kamino-u3b-luts.py --output /tmp/eplyx-u3b-mutations.json
```

The rebuild uses `cargo --offline`, strips RPC/archive/API-key configuration from
the example environment and calls no transport. It pins the original U3A checksum
inventory as well as recomputing the sample fingerprint. Any changed U3A input
stops before derivation. Reversed table/input/filesystem order, relocation and
four concurrent independent processes preserve every canonical artifact byte.
The committed rebuild result is deterministic **failure evidence**; it does not
invent table state for any production transaction.

To resume acquisition after configuring a real archive and independent validation
fixture (credentials only in exported environment variables):

```bash
cargo build --offline -p eplyx-engine --example inspect_lut
python3 scripts/capture-kamino-u3b-luts.py \
  --validation-fixture /path/to/independent-historical-lut-fixture.json \
  --output data/phase-u3b-archive-attempt-2
./scripts/rebuild-kamino-u3b-lut-proofs.sh \
  --evidence data/phase-u3b-archive-attempt-2 --output /tmp/eplyx-u3b-attempt-2
```

The fixture supplies `kind=independent_historical_lut_validation_fixture`,
`synthetic=false`, `semantics_reference`, `independent_fixture_reference` and
at least two `observations` for the same pubkey, with `requested_slot` and
`expected_account_sha256`. Expected hashes must differ and be independently
established; creating them from the archive being tested is circular validation.
An archive provider and those independent expectations are still missing.

## Validation boundaries and phase stop

Synthetic tests exercise official decode/metadata/hash roundtrips, malformed/
empty/wrong-owner tables, same-slot prefix warmup, both out-of-range index types,
active/deactivating/inactive states, missing/invalid historical SlotHashes,
multiple tables, 55 loaded addresses, duplicate-key rejection, signer/writable
flags, exact order and compiled indexing. A real frozen five-table/55-loaded-key
message is used with **explicitly synthetic ALT bytes** reconstructed for an
index regression fixture. It proves decoder/index behavior only. The mandatory
real historical table-to-frozen-RPC assertion cannot run until acquisition exists;
no synthetic result contributes to the production stage table.

U3B stops at historical acquisition. Do not start companion/lifecycle support or
adapter #4 on a presumed next-blocker distribution. The next permitted work is
validate an actual end-of-slot archive and acquire these exact 16 LUT contexts,
then rerun this offline experiment. The next implementation phase must follow
measured blockers. This is not Kamino production replay.


## Recorded controls, mutations, timing and storage

Changes remain uncommitted on `d20c764916aa8ceb84bb8fbb69d1c1c88406c23c`.
The [complete final report](examples/phase-u3b-validation/final-report.json)
records every requested result, including unavailable hashes/matches/timings.
Before/after [product controls](examples/phase-u3b-validation/controls.json) retain
all six canonical U2 hashes and exits 0/0/1/1/3/5. Token-2022 passed 12/12 and
Kamino integration passed 10/10 both before and after. Both normal/reversed U3A
rebuilds preserve the fingerprint and all 344 checksums; all 30 U3A integrity
tests passed.

[Tests](examples/phase-u3b-validation/tests.json): **38 new tests passed**
(21 Rust LUT tests, 17 Python acquisition tests). Broader existing controls passed:
421 library tests, four historical tests and 16 replay tests, in addition to the
Token-2022/Kamino suites. Total executed normal checks: **531 tests passed**.
Clippy passed with warnings denied for the library, LUT tests and all four
old/new audit examples; Rust formatting and `git diff --check` passed.
The mandatory production historical-byte comparison remains blocked explicitly.

**12/12 real behavioral source mutants were killed by named assertion tests**,
with each unmutated named control passing first. Exact fault/hash/test results
are in [mutations.json](examples/phase-u3b-validation/mutations.json).
No compiler-only kill is counted. Production source hash before/after is
`13ea3738a7e91b35385724eec40057f6b45c2a8909db2fe7c08bb891bde8366f`;
every isolated source was restored. Mutation builds now use a separate Cargo
output directory. An initial shared-cache run contaminated normal test binaries;
those engine build artifacts were cleared, and both the complete normal suites
and all twelve mutants were rerun with separate outputs. The retained results
are from the corrected runs.

The official synthetic byte fixture is [synthetic-lut-bytes.json](../engine/tests/fixtures/synthetic-lut-bytes.json),
marked `synthetic=true`, generated with the official `serialize_for_tests` codec.
Acquisition tests use it only in temporary mock-provider directories; arbitrary
synthetic addresses are asserted to fail all eight frozen RPC comparisons.
No synthetic table bytes enter the experimental production evidence directory.

Measured offline rebuild: **0.320 s** normal and
**0.347 s** reversed with four processes, including
frozen integrity checks. Normal message derivation took
**0.225 s**, parsing the eight original
messages and producing missing-evidence failures. Actual successful historical
decode/reconstruction timing is unavailable. Acquisition was **not attempted**
(zero network requests); no archive-latency claim is made.

The complete experimental evidence directory is **109,520 bytes**
in 14 files, including targets, acquisition membership and checksums.
Eight transaction-scoped failure artifacts total **23,908 bytes**. There are
**zero acquired distinct historical states, zero raw historical account bytes
and zero decoded successful proof bytes**. Historical duplication/materiality
cannot yet be measured; no deduplication was added. These measurements are
experimental audit storage, not ReplayRecord overhead.

[Artifact secret scan](examples/phase-u3b-validation/secret-scan.json) found no
credential fields or credential-bearing URLs. Provider identity is scheme/host;
private RPC credentials and unrelated personal directories were never inspected.

**Final decision remains B, with all eight real transactions still unproven.**
The next step is to supply validated historical archive configuration and an
independent divergent LUT fixture, acquire these exact contexts in a separate
attempt directory, and repeat the offline stage table. U3C remains unauthorized
and evidence-dependent. No Kamino production replay is claimed.
