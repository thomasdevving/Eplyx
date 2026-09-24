# U17.2 partial historical evidence

`receipts.tar.gz` retains 125 files: the full target block, 18 exact parent
account receipts, nine exact target-slot writable-account receipts, all
ProgramData header and complete-account slices at both slots, five sysvar
receipts, 62 feature-account receipts, and their manifests and audits. Its
SHA-256 is
`dffad5526dbfa4240d2f6cb1628488825e970773b891da72304026defb7af684`.
Only response bodies are retained; no endpoint credential or request header is
inside the archive. All account reads used `getAccountInfo` with an exact
`slot` parameter. The provider is `solana-mainnet.g.alchemy.com`.

The runtime acquisition is incomplete: 287 of 349 backend-known feature
identities have no target-slot receipt. `runtime-partial.json` names every
missing identity. The public demo returned repeated HTTP 429 and no archive
credential was configured. This archive is research evidence, not a schema-2
corpus or replay bundle.

Offline verification:

```sh
mkdir -p /tmp/u17-2-phoenix-evidence
tar -xzf docs/examples/phase-u17-2-phoenix-evidence/receipts.tar.gz \
  -C /tmp/u17-2-phoenix-evidence
python3 scripts/analyze-u17-2-phoenix-evidence.py \
  --root /tmp/u17-2-phoenix-evidence
python3 scripts/analyze-u17-2-phoenix-closure.py \
  --root /tmp/u17-2-phoenix-evidence
```

The first command's stdout is byte-identical to `analysis.json` inside the
archive. The second reproduces `closure-audit.json` byte for byte. The
acquisition scripts can resume into an extracted directory if an exact-slot
archive credential is configured as `SOLANA_ARCHIVE_RPC_URL`; they never print
or retain the URL.
