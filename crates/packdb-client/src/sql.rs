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

//! SQL client abstraction for Flight SQL and HTTP backends

use crate::credentials::AuthProvider;
use crate::error::{ClientError, ClientResult};
use arrow::array::{ArrayRef, BooleanArray, Float64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use reqwest::header::HeaderMap;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "flight-sql")]
use arrow_flight::sql::client::FlightSqlServiceClient;
#[cfg(feature = "flight-sql")]
use futures::TryStreamExt;
#[cfg(feature = "flight-sql")]
use tonic::transport::{Channel, Endpoint};

/// Output format for query results
#[derive(Debug, Clone, Copy)]
pub enum OutputFormat {
    /// ASCII table format (human-readable)
    Table,

    /// JSON format (machine-readable)
    Json,

    /// CSV format (spreadsheet-compatible)
    Csv,
}

impl OutputFormat {
    /// Parse format string from CLI
    ///
    /// # Errors
    ///
    /// Returns `ClientError::ConfigError` if format string is not "table", "json", or "csv"
    pub fn parse(s: &str) -> ClientResult<Self> {
        match s.to_lowercase().as_str() {
            "table" => Ok(Self::Table),
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            _ => Err(ClientError::ConfigError(format!(
                "Invalid output format '{s}'. Valid options: table, json, csv"
            ))),
        }
    }
}

/// SQL client configuration
#[derive(Clone)]
pub struct SqlClientConfig {
    /// Server URL (HTTP endpoint)
    pub url: String,

    /// Flight SQL URL (gRPC endpoint)
    pub flight_url: Option<String>,

    /// Credential provider shared by HTTP and Flight SQL requests.
    ///
    /// Accepts [`crate::credentials::NoAuth`],
    /// [`crate::credentials::BearerToken`], or caller implementations of
    /// [`crate::credentials::AuthProvider`]. Static CLI credentials can be
    /// adapted with [`crate::AuthConfig::into_provider`].
    pub auth: Arc<dyn AuthProvider>,

    /// Request timeout
    pub timeout: Duration,

    /// Force HTTP mode (skip Flight SQL even if available)
    pub http_only: bool,
}

impl std::fmt::Debug for SqlClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Derive-free Debug so we don't require Debug on the
        // `dyn AuthProvider` behind `auth`. Never prints the
        // auth's contents — just that one is attached.
        f.debug_struct("SqlClientConfig")
            .field("url", &self.url)
            .field("flight_url", &self.flight_url)
            .field("auth", &"<AuthProvider>")
            .field("timeout", &self.timeout)
            .field("http_only", &self.http_only)
            .finish()
    }
}

/// SQL client for executing queries
#[derive(Clone)]
pub struct SqlClient {
    config: SqlClientConfig,
}

impl SqlClient {
    /// Create a new SQL client
    pub fn new(config: SqlClientConfig) -> Self {
        Self { config }
    }

    /// Execute a SQL query and return results formatted as a
    /// human-readable string per [`OutputFormat`].
    ///
    /// Used by the CLI. Server-to-server callers should prefer
    /// [`Self::execute_query_rows`] so they can inspect rows as
    /// structured JSON (and cross-transport: the same shape comes
    /// back from the HTTP and Flight SQL paths).
    ///
    /// # Errors
    ///
    /// Returns `ClientError::ConnectionError` if server is unreachable
    /// Returns `ClientError::ProtocolError` if server returns invalid response
    /// Returns `ClientError::SqlError` if query execution fails
    pub async fn execute_query(&self, sql: &str, format: OutputFormat) -> ClientResult<String> {
        let rows = self.execute_query_rows_inner(sql, HeaderMap::new()).await?;
        match format {
            OutputFormat::Json => format_json(&rows),
            OutputFormat::Table => format_table(&rows),
            OutputFormat::Csv => format_csv(&rows),
        }
    }

