# Provenance

This repository is the **canonical source** of `packdb-otel` and `packdb-client`. Changes are authored,
reviewed and released here.

It began as an export from the private PackDB monorepo. The last export was
generated from monorepo commit `f1b88c9e1aeb96bbc0824e75d41a8967f6d46ca8` and released as
`v0.2.1` (`30ed6e97765f6cdfe468a9f9d862d139ba6b558f`). The monorepo's export kit was then removed
(packdb/packdb#1731), so nothing regenerates this tree and nothing reverts an
edit made here.

`Cargo.lock` is a reviewed file: dependency versions change only through a
reviewed pull request -- Dependabot's or a maintainer's -- never at build time.

Release provenance is recorded per release. The `oss-candidate` workflow builds
from the exact commit to be tagged and publishes a `release-record.json` and
`SHA256SUMS` with every candidate; those files, not a repository-root
inventory, identify what a release contains. The export-time
`FILE_INVENTORY.txt` and `SHA256SUMS` were removed with the kit because they
described the last export, not the current tree.

This file keeps its name because the repository's policy check requires it.

## Deliberate decisions a reader should not mistake for oversights

- **`publish = false` is retained.** Clearing that flag is part of an
  authorised publication, not part of building a release.
- **`packdb-client` accepts caller-supplied credentials.** Issuing and
  validating credentials are responsibilities of the server, not these libraries.
- **Each member carries its own `LICENSE` copy.** `cargo package` includes
  only files inside a crate directory, so a root-only licence would ship
  with the repository but not with the published crate.
