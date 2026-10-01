# Token migration example

`migration.json` contains the pinned STA example terms in MAIN’s snake_case
encoding. The T3 evaluator wraps these terms and the exact candidate artifact in
MAIN’s `ChangeSpec`; the activation moves to its top level.

Follow the [migration walkthrough](../../../docs/token-migration.md) for the exact build, disposable project copy and CLI commands. Build the candidate with `scripts/build-migration-candidate.sh`. This minimal recipe uses `liteSvmBundled` programs and needs no imported capture.
Archived recipes using `pinnedMainnetCapture` separately require exact frozen
bytes from `scripts/import-migration-fixtures.py --sta /path/to/sta-archive`.

The T3 library accepts these terms through `ChangeSpec::token_migration` and
`migration::input::assemble`. The single-binary CLI integration is implemented; run `eplyx doctor` and `eplyx migration analyse` inside the prepared project.
The minimal fixture bytes are unchanged; SPACEX is rebuilt from the four frozen
records described in `fixtures/migration/spacex/provenance.json`. No example
claims an official issuer migration or authorizes live execution.
