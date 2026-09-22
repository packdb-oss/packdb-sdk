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

//! Unified Prometheus API response format.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unified Prometheus API response envelope
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PrometheusApiResponse<T> {
    /// Response status: "success" or "error"
    pub status: String,

    /// Response data (present when status = "success")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,

    /// Error message (present when status = "error")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Error type classification (present when status = "error")
    #[serde(rename = "errorType", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
}

impl<T> PrometheusApiResponse<T> {
    /// Create a successful response with data
    pub fn success(data: T) -> Self {
        Self {
            status: "success".to_string(),
            data: Some(data),
            error: None,
            error_type: None,
        }
    }

    /// Create an error response
    pub fn error(error_type: &str, message: &str) -> PrometheusApiResponse<()> {
        PrometheusApiResponse {
            status: "error".to_string(),
            data: None,
            error: Some(message.to_string()),
            error_type: Some(error_type.to_string()),
        }
    }

    /// Check if response indicates success
    pub fn is_success(&self) -> bool {
        self.status == "success"
    }

    /// Check if response indicates an error
    pub fn is_error(&self) -> bool {
        self.status == "error"
    }
}

/// Inner `data` payload for Prometheus `/api/v1/query` and
/// `/api/v1/query_range` responses.
///
/// Shared wire-shape so the server's handlers, the alert
/// evaluator's PromQL client, and any future SDK consumer all
/// deserialize into the same type. Previously duplicated in
/// `src/servers/prometheus/query_api/handler.rs` and
/// `crates/packdb-alerting/src/query_client/promql.rs`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryData {
    /// Prometheus `resultType`, which selects how each row in `result` is
    /// shaped: `"vector"` and `"scalar"` populate [`QueryResult::value`],
    /// `"matrix"` populates [`QueryResult::values`].
    ///
    /// Carried as a `String` rather than an enum so an unrecognised result
    /// type round-trips instead of failing deserialization.
    #[serde(rename = "resultType")]
    pub result_type: String,
    /// The result rows. An absent field deserializes as empty, which is how a
    /// query that matched no series arrives.
    #[serde(default)]
    pub result: Vec<QueryResult>,
}

/// Individual result row from a Prometheus-compatible query.
///
/// Carries both the instant-vector (`value`) and range-vector
/// (`values`) shapes — exactly one is populated per row depending
/// on the `resultType`. Both are `Option` so the struct round-trips
/// either shape faithfully; callers pattern-match on `result_type`
/// to pick the right field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueryResult {
    /// Label set identifying the series this row belongs to.
    ///
    /// Absent deserializes as empty, which is the shape a scalar result
    /// takes — it has a value but no series.
    #[serde(default)]
    pub metric: HashMap<String, String>,
    /// Instant-query shape: `[unix_seconds, "value_as_string"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<(f64, String)>,
    /// Range-query shape: `[[unix_seconds, "value_as_string"], ...]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<(f64, String)>>,
}

/// Common error type constants for Prometheus API responses
pub mod error_types {
    /// Invalid query syntax or request parameters
    pub const BAD_DATA: &str = "bad_data";
    /// Query exceeded timeout threshold
    pub const TIMEOUT: &str = "timeout";
    /// Query execution failed (runtime error)
    pub const EXECUTION: &str = "execution";
    /// Server internal error
    pub const INTERNAL: &str = "internal";
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_success_response() {
        let response = PrometheusApiResponse::success(vec!["metric1"]);
        assert!(response.is_success());
        assert!(!response.is_error());
    }

    #[test]
    fn test_error_response() {
        let response = PrometheusApiResponse::<()>::error("bad_data", "Invalid syntax");
        assert!(response.is_error());
        assert!(!response.is_success());
    }

    #[test]
    fn test_serialization_success() {
        let response = PrometheusApiResponse::success(vec![1, 2, 3]);
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["status"], "success");
        assert!(json.get("error").is_none());
        assert!(json.get("errorType").is_none());
    }

    #[test]
    fn test_serialization_error() {
        let response = PrometheusApiResponse::<()>::error("timeout", "Query timeout");
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["status"], "error");
        assert_eq!(json["errorType"], "timeout");
    }
}
