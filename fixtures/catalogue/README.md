# Saved first-party catalogue inputs

The two captures retain the pinned STA product-page bytes, retrieval timestamps
and identities. They are saved source assertions, not current issuer verification.
No website or provider is contacted by the tests. Payloads are ignored; import
with `python3 scripts/import-catalogue-fixtures.py --sta /path/to/sta-archive`.
Each original is size-checked and SHA-256 checked against `provenance.json`.
