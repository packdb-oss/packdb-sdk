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

//! Clients for PackDB's PromQL, LogQL and TraceQL HTTP APIs.
//!
//! Each call returns the server's decoded JSON body unchanged, with the
//! completeness carrier from the response headers. Response shapes are those
//! of the Prometheus, Loki and Tempo APIs PackDB serves; this crate does not
//! define types for them.

use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::completeness::{QueryResponse, partial_causes_of};
use crate::credentials::AuthProvider;
use crate::error::{ClientError, ClientResult};

/// Configuration of a [`QueryApiClient`].
#[derive(Clone)]
pub struct QueryApiConfig {
    /// Server URL, for example `http://127.0.0.1:4000`.
    pub url: String,
    /// Credential provider stamped on every request.
    pub auth: Arc<dyn AuthProvider>,
    /// Request timeout.
    pub timeout: Duration,
}

impl std::fmt::Debug for QueryApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryApiConfig")
            .field("url", &self.url)
            .field("auth", &"<AuthProvider>")
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Order of log lines in a LogQL log query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Oldest first.
    Forward,
    /// Newest first.
    Backward,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Forward => "forward",
            Direction::Backward => "backward",
        }
    }
}

/// Limit and order of a LogQL log query's lines.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LogOptions {
    /// Maximum number of lines; the server's default when absent.
    pub limit: Option<u32>,
    /// Line order; the server's default (newest first) when absent.
    pub direction: Option<Direction>,
}

impl LogOptions {
    fn push(self, params: &mut Vec<(&'static str, String)>) {
        push(params, "limit", self.limit.map(|l| l.to_string()));
        push(
            params,
            "direction",
            self.direction.map(|d| d.as_str().to_string()),
        );
    }
}

/// Which API answered, for classifying its failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Api<'a> {
    Prometheus,
    Loki,
    TempoSearch,
    /// A trace lookup, with the ID as requested.
    TempoTrace(&'a str),
}

/// A client for the PromQL, LogQL and TraceQL query APIs.
#[derive(Clone, Debug)]
pub struct QueryApiClient {
    config: QueryApiConfig,
    http: reqwest::Client,
}

impl QueryApiClient {
    /// Create a client.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::ConfigError`] if the HTTP client cannot be built.
    pub fn new(config: QueryApiConfig) -> ClientResult<Self> {
        let http = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| ClientError::ConfigError(format!("Failed to create HTTP client: {e}")))?;
        Ok(Self { config, http })
    }

    /// Evaluate a PromQL expression at one instant: `GET /api/v1/query`.
    ///
    /// `time` is Unix seconds or RFC 3339; the server's current time when absent.
    ///
    /// # Errors
    ///
    /// [`ClientError::QueryError`] when the server rejects the query, and the
    /// taxonomy of [`ClientError`] otherwise.
    pub async fn promql_instant(
        &self,
        query: &str,
        time: Option<&str>,
        headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        let mut params = vec![("query", query.to_string())];
        push(&mut params, "time", time.map(str::to_string));
        self.get(Api::Prometheus, "/api/v1/query", &params, headers)
            .await
    }

    /// Evaluate a PromQL expression over a range: `GET /api/v1/query_range`.
    ///
    /// `start` and `end` are Unix seconds or RFC 3339; `step` is a duration
    /// such as `15s`.
    ///
    /// # Errors
    ///
    /// As [`Self::promql_instant`].
    pub async fn promql_range(
        &self,
        query: &str,
        start: &str,
        end: &str,
        step: &str,
        headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        let params = vec![
            ("query", query.to_string()),
            ("start", start.to_string()),
            ("end", end.to_string()),
            ("step", step.to_string()),
        ];
        self.get(Api::Prometheus, "/api/v1/query_range", &params, headers)
            .await
    }

    /// Evaluate a LogQL metric expression at one instant:
    /// `GET /loki/api/v1/query`. The server rejects log (stream selector)
    /// queries here; use [`Self::logql_range`] for them.
    ///
    /// `time` is Unix nanoseconds, Unix seconds or RFC 3339.
    ///
    /// # Errors
    ///
    /// As [`Self::promql_instant`].
    pub async fn logql_instant(
        &self,
        query: &str,
        time: Option<&str>,
        options: LogOptions,
        headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        let mut params = vec![("query", query.to_string())];
        push(&mut params, "time", time.map(str::to_string));
        options.push(&mut params);
        self.get(Api::Loki, "/loki/api/v1/query", &params, headers)
            .await
    }

    /// Run a LogQL query over a range: `GET /loki/api/v1/query_range`.
    ///
    /// `start` and `end` are Unix nanoseconds, Unix seconds or RFC 3339;
    /// `step` is a duration such as `30s`, used by metric queries.
    ///
    /// # Errors
    ///
    /// As [`Self::promql_instant`].
    pub async fn logql_range(
        &self,
        query: &str,
        start: &str,
        end: &str,
        step: Option<&str>,
        options: LogOptions,
        headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        let mut params = vec![
            ("query", query.to_string()),
            ("start", start.to_string()),
            ("end", end.to_string()),
        ];
        push(&mut params, "step", step.map(str::to_string));
        options.push(&mut params);
        self.get(Api::Loki, "/loki/api/v1/query_range", &params, headers)
            .await
    }