    /// Execute a SQL query and return structured rows, one
    /// `serde_json::Value` object per row.
    ///
    /// This is the server-to-server entry point: Flight SQL is
    /// preferred when `flight_url` is set and the `flight-sql`
    /// feature is compiled in, with fallback to HTTP on any
    /// Flight SQL failure. Both transports produce the same
    /// `Vec<serde_json::Value>` row shape — Flight SQL
    /// `RecordBatch`es are normalized through the Arrow JSON
    /// writer — so the evaluator's alert-semantics projection
    /// (`SqlSample`, per-column labels, threshold fields)
    /// works identically on either transport without a
    /// protocol-aware branch in the caller.
    ///
    /// Only trace context and the configured [`AuthProvider`]'s
    /// credentials are stamped. Callers that need to propagate
    /// tenancy (`X-Scope-OrgID`) or other per-request headers
    /// should use [`Self::execute_query_rows_with_headers`].
    ///
    /// # Errors
    ///
    /// Same taxonomy as [`Self::execute_query`].
    pub async fn execute_query_rows(&self, sql: &str) -> ClientResult<Vec<serde_json::Value>> {
        self.execute_query_rows_inner(sql, HeaderMap::new()).await
    }

    /// Variant of [`Self::execute_query_rows`] that lets the
    /// caller stamp additional request-scoped headers (e.g.
    /// `X-Scope-OrgID` for tenancy). These are forwarded as HTTP
    /// headers on the HTTP path and as gRPC metadata on the
    /// Flight SQL path, so the same caller code works for both
    /// transports.
    ///
    /// The configured [`AuthProvider`] is still stamped AFTER
    /// `extra_headers` so a caller accidentally passing an
    /// `Authorization` header doesn't quietly override the
    /// auth provider's credentials.
    ///
    /// # Errors
    ///
    /// Same taxonomy as [`Self::execute_query`].
    pub async fn execute_query_rows_with_headers(
        &self,
        sql: &str,
        extra_headers: HeaderMap,
    ) -> ClientResult<Vec<serde_json::Value>> {
        self.execute_query_rows_inner(sql, extra_headers).await
    }

    /// Shared entry point for all rows-returning paths. Flight
    /// SQL first, HTTP fallback — same policy as
    /// [`Self::execute_query`].
    async fn execute_query_rows_inner(
        &self,
        sql: &str,
        extra_headers: HeaderMap,
    ) -> ClientResult<Vec<serde_json::Value>> {
        #[cfg(feature = "flight-sql")]
        if !self.config.http_only
            && let Some(ref flight_url) = self.config.flight_url
        {
            match self
                .execute_flight_query(sql, flight_url, &extra_headers)
                .await
            {
                Ok(batches) => return batches_to_json_rows(&batches),
                Err(e) => {
                    tracing::debug!("Flight SQL failed, falling back to HTTP: {e}");
                }
            }
        }

        self.execute_http_query_rows(sql, extra_headers).await
    }

    /// Execute query via HTTP POST to /api/v1/sql and return the
    /// parsed JSON rows.
    async fn execute_http_query_rows(
        &self,
        sql: &str,
        extra_headers: HeaderMap,
    ) -> ClientResult<Vec<serde_json::Value>> {
        let client = reqwest::Client::builder()
            .timeout(self.config.timeout)
            .build()
            .map_err(|e| ClientError::ConfigError(format!("Failed to create HTTP client: {e}")))?;

        // Start from caller-provided headers, then layer trace context and
        // credentials on top. The shared transport leaves application policy
        // at the call boundary instead of duplicating wire-protocol code.
        let mut headers = extra_headers;
        // `http::HeaderMap` and `reqwest::header::HeaderMap` are the
        // same type; re-borrowing keeps this readable without
        // bouncing through a conversion.
        packdb_otel::inject_http_context(&opentelemetry::Context::current(), &mut headers);
        // Dynamic dispatch lets applications refresh caller-supplied
        // credentials per request. Provider failures remain configuration
        // errors at this transport boundary.
        self.config
            .auth
            .apply(&mut headers)
            .await
            .map_err(|e| ClientError::ConfigError(format!("auth provider failed: {e}")))?;

        let response = execute_sql_http(&client, &self.config.url, sql, headers).await?;
        Ok(response.rows)
    }

    /// Execute query via Flight SQL protocol
    #[cfg(feature = "flight-sql")]
    async fn execute_flight_query(
        &self,
        sql: &str,
        flight_url: &str,
        extra_headers: &HeaderMap,
    ) -> ClientResult<Vec<RecordBatch>> {
        let endpoint = Endpoint::from_shared(flight_url.to_string())
            .map_err(|e| ClientError::ConfigError(format!("Invalid flight URL: {e}")))?
            .timeout(self.config.timeout);

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| ClientError::ConnectionError(format!("Flight connect failed: {e}")))?;

