# Phase G1 — Squads V4 program-upgrade binding

## Decision

Eplyx can now answer, for one bounded Squads V4 shape:

> Is the proposal people are approving the same program upgrade this analysis
> evaluated — **at slot S**?

```text
GOVERNANCE PROPOSAL  ==  ANALYSED CHANGESPEC        (observed at slot S)
```

A governance-bound ChangeSpec is an ordinary `program_upgrade` spec with one
new identifying field, `delivery`. Specs without it keep their C1 identity.
Binding is read-only: Eplyx never signs, approves, rejects, cancels, executes
or creates a proposal, and holds no key.

## 1. Supported proposal shape

A Squads V4 `VaultTransaction` whose **stored message** is exactly:

```text
account_keys = [vault (the only signer) | ProgramData w, Program w, Buffer w, spill w | Rent, Clock, BPFLoaderUpgradeable]
instructions = [ loader-v3 Upgrade  accounts [ProgramData, Program, Buffer, spill, Rent, Clock, vault] ]
address_table_lookups = []      ephemeral_signer_bumps = []
```

- exactly one instruction; its program is the upgradeable loader;
- the data is `Upgrade` in a feature-independent form: legacy `03 00 00 00`,
  or the SIMD-0430 explicit `03 00 00 00 01` (close the buffer).
  `close_buffer = false` is refused because its meaning depends on whether the
  cluster has activated SIMD-0430;
- seven distinct static keys in loader order, with ProgramData, Program, Buffer
  and spill writable and not signers, Rent and Clock the real sysvars and
  read-only, and the authority the single signer at index 0;
- ProgramData is the loader derivation of Program;
- every static key is used; no key appears twice.

Anything else — upgrade + transfer, upgrade + SetAuthority, two upgrades,
another loader instruction, a lookup table, an ephemeral signer, a second
signer — is `unsupported_proposal`. ComputeBudget instructions in the outer
proposal-creation transaction are outside the stored message and never seen.

## 2. Source and program

| | |
| --- | --- |
| Program | `SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf` (the only ID a delivery may name) |
| Source | `https://github.com/Squads-Protocol/v4` @ `af94153ff77a28b6effe46b9c94baaa93742b48c` (`squads_multisig_program` 2.1.0) |
| Decoder | `eplyx-squads-v4-decoder-v1` (`engine/src/governance/squads.rs`) |
| Loader | `solana-loader-v3-interface` 8.0.1, the version litesvm already resolves |

Layouts (`Multisig`, `VaultTransaction`, `VaultTransactionMessage`, `Proposal`,
`ProposalStatus`) are transcribed from `state/*.rs`. Discriminators are the
Anchor convention and match the official SDK's generated constants
(`discriminators_are_the_anchor_convention`). This is a pinned interface.
**Eplyx does not claim the deployed Squads program was built from this
source.** Every binding records the program ID and revision under `decoder`.

## 3. Canonical message hash

```text
message_sha256 = sha256( "eplyx-squads-v4-vault-message-v1" || 0x00 || borsh(VaultTransactionMessage) )
```

The Borsh bytes are the bytes Squads stores: signer count, writable-signer
count, writable-non-signer count, every static key in order, and every
instruction's program index, account indexes and data. They also include every
lookup table key with its writable and read-only index lists. Each list is
length-prefixed, so no two messages share an encoding. The decoder re-encodes
the decoded message and requires it to equal the stored slice, so the hashed
form *is* the stored form. Nothing rendered is hashed. The memo is not hashed
either: Squads logs it and never stores it.

Frozen by `the_squads_message_hash_is_frozen`, which also checks the Borsh
encoding against a hand-written byte sequence. It is reproduced independently
by `pnpm verify:governance` (`scripts/verify-squads-binding.mjs`). That script
decodes the stored account by hand, re-encodes each binding's JSON message
view, and recomputes the hash, both change IDs and every binding ID with Node
built-ins only.

## 4. ChangeSpec representation

