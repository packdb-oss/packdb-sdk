//! Minimal use of the PackDB SDK from outside its own workspace.
//!
//! ```text
//! cargo run -p packdb-client --example query --features sql
//! ```
//!
//! The point is not what it prints. It is that this file links only against
//! what a published consumer would get, so an accidental dependency on
//! something private fails here rather than in someone else's build.
//! Compiling is the assertion.

use std::time::Duration;

use packdb_client::{AuthConfig, ClientError, SqlClientConfig};

fn main() {
    let url = std::env::var("PACKDB_URL").unwrap_or_else(|_| "http://localhost:4000".to_string());

    // Credentials are supplied by the caller. This crate does not issue,
    // validate or resolve them -- that is deliberately outside the public
    // SDK, and the absence is what makes it publishable.
    let auth = match std::env::var("PACKDB_API_KEY") {
        Ok(key) if !key.is_empty() => AuthConfig::api_key(key),
        _ => AuthConfig::none(),
    };

    let config = SqlClientConfig {
        url: url.clone(),
        flight_url: None,
        auth: auth.into_provider(),
        timeout: Duration::from_secs(30),
        http_only: true,
    };

    println!("packdb-client example");
    println!("  server:   {}", config.url);
    println!("  timeout:  {:?}", config.timeout);
    println!("  http_only:{}", config.http_only);
    println!();

    // Exercise the error taxonomy without needing a server. These are the
    // same variants the packdb-query CLI derives its documented exit codes
    // from, which is why a consumer can classify a failure the same way a
    // shell script does.
    for error in [
        ClientError::SqlError("bad query".to_string()),
        ClientError::ConnectionError("unreachable".to_string()),
        ClientError::ProtocolError("bad frame".to_string()),
        ClientError::ConfigError("bad argument".to_string()),
    ] {
        println!(
            "  {:<28} -> exit code {}",
            format!("{error}"),
            error.exit_code()
        );
    }
}
