# T4 authority reference captures

The provenance manifest pins the exact source population bytes from STA at
`ad1897e86ee05ff2255bd5277518a3230ee2340f`. The six authority-resolution tests
consume these two captures plus the existing SPACEX population.

Raw captures remain outside Git under the absolute no-provider-URL rule. Import
with `python3 scripts/import-migration-fixtures.py --sta <pinned-STA-archive>`.
The importer checks all sizes and SHA-256 values before writing. Missing bytes
are test failures. These historical captures are never promoted into coherent
final-state execution inputs.
