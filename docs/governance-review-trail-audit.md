# Governance review trail audit (MAIN, 2026-09-29)

## Decision and scope

**Implemented (2026-09-29):** the recommended project-scoped Squads review layer now exists. The bound ChangeSpec is the root; retained G1 checks and G2 occurrences form one authenticated, paginated trail. The report and hosted dashboard render every requested page and preserve each G2 proof's exact G1 binding. No engine semantics or sealed-object identities changed.

Sections 1–9 below retain the **pre-implementation audit** and its original gap analysis. The implementation contract and reviewer guidance follow in section 10. The existing unrelated edit to `docs/bundle-qualification.md` was left alone.

This conclusion is based on the current [ChangeSpec](../engine/src/change.rs), [G1 verifier](../engine/src/governance/mod.rs), [Squads decoder](../engine/src/governance/squads.rs), [G2 attester](../engine/src/governance/attestation.rs), [CLI](../engine/src/main.rs), [hosted API](../server/src/api.rs), [registry](../server/src/registry.rs), [report page](../frontend/src/report.js), [dashboard report loader](../frontend/dashboard/pages.js), and [governance projection](../frontend/src/governance.js), as well as the [G1](phase-g1-squads-governance-binding.md), [G1.1](phase-g1-1-squads-mainnet-qualification.md), and [G2](phase-g2-squads-deployment-attestation.md) records. The existing unrelated edit to `docs/bundle-qualification.md` was left alone.

## 1. Existing capabilities and identity root

- `ChangeSpec::id()` hashes schema, change and activation. For `program_upgrade`, candidate SHA-256 and length, target, optional baseline expectations and optional `delivery` identify the change. Metadata is excluded. `SquadsV4Delivery` records Squads program, multisig, vault, transaction index and PDA, Proposal PDA, and exact stored-message SHA-256; mutable status and votes stay out of the identity ([change.rs](../engine/src/change.rs)).
- An unbound analysis is never relabelled. G1 can derive a **different governance-bound ChangeSpec ID**. The bound spec must be analysed as its own run for a report to name that proposal; the original run/report remains about the unbound change ([G1 §15](phase-g1-squads-governance-binding.md), [server test](../server/tests/governance.rs)). The bound ChangeSpec ID is the reviewer trail's primary identity; run IDs are links to one or more analyses, not the root. The source unbound ID is retained in the G1 record.
- G1 reads the chain without signing. CLI `governance squads acquire` can copy current buffer bytes into a local content-addressed program store and write an unbound spec. `bind` writes a bound spec only on `matched`; `verify` rereads a bound spec. `ci check` does not verify the live proposal. G2 `attest` needs the bound spec, sealed G1 match, candidate bytes from the store and finalized RPC ([main.rs](../engine/src/main.rs)).
- Hosted `POST .../governance/squads/verify`, `POST .../governance/squads/attest` and `GET .../governance/changes/{change_spec_id}` exist. Hosted acquire does not exist; the server does not adopt chain bytes as a candidate ([api.rs](../server/src/api.rs), [G1 §16–17](phase-g1-squads-governance-binding.md)).
- The supported proposal is exactly one loader `Upgrade`, seven loader accounts, no LUT or ephemeral signer, with the vault as sole signer. Other Squads actions and general governance are outside this contract ([G1 §1](phase-g1-squads-governance-binding.md)).

## 2. Existing evidence objects and G1/G2 contracts