```json
"change": {
  "kind": "program_upgrade",
  "target": { "program_id": "…" },
  "candidate": { "sha256": "…", "len": 37 },
  "delivery": {
    "provider": "squads_v4",
    "squads_program_id": "SQDS4ep65T869zMMBKyuUq6aD6EgTu8psMjkvj52pCf",
    "multisig": "…", "vault_index": 0, "vault": "…",
    "transaction_index": 42, "transaction": "…", "proposal": "…",
    "message_sha256": "b869…cd86"
  }
}
```

`delivery` is optional, identifying, and skipped when absent. Every C1 ID is
unchanged (`a_frozen_spec_keeps_its_identity`,
`unbound_specs_keep_their_c1_identity`). Every delivery field moves the ID
(`a_bound_spec_round_trips_and_every_delivery_field_moves_its_id`). Unknown
fields and providers are refused. No status, vote, approval count or timestamp
is part of it. Those are observations. `ChangeSpec::bind` (the bundle check)
ignores the delivery, because a bundle proves the baseline, not which proposal
is open today. `ChangeBinding`, and therefore every report, carries the
delivery when present. A report of an unbound change is byte-identical.

## 5. PDA derivation checks

From `state/seeds.rs`, all under the Squads program:

| Account | Seeds |
| --- | --- |
| Multisig | `["multisig", "multisig", create_key]` |
| Vault | `["multisig", multisig, "vault", vault_index:u8]` |
| VaultTransaction | `["multisig", multisig, "transaction", index:u64le]` |
| Proposal | `["multisig", multisig, "transaction", index:u64le, "proposal"]` |

A delivery must be exactly its own derivation, or `ChangeSpec::validate`
refuses it. A request supplies only `(multisig, transaction_index)`, and the
transaction and proposal addresses are derived, never accepted. On chain, the
checks are: the multisig is the PDA of its own `create_key` with its canonical
bump; the transaction and proposal record the canonical bumps Anchor
re-validates on execute; and `transaction.multisig`, `transaction.index`,
`proposal.multisig` and `proposal.transaction_index` equal the request. The
stored `vault_bump` must also sign as the canonical vault. Squads executes with
`create_program_address(.., [vault_bump])`, so a non-canonical bump would sign
as another address. Any failure is `unverifiable`.

## 6. Loader Upgrade decode

`standard_programs::upgradeable_loader` decodes instruction data with the
official `UpgradeableLoaderInstruction` (wincode) and accepts only the exact
encodings above. Accounts are resolved by position from the interface's own
`upgrade()` builder: `[ProgramData, Program, Buffer, spill, Rent, Clock,
authority]`. Swapping Buffer and spill therefore makes the spill account the
buffer (`m14…`), rather than being matched by name. Program, ProgramData and
Buffer states are decoded through the official `UpgradeableLoaderState`.

## 7. Target binding

| ChangeSpec states | Must equal | Else |
| --- | --- | --- |
| `target.program_id` | the Upgrade's Program | `different_proposal` |
| `target.programdata_address` | the Upgrade's ProgramData | `different_proposal` |
| `expected_upgrade_authority` | the vault | `different_proposal` |
| `replaces` | the executable deployed now (ProgramData after its header) | `different_proposal` |
| `delivery` (bound spec) | the request's multisig and index, vault index, and message hash | `different_proposal` |

The on-chain Program must be a loader program whose ProgramData is the
Upgrade's. Otherwise the result is `unverifiable`.

## 8. Vault authority

The loader requires the signing authority to equal both the ProgramData upgrade
authority and the buffer authority when `Upgrade` executes. The message's only
signer, and the Upgrade authority, must be the canonical vault for the
transaction's `vault_index`. The ProgramData upgrade authority read at slot S
must be that vault. If it is another key or `None`, the result is
`authority_mismatch` (`upgrade_authority_not_vault`): the vault could not
execute the upgrade.

## 9. Buffer ELF extraction

The buffer must be owned by the loader and be in `Buffer` state. The artefact
is **every byte after the 37-byte metadata**: `Upgrade` copies all of it,
including any slack in a buffer allocated larger than the ELF. A padded buffer
is therefore a different artefact from the bare `.so`. It is not trimmed to
match. SHA-256 and length must equal `candidate`. Otherwise the result is
`stale_artifact`. If the bytes match, they must also start with the ELF magic,
as `versions` requires.