        let mut client = FlightSqlServiceClient::new(channel);
        self.apply_flight_auth(&mut client).await?;

        // Forward caller-supplied headers as gRPC metadata. Using
        // `set_header` keeps Flight SQL in parity with HTTP without assigning
        // application-specific meaning to those headers.
        for (name, value) in extra_headers {
            let value_str = value.to_str().map_err(|e| {
                ClientError::ConfigError(format!(
                    "header `{name}` not ASCII-safe for Flight SQL: {e}"
                ))
            })?;
            client.set_header(name.as_str(), value_str);
        }

        // Inject W3C trace context as gRPC metadata headers so the Flight SQL
        // server can adopt this as the parent span. FlightSqlServiceClient
        // doesn't expose metadata_mut(), so we build a MetadataMap, let the
        // propagator write into it, then forward each entry via set_header().
        {
            let mut metadata = tonic::metadata::MetadataMap::new();
            packdb_otel::inject_grpc_context(&opentelemetry::Context::current(), &mut metadata);
            for key_ref in metadata.keys() {
                if let tonic::metadata::KeyRef::Ascii(key) = key_ref
                    && let Some(val) = metadata.get(key.as_str())
                    && let Ok(val_str) = val.to_str()
                {
                    client.set_header(key.as_str(), val_str);
                }
            }
        }

        let flight_info = client
            .execute(sql.to_string(), None)
            .await
            .map_err(|e| ClientError::ProtocolError(format!("Flight execute failed: {e}")))?;

        let ticket = flight_info
            .endpoint
            .first()
            .and_then(|ep| ep.ticket.clone())
            .ok_or_else(|| ClientError::ProtocolError("No endpoint in FlightInfo".into()))?;

        let stream = client
            .do_get(ticket)
            .await
            .map_err(|e| ClientError::ProtocolError(format!("Flight do_get failed: {e}")))?;

        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| ClientError::ProtocolError(format!("Flight stream error: {e}")))?;

        Ok(batches)
    }

    /// Apply authentication to the Flight SQL client via the
    /// shared [`AuthProvider`] seam.
    ///
    /// Asks the provider to stamp its headers on a scratch
    /// `HeaderMap`, then forwards every entry as a `set_header` on
    /// the `FlightSqlServiceClient`. This bridges the two
    /// interfaces (HTTP-style `HeaderMap` vs. Flight SQL's
    /// per-client metadata setter) without requiring the provider
    /// to know anything about tonic.
    ///
    /// Both the HTTP transport (above) and this gRPC transport now
    /// consume the exact same provider — which means short-lived
    /// service JWTs work on Flight SQL without any Flight-specific
    /// credential-minting code path.
    #[cfg(feature = "flight-sql")]
    async fn apply_flight_auth(
        &self,
        client: &mut FlightSqlServiceClient<Channel>,
    ) -> ClientResult<()> {
        let mut scratch = reqwest::header::HeaderMap::new();
        self.config
            .auth
            .apply(&mut scratch)
            .await
            .map_err(|e| ClientError::ConfigError(format!("auth provider failed: {e}")))?;
        for (name, value) in &scratch {
            let value_str = value.to_str().map_err(|e| {
                ClientError::ConfigError(format!(
                    "auth provider produced non-ASCII header `{name}`: {e}"
                ))
            })?;
            client.set_header(name.as_str(), value_str);
        }
        Ok(())
    }

    /// Check server connectivity
    ///
    /// # Errors
    ///
    /// Returns `ClientError::ConnectionError` if server is unreachable or returns non-2xx status
    pub async fn ping(&self) -> ClientResult<()> {
        let client = reqwest::Client::builder()
            .timeout(self.config.timeout)
            .build()
            .map_err(|e| ClientError::ConfigError(format!("Failed to create HTTP client: {e}")))?;

        let health_url = format!("{}/health", self.config.url.trim_end_matches('/'));
        // Keep this a transport-level connectivity probe. Applications that
        // need to validate credentials should execute an authenticated query.
        let response = client.get(&health_url).send().await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(ClientError::ConnectionError(format!(
                "Server returned status {}",
                response.status()
            )))
        }
    }
}

