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

Part of the [PackDB SDK](https://github.com/packdb-oss/packdb-sdk) workspace.
Not published to crates.io; the manifest keeps `publish = false` until registry
publication is approved. Depend on this crate by path within a source checkout.

See the [HTTP query quickstart](../../README.md#quickstart-query-over-http) for a
pinned checkout, application dependencies, environment-based credentials, and a
first query that preserves and checks partial-result causes.

## Licence

Apache-2.0. See [`LICENSE`](../../LICENSE) at the workspace root.
