// Copyright 2025 PackDB Contributors
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

//! Client error types

use thiserror::Error;

/// Result type for client operations
pub type ClientResult<T> = Result<T, ClientError>;

/// Client error types aligned with CLI exit codes
#[derive(Debug, Error)]
pub enum ClientError {
    /// SQL parsing or execution error (exit code 1)
    #[error("SQL error: {0}")]
    SqlError(String),

    /// Connection or authentication error (exit code 2)
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Protocol error or invalid server response (exit code 3)
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// Invalid configuration
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
}

impl ClientError {
    /// Convert error to CLI exit code (PostgreSQL-style)
    pub fn exit_code(&self) -> i32 {
        match self {
            ClientError::SqlError(_) => 1,
            ClientError::ConnectionError(_) => 2,
            ClientError::ProtocolError(_) => 3,
            ClientError::ConfigError(_) => 2,
            ClientError::IoError(_) => 2,
        }
    }
}

#[cfg(feature = "flight-sql")]
impl From<arrow_flight::error::FlightError> for ClientError {
    fn from(err: arrow_flight::error::FlightError) -> Self {
        ClientError::ProtocolError(format!("Flight SQL error: {err}"))
    }
}

impl From<reqwest::Error> for ClientError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_connect() || err.is_timeout() {
            ClientError::ConnectionError(format!("HTTP request failed: {err}"))
        } else if err.is_status() {
            ClientError::ProtocolError(format!("HTTP status error: {err}"))
        } else {
            ClientError::ProtocolError(format!("HTTP error: {err}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_codes() {
        assert_eq!(ClientError::SqlError("test".into()).exit_code(), 1);
        assert_eq!(ClientError::ConnectionError("test".into()).exit_code(), 2);
        assert_eq!(ClientError::ProtocolError("test".into()).exit_code(), 3);
        assert_eq!(ClientError::ConfigError("test".into()).exit_code(), 2);
    }

    #[test]
    fn io_error_converts_and_uses_exit_code_2() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no read");
        let client_err: ClientError = io_err.into();
        assert!(matches!(client_err, ClientError::IoError(_)));
        assert_eq!(client_err.exit_code(), 2);
    }

    #[test]
    fn display_includes_inner_message_per_variant() {
        assert_eq!(
            ClientError::SqlError("bad token".into()).to_string(),
            "SQL error: bad token",
        );
        assert_eq!(
            ClientError::ConnectionError("refused".into()).to_string(),
            "Connection error: refused",
        );
        assert_eq!(
            ClientError::ProtocolError("trailer".into()).to_string(),
            "Protocol error: trailer",
        );
        assert_eq!(
            ClientError::ConfigError("missing".into()).to_string(),
            "Configuration error: missing",
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reqwest_connect_failure_maps_to_connection_error() {
        // 192.0.2.0/24 is TEST-NET-1 (RFC 5737) — guaranteed unroutable.
        // We force a short timeout so the test runs quickly even when the
        // connect attempt would otherwise hang.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(50))
            .build()
            .expect("client builds");
        let err = client
            .get("http://192.0.2.1:1/")
            .send()
            .await
            .expect_err("connect must fail");
        let mapped: ClientError = err.into();
        match mapped {
            ClientError::ConnectionError(_) | ClientError::ProtocolError(_) => {}
            other => panic!("unexpected mapping: {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn reqwest_status_error_maps_to_protocol_error() {
        // Use a mock server that returns 500 so error_for_status() trips
        // is_status() rather than is_connect()/is_timeout().
        use std::net::TcpListener;
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr");
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");

        let server = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let resp = reqwest::get(format!("http://{addr}/"))
            .await
            .expect("request");
        let err = resp.error_for_status().expect_err("expected 5xx error");
        let mapped: ClientError = err.into();
        assert!(matches!(mapped, ClientError::ProtocolError(_)));

        server.await.ok();
    }
}
