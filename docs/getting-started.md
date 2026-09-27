# Choose an Eplyx workflow

Eplyx is in development. Choose a question first, then an interface. The public
site's **Try Eplyx** action opens `/start`, where each workflow lists its inputs,
results, limits and browser/CLI entry points. `/cli` contains copyable command
examples, prerequisites and links to the detailed input contracts.

| Your question | Start here | Required preparation |
| --- | --- | --- |
| What does an Eplyx result look like? | `/runs/demo` | None. This is a labelled saved demonstration, not a new analysis. |
| What changes if I deploy this program build? | `/analyse` for a hosted check; `/cli#upgrades` locally | Compiled candidate, validated bundle and supported replay coverage. The browser additionally needs access to a hosted project with an active bundle. |
| Can this migration account for its holders and funds? | `/cli#migration` | Migration terms, captured or synthetic world, compatible candidate and local project configuration. Templates alone are not runnable evidence. |
| What happens when these declared terms change? | `/cli#lifecycle`; Proposed scenario in a configured workspace | Snapshot, scenario, evaluation time and any independent execution evidence. Policy time does not refresh state. |
| Can an exact token account use this path? | A configured workspace for Transfer/bounded Meteora exit; `/cli#paths` for retained inputs and withdrawal | Exact account/path terms, retained state and deployed code. Hosted acquisition needs an operator-configured provider. |
| Does a Squads proposal match the analysed candidate? | `/cli#governance`, or the hosted API; review evidence in the run report | Supported single loader-v3 upgrade, ChangeSpec, multisig/index and read-only RPC. Deployment attestation additionally needs a sealed prior binding and exact candidate bytes. |
| Can this run on every pull request? | `/cli#ci` | Candidate built in the team's runner, a provisioned local bundle or hosted project, and version-controlled expected changes. |
| How do I prepare historical inputs? | `/cli#prepare` | Suitable archive access, supported transaction shapes and validated records. Discovery is not historical-state validation. |
| How do I share a local run? | `/cli#sync` | Saved migration/lifecycle/current-state records, a configured workspace and project access. Sync uploads analytical documents; it does not execute an analysis. |

## CLI installation

The current installation path is a source build with Git, Rust stable and native
build tools. There is no prebuilt release or one-line installer advertised.

```sh
git clone --branch main https://github.com/thomasdevving/Eplyx.git
cd Eplyx
cargo build --locked --release -p eplyx-engine
export PATH="$PWD/target/release:$PATH"
eplyx --help
```

The PATH setting applies to that terminal. Examples in the website guide use
POSIX shell syntax. Candidate SBF builds have their own toolchain requirements;
building the analysis CLI does not create candidate programs or captures.

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
and the [asynchronous workflow example](../.github/workflows/eplyx.yml). It uploads
the candidate, waits for the run and checks the returned identities. This differs
from `eplyx sync`, which shares saved analytical documents without uploading code.
The older `examples/github/eplyx-check.sh` expects a synchronous response and is
not the client used by this guide.
