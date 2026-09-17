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

Being in the image does not make it active. After deploying, in a shell on the
running service:

```bash
eplyx-server admin install-bundle --path /opt/eplyx/bundle
eplyx-server admin activate-bundle --project <id> --bundle <sha256>
```

Installing and activating are separate on purpose: activation changes what
every pull request is compared against, so a person chooses when that happens.
