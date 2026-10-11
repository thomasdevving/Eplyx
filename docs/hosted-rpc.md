# Hosted Solana RPC

Developers do not bring an RPC provider. The Eplyx service holds one Solana
provider, configured by the operator, and relays read-only JSON-RPC to it at
`POST /v1/rpc`. The `eplyx` CLI uses it automatically whenever no endpoint of
the developer's own is named. Access is gated by one password; it is meant to
become a paid subscription later, and for now it is a single shared secret.

## For developers

```sh
eplyx access            # prompts for the password, verifies it, saves it
eplyx access --status   # is it unlocked, and does the hosted RPC answer?
```

That is the whole setup. Afterwards every command that reads the chain
(`governance squads …`, `historical acquire`, `versions …`, `discover`,
`ingest`, `rpc inspect`, `lifecycle snapshot`, `path`/`observe` capture and
`migration analyse` with mainnet state) reads Solana mainnet through Eplyx.
Offline commands (`ci check`, `compare`, `bundle`, replay, reproduce) never read
the chain and are unaffected.

`eplyx access` verifies the password by asking the hosted RPC for the genesis
hash and only saves it when the answer is Solana mainnet. It is saved owner-only
(`0600`) as `access.json` in the Eplyx config directory, beside the cloud
credentials, and is only ever sent to the server it was saved for. Other ways to
supply it:

| Situation | Use |
| --- | --- |
| CI or a script | `EPLYX_ACCESS_PASSWORD` (nothing is saved) |
| Pipe it in | `printf %s "$PASSWORD" \| eplyx access --password-stdin` |
| Another Eplyx service | `eplyx access --server https://…` or `EPLYX_URL` |
| Remove it | `eplyx access --forget` |

Endpoint resolution, in order: an explicit flag (`--rpc-url`,
`--archive-rpc-url`, …), then the usual environment variables
(`SOLANA_RPC_URL`, `SOLANA_ARCHIVE_RPC_URL`, `SOLANA_BLOCK_RPC_URL`), then the
hosted RPC at `https://eplyx.dev/v1/rpc`. Bringing your own provider therefore
still works exactly as before and always wins.

The password travels as `Authorization: Bearer` over curl's standard input,
never in process arguments. It is not part of any cache key (caches are
namespaced by the relay URL alone), record, report or synced document; `eplyx
sync` refuses to upload text containing it.

## For the operator

Set two variables on the API service (Railway → service → Variables):

| Setting | Purpose |
| --- | --- |
| `EPLYX_RPC_URL` | Your Solana mainnet provider, API key included (e.g. a Helius, Triton or Alchemy URL). |
| `EPLYX_ACCESS_PASSWORD` | The password you hand out. 8-256 printable characters, no spaces, quotes or backslashes. |

The relay is on only when both are set; otherwise `/v1/rpc` answers `503` and
is never open. Startup logs `hosted Solana RPC (/v1/rpc): on` or `off`, never
either value. `EPLYX_RPC_URL` is also the default for hosted current-state
observation and governance verification when `EPLYX_OBSERVATION_RPC_URL` or
`EPLYX_GOVERNANCE_RPC_URL` are not set, so one provider serves the whole
service. On eplyx.dev the Cloudflare front already forwards `/v1/*` to the API,
so nothing else changes.

To change the password, change the variable and redeploy; developers run
`eplyx access` again. A wrong password is answered `401`.

### What the relay will and will not do

- **Read-only.** Only these methods are forwarded: `getAccountInfo`,
  `getBalance`, `getBlock`, `getBlockHeight`, `getBlockTime`, `getEpochInfo`,
  `getEpochSchedule`, `getFirstAvailableBlock`, `getGenesisHash`, `getHealth`,
  `getLatestBlockhash`, `getMinimumBalanceForRentExemption`,
  `getMultipleAccounts`, `getProgramAccounts`, `getSignatureStatuses`,
  `getSignaturesForAddress`, `getSlot`, `getTokenAccountBalance`,
  `getTokenAccountsByOwner`, `getTokenLargestAccounts`, `getTokenSupply`,
  `getTransaction`, `getVersion`. Anything else, including `sendTransaction`,
  `simulateTransaction` and `requestAirdrop`, is refused with JSON-RPC
  `-32601` and never reaches the provider. Batches are refused.
- **The provider stays private.** Its URL and key never leave the server. The
  provider's JSON-RPC error *code* is passed through (callers act on codes such
  as `-32016`); its message only when it names no URL. A provider rate limit
  arrives as HTTP `429`, which the CLI backs off and retries; any other provider
  failure is a generic `502`.
- **Bounded.** Requests up to 64 KiB, responses up to 64 MiB, 60 s per upstream
  call, at most 16 relayed requests at once.
- **Guessing is slow.** The password is compared by SHA-256 digest in constant
  time. Each wrong attempt is answered only after one second, one at a time
  across all callers, so guessing is limited to about one attempt per second
  without ever locking out someone who has the password. Choose a password that
  is not a dictionary word.
- **Analysis is unchanged.** Hosted analysis workers still start with an empty
  environment; no run reads the provider or the password.

### Limits worth knowing

- The relay can only answer what the provider can. Exact-slot account history
  (`versions resolve`, `historical acquire`) needs an archive-capable provider;
  a standard one fails loudly rather than returning today's state.
- Every developer shares the operator's provider quota. A public endpoint such
  as `api.mainnet-beta.solana.com` works for a demo but rate-limits quickly; use
  a paid provider for anything more.
- One password for everyone is a demo control, not per-user billing. Revoking
  one person means changing it for all.

## Demo checklist

1. Set `EPLYX_RPC_URL` and `EPLYX_ACCESS_PASSWORD` on the API service and
   redeploy. The log shows `hosted Solana RPC (/v1/rpc): on`.
2. On a fresh machine with only the `eplyx` binary and no `SOLANA_RPC_URL`:
   `eplyx access`, enter the password, see `Access granted`.
3. `eplyx rpc inspect` prints the mainnet genesis hash, read through Eplyx.
4. A real chain check, for example re-verifying the committed Squads witness:
   `eplyx governance squads verify --change-spec docs/examples/phase-g1-1-squads-mainnet/witness-primary/bound-change-spec.json`.