## 10. Buffer-authority guarantee

A matched binding requires `buffer authority == vault`. Only a buffer's
authority can `Write`, `SetAuthority` or `Close` it. So after a match, the bytes
can change only through a transaction the vault signs, which means another
approved and executed proposal of the same multisig. A buffer with any other
authority, or none, is `authority_mismatch` (`buffer_authority_not_vault`),
never matched. It could be rewritten with no vote, or could never satisfy
Upgrade.

## 11. Freshness model

- One atomic `getMultipleAccounts` (multisig, transaction, proposal, program,
  ProgramData, buffer) at one context slot `S`, preceded by a read that locates
  the message. The second read uses `minContextSlot` and must see the identical
  transaction bytes.
- Every binding records `observation.slot`, the commitment (`finalized` by
  default), and per-account digests: owner, lamports as a decimal string,
  executable flag, data length and data SHA-256. The digests are of the
  normalized account, so they do not depend on the provider.
- Every statement names its slot. "Proposal matched analysed ChangeSpec at slot
  S" is the claim. "Can never diverge" is never made.
- `eplyx governance squads verify` (or the API) re-reads everything. There is
  no cached path: `verify_squads_upgrade` takes no stored binding
  (`m20…`: after a buffer rewrite the re-check returns `stale_artifact` at a
  later slot).

## 12. Proposal status

`draft`, `active`, `approved`, `rejected`, `executing` (deprecated upstream),
`executed` and `cancelled` are decoded and reported with the approval,
rejection and cancellation counts. `stale` means
`transaction_index <= stale_transaction_index`: Squads refuses new votes, but
an already-approved vault transaction can still execute. Status never affects
identity or the binding outcome. A cancelled proposal still `matched`, shown as
"Cancelled (final)". An executed proposal has consumed its buffer, so its bytes
cannot be re-checked. The result is `unverifiable` (`buffer_missing`), and the
detail says so.

## 13. Typed outcomes

Listed in precedence order. The first applicable outcome heads the result, and
every reason is kept.

| Outcome | Meaning | CLI exit |
| --- | --- | ---: |
| `unverifiable` | RPC, account, owner, discriminator, PDA or consistency evidence missing or corrupt | 2 |
| `unsupported_proposal` | valid Squads proposal outside the bounded shape | 4 |
| `different_proposal` | reference, message or target differs from the spec | 1 |
| `stale_artifact` | message matches; the buffer does not hold the candidate at S | 1 |
| `authority_mismatch` | message and bytes match; the buffer or upgrade authority is not the vault | 1 |
| `matched` | exact message, target and current buffer bytes match | 0 |

## 14. Durable evidence

`GovernanceBinding` (`kind: squads_v4_program_upgrade`, schema 1) holds:
decoder provenance, commitment, request, `analysed_change_spec_id`,
`bound_change_spec_id`, what the spec expected, and the complete observation
(delivery, message view, upgrade accounts, current program, buffer authority
and artefact, proposal and multisig state, account digests). It also holds the
outcome, the reasons and the statement.

`binding_id = sha256(canonical JSON of ("eplyx-governance-binding-v1", body))`.
`GovernanceBinding::parse` refuses a record whose id disagrees, an unsealed
record, and a resealed record whose outcome contradicts its own reasons
(`m19…`). The id proves the record was not edited. It does not prove the
record true: only a re-read does that. No credentials or endpoints are stored.

## 15. Identity integration (option A)

An unbound analysis is never relabelled. Binding an unbound spec yields the
**governance-bound spec**, a different ID. That spec is analysed again, with no
upload, because P2 retains the artefact. Its report's `change` carries the
delivery and the bound ID. The chain of identity is:

```text
Squads message (hash H) ── delivery.message_sha256 = H ── change_spec_id ── report.change.change_spec_id
buffer bytes at slot S ── sha256 = candidate.sha256 ──┘
```

## 16. Artefact reuse

