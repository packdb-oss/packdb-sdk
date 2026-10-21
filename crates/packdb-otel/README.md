# `packdb-otel`

Protocol-neutral W3C trace-context propagation for PackDB clients and
services. It injects and extracts `traceparent`, `tracestate`, and baggage
across HTTP and gRPC transports.

The crate does not choose exporters, credentials, tenants, or routing policy.
Applications install their preferred global OpenTelemetry propagator and own
their telemetry providers.

## Status

Part of the [PackDB SDK](https://github.com/packdb-oss/packdb-sdk) workspace.
Not published to crates.io; the manifest keeps `publish = false` until registry
publication is approved. Depend on this crate by path within a source checkout.

## Licence

Apache-2.0. See [`LICENSE`](../../LICENSE) at the workspace root.