| Object | Created when | Binds what | Evidence source | Immutable? | What it proves | What it does NOT prove |
| --- | --- | --- | --- | --- | --- | --- |
| `ChangeSpec` / `change_spec_id` | Submitted before analysis | Target, candidate SHA/length, expectations, activation, optional Squads delivery | Caller document; hosted run spec or project governance spec | ID recomputed on read; saved spec is not edited in normal flow | Proposed identity and expectations | Analysis verdict or live proposal state |
| Candidate program object, `programs/<sha256>` | Candidate accepted for a run or local `acquire` | Exact bytes and length | Project-held content-addressed artifact or local evidence store | Hash checked on resolution | Bytes analysis and G2 compare | That governance will deploy them |
| `RunChange`, analytical run/report | Run accepted/completed | Run ID → exact ChangeSpec ID, pinned bundle and candidate; report has `ChangeBinding` and verdict | Hosted run and analytical store | Historical run retained and verified on read | Impact under that run's inputs | Live proposal verification or deployment |
| `SquadsV4Delivery` | G1 derives a bound spec | Proposal PDAs and stored message hash | Decoded Proposal/VaultTransaction; field of bound ChangeSpec | Identifying spec content, not mutable status | Which stored message analysis names | Future buffer contents, votes or execution |
| `GovernanceBinding` / `binding_id` | Each G1 `bind`/`verify` | Source and possible bound ChangeSpec IDs, candidate, target, proposal, account digests and outcome | Two-read chain observation; CLI JSON if saved or hosted binding file | Content sealed; earlier hosted records retained and hash-verified | `matched` means supported message, target, buffer bytes and authorities agreed **at its slot** | Current match, approval, execution or deployment |
| `SquadsObservation` and `ProposalState` inside binding | G1 read | Multisig, transaction, proposal, Program, ProgramData, Buffer, status/counts and digests | One atomic second account read at one context slot, following message-locating read | Sealed inside binding | Proposal and buffer/program state then | Execution signature or deployed bytes from `executed` status alone |
| `GovernanceCheck` | Each hosted G1 request | Check ID, project, indexed ChangeSpec ID, binding ID, `checked_at_unix_seconds` | Filesystem index under source and derived bound IDs | Append-style marker; timestamp is outside binding seal | When server recorded a G1 attempt | Chain time or fresh result by itself |
| `DeploymentAttestation` / `attestation_id` | Each G2 `attest` | Bound ChangeSpec, **specific G1 binding ID**, proposal/message, target, candidate, possible execution and ProgramData | Finalized RPC and retained candidate; hosted file under bound ID or CLI JSON | Content sealed and hash-checked; identical ID idempotent | Typed outcome; `execution` proves matching successful execute/CPI; `deployed_match` adds attributable candidate bytes and zero padding | Byte equality from a bare `Executed` status or deployment slot |

G1's `binding_id` is the hash of its sealed body; G2's `attestation_id` is a separate hash of its body. Both hashes detect edited stored statements, not dishonest RPC data or source-to-binary provenance. G1 stores `observation.slot` and `message_read_slot`; G2 stores `observed_slot` for its current Program/ProgramData read, `execution.slot` and signature when identified, and `deployed.deploy_slot` when decoded. G2 does **not** store the hosted recording timestamp or a check/attempt ID. See [governance types](../engine/src/governance/mod.rs), [attestation types](../engine/src/governance/attestation.rs), and [registry](../server/src/registry.rs).

## 3. Current reviewer journey

| Reviewer question | Current classification | Actual path and limit |
| --- | --- | --- |
| Before approval: does this proposal contain the analysed candidate? | **Already directly answerable**, once the bound spec has its own run | G1 `bind`/`verify` or hosted verify checks stored message, target, buffer SHA/length and authorities; the bound run's report names the proposal. An unbound report alone cannot answer it. |
| Before execution: is the binding still fresh, and did the proposal/buffer change? | **Already directly answerable** | CLI `verify` or hosted verify rereads. Stored G1 result remains dated. The run card shows only the latest hosted check; the API exposes up to 20. A fresh check is an action, never inferred from the old match. |
| After Squads execution: did it execute? | **Already directly answerable**, if G2 finds a witness | `attest` looks for a successful matching execute transaction and loader CPI; a Proposal `Executed` label is only a search trigger. If the bounded search cannot establish it, outcome is `unverifiable`. |
| After deployment: do deployed bytes match the candidate? | **Dependent on a future real witness** for current end-to-end qualification | G2 can return `deployed_match`/`deployed_mismatch` only after execution and ProgramData attribution. The current documented G1.1 proposals were Active; the implementation exists. |
| Historical review: what existed at each point? | **Answerable but fragmented** | Hosted G1 and G2 records are retained separately. API caps both lists at 20 and supplies no unified order or full-history pagination. The frontend discards every record after the first of each list. Local CLI files have no automatic project linkage. |

Normal authenticated project members, project tokens and the operator can call the project routes; the routes are not operator-only. Project identity is checked before registry access ([api.rs](../server/src/api.rs)). A normal hosted reviewer can see the **latest** governance summary in a bound run's report without downloading CLI artifacts; they cannot follow every observation there. The project run list shows the change and `via Squads #N`, not the governance trail ([projects.js](../frontend/src/projects.js)).

