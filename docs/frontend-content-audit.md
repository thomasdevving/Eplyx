# Frontend and documentation review — 2026-10-01

This review compares public guidance and analytical presentation with the reviewed
engine/server source and preserved qualification evidence. It does not attest the
readiness or contents of a remote deployment.

## Corrections

| Previous gap | Resulting guidance / behavior | Source of truth |
| --- | --- | --- |
| Fee analysis described only as future work | Two bounded fee operations, their inputs, browser/API entries and authority limits are explicit. | [Parameter contract](protocol-parameter-change.md) |
| Code/fee interaction absent from onboarding | A separate CLI section and walkthrough explain qualified candidates, independent cells and the retained hosted parent. | [Interaction contract](upgrade-parameter-interaction.md) |
| New local search/case work undiscoverable | Current source includes the verified consolidation; older binaries may lack it, and the fixed review deployment is separate. Private reference inputs are not installation outputs. | [Integration](analysis-integration.md), [search](parameter-edge-search.md), [cases](parameter-case-set.md) |
| Public example used placeholder identities and different counts | Landing and demo report read the same preserved canonical regression: one deposit decrease and nine withdrawal reverts, with actual hashes/slots. Absent ChangeSpec/replay proof stays unavailable. | [Archived JSON](examples/phase-u3a-validation/reports/regression.json), [example guide](../examples/stake-pool-upgrade/README.md) |
| Migration browser flow described as unavailable | Prepared migration and lifecycle forms are explained with their actual supported shapes and required files. | [Migration](token-migration.md), [lifecycle](lifecycle.md), source forms |
| Minimal migration example incorrectly required a private capture | Copy/build/run instructions use its declared LiteSVM-bundled synthetic recipe; archived pinned-capture recipes retain separate requirements. | [Minimal recipe](../examples/migrations/minimal/fixtures/world.json), [migration walkthrough](token-migration.md) |
| Deployment guide described synchronous execution without accounts/dashboard | Operator commands, generated IDs, registered bundle IDs, durable queue, retained candidate bytes and optional identity match the server. | [Service guide](railway-deployment.md), server CLI/storage/config |
| Dashboard detail choice reset on reload | Initialization restores the saved Overview/Technical choice; public pages retain one presentation. | Existing persistence browser regression |

[The documentation index](README.md) now separates task guides from dated phase,
design and qualification records and explains common result terms. Product next
steps distinguish implemented forms/readiness/governance history from future work.
A missing historical F1 reference is labelled unavailable rather than replaced
with fabricated evidence. Historical reports and qualification bytes are unchanged.

## Verification

- Complete frontend presentation/analytical checks and static build passed in
  the final combined checkout.
- All **24** public browser checks passed there, including desktop/mobile layouts,
  task anchors, exact command copy, fee entry, retained demo, legal routes and
  absence of public browsing cookies/preference storage.
- **24 distinct** dashboard browser flows passed across the initial run and
  focused confirmation: the 22 initially passing flows plus the two repaired
  persistence/interaction flows. Hosted parameter and interaction flows use mocked
  authenticated HTTP; they do not claim fresh chain or remote-service validation.
- **49** CLI examples were checked against actual command help: 42 existing
  commands and seven local integration commands. The prepared minimal migration
  ran offline in a disposable project and reported warnings for retained
  authority/frozen-holder/population gaps.
- The final documentation link scan covered **101 documents / 686 local links**
  with no missing targets before adding this audit record. Desktop/mobile
  screenshots were inspected; the scoped mechanical design scan returned no
  findings.
- Engine/server locked offline builds and formatting were checked. The previous
  integration's **82 distinct tests** and enforced offline qualification remain
  recorded in its existing receipts; this frontend review does not claim to have
  repeated that full analytical qualification.

The legal notices remain visibly drafted with twelve unresolved operator review
items. `check:legal` reports them; `check:legal:release` intentionally requires
their resolution. Source/build verification does not finalize legal facts.
No private capture, credential, generated executable, dependency directory or
build output is included in the publication change.