    /// Search traces with TraceQL: `GET /api/search`.
    ///
    /// `start` and `end` are Unix seconds.
    ///
    /// # Errors
    ///
    /// [`ClientError::QueryError`] when the server rejects the query or does
    /// not support a construct in it, and the taxonomy of [`ClientError`]
    /// otherwise.
    pub async fn traceql_search(
        &self,
        query: &str,
        start: Option<u64>,
        end: Option<u64>,
        limit: Option<u32>,
        headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        let mut params = vec![("q", query.to_string())];
        push(&mut params, "start", start.map(|s| s.to_string()));
        push(&mut params, "end", end.map(|e| e.to_string()));
        push(&mut params, "limit", limit.map(|l| l.to_string()));
        self.get(Api::TempoSearch, "/api/search", &params, headers)
            .await
    }

    /// Read one trace by its ID: `GET /api/v2/traces/{id}`, as JSON.
    ///
    /// `start` and `end` are Unix seconds; when both are given, only that
    /// window is searched.
    ///
    /// # Errors
    ///
    /// [`ClientError::ConfigError`] if `trace_id` is not hexadecimal, before
    /// any request; [`ClientError::NotFound`] only when the server answers
    /// that it found no part of this trace (`404` with the body
    /// `{"error": "trace not found: <trace_id>"}` and no partial causes); and
    /// the taxonomy of [`ClientError`] otherwise. Any other `404`, such as a
    /// missing route or a proxy's page, is a [`ClientError::ProtocolError`].
    pub async fn trace(
        &self,
        trace_id: &str,
        start: Option<u64>,
        end: Option<u64>,
        headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        if trace_id.is_empty() || !trace_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ClientError::ConfigError(
                "a trace ID must be hexadecimal".into(),
            ));
        }
        let mut params = Vec::new();
        push(&mut params, "start", start.map(|s| s.to_string()));
        push(&mut params, "end", end.map(|e| e.to_string()));
        let path = format!("/api/v2/traces/{trace_id}");
        self.get(Api::TempoTrace(trace_id), &path, &params, headers)
            .await
    }

    async fn get(
        &self,
        api: Api<'_>,
        path: &str,
        params: &[(&str, String)],
        mut headers: HeaderMap,
    ) -> ClientResult<QueryResponse<Value>> {
        #[cfg(feature = "sql")]
        packdb_otel::inject_http_context(&opentelemetry::Context::current(), &mut headers);
        self.config
            .auth
            .apply(&mut headers)
            .await
            .map_err(|e| ClientError::ConfigError(format!("auth provider failed: {e}")))?;
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let url = format!("{}{path}", self.config.url.trim_end_matches('/'));
        let response = self
            .http
            .get(&url)
            .query(params)
            .headers(headers)
            .send()
            .await?;
        let status = response.status();
        let partial_causes = partial_causes_of(response.headers());
        if !status.is_success() {
            let body = crate::diagnostics::best_effort_response_body(response).await;
            return Err(classify(api, status, &body, !partial_causes.is_empty()));
        }
        let data = response.json::<Value>().await?;
        Ok(QueryResponse {
            data,
            partial_causes,
        })
    }
}