## 4. Freshness and proposal mutation

G1's durable statement is an observation at slot S, not a lease. The hosted registry adds each recheck as a new `GovernanceCheck`; it does not rewrite the old sealed binding. CLI `verify` likewise rereads and emits a new binding, but retaining both local files is the caller's responsibility. The frontend derives an age from the hosted `checked_at_unix_seconds`: a `matched` result older than 15 minutes becomes `matched_dated`. That threshold is presentation policy, not a chain guarantee. No automatic/scheduled recheck or expiry enforcement exists ([G1 §11, §18](phase-g1-squads-governance-binding.md), [governance.js](../frontend/src/governance.js)).

The identity hash covers the **stored Squads VaultTransaction message**: signer/writable counts, ordered account keys, instruction program/account indexes and instruction data, and lookup-table descriptors. The supported shape rejects LUTs. Memo, Proposal account status/votes and Buffer contents are excluded. A changed message, including account metas or target/buffer address, changes `message_sha256` and fails a recheck against the bound spec. A changed Buffer byte changes the observed artifact SHA/length and yields `stale_artifact`; changed buffer or ProgramData authority yields `authority_mismatch`; changed ProgramData target/current baseline can yield `different_proposal` or `unverifiable` according to the failed check. Proposal status/votes/staleness are observed and can change without changing the bound ChangeSpec ID. G1 checks PDA derivations, transaction/proposal ownership, vault signer and the Program → ProgramData pointer; it compares stated `replaces` to the current executable when supplied ([squads.rs](../engine/src/governance/squads.rs), [mod.rs](../engine/src/governance/mod.rs), [G1 §7–13, §19](phase-g1-squads-governance-binding.md)).

The message only names the Buffer account; it does **not** hash future Buffer bytes. A match requires Buffer authority to be the Squads vault, limiting who may rewrite it, but a later vault-signed action could do so. An executed proposal commonly consumes the Buffer, so a post-execution G1 recheck can be `unverifiable`/`buffer_missing`; that does not retroactively invalidate the sealed pre-execution match. G2 uses that earlier seal instead. An old `matched` can appear current only if the consumer drops its slot/time or treats the latest G2 status as a fresh G1 result. The current card tries to date G1, but it does not show the full sequence and can overlay one attestation on an unrelated latest G1 check ([governance.js](../frontend/src/governance.js)).

## 5. Deployment evidence: separate conclusions

G2 requires a sealed, **finalized**, `matched` G1 binding whose bound ChangeSpec ID, delivery, target and candidate agree with the supplied spec and verified candidate bytes. It reads current Program/ProgramData and Proposal at finalized commitment. If Proposal is not `Executed`, it returns `not_executed` with no execution claim. If `Executed`, it searches up to ten pages of 1,000 finalized signatures for that Proposal PDA, fetches successful transactions, and identifies the exact top-level Squads execute and direct loader `Upgrade` CPI with the G1 accounts. An `Executed` status with no matching transaction yields `unverifiable`, not deployed match ([attestation.rs](../engine/src/governance/attestation.rs)).

The resulting conclusions remain separate:

| Conclusion | Required evidence | What remains unproved |
| --- | --- | --- |
| Proposal status `Executed` | Current decoded Proposal state | Specific successful execute, deployment, bytes |
| Matching execution occurred | Signature, successful transaction, Squads execute identity and loader CPI; execution slot/index where available | Current ProgramData attributable to it |
| ProgramData changed/deployment slot | Current loader-owned ProgramData account, digest and header `deploy_slot` | Candidate match; an older execution may be superseded |
| Deployment attributable to execution | `deploy_slot == execution.slot`, complete finalized block screening with no later same-slot possible writer | Candidate byte equality until comparison |
| `deployed_match` | Attributable full ProgramData executable begins with all candidate bytes and remaining allocated bytes are zero | Source/build provenance; permanent current-state equality after a later upgrade |

