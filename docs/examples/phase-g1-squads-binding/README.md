# Phase G1 fixtures — Squads V4 program-upgrade binding

Engine output from the real verifier (`governance::verify_squads_upgrade`)
reading a **simulated** Squads V4 + loader-v3 world
(`governance::simulated::World`). None of these accounts exist on mainnet, and
none of these files are evidence about a real proposal.

| File | What it is |
| --- | --- |
| `vault-transaction.json` | The stored `VaultTransaction` account bytes (base64), its canonical message hash and the delivery derived from it |
| `analysed-change-spec.json` | The unbound analysed spec: upgrade the program to the candidate bytes |
| `bound-change-spec.json` | The same spec bound to Squads transaction #42 (a different `change_spec_id`) |
| `binding-matched.json` | Healthy Active proposal; buffer holds the candidate; every authority is the vault |
| `binding-stale-artifact.json` | Same message; the buffer was rewritten after analysis |
| `binding-authority-mismatch.json` | Same message and bytes; the buffer's authority is not the vault |
| `binding-unsupported.json` | The message carries an upgrade *and* a System transfer |
| `binding-cancelled.json` | Healthy binding; proposal status Cancelled (status is never identity) |

Freshness check: `governance_fixtures_are_current` (engine unit
test) asserts every file byte-for-byte. Regenerate with
`make governance-fixtures`.

Independent reproduction: `pnpm verify:governance`
(`scripts/verify-squads-binding.mjs`, Node built-ins only) decodes the account
by hand, re-encodes each binding's JSON message view, and recomputes the
message hash, both change IDs and every binding ID without the Rust crate.

`frontend/governance.test.mjs` renders these bindings through the real page.
