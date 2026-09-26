# Current-path reference inputs

These append T7 dependencies to the unchanged [T6 fixture package](../lifecycle/README.md).
`provenance.json` pins three original STA files from commit
`ad1897e86ee05ff2255bd5277518a3230ee2340f`, totalling 7,955,227 bytes.
The withdrawal report is already a T6 input; this independent derivative also
encodes the decoded position timestamp as a decimal string. No T6 original or
accepted projection is overwritten.

From MAIN, import using a local archive path:

```sh
python3 scripts/project-lifecycle-reference.py --position-timestamps \
  --out fixtures/lifecycle/main-t7 --record fixtures/lifecycle/encoding-projection-t7.json
python3 scripts/import-path-fixtures.py --sta "$STA_ARCHIVE"
python3 scripts/project-path-reference.py
```

The importer checks sizes and SHA-256 before writing. The projection applies
MAIN's integer encoding and the already accepted T6 hash mapping, without executing
an evaluator or generating expected outcomes. `encoding-projection.json` records
both byte counts and digests. Tests compare complete position and withdrawal
outputs against these independently projected bytes.

Payloads are ignored. Only provenance, scripts and this documentation are committed.
The original source archive stays authoritative; missing imports fail the tests.