If the ProgramData deploy slot is later than execution, G2 says `superseded`; if earlier or the full-block screen is unavailable/has a later possible writer, it says `unverifiable`. Only attributable state can yield `deployed_mismatch`. The current ProgramData read slot is not the deployment slot or execution slot. G2 retains the execution evidence even when byte attribution fails. The serialized enum also names `unsupported`, but the current attester has no branch that produces it; it must not be presented as an observed current result ([attestation.rs](../engine/src/governance/attestation.rs), [G2](phase-g2-squads-deployment-attestation.md)).

## 6. Persistence, API and frontend gaps

The hosted store is filesystem based, not a governance database row: `data/projects/<project>/governance/<change_spec_id>/bindings/<binding_id>.json`, `checks/<check_id>.json`, `attestations/<attestation_id>.json`, and a derived `change_spec.json`. Runs are separately indexed under project/change, and candidate bytes live in a content-addressed program store ([storage.rs](../server/src/storage.rs), [registry.rs](../server/src/registry.rs)). G1 checks are indexed under both the asked-about and derived bound IDs; this ensures a changed proposal recheck remains visible under the old bound change. G2 records explicitly carry `binding_id`, so multiple pre-execution observations need not be conflated. Stored records are verified on read; historical statements are append-style, though a filesystem administrator can alter files and local CLI output paths can be overwritten. No database redesign is needed.

Hosted GET returns `{change_spec_id, checks, attestations}`. Checks are newest first by time-bearing check ID, capped at 20. Attestations are sorted by descending `observed_slot`, capped at 20; a repeated result at the same slot has the same content ID, and a G2 record has no recording time. `observed_slot` can be absent on an early failure, and sorting it does not establish request order. There is no unified event cursor, so this endpoint cannot answer a complete historical audit after 20 observations or establish exactly when a G2 attempt was recorded. Reading the arrays makes no new chain observation ([api.rs](../server/src/api.rs), [registry.rs](../server/src/registry.rs)).

The hosted report and dashboard report loader each take `checks[0]` and `attestations[0]`; `governanceView` renders one G1 card and can let the selected G2 outcome replace its heading. It shows IDs, slot, proposal status, target, candidate, execution signature and ProgramData fields in technical rows, but no earlier observations or binding → attestation sequence. Its `not_executed` heading says “Proposal matches analysed change” even when the selected G1 check is a different later outcome; the view does not check `attestation.binding_id` against the selected G1 binding. Thus the UI is **partial**, not a reliable historical review surface. The API is richer but still fragmented. There is no hosted action button for recheck; the authenticated POST route and CLI are available ([report.js](../frontend/src/report.js), [dashboard report loader](../frontend/dashboard/pages.js), [governance.js](../frontend/src/governance.js)).

## 7. Live-witness limitation

The [G2 qualification record](phase-g2-squads-deployment-attestation.md) says the engine and hosted path exist, with deterministic tests for execution, matches, mismatches, supersession and missing evidence. Its remaining acceptance condition is a **full real-mainnet deployed match** using a sealed pre-execution G1 match and retained candidate. The three committed G1.1 matches were still `Active` at finalized slot 450,382,760. A production CLI G2 call for the primary candidate returned sealed `not_executed` at slot 450,385,784. Other already-executed mainnet proposals demonstrate execution and supersession evidence shapes but lack a sealed pre-execution G1 candidate binding. This is principally **B: no qualifying real witness had executed**, not evidence that the G2 implementation is absent. MAIN has no separate G2.1 implementation or status document found in current code/docs; the cited G2 record is the operative qualification statement. No later live status is inferred here.

## 8. Minimal factual review timeline and vocabulary

The trail should use existing identities and show each observation with its own source and time coordinate. Host recording time and chain slot must have distinct labels; a missing coordinate stays missing.

