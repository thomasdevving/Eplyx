# Committed dependency binary

One file, committed because the CPI execution tests have to load a real token
program and must not depend on network access to do it.

| | |
|---|---|
| Program | `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA` (SPL Token) |
| Loader | upgradeable, immutable (no upgrade authority) |
| Deployed at slot | 419,472,000 |
| SHA-256 | `8190d3f7ceb6cb7a7a8d8924bff89f9f611e15ce1f806f2b6237f3311a98f697` |
| Read at slot | 429,878,777 |

These are the executable bytes the upgradeable loader hands the VM: ProgramData
minus its 45-byte header, trailing padding included. Reproduce the file with

```bash
eplyx versions resolve --program TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA \
  --slot 429878777 --archive-rpc-url <archive> --output <dir> \
  --out fixtures/dependencies/TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA.so
```

and check the hash. Nothing verifies it at load time except that hash: the
replay records that use it pin the same value, and `DependencyBundle::load`
refuses a file that does not match.

This is the deployment that was live when the Phase 8 transaction executed. It
is not necessarily the deployment live today - that is the whole point of
resolving dependencies at a slot - so do not refresh it without also updating
the records that pin it.