/// SQL query request payload
#[derive(Serialize)]
struct SqlQueryParams {
    q: String,
    format: Option<String>,
}

/// Raw structured response from `POST /api/v1/sql`.
///
/// This is the wire-shape returned by the shared transport
/// [`execute_sql_http`]. Consumers layer their own formatting
/// or sample-projection on top of `rows`. `schema` is captured
/// verbatim for callers that need column metadata; most call
/// sites ignore it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SqlJsonResponse {
    /// Column schema as returned by the server, e.g.
    /// `[{"name": "service", "data_type": "Utf8"}, ...]`.
    #[serde(default)]
    pub schema: serde_json::Value,
    /// One JSON object per row. Keys match the column names in
    /// `schema`; values are the server's JSON rendering of each
    /// cell (strings for text columns, numbers for numeric, etc.).
    #[serde(default)]
    pub rows: Vec<serde_json::Value>,
}

/// Execute a SQL query against a PackDB instance via HTTP POST to
/// `{base_url}/api/v1/sql` and return the raw parsed response.
///
/// Shared between the CLI [`SqlClient`] (formats `rows` to string)
/// and `packdb-alerting`'s evaluator (projects `rows` onto per-
/// sample label sets). Callers pass a pre-composed `headers` map, keeping
/// application policy outside the wire transport. Both the async
/// [`crate::credentials::AuthProvider`] interface and static
/// [`crate::AuthConfig`] feed into the same `HeaderMap` upstream.
///
/// # Errors
///
/// Returns [`ClientError::ConfigError`] if `base_url` cannot be
/// turned into a valid POST URL, [`ClientError::ProtocolError`]
/// for non-2xx responses (with the first ~4KB of the response
/// body captured for diagnostics), and the default
/// [`ClientError`] transport errors from `reqwest` for connect /
/// timeout / JSON-parse failures.
pub async fn execute_sql_http(
    http: &reqwest::Client,
    base_url: &str,
    sql: &str,
    headers: reqwest::header::HeaderMap,
) -> ClientResult<SqlJsonResponse> {
    // Tolerate callers that pass the full endpoint path
    // (`http://host:4000/api/v1/sql`) AND callers that pass a bare
    // origin. The evaluator and the CLI both historically accepted
    // either shape; centralizing the normalization here means
    // `SqlClient` / `SqlTarget` / any future caller stays
    // suffix-agnostic without repeating the trim logic.
    let base = base_url
        .trim_end_matches('/')
        .trim_end_matches("/api/v1/sql")
        .trim_end_matches('/');
    let url = format!("{base}/api/v1/sql");
    let response = http
        .post(&url)
        .headers(headers)
        .json(&SqlQueryParams {
            q: sql.to_string(),
            format: Some("json".to_string()),
        })
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = crate::diagnostics::best_effort_response_body(response).await;
        return Err(ClientError::ProtocolError(format!(
            "Server returned status {status}: {body}"
        )));
    }

    response
        .json::<SqlJsonResponse>()
        .await
        .map_err(ClientError::from)
}

/// Normalize a slice of Arrow `RecordBatch`es into the same
/// `Vec<serde_json::Value>` shape that the HTTP `/api/v1/sql`
/// endpoint returns, so the Flight SQL and HTTP transports
/// present an identical row shape to callers.
///
/// Uses Arrow's `ArrayWriter` (the same formatter the CLI's
/// `OutputFormat::Json` path uses) so numeric / boolean / string
/// cells land in JSON exactly as a downstream consumer sees them
/// from the HTTP path. An empty batch set yields an empty Vec
/// rather than `[]`-as-string — callers handle presence/absence
/// at the `Vec` level.
#[cfg(feature = "flight-sql")]
fn batches_to_json_rows(batches: &[RecordBatch]) -> ClientResult<Vec<serde_json::Value>> {
    if batches.is_empty() {
        return Ok(Vec::new());
    }
    let mut buf = Vec::new();
    {
        let mut writer = arrow::json::ArrayWriter::new(&mut buf);
        for batch in batches {
            writer
                .write(batch)
                .map_err(|e| ClientError::ProtocolError(format!("Flight JSON write: {e}")))?;
        }
        writer
            .finish()
            .map_err(|e| ClientError::ProtocolError(format!("Flight JSON finish: {e}")))?;
    }
    serde_json::from_slice::<Vec<serde_json::Value>>(&buf)
        .map_err(|e| ClientError::ProtocolError(format!("Flight JSON decode: {e}")))
}