| Timeline item (existing name) | Identity and source | Coordinate, conclusion, relationship and limit |
| --- | --- | --- |
| `ChangeSpec` and analysis run | Bound ChangeSpec ID; run ID/report and pinned bundle | Run creation/completion time; analytical verdict for exact candidate/target. Linked by ChangeSpec ID; no live governance result. An earlier unbound run stays linked as source context, not silently treated as the bound analysis. |
| `GovernanceCheck` → `GovernanceBinding` | Check ID, binding ID, source and bound ChangeSpec IDs; finalized/confirmed G1 RPC | Host `checked_at` plus chain `observation.slot`; one proposal/buffer match or typed failure then. Rechecks append as peers. A `matched` item never becomes permanent. |
| `DeploymentAttestation` with `not_executed` | Attestation ID, binding ID; finalized G2 RPC | Current `observed_slot`; awaiting execution as observed then. No execution or failed deployment comparison. Hosted recording time is missing today. |
| `DeploymentAttestation.execution` | Same attestation ID and binding ID; finalized transaction RPC | Historical execution signature/slot and possible block index; successful matching Squads execute and Upgrade CPI. No deployed-byte equality by itself. |
| `DeploymentAttestation.deployed` and outcome | Same attestation ID; current ProgramData RPC plus block screen | Current read `observed_slot`, ProgramData `deploy_slot`, digest, prefix/padding results where attributable; `deployed_match`, `deployed_mismatch`, `superseded` or `unverifiable`. A later observation appends; earlier proof is not rewritten. |

Use the existing G1 outcomes (`matched`, `stale_artifact`, `different_proposal`, `authority_mismatch`, `unsupported_proposal`, `unverifiable`) and G2 outcomes (`not_executed`, `deployed_match`, `deployed_mismatch`, `superseded`, `unverifiable`) as separate factual axes. Show Proposal status (`Active`, `Approved`, `Executed`, etc.) separately. “Dated match” is a UI age label for an old `matched`, not a new chain outcome. “Execution observed” follows `execution: Some(...)`, even if deployed proof is `unverifiable`. Keep analytical PASS/BLOCK findings independent; a correctly bound candidate may have BLOCK findings. Do not create one overall `safe` or `approved` state.

## 9. Dominant gap and exactly one next implementation

**Dominant gap: D, reviewer presentation**, with a narrow history-contract defect in G2 timing. Persistence/linking, manual freshness rechecks and hosted routes already exist. The next implementation is **a project-scoped Squads program-upgrade review trail for one governance-bound ChangeSpec**. It should adapt the existing filesystem index and sealed G1/G2 objects into a paginated, identity-checked read model and render that model in the bound run/project review view. It should append a small G2 observation occurrence/recording-time index, analogous to `GovernanceCheck`, rather than changing the sealed G2 proof's meaning. This is one end-to-end reviewer feature, not a new governance engine or a second database.

The same ChangeSpec-linked read model could later accept evidence from other kinds, because ChangeSpec IDs and run indexes already cover multiple kinds. This phase is limited to the existing Squads upgrade path.

### Acceptance criteria for that phase

1. **Storage/history:** Each hosted G1 recheck remains a separate `GovernanceCheck`. Each hosted G2 request gets a durable project-scoped occurrence with recording time, attestation ID and bound ChangeSpec ID, including `not_executed`/`unverifiable`; identical sealed proof content may be shared, but repeated attempts remain distinguishable. Existing sealed files and earlier observations are never rewritten. Existing G2 files without occurrence metadata remain readable with “recorded time unavailable.”
2. **Identity:** The trail root is the governance-bound ChangeSpec ID. Every G1 item verifies that ID is its asked-about or derived bound ID; every G2 item verifies its `change_spec_id`, `binding_id`, candidate, proposal/message and target against a stored G1 match for that same bound change. Source unbound ChangeSpec ID and linked run IDs are shown without relabelling old reports. A G2 proof must never be paired visually with a different latest G1 check as though they were one observation.
3. **Chronology/freshness:** Expose paginated, stable event ordering with both host recorded time (when available) and the original chain coordinates. Preserve G1 `observation.slot`, G2 `observed_slot`, execution slot/signature and ProgramData deploy slot as distinct facts. “Latest” is derived from occurrence order and never updates an older item. The existing verify/attest POST actions remain explicit requests; merely viewing history performs no new chain read. A later `stale_artifact` or `different_proposal` stays visible beside an earlier `matched`.
4. **API/auth:** A project-scoped authenticated trail read endpoint or backward-compatible extension of the existing GET returns verifiable evidence references and a next cursor beyond 20 items. Project members, that project's token and operator retain current access rules; another project's token and unauthenticated callers cannot read it. Tampered or missing sealed evidence fails closed rather than becoming a green summary.
5. **Reviewer UI:** The bound run/project view shows the analysis verdict separately from a chronological Squads trail. Each row names source, outcome, proposal ID, candidate/target identity, recorded time if known, observed slot, and a link/expansion to exact sealed evidence. It distinguishes original binding from later rechecks, `not_executed` from mismatch, successful execution from deployment attribution, and `deployed_match` from a later `superseded` observation. The unbound run is not presented as an analysed bound proposal.
6. **Tests:** Use current simulated Squads fixtures to cover match → later stale buffer/message change, Active → `not_executed`, execution without attributable bytes, match/mismatch/supersession, repeated identical G2 attempts, pagination/order, missing legacy G2 time, cross-project denial and tamper rejection. Assert the boundaries: **proposal binding ≠ deployment proof; old observation ≠ current observation; `Executed` ≠ deployed-byte match; governance outcome ≠ analytical verdict; missing execution witness ≠ failed match**. No fabricated mainnet witness is needed for this product phase.

