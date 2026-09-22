# Compatibility

All packages currently use the 0.2.x version line. Before 1.0, a breaking Rust API
change increments the minor version. Patch releases preserve the documented public
API and serialized response shapes. Release notes identify intentional changes.

## Package contracts

- `packdb-otel`: W3C trace-context and baggage propagation through HTTP headers and
  gRPC metadata. Applications own exporter configuration and routing.
- `packdb-client`: no default features. `sql` enables SQL over HTTP;
  `flight-sql` includes `sql` and enables Flight SQL. `ClientError::exit_code`
  is also consumed by the query CLI and changes to it require CLI contract review.

Every release gate exercises no-default and all-feature builds. Compatibility gates
also test `packdb-client` separately with `sql` and `flight-sql` so feature
unification cannot hide a broken configuration.

## Exclusions

The SDK transports credentials supplied by its caller. It does not ship server
lifecycle/build primitives or duplicate Loki/Prometheus response DTOs, issue or
validate credentials, resolve organizations or tenants, call the control plane,
enforce quotas, or perform cell placement and routing. Those are not compatibility
promises of these packages.

The MSRV is Rust 1.94.1. Raising it is a documented release change. Support is
community best effort; this compatibility statement makes no quality, availability,
HA, fitness, or commercial guarantee.