fn push(params: &mut Vec<(&'static str, String)>, name: &'static str, value: Option<String>) {
    if let Some(value) = value {
        params.push((name, value));
    }
}

/// The message a failure body carries: the `error` field of a Prometheus,
/// Loki or Tempo JSON error, the first message of a Jaeger error envelope, or
/// the body itself.
fn message(body: &str) -> String {
    if let Ok(json) = serde_json::from_str::<Value>(body) {
        if let Some(error) = json.get("error").and_then(Value::as_str) {
            return error.to_string();
        }
        if let Some(msg) = json
            .get("errors")
            .and_then(Value::as_array)
            .and_then(|errors| errors.first())
            .and_then(|first| first.get("msg"))
            .and_then(Value::as_str)
        {
            return msg.to_string();
        }
    }
    body.trim().to_string()
}

/// Whether a trace lookup's `404` is the server's answer that no part of the
/// trace exists: the exact error the server's trace handler writes for this
/// ID, on a response that reports no partial causes. A missing route, a
/// proxy's page, a message about another trace or a `404` that also reports
/// partial causes is not that answer.
fn is_trace_absence(trace_id: &str, body: &str, reported_partial: bool) -> bool {
    !reported_partial
        && serde_json::from_str::<Value>(body).is_ok_and(|json| {
            json.get("error").and_then(Value::as_str)
                == Some(format!("trace not found: {trace_id}").as_str())
        })
}

/// Classify a non-2xx response. `400` is a rejected query on every API, as is
/// Tempo search's `422` for an unsupported construct; `401` and `403` are told
/// apart; a trace lookup's `404` is [`ClientError::NotFound`] only when
/// [`is_trace_absence`] holds. Everything else, including execution failures,
/// budgets, timeouts and unrecognised `404`s, is a protocol or server failure.
fn classify(api: Api<'_>, status: StatusCode, body: &str, reported_partial: bool) -> ClientError {
    let message = message(body);
    match (status, api) {
        (StatusCode::BAD_REQUEST, _) => ClientError::QueryError(message),
        (StatusCode::UNPROCESSABLE_ENTITY, Api::TempoSearch) => ClientError::QueryError(message),
        (StatusCode::UNAUTHORIZED, _) => {
            ClientError::Unauthenticated(format!("Server returned status {status}: {message}"))
        }
        (StatusCode::FORBIDDEN, _) => {
            ClientError::PermissionDenied(format!("Server returned status {status}: {message}"))
        }
        (StatusCode::NOT_FOUND, Api::TempoTrace(trace_id))
            if is_trace_absence(trace_id, body, reported_partial) =>
        {
            ClientError::NotFound(message)
        }
        _ => ClientError::ProtocolError(format!("Server returned status {status}: {message}")),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn messages_come_from_each_error_shape() {
        assert_eq!(
            message(r#"{"status":"error","errorType":"bad_data","error":"parse error"}"#),
            "parse error"
        );
        assert_eq!(
            message(r#"{"data":null,"errors":[{"code":500,"msg":"store down"}]}"#),
            "store down"
        );
        assert_eq!(message(" plain text \n"), "plain text");
        assert_eq!(message(r#"{"other":1}"#), r#"{"other":1}"#);
    }

    #[test]
    fn statuses_are_classified_per_api() {
        let kind = |api, status| match classify(api, status, "m", false) {
            ClientError::QueryError(_) => "query",
            ClientError::Unauthenticated(_) => "unauthenticated",
            ClientError::PermissionDenied(_) => "permission",
            ClientError::NotFound(_) => "not_found",
            ClientError::ProtocolError(_) => "protocol",
            _ => "other",
        };
        let cases = [
            (Api::Prometheus, StatusCode::BAD_REQUEST, "query"),
            (
                Api::Prometheus,
                StatusCode::UNPROCESSABLE_ENTITY,
                "protocol",
            ),
            (Api::Loki, StatusCode::INTERNAL_SERVER_ERROR, "protocol"),
            (Api::TempoSearch, StatusCode::UNPROCESSABLE_ENTITY, "query"),
            (Api::TempoSearch, StatusCode::NOT_FOUND, "protocol"),
            (Api::TempoTrace("ab"), StatusCode::NOT_FOUND, "protocol"),
            (Api::Loki, StatusCode::UNAUTHORIZED, "unauthenticated"),
            (Api::TempoTrace("ab"), StatusCode::FORBIDDEN, "permission"),
            (Api::Prometheus, StatusCode::TOO_MANY_REQUESTS, "protocol"),
            (Api::Prometheus, StatusCode::SERVICE_UNAVAILABLE, "protocol"),
            (Api::Loki, StatusCode::GATEWAY_TIMEOUT, "protocol"),
        ];
        for (api, status, expected) in cases {
            assert_eq!(kind(api, status), expected, "{api:?} {status}");
        }
    }

    #[test]
    fn only_the_servers_answer_for_this_trace_is_not_found() {
        let not_found = |body: &str, partial: bool| {
            matches!(
                classify(
                    Api::TempoTrace("ab12"),
                    StatusCode::NOT_FOUND,
                    body,
                    partial
                ),
                ClientError::NotFound(_)
            )
        };
        assert!(not_found(r#"{"error":"trace not found: ab12"}"#, false));
        // A missing route, a proxy page, an empty body, another trace's
        // answer, a near miss and a contradictory partial 404 are not.
        for body in [
            "404 page not found",
            "<html><body><h1>404 Not Found</h1></body></html>",
            "",
            r#"{"error":"not found"}"#,
            r#"{"error":"trace not found: ffff"}"#,
            r#"{"error":"trace not found: ab12 "}"#,
            r#"{"errors":[{"code":404,"msg":"trace not found: ab12"}]}"#,
        ] {
            assert!(!not_found(body, false), "{body:?}");
        }
        assert!(!not_found(r#"{"error":"trace not found: ab12"}"#, true));
        // The same body on the search API is not a trace miss either.
        assert!(matches!(
            classify(
                Api::TempoSearch,
                StatusCode::NOT_FOUND,
                r#"{"error":"trace not found: ab12"}"#,
                false
            ),
            ClientError::ProtocolError(_)
        ));
    }

    #[test]
    fn directions_have_their_wire_names() {
        assert_eq!(Direction::Forward.as_str(), "forward");
        assert_eq!(Direction::Backward.as_str(), "backward");
    }

    #[test]
    fn the_config_debug_hides_the_credentials() {
        let config = QueryApiConfig {
            url: "http://h".into(),
            auth: crate::AuthConfig::api_key("secret".into()).into_provider(),
            timeout: Duration::from_secs(1),
        };
        let text = format!("{config:?}");
        assert!(!text.contains("secret"), "{text}");
    }
}