### Non-goals

No proposal signing, approval or execution; no broader Squads transaction shapes, LUT support, other governance providers or other ChangeSpec kinds; no new impact analysis engine, provider/onboarding work, CI-run-ID root, database replacement, automatic polling/alerting, or reinterpretation of `not_executed` as failure. The outstanding real-mainnet G2 deployed-match witness remains an external qualification event, not something this phase may simulate into existence.


## 10. Implemented review layer

### Reading the evidence

**Analysis ≠ governance binding ≠ execution evidence ≠ deployed-byte proof.**
The analysis section keeps the run's analytical result, including a BLOCK or other
nonzero exit code. The linked-run list includes only analyses of the exact bound
ChangeSpec; a source unbound analysis is never relabelled.

The Squads governance trail shows G1 and G2 as separate retained observations.
An old G1 `matched` remains `matched`, with a dated presentation after 15 minutes;
even a recent match is explicitly limited to its observation. A G2 row says
“Based on G1 binding” with the actual sealed ID, regardless of any later G1
`stale_artifact`, changed-message, or missing-buffer check.

`not_executed` means execution had not been established at that observation. It is
not a failed deployment. `unverifiable` with execution names the matching
transaction but says attribution/byte proof was not established. Only the G2
`deployed_mismatch` outcome states an attributable mismatch. `superseded` retains
the earlier execution and identifies later ProgramData deployment state.
`deployed_match` is an observation, not a permanent current-state guarantee.

Each row separates **Recorded** host time from **Observed** slot. Execution slot,
transaction signature, ProgramData deploy slot, prefix/padding comparison, proposal
status, message-read slot and safe account digests remain available in expandable
Technical evidence. No filesystem paths are exposed by the trail response.

### API and read model

