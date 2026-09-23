# U13 research witness

This directory contains public finalized mainnet and historical account RPC receipts for the transaction identified in [the U13 report](../../phase-u13-drift-settle-pnl-witness.md). It is a research artifact, not an Eplyx schema-2 bundle or an observed transaction-boundary account proof.

Run from the repository root:

```powershell
node scripts/analyze-u13-drift-witness.cjs
```

The command reads only files in this directory and reproduces `analysis.json`. `checksums.sha256` records the SHA-256 of each retained file except itself. The block receipt is the complete account-mode finalized block; the ProgramData receipt is only a 64-byte header slice. Parent-slot and block-final account receipts carry their requested historical slots in `result.context.slot`.

Source snapshots are interface references from `velocity-exchange/protocol-v2` commit `73d22383e621040cd11b94375b6bd2f728f7537e`. The audit checks their Git blob IDs. They are not a build witness for the historical deployed program.