/// Format query results as JSON
fn format_json(rows: &[serde_json::Value]) -> ClientResult<String> {
    serde_json::to_string_pretty(rows)
        .map_err(|e| ClientError::ProtocolError(format!("JSON formatting failed: {e}")))
}

/// Format query results as CSV using Arrow's CSV writer
fn format_csv(rows: &[serde_json::Value]) -> ClientResult<String> {
    if rows.is_empty() {
        return Ok(String::new());
    }

    let batch = json_to_record_batch(rows)?;

    let mut buffer = Vec::new();
    {
        let mut writer = arrow::csv::Writer::new(&mut buffer);
        writer
            .write(&batch)
            .map_err(|e| ClientError::ProtocolError(format!("CSV writing failed: {e}")))?;
    }

    String::from_utf8(buffer)
        .map_err(|e| ClientError::ProtocolError(format!("CSV encoding failed: {e}")))
}

/// Format query results as ASCII table using Arrow's pretty formatter
fn format_table(rows: &[serde_json::Value]) -> ClientResult<String> {
    if rows.is_empty() {
        return Ok(String::from("(empty result)"));
    }

    let batch = json_to_record_batch(rows)?;

    let formatted = arrow::util::pretty::pretty_format_batches(&[batch])
        .map_err(|e| ClientError::ProtocolError(format!("Table formatting failed: {e}")))?;

    Ok(formatted.to_string())
}

