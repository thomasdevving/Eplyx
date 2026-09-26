# T0 reference records

These records accompany [the T0 ADR](../../phase-t0-stock-transition-integration.md).
They freeze the STA reference at `ad1897e86ee05ff2255bd5277518a3230ee2340f`
(`Post-Hackathon`) before any feature code moves to MAIN. MAIN's baseline is
`5bfd81399eaff23301d14a05d6869a86a8e4720e`; the adapter-count correction is
`73a1c9d`.

| File | Purpose |
| --- | --- |
| `sta-migration-reference.json` | Observed migration values from STA, including exact invariant statuses, equations, stress verdicts, both policies and searches |
| `sta-test-contract.json` | Source test names, assertion expressions, helper assertions, original source/test hashes and static fixture references |
| `sta-fixture-inventory.json` | Original path/size/hash inventory for conservative test dependencies; no fixture bytes are copied here |
| `loader-experiment.json` | Scratch BPFLoader2 → upgradeable-loader comparison, including unchanged compute totals for the recorded cases |
| `verification.json` | Actual commands, exit codes, execution counts, Rust test results, retries, toolchain context, limitations and checks not run |
| `owner-review.json` | Approved architecture decisions, explicit stress semantics, per-mismatch source/test hashes and exact pnpm 11.24.0 check results |
| `cloud-copy-corrections.patch` | The three accepted copy-only assertion replacements, tested in scratch; STA remains untouched |

The migration values come from saved `migration_cases` outputs, built-binary
minimal analyses/search/reproduction and a diagnostic invocation of STA's
`migration::pipeline::evaluate_world` using the exact frozen-world inputs from
`migration_demo`. Both gates were evaluated through STA's
`conversion::package_gate::evaluate_migration`. A search value of `null` means
that case was not searched by this export. The complete search results for cases
A–H and the minimal deadline defect are retained. No MAIN “after” result exists
yet; those are acceptance checks for T3 and later.

Assertion snippets omit transport literals and machine path prefixes. Their
original unmodified sources remain authoritative at the recorded hashes.
`source_sha256` on a test hashes its original extracted test body; file `sha256`
hashes the complete original file. Assertions are expectations, not a claim that
every test passed. See the verification record for actual outcomes, opt-in live
checks not run, and the ADR for unresolved reference discrepancies.

Fixture references were collected statically from suites and shared helpers,
then expanded through existing relative JSON references. This intentionally
over-approximates the necessary closure and includes source-reading assertions.
It is not a dynamic file-access trace or a final fixture copy list. The inventory
currently describes 559 files totalling 442,476,524 bytes; these bytes have **not**
been added to MAIN. Each port must identify the files its tests actually consume,
preserve raw bytes, check size, and add per-directory provenance.

The verification record uses symbolic scratch paths so it contains no machine
paths or provider endpoints. Test logs remain scratch records; their digests are
included when available, not a promise that scratch storage is durable. Command
failures and subsequent successful retries remain distinct. Rust execution counts
include repeats and must not be read as unique test counts.

T0's saved records contain approximately 1.8 MB of documentation/JSON, zero SBF
binaries and zero copied capture bytes. Historical evidence in both repositories
is unchanged. Future phases should append reviewable comparisons, never overwrite
these reference values to make a port pass.

The owner-review addendum preserves the initial records and supersedes only the
three stale cloud assertion strings. All seven cloud tests pass with that patch
in scratch. MAIN's frontend/report/governance checks also pass using exact pinned
pnpm 11.24.0. The accepted minimal statement is **20/20 stress cases behaved as
specified**: 11 migrated and reconciled; nine rejected as expected with rollback
verified. Its separate population rehearsal migrated four of four attempted
holders. T1 is authorized; the actual migration port remains to be verified in T3.
