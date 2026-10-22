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
automatically from repository visibility. Use a pinned source checkout as shown
below.

The crates use Rust 1.94.1 and edition 2024. `packdb-client` has no default features:
`sql` enables HTTP SQL support, and `flight-sql` includes `sql` plus Flight SQL.
See [COMPATIBILITY.md](COMPATIBILITY.md) for the supported contract.

## Quickstart: query over HTTP

You need Rust 1.94.1 or newer, an existing PackDB HTTP endpoint, and an API key
issued for that endpoint. This example does not start a server, provision an
account, or issue credentials. Use HTTPS for remote endpoints; the loopback URL
below is only for an operator-provided local deployment.

In a new directory, check out the released SDK and create a sibling application:

```sh
git clone --branch v0.2.4 --depth 1 https://github.com/packdb-oss/packdb-sdk.git
cargo new packdb-query-example
cd packdb-query-example
```

Add these entries under `[dependencies]` in the application's `Cargo.toml`:

```toml
packdb-client = { path = "../packdb-sdk/crates/packdb-client", features = ["sql"] }
tokio = { version = "1.49", features = ["macros", "rt-multi-thread"] }
```

The path must point to the `v0.2.4` checkout above. A path dependency does not pin
a Git revision; do not substitute an older or modified SDK checkout.

Use this as `src/main.rs`:

```rust
use std::time::Duration;

use packdb_client::{AuthConfig, SqlClient, SqlClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("PACKDB_URL")?;
    let api_key = std::env::var("PACKDB_API_KEY")?;
    if api_key.trim().is_empty() {
        return Err("PACKDB_API_KEY must not be empty".into());
    }

    let client = SqlClient::new(SqlClientConfig {
        url,
        flight_url: None,
        auth: AuthConfig::api_key(api_key).into_provider(),
        timeout: Duration::from_secs(30),
        http_only: true,
    });

    let response = client
        .execute_query_rows_with_completeness("SELECT 1 AS connected", Default::default())
        .await?;
    if !response.partial_causes.is_empty() {
        eprintln!("Incomplete query response: {:?}", response.partial_causes);
        return Err("query returned partial data; do not treat it as complete".into());
    }

    println!("{:#?}", response.data);
    Ok(())
}
```

Load `PACKDB_API_KEY` into the process environment through your secret manager.
Do not put credentials in source files, command-line arguments, or shell history.
Then set the endpoint and run:

```sh
export PACKDB_URL='http://127.0.0.1:4000'
cargo run
```

The result should contain `connected: 1`. A missing environment variable, a
transport/query failure, or a reported partial response exits unsuccessfully.
`partial_causes` preserves upstream completeness information; an empty list means
the answering transport reported no partial causes, not an independent audit of
the underlying telemetry. Avoid the rows-only methods when completeness matters.

The application gets its own `Cargo.lock`: the SDK checkout's lockfile does not
pin a consuming application's transitive dependencies. Review and commit the
application lockfile, audit it, and use `cargo run --locked` on subsequent runs.

For Flight SQL, enable `flight-sql` instead of `sql`, set `flight_url` to the
endpoint supplied by your operator, and set `http_only: false`. The client prefers
Flight SQL and falls back to HTTP on a Flight failure; completeness comes from
the transport that supplies the answer. This does not provision a Flight endpoint.

For context propagation rather than SQL, see
[`packdb-otel`](crates/packdb-otel/README.md). It is not an OTLP exporter.

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
