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

//! API response envelope types shared between clients and servers.

use serde::{Deserialize, Serialize};

/// Generic success/error response envelope.
///
/// Shared serialization keeps callers and endpoints consistent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    /// Whether the call succeeded.
    ///
    /// Redundant with `error.is_some()` for envelopes built through
    /// [`ApiResponse::success`] and [`ApiResponse::error`], but it is the
    /// field the wire carries and the one clients branch on.
    pub success: bool,
    /// The payload, present when `success` is `true`.
    ///
    /// Omitted from the serialized JSON when absent rather than emitted as
    /// `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Human-readable failure description, present when `success` is `false`.
    ///
    /// Free text for operators, not a code to match on; endpoints that need a
    /// machine-readable classification use [`ApiError`] instead. Omitted from
    /// the serialized JSON when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<T> ApiResponse<T> {
    /// Create a success response with data.
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    /// Create an error response.
    pub fn error(message: impl Into<String>) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// Structured response envelope for trace and log APIs.
///
/// Supports both complete success, complete failure, and partial success scenarios.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StructuredResponse<T> {
    /// Query results (present on success or partial success)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,

    /// Collection of errors or warnings
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ApiError>,
}

/// API error with type-safe error code and human-readable message
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiError {
    /// Error classification (type-safe enum)
    pub code: ApiErrorCode,

    /// Human-readable error description
    pub message: String,
}

/// Fixed error code taxonomy for API responses
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    /// Query exceeded configured timeout threshold
    Timeout,
    /// Runtime error during query execution
    Execution,
    /// Invalid query parameters or request syntax
    BadRequest,
    /// Server internal error
    Internal,
    /// Requested resource does not exist
    NotFound,
    /// Authentication or authorization failure
    Unauthorized,
}

impl<T> StructuredResponse<T> {
    /// Create a successful response with data
    pub fn success(data: T) -> Self {
        Self {
            data: Some(data),
            errors: Vec::new(),
        }
    }

    /// Create an error response without data
    pub fn error(code: ApiErrorCode, message: impl Into<String>) -> StructuredResponse<()> {
        StructuredResponse {
            data: None,
            errors: vec![ApiError {
                code,
                message: message.into(),
            }],
        }
    }

    /// Create a partial success response with data and errors
    pub fn partial(data: T, errors: Vec<ApiError>) -> Self {
        Self {
            data: Some(data),
            errors,
        }
    }

    /// Check if response indicates complete success (no errors)
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }

    /// Check if response has any errors
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

impl ApiError {
    /// Create a new API error
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Create a timeout error
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Timeout, message)
    }

    /// Create an execution error
    pub fn execution(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Execution, message)
    }

    /// Create a bad request error
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::BadRequest, message)
    }

    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Internal, message)
    }

    /// Create a not found error
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::NotFound, message)
    }

    /// Create an unauthorized error
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(ApiErrorCode::Unauthorized, message)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_success_response() {
        let response = StructuredResponse::success(vec!["trace1", "trace2"]);
        assert_eq!(response.data, Some(vec!["trace1", "trace2"]));
        assert!(response.errors.is_empty());
        assert!(response.is_success());
        assert!(!response.has_errors());
    }

    #[test]
    fn test_error_response() {
        let response =
            StructuredResponse::<()>::error(ApiErrorCode::BadRequest, "Invalid trace ID");
        assert!(response.data.is_none());
        assert_eq!(response.errors.len(), 1);
        assert_eq!(response.errors[0].code, ApiErrorCode::BadRequest);
    }

    #[test]
    fn test_partial_response() {
        let errors = vec![ApiError::timeout("Cold tier timeout")];
        let response = StructuredResponse::partial(vec!["trace1"], errors);
        assert!(response.data.is_some());
        assert!(!response.is_success());
    }

    #[test]
    fn test_api_error_constructors() {
        assert_eq!(ApiError::timeout("t").code, ApiErrorCode::Timeout);
        assert_eq!(ApiError::execution("e").code, ApiErrorCode::Execution);
        assert_eq!(ApiError::bad_request("b").code, ApiErrorCode::BadRequest);
        assert_eq!(ApiError::internal("i").code, ApiErrorCode::Internal);
        assert_eq!(ApiError::not_found("n").code, ApiErrorCode::NotFound);
        assert_eq!(ApiError::unauthorized("u").code, ApiErrorCode::Unauthorized);
    }

    #[test]
    fn test_error_code_serialization() {
        let codes = vec![
            (ApiErrorCode::Timeout, "TIMEOUT"),
            (ApiErrorCode::Execution, "EXECUTION"),
            (ApiErrorCode::BadRequest, "BAD_REQUEST"),
            (ApiErrorCode::Internal, "INTERNAL"),
            (ApiErrorCode::NotFound, "NOT_FOUND"),
        ];

        for (code, expected) in codes {
            let error = ApiError::new(code, "test");
            let json = serde_json::to_value(&error).unwrap();
            assert_eq!(json["code"], expected);
        }
    }

    #[test]
    fn test_api_response_success() {
        let resp = ApiResponse::success(42);
        assert!(resp.success);
        assert_eq!(resp.data, Some(42));
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_api_response_error() {
        let resp = ApiResponse::<()>::error("something failed");
        assert!(!resp.success);
        assert!(resp.data.is_none());
        assert_eq!(resp.error, Some("something failed".to_string()));
    }
}