The hosted endpoint binds only a spec this project already analysed or bound.
Its candidate is in P2's content-addressed store, and the response reports
`expected_candidate.held_by_project`. **The service never turns chain bytes
into a candidate.** That is an explicit local decision: `eplyx governance
squads acquire --store DIR` writes the buffer's current bytes into the same
evidence-store layout (`programs/<sha256>`) and writes the unbound spec. That
spec must then be analysed and bound like any other.

## 17. API

`POST /v1/projects/{p}/governance/squads/verify` (project token or operator):

```json
{ "multisig": "…", "transaction_index": 42, "run_id": "run_…" | "change_spec_id": "…", "commitment": "finalized" }
```

The response carries: `status`, `exit_code`, `statement`, `observed_slot`,
`commitment`, `change_spec_id` (bound), `analysed_change_spec_id`,
`governance_bound`, `proposal` (addresses, message hash, status, stale,
approvals, threshold), `target`, `buffer` (address, sha256, len, authority),
`expected_candidate` (with `held_by_project`), `analysis.runs` (analyses of the
bound change), `analysis.bound_change_spec` (the document to submit, when the
analysed spec was not yet bound), `binding_id` and the full `binding`.

`GET /v1/projects/{p}/governance/changes/{change_spec_id}` returns recorded
checks, newest first, each re-verified on read. A tampered file is a 500 and
is never served. A check is indexed under the change it was asked about *and*
the bound change it derived. Otherwise a re-check of a bound change that finds
a different message would be filed elsewhere, and the bound change would keep
showing its last match.

The endpoint needs `EPLYX_GOVERNANCE_RPC_URL` (503 without it). No run ever
reads that endpoint, so the analysis path stays offline. A blocking RPC call
runs on `spawn_blocking`.

CLI:

```bash
eplyx governance squads bind   --multisig M --transaction-index N --change-spec analysed.json --out bound.json [--evidence-out b.json] [--format json]
eplyx governance squads verify --change-spec bound.json [--multisig M --transaction-index N] [--evidence-out b.json]
eplyx governance squads acquire --multisig M --transaction-index N --store DIR --out analysed.json
# all take --rpc-url (or SOLANA_RPC_URL) and --commitment confirmed|finalized
```

`bind` writes the bound spec only when the result is `matched`. `ci check`
text output names a delivery and says the report does not verify the proposal.

## 18. Frontend

A compact **Governance proposal** card under the P3 change card, shown only
when the run's spec has a delivery. P3 is otherwise unchanged. `governance.js`
is a pure projection of the newest check's sealed binding:

- matched and fresh (≤ 15 minutes): "✓ Matches analysed change", Squads #N,
  "Buffer checked: slot S · finalized · 3 minutes ago", proposal status, and
  the re-verify note;
- matched but older, or with no recorded time: *dated*, not green, "Matched at
  slot S, 3 hours ago";
- `stale_artifact`: an alert headed "The proposal's buffer no longer matches
  the candidate Eplyx analysed.";
- a check about another change, or evidence the server refused: an alert,
  never a result;
- never checked: neutral "Not verified against the chain yet".

The full binding appears under **Technical details → Governance binding**.
History rows say "via Squads #N".

## 19. Mutation results

All are engine unit tests (`governance::tests`), driven through the real reads
and decoders against `governance::simulated::World`:

