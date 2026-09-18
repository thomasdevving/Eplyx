# Deployment inputs

## `deploy/bundle/`

The CI bundle the hosted service serves from, copied into the image at
`/opt/eplyx/bundle`.

It has to be here at image build time. A managed host builds from the git
remote, so a bundle that exists only on your machine is not reachable, and
there is no HTTP endpoint that installs one — an upload that could replace the
corpus a project is measured against is exactly what a CI credential must not
be able to do.

Build one from validated records, verify it, and copy it in:

```bash
eplyx bundle build --corpus <dir> --baseline <baseline.so> \
  --dependencies <dir>/dependencies --target-size 10 --out deploy/bundle
eplyx bundle verify --bundle deploy/bundle
```

Being in the image does not make it active, and a bundle belongs to a project,
so the project comes first. After deploying, in a shell on the running service:

```bash
eplyx-server admin create-project --name "<name>" --program-id <program id>
eplyx-server admin register-bundle --project proj_… --path /opt/eplyx/bundle
eplyx-server admin activate-bundle --project proj_… --bundle bndl_…
eplyx-server admin create-token --project proj_… --label ci
```

Each command prints the id the next one needs, and `register-bundle` prints the
exact `activate-bundle` line to run. `--bundle` takes the registered bundle id
(`bndl_…`), not the bundle's sha256: the same bytes can be registered to more
than one project, and activation is per project.

Registering and activating are separate on purpose: activation changes what
every pull request is compared against, so a person chooses when that happens.
`register-bundle` verifies every hash against the bytes on disk and copies the
bundle onto the volume, so it survives the next image build.

`create-token` prints the project's API token once and never again — only a
hash is stored. That is the value CI sends as `Authorization: Bearer …`; it can
submit checks for its own project and nothing else.
