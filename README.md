# PackDB SDK

Rust libraries for building clients and integrations against PackDB's public
single-node query surfaces. This workspace contains four independently versioned
packages:

| Package | Scope |
| --- | --- |
| `packdb-core` | Build identity, request IDs, public-origin validation, service outcomes and orderly shutdown helpers |
| `packdb-api` | Generic response envelopes and Prometheus/Loki response types |
| `packdb-otel` | Protocol-neutral W3C trace-context propagation for HTTP and gRPC |
| `packdb-client` | SQL over HTTP or Flight SQL, output formatting, caller-supplied credentials and client errors |

Control-plane access, credential issuance, tenant resolution, admission, placement,
routing and other commercial service contracts are outside this repository.

## Status and use

The packages are staged privately and are not published to crates.io. Every manifest
keeps `publish = false`; changing that is a separate reviewed release action. Until a
public release exists, this repository is source and candidate evidence rather than a
supported registry installation channel.

The crates use Rust 1.94.1 and edition 2024. `packdb-client` has no default features:
`sql` enables HTTP SQL support, and `flight-sql` includes `sql` plus Flight SQL.
See [COMPATIBILITY.md](COMPATIBILITY.md) for the supported contract.

## Development

```sh
OSS_CHECK_SCOPE=full scripts/oss-ci.sh checks
OSS_CHECK_SCOPE=full scripts/oss-ci.sh compatibility
OSS_CHECK_SCOPE=full scripts/oss-ci.sh coverage
OSS_CHECK_SCOPE=full scripts/oss-ci.sh security
```

See [CONTRIBUTING.md](CONTRIBUTING.md), [CODING_STANDARDS.md](CODING_STANDARDS.md)
and [AI_POLICY.md](AI_POLICY.md) before contributing.

## License

Apache License 2.0; see [LICENSE](LICENSE). Each package carries its own copy of the
license so its eventual crate archive is self-contained.
