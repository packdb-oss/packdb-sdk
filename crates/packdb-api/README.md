# `packdb-api`

Wire-format response types for PackDB's public query protocols.

The crate contains the generic API envelope plus Prometheus and Loki query
response structures. Control-plane administration, tenancy, placement,
quotas, evidence, investigations, reliability, RED configuration, and role
contracts remain private.

## Status

Part of the [PackDB](https://packdb.io) workspace. **Not published to
crates.io**, and `publish = false` in its manifest while its open-source
repository is staged privately. Depend on it by path within this workspace.

## Licence

Apache-2.0. See [`LICENSE`](../../LICENSE) at the workspace root.
