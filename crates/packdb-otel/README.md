# `packdb-otel`

Protocol-neutral W3C trace-context propagation for PackDB clients and
services. It injects and extracts `traceparent`, `tracestate`, and baggage
across HTTP and gRPC transports.

The crate does not choose exporters, credentials, tenants, or routing policy.
Applications install their preferred global OpenTelemetry propagator and own
their telemetry providers.

## Status

Part of the [PackDB](https://packdb.io) workspace. **Not published to
crates.io**, and `publish = false` in its manifest while the open-source
repository is staged privately.

## Licence

Apache-2.0. See [`LICENSE`](../../LICENSE) at the workspace root.