| # | Mutation | Result |
| ---: | --- | --- |
| 1 | different multisig | `different_proposal` (`proposal_reference_differs`) |
| 2 | wrong transaction index | `different_proposal`; a never-created index: `unverifiable` |
| 3 | wrong derived transaction PDA | spec refused by `validate`; wrong stored index or bump: `unverifiable` |
| 4 | wrong Proposal PDA | spec refused; wrong stored index: `unverifiable`; no proposal: `unverifiable` |
| 5 | transaction or proposal multisig mismatch | `unverifiable` (`multisig_mismatch`) |
| 6 | different target Program | `different_proposal` |
| 7 | different ProgramData | `different_proposal`; non-derived ProgramData in the message: `unsupported_proposal` |
| 8 | different buffer address | `different_proposal` (`message_differs`) |
| 9 | same message, buffer rewritten | `stale_artifact` |
| 10 | buffer authority changed, or none | `authority_mismatch` |
| 11 | upgrade authority not the vault, or none | `authority_mismatch` |
| 12 | extra System transfer | `unsupported_proposal` |
| 13 | two upgrades; upgrade + SetAuthority | `unsupported_proposal` |
| 14 | SetAuthority, Close, ExtendProgram, `close_buffer = false`, reordered or read-only accounts, second signer | `unsupported_proposal` (swapped buffer and spill: `unverifiable`, spill is the buffer) |
| 15 | altered message, same memo | `different_proposal`; unbound, it binds to a different ID |
| 16 | lookup table present, or changed | `unsupported_proposal`; the table key moves the message hash |
| 17 | approved, cancelled, rejected, draft, stale; executed | status reported, identity unchanged; executed: `unverifiable` (`buffer_missing`) |
| 18 | wrong owner, bad discriminator, trailing bytes, account type confusion, non-canonical vault bump, no RPC | `unverifiable` |
| 19 | tampered evidence (slot, buffer hash, outcome, statement, bound id); unsealed; resealed lie | refused by `GovernanceBinding::parse`; hosted: 500, never shown |
| 20 | re-check after a buffer rewrite | re-reads, `stale_artifact` at a later slot, never a cached match |

Through the binary (`engine/tests/governance_cli.rs`, real HTTP, real curl):
bind → verify → rewrite → exit 1; lookup table → exit 4; RPC down → exit 2;
foreign authority → exit 1 with no bound spec written; acquire. Hosted
(`server/tests/governance.rs`): a real analysis is bound, the bound spec is
analysed with no upload, its report carries the delivery, the old report is
untouched, re-checks are recorded and indexed, tampered evidence is refused,
and requests are project-scoped.

## 20. Lookup-table decision

Unsupported. Squads resolves lookup indexes against the live tables at
execution time (`ExecutableTransactionMessage::new_validated`). A LUT-backed
proposal therefore depends on mutable, append-only table state that the message
does not commit to. Eplyx's existing LUT reconstruction (U3B) proves
*historical* resolution at a past slot, which is a different question. Binding
LUT-backed proposals would need a second freshness dimension per table. A
program upgrade needs seven static keys, so LUT-backed proposals are
`unsupported_proposal`. The lookup entries are still hashed, so a changed table
is a different message.

## 21. Frozen regressions

C1 IDs (`b5a894cd…` and every other pinned ID), P1 run/report identity (now
also comparing `delivery`), P2 artefact and recovery behaviour, P3 impact
fixtures, Drift U14, Orca U12, checkpoint contracts 1/2/3, no-adapter replay
and the existing hosted API are all unchanged. Every existing test passes. No
replay, semantic adapter or report field of an unbound change moved.

## Exact limits of "proposal matches analysed change"

It means that at slot S, at the stated commitment:

- the Squads V4 program's accounts at the derived addresses decode under the
  pinned layouts;
- the stored message hashes to the spec's `message_sha256`, and is exactly one
  loader Upgrade of the spec's target;
- the buffer it names held exactly the analysed candidate bytes;
- the buffer and upgrade authorities were the vault that message signs as.

It does **not** mean:

- that the buffer cannot change later. Another executed proposal of the same
  multisig's vault can rewrite it;
- that the proposal will be approved, executed, or executed before any other
  proposal;
- that the deployed Squads program matches the pinned source;
- that the upgrade is safe, or that the analysis passed. The analysis verdict
  is the report's, about the corpus it replayed;
- that the multisig's membership or threshold is appropriate. A controlled
  multisig's `config_authority` is recorded, not judged;
- anything about proposals using lookup tables, ephemeral signers, or more than
  one instruction.

## Recommendation for the next governance phase

Build **execution-time verification** before generalising. The residual risk
is the window between the last check and execution. Two steps close most of
it: (a) record the executed transaction's post-upgrade ProgramData hash and
compare it with the analysed candidate, which is a verdict about what actually
deployed; and (b) a CI or bot hook that runs `verify` on every proposal status
transition. A generic `GovernanceExecution` kind, other providers (Realms, SPL
Governance) and LUT-backed messages should wait until real Squads usage shows
which of these shapes matter.
