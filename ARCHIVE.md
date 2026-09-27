# Stock Transition Assurance archive

The frozen Eplyx Stock Transition Assurance repository is
[thomasdevving/eplyx-stock-transition-assurance](https://github.com/thomasdevving/eplyx-stock-transition-assurance),
branch `Post-Hackathon`, commit
[`ad1897e86ee05ff2255bd5277518a3230ee2340f`](https://github.com/thomasdevving/eplyx-stock-transition-assurance/tree/ad1897e86ee05ff2255bd5277518a3230ee2340f).
It holds the original SPACEX hackathon evidence, source implementations and
fixed-ratio conversion product. The migration did not modify that repository.

MAIN is the maintained product implementation. It uses MAIN ChangeSpec identity,
upgradeable-loader execution, shared token/evidence layers, one CLI, one hosted
service and the existing registry/CAS. The fixed-ratio product, second resolver,
Node analysis services, STA landing page and blue presentation are not parallel
products in MAIN.

The [T0 records](docs/examples/phase-t0-stock-transition-integration/README.md)
retain observed values, assertion contracts, file hashes, loader experiments and
actual verification outcomes. [The T0 ADR](docs/phase-t0-stock-transition-integration.md)
and subsequent phase records explain intentional identity, numeric-encoding,
exit-code, storage and presentation changes. These records are append-only
references; changing a port must not rewrite them to make a test pass.

Required frozen inputs have per-directory provenance. Import scripts read explicit
allowlists from the pinned checkout and check both raw size and SHA-256. Some
captures include provider-origin fields and remain exact ignored files instead of
entering Git. Never read, copy or publish the archive's provider configuration.
Do not edit archived snapshots or reports, replace captured bytes with synthetic
ones, or treat historical issuer assumptions as current evidence.

The complete phase and verification report is
[Stock transition integration](docs/stock-transition-integration-report.md).
