// Copyright 2026 PackDB Contributors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

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

use packdb_client::{AuthConfig, ClientError, SqlClientConfig, SqlTransport};

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
        transport: SqlTransport::Http,
    };

    println!("packdb-client example");
    println!("  server:   {}", config.url);
    println!("  timeout:  {:?}", config.timeout);
    println!("  transport:{:?}", config.transport);
    println!();

    // Exercise the error taxonomy without needing a server. Each variant
    // names what failed, so a caller can classify a failure without parsing
    // the message.
    for error in [
        ClientError::SqlError("bad query".to_string()),
        ClientError::Unauthenticated("401".to_string()),
        ClientError::PermissionDenied("403".to_string()),
        ClientError::ConnectionError("unreachable".to_string()),
        ClientError::ProtocolError("bad frame".to_string()),
        ClientError::ConfigError("bad argument".to_string()),
    ] {
        println!("  {error}");
    }
}
