# `packdb-core`

Dependency-light primitives shared by the public PackDB SDK: build identity,
request IDs, public-origin validation, service outcomes, and ordered shutdown.

The crate is the leaf of the SDK dependency graph. Tenant identity, admission,
storage accounting, topology, placement, migration, database helpers, service
JWTs, and the server error taxonomy live in private commercial crates.

## Status

Part of the [PackDB](https://packdb.io) workspace. **Not published to
crates.io**, and `publish = false` in its manifest while its open-source
repository is staged privately. Depend on it by path within this workspace.

## Licence

Apache-2.0. See [`LICENSE`](../../LICENSE) at the workspace root.
