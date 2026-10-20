# PackDB SDK

Rust libraries for building clients and integrations against PackDB's public
single-node query surfaces. This workspace contains two independently versioned
packages:

| Package | Scope |
| --- | --- |
| `packdb-otel` | Protocol-neutral W3C trace-context propagation for HTTP and gRPC |
| `packdb-client` | SQL over HTTP or Flight SQL, output formatting, caller-supplied credentials and client errors |

Server lifecycle/build primitives, duplicate wire DTOs, control-plane access,
credential issuance, tenant resolution, admission, placement, routing and other
commercial service contracts are outside this repository.

## Status and use

Source releases are available from [GitHub Releases](https://github.com/packdb-oss/packdb-sdk/releases).
The packages are not published to crates.io. Every manifest keeps `publish = false`;
registry publication is a separate reviewed release action and does not follow
automatically from repository visibility. Use a source checkout for development.

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
