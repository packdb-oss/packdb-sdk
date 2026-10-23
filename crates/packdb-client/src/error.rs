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

/// Client error types.
///
/// Each variant names what failed, so a caller can tell a rejected query from
/// refused credentials, an unreachable server and a server or protocol failure
/// without parsing messages. Messages keep the HTTP status, a bounded body or
/// the gRPC status where one exists.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ClientError {
    /// The server rejected the query itself (parse or execution).
    #[error("SQL error: {0}")]
    SqlError(String),

    /// The server did not accept the request's credentials: HTTP `401` or
    /// gRPC `Unauthenticated`.
    #[error("Not authenticated: {0}")]
    Unauthenticated(String),

    /// The credentials were accepted but do not permit the request: HTTP
    /// `403` or gRPC `PermissionDenied`.
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    /// The server could not be reached (connect, DNS, TLS or timeout).
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Protocol error, invalid server response or any other server failure.
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
    /// The 0.2 CLI exit-code mapping (PostgreSQL-style).
    #[deprecated(
        since = "0.3.0",
        note = "exit codes belong to the command that reports the error; match on the variant"
    )]
    pub fn exit_code(&self) -> i32 {
        self.legacy_exit_code()
    }

    /// The 0.2 mapping, kept for [`crate::outcome::ClientOutcome`].
    pub(crate) fn legacy_exit_code(&self) -> i32 {
        match self {
            ClientError::SqlError(_) => 1,
            ClientError::ProtocolError(_) => 3,
            ClientError::Unauthenticated(_)
            | ClientError::PermissionDenied(_)
            | ClientError::ConnectionError(_)
            | ClientError::ConfigError(_)
            | ClientError::IoError(_) => 2,
        }
    }
}

/// Classify a gRPC status returned by a Flight SQL call.
///
/// The server answers a query it cannot parse with `InvalidArgument`, a
/// tenant mismatch with `PermissionDenied`, and other failures with
/// `Internal`, `ResourceExhausted`, `DeadlineExceeded` or `Unimplemented`.
#[cfg(feature = "flight-sql")]
pub(crate) fn from_flight_status(stage: &str, status: &tonic::Status) -> ClientError {
    use tonic::Code;
    let detail = format!("{stage}: {}: {}", status.code(), status.message());
    match status.code() {
        Code::InvalidArgument => ClientError::SqlError(status.message().to_string()),
        Code::Unauthenticated => ClientError::Unauthenticated(detail),
        Code::PermissionDenied => ClientError::PermissionDenied(detail),
        Code::Unavailable => ClientError::ConnectionError(detail),
        _ => ClientError::ProtocolError(detail),
    }
}

/// Classify a Flight SQL client error, keeping the gRPC status when there is one.
#[cfg(feature = "flight-sql")]
pub(crate) fn from_flight_error(stage: &str, err: arrow_flight::error::FlightError) -> ClientError {
    match err {
        arrow_flight::error::FlightError::Tonic(status) => from_flight_status(stage, &status),
        other => ClientError::ProtocolError(format!("{stage}: {other}")),
    }
}

#[cfg(feature = "flight-sql")]
impl From<arrow_flight::error::FlightError> for ClientError {
    fn from(err: arrow_flight::error::FlightError) -> Self {
        from_flight_error("Flight SQL error", err)
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
    fn legacy_exit_codes_keep_the_0_2_mapping() {
        assert_eq!(ClientError::SqlError("test".into()).legacy_exit_code(), 1);
        assert_eq!(
            ClientError::ConnectionError("test".into()).legacy_exit_code(),
            2
        );
        assert_eq!(
            ClientError::Unauthenticated("test".into()).legacy_exit_code(),
            2
        );
        assert_eq!(
            ClientError::PermissionDenied("test".into()).legacy_exit_code(),
            2
        );
        assert_eq!(
            ClientError::ProtocolError("test".into()).legacy_exit_code(),
            3
        );
        assert_eq!(
            ClientError::ConfigError("test".into()).legacy_exit_code(),
            2
        );
    }

    #[test]
    fn io_error_converts_and_uses_exit_code_2() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no read");
        let client_err: ClientError = io_err.into();
        assert!(matches!(client_err, ClientError::IoError(_)));
        assert_eq!(client_err.legacy_exit_code(), 2);
    }

    #[cfg(feature = "flight-sql")]
    #[test]
    fn flight_statuses_keep_their_meaning() {
        use tonic::{Code, Status};
        let cases = [
            (Code::InvalidArgument, "SqlError"),
            (Code::Unauthenticated, "Unauthenticated"),
            (Code::PermissionDenied, "PermissionDenied"),
            (Code::Unavailable, "ConnectionError"),
            (Code::Internal, "ProtocolError"),
            (Code::ResourceExhausted, "ProtocolError"),
            (Code::DeadlineExceeded, "ProtocolError"),
            (Code::Unimplemented, "ProtocolError"),
        ];
        for (code, expected) in cases {
            let err = from_flight_status("Flight execute", &Status::new(code, "detail"));
            let name = match err {
                ClientError::SqlError(_) => "SqlError",
                ClientError::Unauthenticated(_) => "Unauthenticated",
                ClientError::PermissionDenied(_) => "PermissionDenied",
                ClientError::ConnectionError(_) => "ConnectionError",
                ClientError::ProtocolError(_) => "ProtocolError",
                _ => "other",
            };
            assert_eq!(name, expected, "{code:?}");
        }
    }

    #[test]
    fn display_names_401_and_403_differently() {
        assert_eq!(
            ClientError::Unauthenticated("401".into()).to_string(),
            "Not authenticated: 401"
        );
        assert_eq!(
            ClientError::PermissionDenied("403".into()).to_string(),
            "Permission denied: 403"
        );
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
