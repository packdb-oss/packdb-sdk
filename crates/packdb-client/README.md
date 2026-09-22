# `packdb-client`

PackDB client errors, caller-supplied credential transport, and an optional
SQL client behind the `sql` feature.

Carries the typed error taxonomy the `packdb-query` CLI's exit codes are
derived from, so a library caller and a shell script classify the same
failure the same way.

With the `flight-sql` feature the SQL client prefers Arrow Flight SQL when
an endpoint is configured, and uses HTTP otherwise.

The crate accepts credentials but does not issue or validate them. Tenant
resolution, service JWTs, control-plane access, PromQL/LogQL administration,
placement, and routing live in private commercial crates.

## Status

Part of the [PackDB](https://packdb.io) workspace. **Not published to
crates.io**, and `publish = false` in its manifest while its open-source
repository is staged privately. Depend on it by path within this workspace.

## Licence

Apache-2.0. See [`LICENSE`](../../LICENSE) at the workspace root.
