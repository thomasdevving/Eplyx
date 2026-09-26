# T5 — Migration CLI and local run store

MAIN's `eplyx` binary now exposes Token Migration V1. Program-upgrade commands keep
their meanings, including the existing `reproduce <fixture|cluster>` command.
STA's fixed-ratio product and second binary remain archived.

## Commands and configuration

| Command | Purpose |
| --- | --- |
| `eplyx init --migration [--fixture]` | Write editable terms and strict project configuration; `--force` replaces templates only |
| `eplyx doctor` | Check installation, configuration, candidate, terms and state recipe without contacting a provider |
| `eplyx change token-migration --spec migration.json --mechanism migration.so --out change.json` | Build MAIN's identifying proposal; accepts `--program-id`, `--activation-slot` or `--activation-unix` |
| `eplyx migration analyse` | Assemble proposal/state/CAS inputs, acquire or build state, and evaluate in an isolated worker |
| `eplyx migration search [--run ID]` | Search saved bytes offline; without an ID, analyse the configured project first |
| `eplyx migration gate --run ID [--policy strict]` | Re-execute saved evidence and any saved search, then evaluate the policy |
| `eplyx migration plan --run ID [--out unsigned.json]` | Verify and export the unsigned plan; existing output files are never overwritten |
| `eplyx migration fixture recipe.json` | Inspect deterministic fixture addresses derived from public labels |
| `eplyx migration reproduce CX_ID` | Re-execute the exact witness and append successful or failed reproduction history |
| `eplyx runs --json`, `eplyx show ID` | View saved history, explicitly without claiming fresh verification |
| `eplyx --version`, `eplyx version --json` | Show build identity and supported MAIN/migration schemas without a project or network |

Migration, doctor, init, change and show commands accept `--format json`; aborts
carry structured errors and exit codes. `--config` selects the project configuration.
The example at `examples/migrations/minimal/eplyx.toml` uses the existing unchanged
recipe and the same nine source invariants. Build the reference candidate and copy
`artifacts/eplyx_token_migration.so` into the example's `target/deploy/migration.so`.
The hashes and platform-tools pins remain those recorded in T3.

`eplyx.toml` accepts only `[project]`, `[transition]`, `[program]`, `[state]`,
`[rehearsal]`, `[[invariants]]` and `[gate]`. The only implemented adapter value is
`token_migration_v1`. Unknown sections, keys, modes and invariant definitions fail.
Project members must be relative paths that remain inside the canonical project
root, including through symlinks. Reads have byte bounds before allocation.
Inputs are validated before any capture. Doctor never reads a provider URL or
Eplyx token and never makes a provider request.

Detached migration terms use MAIN's snake_case representation. Creating a
ChangeSpec moves activation to its top-level field and retains deadline within
the kind. Candidate bytes enter execution only through MAIN's resolver; saved
runs use a SHA-256-named program file, rechecking both hash and length on replay.
There is no STA schema-3 combined package or additional executable resolver.

## Local layout and identity

```text
.eplyx/
  bundle/                       # existing MAIN pinned upgrade bundle
  expected-changes.toml          # existing MAIN upgrade review policy
  project.json                  # local checkout identity, not a hosted project
  runs/run_<time>_<candidate>/
    metadata.json               # written last; marks a complete local analysis
    input/change.json           # MAIN ChangeSpec
    input/state.json            # independent state/clock/invariant descriptor
    input/programs/<sha256>     # candidate bytes
    input/fixture.json          # fixture state only
    result/                     # bound captures, reports, plans and VM outcomes
    search/                     # optional saved bounded offline search
  counterexamples/cx_<hash>.json
  reproductions/repro_<time>_<hash>.json
  cache/                        # temporary validation inputs
  sync/                         # reserved local sync bookkeeping
```

The existing bundle and expected-change paths are preserved. Init ignores only
the added local store paths. `project.json` uses a `local_` checkout identity;
`EPLYX_PROJECT_ID` continues to mean a hosted MAIN `proj_...` ID. T9 will map auth
and workspace state to that existing hosted filesystem registry. Neither this
store nor a Postgres table establishes a second hosted evidence registry.

Reports omit timestamps, run IDs, Git state and binary build identity. Metadata
holds those fields and the exact ChangeSpec/input/candidate identities. Repeating
an analysis over identical bytes produces identical analytical reports. Metadata,
saved witness JSON and reproduction history are bookkeeping, never proof objects.
Gate, plan and reproduction verify the saved bytes without requiring the original
project config; search checks that a selected run still matches the current config.

Files are created exclusively and synced. A complete saved search can be replayed
idempotently; changed existing witnesses are refused. No operation rewrites a
historical report to accommodate changed inputs. Partial failed runs remain
incomplete and are excluded from complete-run history. Symlinked store roots, run directories and nested run members are refused.
Traversal has explicit depth and member-count bounds.

## Isolation and exits

A capture parent alone can read `SOLANA_RPC_URL`. Fixture analysis reads no RPC.
The finishing worker, offline search, gate, plan and reproduction are spawned with
`env_clear`. Workers reject an inherited environment before evaluation. On macOS,
startup removes the system-inserted `__CF_USER_TEXT_ENCODING` hint first; a local
diagnostic confirmed that this hint appears even after `env_clear`. All other
variables remain forbidden. Evaluation reads no cloud token. Reproduction records
bind input, candidate, world, witness kind, expected and reproduced signatures,
and the gate consequence; failed attempts remain history as well.

MAIN's exit mapping is retained: 0 passed (including warnings), 1 gate failed,
2 configuration/fidelity/invalid input, 3 stale declaration in existing MAIN
commands, 4 incompatible state, and 5 unevaluable. Typed incompatibility errors
cover mismatched saved proposal/state identities, captured-world digests and
non-mainnet capture identity. Strict known frozen/authority requirement violations
exit 1. Evidence-only strict gaps retain T3's exit 5. A verified saved search finding
adds `COUNTEREXAMPLE_FOUND` and blocks either policy. Gate evaluation never relies
on a claimed reproduction-history outcome.

Unsigned plans are descriptors with fresh-VM cross-checks. They contain no signed
transactions or private keys and are never submitted by these commands. Exact
public-label fixture signing is confined to the local VM.

## Verification

Verification passed: `make test` ran 1,020 passing tests, and the final
`cargo test --no-fail-fast` ran 969. Both had zero failures and the same two
existing/source ignores. Final formatting and lint pass, as do frontend, report
and governance checks with exact pnpm 11.24.0. The [verification record](phase-t5-verification.json)
retains failed attempts, retries and limitations. The [source mapping](phase-t5-test-mapping.json)
records every adaptation. All 1,633 inventoried STA files retain their hashes and
STA is clean.

The CLI tests retain
all three migration CLI scenarios from pinned STA and adapt the supported build/
doctor assertions from its release CLI suite. The fixed-ratio archived-store test
belongs to the archived product, not a MAIN migration input. T8 will cover the new
MAIN dashboard's store projections. Additional CLI checks cover structured errors,
immutable candidates, byte-identical reports, worker isolation, symlinks, proposal
identity, and history access without a config. No live-provider checks are run.
