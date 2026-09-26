# T2 — Shared primitives for token transitions

T2 implements the shared foundation in the [T0 ADR](phase-t0-stock-transition-integration.md).
It adds no change kind or migration evaluator.

## Token layouts

`standard_programs::token2022` now describes all 28 pinned extension types,
including confidential fields as opaque ciphertexts, authority pointers, metadata,
groups, interest timestamps and PermissionedBurn. Base accounts and mints still
use MAIN's `spl_token` decoder. Unknown types retain their numbers and bytes.
The legacy `decode_extension` interface maps malformed entries to raw
`Unrecognized`; `decode_checked_extension` distinguishes malformed from unknown.
`checked_extensions` also detects duplicate and misplaced entries, malformed
lengths, nonzero padding and nonzero data after a terminator. These are layout
facts. Migration/lifecycle support policy must refuse unverifiable execution.

SPL's unaligned boolean convention is preserved: any nonzero byte means true.
Scaled display multipliers are retained as integer bit patterns. Exponent bits
reject infinity and NaN; no floating-point arithmetic or display-value conversion
is introduced. New 64-bit fields serialize through MAIN's decimal-string helpers.
The captured epoch selects a transfer-fee schedule; fee arithmetic uses integer
ceiling and the recorded cap.

Independent tests construct the official POD/Borsh layouts from
`spl-token-2022-interface 3.1.1`, `spl-token-group-interface 0.7.2` and
`spl-token-metadata-interface 1.0.1`, pinned as development dependencies. They
cover every fixed layout, variable metadata, malformed input, large counters,
opaque ciphertexts and fee schedule boundaries. No engine transport dependency
or async runtime is added. Existing dependency versions are retained; the lockfile
adds the interfaces' dependency closure.

## Capture and execution primitives

MAIN's curl transport reads at most its byte ceiling plus one sentinel byte before
JSON parsing, terminates and reaps an oversized response, and accepts a configurable
ceiling up to the 64 MiB default. It disables ambient curl configuration, makes one
attempt and does not follow redirects. Error diagnostics retain only the numeric
provider code, since provider text can contain credentials.

Read-only helpers check the complete pinned mainnet genesis hash, request finalized
contextual program-account responses with bounded memcmp filters, and request an
exact ordered batch of 1–100 accounts with `minContextSlot`. Missing/older context
and a mismatched response count fail. Raw contextual results remain available to
the capture transcript. This does not claim an atomic bank or authorize live RPC.

`executor::execute_probe_message` adds the narrow STA local-probe path to MAIN's
fresh LiteSVM backend. It uses the existing `LoadedProgram`, captured loader
identities and account snapshots; signature possession and blockhash freshness
are explicit local assumptions. Actual inner-instruction payloads and watched
post-accounts are returned, with checked instruction indexes. Execution evidence
has no deserialization constructor. A local fixture checks deterministic fresh
execution, fees and failed-transfer rollback.

`canonical::{document, digest}` supplies pretty JSON with one final newline for
new artifacts. Callers must use deterministically ordered inputs. MAIN's existing
compact ChangeSpec identity encoding is unchanged.

## Verification

The command receipts and log digests are in `phase-t2-verification.json`.
The initial compile found the new malformed-reason display arm missing; it was
added. Initial lint found two test initializer issues; they were corrected.
Review against pinned STA caught an incomplete genesis constant before the final
full suites; the corrected constant also has a decoded-length assertion.
These development failures did not change a frozen analytical expectation.

No capture fixtures or SBF binaries are added. MAIN's five upgrade adapters,
program-upgrade wire format, frozen ChangeSpec identity and historical evidence
are preserved. STA remains unchanged. Live-provider checks are intentionally not
run; migrated feature and Postgres/browser checks remain pending their phases.
