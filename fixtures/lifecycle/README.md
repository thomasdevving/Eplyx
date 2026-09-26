# Lifecycle regression inputs

These files freeze STA commit `ad1897e86ee05ff2255bd5277518a3230ee2340f`.
The 92 files in `provenance.json` total 371,438,848 bytes. The original paths,
lengths and SHA-256 values are the import allowlist. Each source directory has a
provenance note. No private provider configuration is read or copied.

```sh
python3 scripts/import-lifecycle-fixtures.py --sta /path/to/local/STA/archive
python3 scripts/project-lifecycle-reference.py
```

`sta/` retains exact original bytes. `main/` is a separate encoding projection for
MAIN's regression tests. Both sets of payloads are ignored because historical
captures include provider-origin fields; only provenance, scripts and markers are
committed. Missing payloads fail the tests. No download, provider call or silent
fallback occurs. Import and projection refuse changed existing files.

The projection does not execute Eplyx or generate expected outcomes. It verifies
all original hashes, converts analytical wide integers to decimal strings and
Scaled UI multipliers to their exact IEEE bit strings, maps the approved MAIN exit
codes, and propagates the resulting digest changes. Raw RPC transcripts, account
bytes, captured executables, notice HTML and source JavaScript stay exact.
`encoding-projection.json` lists every before/after file hash and size, plus the
preimages of internal hashes (normalized snapshot, sorted coverage, embedded
policies, candidate declarations and counterfactual tuples).

The frozen matrix, findings, balances, execution results, reconciliation and
assurance decisions are not rewritten. Tests compare complete projected records
with independently evaluated MAIN reports, including fresh execution for the
five-path replay. Decimal-string and derived-hash differences are the T0-approved
encoding changes; readiness's blocked/incomplete codes follow MAIN's 1/5 mapping.

The T0 static inventory missed two dependencies: the captured Token-2022 mint
fixture and the captured issuer FAQ JavaScript. This closure additionally pins
those files against the original source inventory. T0's records remain unchanged.
The captured issuer JavaScript is historical evidence only; Eplyx never executes it.

The notice impact report is also retained as the preimage of the notice pipeline
report digest; it is compared in full with MAIN’s independently evaluated impact.
