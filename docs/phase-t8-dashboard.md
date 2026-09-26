# T8 — Local dashboard and shared presentation

`eplyx dashboard` now serves immutable local migration, lifecycle and current-state
results. The loopback service accepts only GET/HEAD, validates Host and path
components, bounds headers/connections/reads, and exposes a fixed artifact list.
It never executes an analysis or reads provider configuration. The index is a
rebuildable cache, never an analytical source. Local and future hosted views use
`dashboard::view::from_bytes` with the same identity and consistency checks.

## Distinct analytical scopes

Migration pages retain the eight answers, engine gate, exact quantities,
counterexamples, provenance and compare story. Compare requires the same kind.
Equivalent-search resolution requires matching search conditions and an explicit
trace for the exact derived state, source account and message. An absent finding
alone cannot establish resolution. Small reports do not include a complete
per-account sequential execution trace, so observed-population resolution is not
inferred. This display limitation does not change any search finding.

Lifecycle pages keep declared policy, public state and evaluated consequences
separate. Current observations keep Transfer, exit, withdrawal, redemption and
conversion statuses independent. Neither gets an invented migration gate or
candidate. `lifecycle analyse`, `observe replay` and offline `path` commands can
append their exact output with `--record <existing-project-directory>`.
Metadata schema 3 binds these outputs and optional input descriptors by digest;
legacy migration metadata stays readable. A current observation is not execution,
owner identity does not prove key possession, and refreshed observations begin
untested.

## MAIN presentation

The shared glossary supplies frontend labels, Markdown and human CLI output.
Machine codes and canonical analytical JSON do not change. New migration bindings
name Markdown presentation version 2; historical absent/version-1 bindings retain
the original renderer. A replay test checks both versions and rejects substituting
a new Markdown file into an older record.

The dashboard keeps STA's shell, sidebar, crumbs, subnav, tiles, counterexample
pages and Overview/Technical mode. MAIN's role tokens, logo, violet palette,
DM Sans and Manrope replace STA branding. Fonts are embedded with their licences;
loopback browsing makes no external request. CSS tests forbid colour literals
outside the token block. The mode setting also controls MAIN's existing technical
sections. A public `/token-transitions` page and landing section explain the
bounded capabilities without replacing MAIN's site.

## References and checks

[The test mapping](phase-t8-test-mapping.json) records hashes and dispositions for
35 source tests. Archived fixed-ratio UI examples are replaced by fresh MAIN
migration presentation inputs; their old economic expectations remain frozen at
T0. The source migration-dashboard sync assertions are assigned to T9.

[Presentation fixtures](../fixtures/dashboard/README.md) contain exact small
analytical artifacts plus a separately identified derived view-model fixture.
They contain no SBF or raw capture bytes and are not a replay package. Lifecycle
and current-observation examples use synthetic providers. Every browser service
writes only to scratch directories; historical evidence is unchanged.

Verification passed: `make test` recorded 1,309 passing executions and
`cargo test --no-fail-fast` recorded 1,259, each with the same two pre-existing
ignored tests. The latter includes the final additional CLI glossary unit test.
All 17 browser tests and 67 rendered-view assertions passed, along with formatting,
workspace/program clippy, existing frontend, report and governance checks.
[The phase record](phase-t8-verification.json) retains failures and successful
retries separately. These counts include repeated executions, not unique tests.

The audit matched all 1,633 pinned STA files, and STA is clean. The staged delta
contains no private endpoints, machine paths, keys, SBF or raw captures. The only
added binary assets are the two explicitly required licensed fonts (61,556 bytes).
License line endings/trailing whitespace are normalized with both downloaded and
retained hashes recorded. Live-provider checks remain intentionally not run.
No live-cluster transaction, deployment or push occurred.
