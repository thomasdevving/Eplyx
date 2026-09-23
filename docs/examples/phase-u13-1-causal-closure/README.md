# U13.1 causal closure research artifact

This directory retains the conservative transaction dependency frontier for the frozen U13 Drift block, 20 new raw exact-slot Alchemy responses, and offline audits. It is not a replay observation or a product proof.

From the repository root, reproduce the deterministic analysis and checks:

```powershell
node scripts/analyze-u13-1-causal-closure.cjs docs/examples/phase-u13-1-causal-closure/closure.json
node scripts/verify-u13-1-frontiers.cjs
node scripts/test-u13-1-causal-closure.cjs
```

The archived acquisition is already complete. `scripts/acquire-u13-1-frontiers.cjs` refuses to reacquire unless explicitly passed `--force`. Its RPC URL is read only from `SOLANA_RPC_URL`; neither the URL nor credential is retained. `acquisition.json` records sanitized provider identity, exact requested and returned context slots, response files, lengths, and hashes. The raw responses and frozen U13 block are the evidence; planner outputs are derived conclusions.

`checksums.sha256` covers every retained file in this directory except itself, using paths relative to this directory.