`GET /v1/projects/{project_id}/governance/changes/{change_spec_id}/trail`
uses the existing project authorization (operator, authorized member, or that
project's token). The original governance GET and attest POST response are retained.
The root must resolve to a stored governance-bound program-upgrade ChangeSpec.

```text
{
  project_id, change_spec_id, source_unbound_change_spec_id,
  candidate, target, delivery,
  runs: [{run_id, status, exit_code, candidate_sha256, bundle_sha256,
          created_at_unix_seconds, completed_at_unix_seconds}],
  runs_next_cursor,
  events: [
    {type: "governance_check", event_id, recorded_at_unix_seconds,
     check: GovernanceCheck, binding: GovernanceBinding},
    {type: "deployment_attestation", event_id, recorded_at_unix_seconds,
     legacy, occurrence: AttestationOccurrence | null,
     attestation: DeploymentAttestation}
  ],
  next_cursor
}
```

A source unbound ID is reported when a G1 event on the returned page establishes
that source; each G1 always includes its own asked-about and derived identities.
Full sealed evidence retains its original IDs and fields. Run summaries are an
explicit allowlist, never full reports or storage metadata.

`limit` defaults to 20 and is clamped to 1–100. `cursor` is the last event ID of the
preceding page, exclusive. Hosted events are oldest first by the sortable suffix
of `gchk_…` / `gocc_…`, using the existing shared monotonic millisecond ID mint;
the different prefixes do not decide order. Seconds can be equal. Chain slots
never sort hosted events. This is keyset pagination, not offsets or a frozen
snapshot: later requests can append events while a reviewer pages.

Legacy proofs follow hosted events in deterministic attestation-hash order with
`legacy_<attestation_id>` event IDs, `legacy: true`, null occurrence and null
recording time. Their placement does **not** claim they happened later. Legacy
relative request chronology cannot be reconstructed. Newly appended hosted
events after a reader has reached the legacy group require restarting from the
first page. Linked runs use the existing newest-first run ID ordering with an
independent `run_cursor` / `runs_next_cursor` on this same endpoint.

Filesystem discovery scans filenames and small occurrence metadata only within
this ChangeSpec. It reads at most one page of evidence bodies and linked run
summaries, plus each G2 event's referenced binding. It never loads every project's
proof history. Discovery remains linear in this change's index size; there is no
new database. Every displayed event is verified before a page is returned; an
inconsistent event fails its page with HTTP 500 rather than being silently dropped.

### Durable G2 attempts and legacy compatibility

```text
governance/<bound_change_spec_id>/
  bindings/<binding_id>.json
  checks/<check_id>.json
  attestations/<attestation_id>.json
  attestation-occurrences/<occurrence_id>.json
  legacy-attestations/<attestation_id>.json  # only when reusing a legacy seal
```

Each successful hosted attest request writes this immutable hosted record after
the seal has been persisted:

```json
{
  "occurrence_id": "gocc_<sortable-id>",
  "project_id": "proj_...",
  "change_spec_id": "<bound-id>",
  "binding_id": "<exact-G1-id>",
  "attestation_id": "<sealed-G2-id>",
  "recorded_at_unix_seconds": 1790000000
}
```

Concurrent G2 recording is serialized with the existing registry transition lock;
trail reads share that lock to avoid seeing a half-published occurrence.
Identical content shares one attestation file and receives distinct occurrences.
The attest POST still returns the sealed proof; occurrences are discoverable in
the trail. A pre-proof failure produces no invented proof or occurrence. An
interruption between proof and occurrence persistence can leave an unknown-time
proof, which uses the same conservative legacy presentation.

No migration is needed. GET never rewrites or indexes old evidence. If an explicit
later POST returns the same seal as a legacy proof, it first preserves that proof's
unknown-time provenance with an immutable marker containing only the attestation
ID. This keeps the legacy entry visible alongside the new timed occurrence;
neither the seal nor its timestamp is fabricated.

### Integrity and read-only behavior

G1 reads reparse the seal, check the index's project/change/ID, reconstruct the
asked-about spec using the expected delivery, and recompute both source and
possible derived identities. Negative rechecks remain related to the change they
were asked about, even when their observed message derives another bound ID.

G2 reparses the seal and loads its exact binding directly by hash, with no latest
selection or 100-check search cap. That binding must be a finalized sealed G1
match for the root. Candidate, target, ProgramData and complete Squads delivery
must agree with the root and proof, including proposal, transaction and message
identity. Occurrence project/change/binding/attestation references are checked.
Integrity errors are explicit and do not reveal internal paths. GET needs no RPC
configuration and performs no observation, freshness update, evidence write, or
project mutation. Recheck/attest remain explicit POST actions; no polling or new
action workflow was introduced.

### Validation and scope

`server/tests/governance.rs` covers the authenticated route, identical POST results
with distinct occurrences, bound/unbound run separation, linked-run pagination,
negative rechecks, and read-only reads. `server/tests/governance_trail.rs` covers 50
mixed events over multiple pages, stable cursors, legacy compatibility, exact
binding linkage, corrupt indexes/hashes and resealed cross-object mismatches, and
real simulated attester outcomes (match, mismatch, supersession and execution with
unattributable bytes). Frontend unit and browser regressions cover factual wording,
separate analysis, dated matches, technical expansion and explicit pagination.

The historical [G2 qualification statement](phase-g2-squads-deployment-attestation.md)
is unchanged: a full real-mainnet `deployed_match` witness remains an external
future event. Simulated fixtures qualify the product integration only. No broader
Squads support, signing/execution, monitoring, notifications, automatic rechecking,
other provider, or new G1/G2 proof rule is included.


Validation run: 7 hosted/trail integration tests, 34 existing engine governance
tests and 27 server unit tests passed. `npm run check:frontend` passed, and
`npm run test:governance-browser` passed at 1280px and 390px using isolated fixture
responses (no live RPC). The dependency was installed in a temporary test directory
because this checkout did not have its declared Playwright dependency installed.
