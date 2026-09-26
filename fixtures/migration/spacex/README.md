# Frozen migration inputs

Exact bytes from STA commit `ad1897e86ee05ff2255bd5277518a3230ee2340f`.
See `provenance.json` for original paths, byte counts and SHA-256 values.
These captures are frozen observations; no live-provider request was made.

The raw capture JSON contains historical RPC-origin fields and is intentionally
excluded from Git under the no-endpoint rule. Import the exact bytes from a local
checkout of the pinned archive:

```sh
python3 scripts/import-migration-fixtures.py --sta /path/to/sta-archive
```

The importer reads only the five allowlisted capture/binding files, checks every
size and SHA-256 before writing, and refuses to replace different local bytes.
Missing captures fail tests; they are never treated as skips. No network is used.
