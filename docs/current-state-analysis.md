# Current-state analysis

In a hosted project, choose **Analyse**, select a saved catalogue identity or enter
a canonical mint, and request a new observation. The catalogue supplies public
source assertions. The engine independently decodes current state; a custom mint
has unconfirmed issuer association. A saved catalogue remains dated evidence if
its public source cannot be refreshed.

Select **Public owner and mint** to inspect exact token accounts for one public
owner. Choose one focused account before requesting a path or candidate check.
The owner address does not establish signing possession. Balances and typed amounts
retain integer precision, including values above JavaScript's safe integer range.
Zero balances and unsupported account features show an availability reason.

The browser sends terms only. The server acquires bounded read-only observations,
stores their identities and queues a separate offline worker. It never accepts
caller-supplied executable code, instructions, account metas, transaction bytes,
providers, filesystem paths or claimed status.

## Distinct checks

**Transfer** evaluates exact token movement to the selected recipient. **Meteora
market exit** uses bounded route discovery, exact captured deployed code and the
chosen minimum output. A successful Transfer does not prove market exit. Liquidity
withdrawal is available through the offline path CLI with exact PositionV2
ownership, range, shares and reconciliation; it is not a pool-vault ownership claim.

**Candidate check** turns the proposed replacement terms into MAIN's one-account
token-migration ChangeSpec. The registered reference mechanism executes only after
exact final-state coherence. Custom amounts bind a positive `ExactRaw` amount;
source drift yields Indeterminate without adjustment. Success establishes only that
candidate execution and reconciliation in the recorded VM context.

**Proposed scenario** evaluates user-declared effective/deadline times and an optional
replacement mint, using selected retained checks. It keeps mobility, candidate
execution and full-transition findings separate. Neither a proposed successor nor
a successful candidate establishes an official transition.

**Bounded stress** starts from a completed candidate in this observation. The
service fixes the budget, captures a fresh population, freezes at most ten cases,
and executes their full final balances against exact final state. It preserves
failed and unsupported selections. Its sample and state-shape coverage are not
population or entity coverage; non-wallet authority resolution never supplies a
wallet signer.

A result marked Proven requires actual local execution and exact reconciliation.
No check moves funds on a cluster. Refresh creates a new observation with all paths
untested. Old runs remain immutable and cannot lend statuses to the new one. A
failed acquisition does not fall back to historical evidence.

## Offline use and boundaries

`eplyx observe replay --input capture.json --format json` re-decodes saved bytes.
`eplyx path capabilities`, `validate`, `replay`, `probe`, `discover-position` and
`probe-withdrawal` expose the same bounded engine paths; consult `--help` for their
explicit inputs. Capture commands are separate read-only acquisition steps and
require an explicitly configured provider. No current-state run is a historical
validator replay or a proof of an atomic bank across every observation.

The migration was verified with frozen fixtures and loopback mock providers only.
Live-provider checks were intentionally not run. See [T7](phase-t7-current-paths.md)
and [T9](phase-t9-cloud-hosted-analysis.md) for exact interfaces, budgets and tests.
