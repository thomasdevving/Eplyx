# Choose an Eplyx workflow

## First Eplyx analysis

Begin with the canonical [offline SPL Stake Pool example](../examples/stake-pool-upgrade/README.md):

```text
build the Eplyx CLI
→ build and hash-check the known regression candidate
→ verify deploy/bundle
→ run the pinned current program as the control
→ run the constructed regression candidate
→ compare why the control exits 0 and the regression exits 1
```

It uses ten retained historical observations, the pinned baseline and dependency
programs, the normal `eplyx ci check` engine, semantic coverage reporting and the
same candidate identity protected by clean-checkout CI. It is the shortest route
to executing a meaningful analysis, not a universal Stake Pool or Solana safety
claim. The [root quick start](../README.md#quick-start-one-real-historical-analysis-offline)
has the copyable command sequence; the example guide is authoritative for exact
results, provenance, limitations and the current macOS reproducibility boundary.

## After that

Choose the specialized workflow that matches the next question. These paths have
their own inputs and evidence boundaries; they are not prerequisites for the first
analysis.

| Your question | Start here | Required preparation |
| --- | --- | --- |
| How do I compare another program upgrade? | [Upgrade gate, expectations and bundles](phase-10-hosted-ci.md) or `/cli#upgrades` | Compiled candidate, validated bundle and supported replay coverage. |
| Can this migration account for its holders and funds? | [Token migration](token-migration.md) or `/cli#migration` | Migration terms, captured or synthetic world, compatible candidate and local project configuration. |
| What happens when declared lifecycle terms change? | [Lifecycle analysis](lifecycle.md) or `/cli#lifecycle` | Snapshot, scenario, evaluation time and any independent execution evidence. |
| Can an exact token account use this current-state path? | [Current-state analysis](current-state-analysis.md) or `/cli#paths` | Exact account/path terms, retained state and deployed code. |
| Should a protocol team use hosted analysis? | [Pilot onboarding](pilot-onboarding.md) | A hosted project, active bundle, project token and operator-configured service. |
| Does a Squads proposal match the analysed candidate? | [Squads binding](phase-g1-squads-governance-binding.md) and [deployment attestation](phase-g2-squads-deployment-attestation.md) | Supported loader-v3 upgrade, ChangeSpec, multisig/index and read-only RPC. |
| How do I acquire and prepare historical inputs? | [Historical state](phase-6-historical-state.md), [discovery](phase-5-mainnet-discovery.md) and [corpus preparation](phase-9-production-corpus.md) | Suitable archive access, supported transaction shapes and validated records. |
| How do I share a local run? | [Workspace sync](cloud.md) or `/cli#sync` | Saved analytical records, a configured workspace and project access. Sync does not execute analysis. |

## CLI installation

### Versioned release: macOS Apple Silicon

The only supported prebuilt target is `aarch64-apple-darwin`. Select a version
that actually exists on the [GitHub Releases
page](https://github.com/thomasdevving/Eplyx/releases), then download and verify
its archive and checksum:

```sh
VERSION="X.Y.Z" # replace with the published version
ASSET="eplyx-v${VERSION}-aarch64-apple-darwin.tar.gz"
BASE_URL="https://github.com/thomasdevving/Eplyx/releases/download/v${VERSION}"

curl -fLO "${BASE_URL}/${ASSET}"
curl -fLO "${BASE_URL}/${ASSET}.sha256"
shasum -a 256 -c "${ASSET}.sha256"
tar -xzf "${ASSET}"
./eplyx version --json
./eplyx --help
```

Only versions actually listed on the Releases page are available. If it lists no
matching release, use the source build below. A release archive contains the CLI
only: it does not contain a historical bundle, SBF fixture, provider
configuration, or `.eplyx` state. Run the extracted binary directly or move it
into a directory already on `PATH`; no installer, `sudo`, or automatic profile
edit is involved. Linux, Windows, Intel macOS, and universal macOS prebuilt
binaries are not currently supported.

The published SHA-256 authenticates the downloaded archive against the checksum
attached to that release. It does not assert reproducible CLI bytes across
independent builds.

### Source build

The source path requires Git, Rust stable and native build tools:

```sh
git clone --branch main https://github.com/thomasdevving/Eplyx.git
cd Eplyx
cargo build --locked --release -p eplyx-engine
export PATH="$PWD/target/release:$PATH"
eplyx --help
```

The PATH setting applies to that terminal. Examples in the website guide use
POSIX shell syntax. Candidate SBF builds have their own toolchain requirements;
installing or building the analysis CLI does not create candidate programs,
historical bundles, or captures.

## Three different kinds of work

**Preparation** acquires and validates evidence. Historical archive calls and
current observations are explicit provider operations. Inspecting command help
does not perform either operation.

**Analysis** consumes prepared inputs. The offline upgrade entry is
`eplyx ci check`; detailed retained-corpus investigations use `eplyx compare`.
Migration, lifecycle and current-path commands have their own input and assurance
contracts. A result's coverage is part of the result, even when its gate passes.

**Review** reads the report. A reviewer can inspect an existing web report without
installing the engine. The local dashboard reads saved records and does not start
an analysis. Public demos need no account; private hosted reports need access.

## Command references

- [Upgrade gate, expectations and bundles](phase-10-hosted-ci.md)
- [Historical corpus preparation](phase-9-production-corpus.md)
- [Token migration inputs, candidate and fixture requirements](token-migration.md)
- [Lifecycle policy inputs](lifecycle.md)
- [Current observations and exact paths](current-state-analysis.md)
- [Squads binding](phase-g1-squads-governance-binding.md) and
  [deployment attestation](phase-g2-squads-deployment-attestation.md)
- [Local result dashboard](dashboard.md) and [workspace sync](cloud.md)

Hosted upgrade automation uses [scripts/eplyx-submit.sh](../scripts/eplyx-submit.sh)
and the [external repository workflow example](../examples/github/eplyx-upgrade-impact.yml).
The [pilot onboarding checklist](pilot-onboarding.md) is the supported setup
path. The client checks readiness, uploads the candidate, waits for the run and
checks returned identities. This differs
from `eplyx sync`, which shares saved analytical documents without uploading code.
The older `examples/github/eplyx-check.sh` expects a synchronous response and is
not the client used by this guide.