/// Convert JSON rows to Arrow RecordBatch
fn json_to_record_batch(rows: &[serde_json::Value]) -> ClientResult<RecordBatch> {
    if rows.is_empty() {
        return Err(ClientError::ProtocolError(
            "Cannot convert empty result to RecordBatch".into(),
        ));
    }

    let first_row = rows
        .first()
        .and_then(|r| r.as_object())
        .ok_or_else(|| ClientError::ProtocolError("Invalid row format".into()))?;

    let fields: Vec<Field> = first_row
        .iter()
        .map(|(key, value)| {
            let dtype = match value {
                serde_json::Value::Number(_) => DataType::Float64,
                serde_json::Value::String(_) => DataType::Utf8,
                serde_json::Value::Bool(_) => DataType::Boolean,
                _ => DataType::Utf8,
            };
            Field::new(key.clone(), dtype, true)
        })
        .collect();

    let schema = Arc::new(Schema::new(fields.clone()));
    let num_rows = rows.len();

    let arrays: Result<Vec<ArrayRef>, ClientError> = fields
        .iter()
        .map(|field| {
            let col_name = field.name();

            match field.data_type() {
                DataType::Float64 => {
                    let arr: Float64Array = (0..num_rows)
                        .map(|i| {
                            rows.get(i)
                                .and_then(|row| row.get(col_name))
                                .and_then(|v| v.as_f64())
                        })
                        .collect();
                    Ok(Arc::new(arr) as ArrayRef)
                }
                DataType::Boolean => {
                    let arr: BooleanArray = (0..num_rows)
                        .map(|i| {
                            rows.get(i)
                                .and_then(|row| row.get(col_name))
                                .and_then(|v| v.as_bool())
                        })
                        .collect();
                    Ok(Arc::new(arr) as ArrayRef)
                }
                DataType::Utf8 => {
                    let arr: StringArray = (0..num_rows)
                        .map(|i| {
                            rows.get(i)
                                .and_then(|row| row.get(col_name))
                                .and_then(|v| v.as_str())
                        })
                        .collect();
                    Ok(Arc::new(arr) as ArrayRef)
                }
                _ => Err(ClientError::ProtocolError(format!(
                    "Unsupported type: {}",
                    field.data_type()
                ))),
            }
        })
        .collect();

    let arrays = arrays?;

    RecordBatch::try_new(schema, arrays)
        .map_err(|e| ClientError::ProtocolError(format!("RecordBatch creation failed: {e}")))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_parsing() {
        assert!(matches!(
            OutputFormat::parse("table"),
            Ok(OutputFormat::Table)
        ));
        assert!(matches!(
            OutputFormat::parse("json"),
            Ok(OutputFormat::Json)
        ));
        assert!(matches!(OutputFormat::parse("csv"), Ok(OutputFormat::Csv)));
        assert!(matches!(
            OutputFormat::parse("TABLE"),
            Ok(OutputFormat::Table)
        ));
        assert!(OutputFormat::parse("invalid").is_err());
    }

    #[test]
    fn test_sql_client_creation() {
        let config = SqlClientConfig {
            url: "http://localhost:4000".into(),
            flight_url: None,
            auth: Arc::new(crate::credentials::NoAuth),
            timeout: Duration::from_secs(30),
            http_only: false,
        };
        let _client = SqlClient::new(config);
    }

    #[test]
    fn test_format_json() {
        let rows = vec![
            serde_json::json!({"name": "metric1", "value": 42}),
            serde_json::json!({"name": "metric2", "value": 43}),
        ];
        let result = format_json(&rows);
        assert!(result.is_ok());
        let output = result.expect("format_json failed");
        assert!(output.contains("metric1"));
        assert!(output.contains("42"));
    }

    #[test]
    fn test_format_json_empty() {
        let rows: Vec<serde_json::Value> = vec![];
        let result = format_json(&rows);
        assert!(result.is_ok());
        assert_eq!(result.expect("format_json failed"), "[]");
    }

    #[test]
    fn test_format_table() {
        let rows = vec![
            serde_json::json!({"name": "metric1", "value": 42.0}),
            serde_json::json!({"name": "metric2", "value": 43.0}),
        ];
        let result = format_table(&rows);
        assert!(result.is_ok());
        let output = result.expect("format_table failed");
        assert!(output.contains('+') || output.contains('|'));
        assert!(output.contains("metric1"));
    }

    #[test]
    fn test_format_table_empty() {
        let rows: Vec<serde_json::Value> = vec![];
        let result = format_table(&rows);
        assert!(result.is_ok());
        assert_eq!(result.expect("format_table failed"), "(empty result)");
    }

    #[test]
    fn test_json_to_record_batch() {
        let rows = vec![
            serde_json::json!({"name": "metric1", "value": 42.0, "active": true}),
            serde_json::json!({"name": "metric2", "value": 43.0, "active": false}),
        ];
        let result = json_to_record_batch(&rows);
        assert!(result.is_ok());
        let batch = result.expect("json_to_record_batch failed");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
    }

    #[test]
    fn test_json_to_record_batch_empty() {
        let rows: Vec<serde_json::Value> = vec![];
        let result = json_to_record_batch(&rows);
        assert!(result.is_err());
    }

    #[test]
    fn format_csv_empty_returns_empty_string() {
        let rows: Vec<serde_json::Value> = vec![];
        let out = format_csv(&rows).expect("format_csv ok");
        assert_eq!(out, "");
    }

    #[test]
    fn format_csv_renders_header_and_rows() {
        let rows = vec![
            serde_json::json!({"name": "metric1", "value": 42.0}),
            serde_json::json!({"name": "metric2", "value": 43.0}),
        ];
        let out = format_csv(&rows).expect("format_csv ok");
        assert!(out.contains("name"), "csv header missing: {out}");
        assert!(out.contains("metric1"), "csv data missing: {out}");
        assert!(out.contains("42"), "csv numeric value missing: {out}");
    }

    #[test]
    fn json_to_record_batch_rejects_non_object_first_row() {
        let rows = vec![serde_json::json!("not an object")];
        let err = json_to_record_batch(&rows).expect_err("must reject non-object");
        match err {
            ClientError::ProtocolError(msg) => {
                assert!(
                    msg.contains("Invalid row format"),
                    "unexpected message: {msg}",
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn json_to_record_batch_handles_mixed_value_types() {
        let rows = vec![
            serde_json::json!({"label": "a", "count": 7.0, "ok": true}),
            // Second row missing one column and using nulls — exercises the
            // null-collection paths in each typed branch.
            serde_json::json!({"label": serde_json::Value::Null, "count": serde_json::Value::Null, "ok": serde_json::Value::Null}),
        ];
        let batch = json_to_record_batch(&rows).expect("batch ok");
        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
    }

    // -----------------------------------------------------------------
    // HTTP client paths: ping + execute_http_query
    //
    // We spin up a tiny loopback server per test so we exercise the full
    // request / status / body decode logic without depending on any external
    // service. Each handler reads one HTTP request and writes a single fixed
    // response — enough for the synchronous client path.
    // -----------------------------------------------------------------

    // `AuthConfig` is no longer on `SqlClientConfig`; the tests use
    // `Arc<dyn AuthProvider>` directly via `crate::credentials::NoAuth`.
    use std::net::TcpListener as StdTcpListener;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Start a one-shot HTTP server bound to a random loopback port that
    /// returns the given raw HTTP response on the first connection. Returns
    /// the bound `SocketAddr` and the server `JoinHandle` for cleanup.
    async fn one_shot_server(
        response: impl Into<Vec<u8>>,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let response = response.into();
        let std_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind");
        std_listener.set_nonblocking(true).expect("nonblocking");
        let addr = std_listener.local_addr().expect("local addr");
        let listener = tokio::net::TcpListener::from_std(std_listener).expect("tokio listener");
        let handle = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(&response).await;
            }
        });
        (addr, handle)
    }

    fn client_for(addr: std::net::SocketAddr) -> SqlClient {
        SqlClient::new(SqlClientConfig {
            url: format!("http://{addr}"),
            flight_url: None,
            auth: Arc::new(crate::credentials::NoAuth),
            timeout: Duration::from_secs(2),
            http_only: true,
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ping_returns_ok_on_2xx_response() {
        let (addr, server) = one_shot_server(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        let client = client_for(addr);
        client.ping().await.expect("ping should succeed on 200");
        server.await.ok();
    }

    /// Variant of [`one_shot_server`] that also reports the bytes
    /// of the incoming HTTP request so tests can assert on
    /// headers / paths / method without a full HTTP parser.
    async fn one_shot_server_capture(
        response: impl Into<Vec<u8>>,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<Vec<u8>>) {
        let response = response.into();
        let std_listener = StdTcpListener::bind("127.0.0.1:0").expect("bind");
        std_listener.set_nonblocking(true).expect("nonblocking");
        let addr = std_listener.local_addr().expect("local addr");
        let listener = tokio::net::TcpListener::from_std(std_listener).expect("tokio listener");
        let handle = tokio::spawn(async move {
            let mut captured = Vec::new();
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                // Read a single window's worth; the HTTP request
                // head fits comfortably in 4 KiB for our tests.
                if let Ok(n) = sock.read(&mut buf).await {
                    captured.extend_from_slice(&buf[..n]);
                }
                let _ = sock.write_all(&response).await;
            }
            captured
        });
        (addr, handle)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ping_does_not_stamp_auth_headers_even_with_bearer_provider() {
        let jwt = crate::credentials::BearerToken::new("test-token").expect("valid header");

        let (addr, server) =
            one_shot_server_capture(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        let client = SqlClient::new(SqlClientConfig {
            url: format!("http://{addr}"),
            flight_url: None,
            auth: Arc::new(jwt),
            timeout: Duration::from_secs(2),
            http_only: true,
        });
        client.ping().await.expect("ping returns ok on 200");

        let received = server.await.expect("server task joined");
        let text = String::from_utf8_lossy(&received);
        assert!(
            text.to_ascii_lowercase().contains("get /health"),
            "ping must hit `/health`; request was:\n{text}"
        );
        assert!(
            !text.to_ascii_lowercase().contains("\nauthorization:"),
            "ping must NOT stamp `Authorization` header; request was:\n{text}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ping_maps_non_2xx_to_connection_error() {
        let (addr, server) =
            one_shot_server(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n").await;
        let client = client_for(addr);
        let err = client.ping().await.expect_err("ping should fail on 503");
        assert!(matches!(err, ClientError::ConnectionError(_)));
        server.await.ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_query_decodes_json_response_per_format() {
        let body = br#"{"rows":[{"name":"a","value":1.0},{"name":"b","value":2.0}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\r\n",
            len = body.len(),
        );
        let mut payload = response.into_bytes();
        payload.extend_from_slice(body);

        let (addr, server) = one_shot_server(payload).await;
        let client = client_for(addr);
        let out = client
            .execute_query("select 1", OutputFormat::Json)
            .await
            .expect("execute_query json ok");
        assert!(out.contains("\"name\""), "json output missing field: {out}");
        assert!(out.contains("\"a\""), "json output missing row: {out}");
        server.await.ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_query_propagates_server_errors_as_protocol_error() {
        let body = b"upstream blew up";
        let response = format!(
            "HTTP/1.1 500 Internal Server Error\r\n\
             Content-Length: {len}\r\n\r\n",
            len = body.len(),
        );
        let mut payload = response.into_bytes();
        payload.extend_from_slice(body);

        let (addr, server) = one_shot_server(payload).await;
        let client = client_for(addr);
        let err = client
            .execute_query("select 1", OutputFormat::Json)
            .await
            .expect_err("expected upstream error");
        match err {
            ClientError::ProtocolError(msg) => {
                assert!(msg.contains("500"), "expected 500 in message, got {msg}");
                assert!(
                    msg.contains("upstream blew up"),
                    "expected body in message, got {msg}",
                );
            }
            other => panic!("unexpected variant: {other:?}"),
        }
        server.await.ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_query_csv_and_table_formats_render_known_payload() {
        let body = br#"{"rows":[{"name":"row1","value":99.0}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\r\n",
            len = body.len(),
        );
        let mut payload = response.into_bytes();
        payload.extend_from_slice(body);

        // CSV
        let (addr_csv, server_csv) = one_shot_server(payload.clone()).await;
        let client = client_for(addr_csv);
        let csv_out = client
            .execute_query("select 1", OutputFormat::Csv)
            .await
            .expect("csv ok");
        assert!(csv_out.contains("row1"), "csv missing data: {csv_out}");
        server_csv.await.ok();

        // Table — second connection on a fresh listener
        let (addr_tbl, server_tbl) = one_shot_server(payload).await;
        let client = client_for(addr_tbl);
        let tbl_out = client
            .execute_query("select 1", OutputFormat::Table)
            .await
            .expect("table ok");
        assert!(tbl_out.contains("row1"), "table missing data: {tbl_out}");
        assert!(
            tbl_out.contains('+') || tbl_out.contains('|'),
            "table missing border chars: {tbl_out}",
        );
        server_tbl.await.ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_query_rows_returns_parsed_row_vec() {
        // Server-to-server callers (evaluator, future service
        // workers) depend on `execute_query_rows` returning the
        // raw JSON rows so they can layer their own projection
        // (label sets, threshold fields) without re-parsing a
        // pretty-printed string. Lock in the shape.
        let body = br#"{"rows":[{"service":"api","count":7},{"service":"web","count":12}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\r\n",
            len = body.len(),
        );
        let mut payload = response.into_bytes();
        payload.extend_from_slice(body);

        let (addr, server) = one_shot_server(payload).await;
        let client = client_for(addr);
        let rows = client
            .execute_query_rows("select service, count from metrics")
            .await
            .expect("execute_query_rows ok");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["service"], serde_json::json!("api"));
        assert_eq!(rows[0]["count"], serde_json::json!(7));
        assert_eq!(rows[1]["service"], serde_json::json!("web"));
        server.await.ok();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execute_query_rows_with_headers_forwards_extra_headers() {
        let body = br#"{"rows":[]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {len}\r\n\r\n",
            len = body.len(),
        );
        let mut payload = response.into_bytes();
        payload.extend_from_slice(body);

        let (addr, server) = one_shot_server_capture(payload).await;
        let client = client_for(addr);
        let mut extra = HeaderMap::new();
        extra.insert(
            reqwest::header::HeaderName::from_static("x-client-request-id"),
            reqwest::header::HeaderValue::from_static("request-123"),
        );
        let rows = client
            .execute_query_rows_with_headers("select 1", extra)
            .await
            .expect("execute_query_rows_with_headers ok");
        assert!(rows.is_empty());

        let received = server.await.expect("server task joined");
        let text = String::from_utf8_lossy(&received);
        assert!(
            text.to_ascii_lowercase()
                .contains("x-client-request-id: request-123"),
            "caller header must propagate; request was:\n{text}"
        );
    }
}
