# Token migration example

`migration.json` contains the pinned STA example terms in MAIN’s snake_case
encoding. The T3 evaluator wraps these terms and the exact candidate artifact in
MAIN’s `ChangeSpec`; the activation moves to its top level.

Build with `make programs`. Import the frozen token-program captures once with
`scripts/import-migration-fixtures.py --sta /path/to/sta-archive`.

The T3 library accepts these terms through `ChangeSpec::token_migration` and
`migration::input::assemble`. The single-binary CLI integration follows in T5.
The minimal fixture bytes are unchanged; SPACEX is rebuilt from the four frozen
records described in `fixtures/migration/spacex/provenance.json`. No example
claims an official issuer migration or authorizes live execution.
