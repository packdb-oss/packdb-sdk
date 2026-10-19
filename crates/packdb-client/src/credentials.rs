// Copyright 2025 PackDb Contributors
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

//! Credential-provider interface for outbound query transport.

use async_trait::async_trait;
use reqwest::header::HeaderMap;

/// Pluggable authenticator for outbound calls to PackDB query APIs.
///
/// Implementations stamp credentials onto a mutable `HeaderMap`
/// immediately before the request is sent. The seam is deliberately
/// synchronous + `&mut HeaderMap` (not `async` returning a clone) so
/// the happy path stays allocation-free and the trait stays object-
/// safe for `Arc<dyn AuthProvider>`.
#[async_trait]
pub trait AuthProvider: Send + Sync + std::fmt::Debug {
    /// Apply credential headers immediately before an outbound request.
    /// Calling per request lets an application refresh its caller-supplied
    /// credential without coupling the transport to an issuance mechanism.
    async fn apply(&self, headers: &mut HeaderMap) -> Result<(), AuthError>;
}

/// Error from an [`AuthProvider::apply`] call.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The configured provider cannot supply a credential right now.
    #[error("auth provider unavailable: {0}")]
    Unavailable(String),

    /// The provider cannot use its configuration without operator action.
    #[error("auth provider misconfigured: {0}")]
    Misconfigured(String),
}

/// Dev/no-auth provider. Stamps no credentials; callers must be
/// pointing at a PackDB deployment that also has auth disabled.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoAuth;

#[async_trait]
impl AuthProvider for NoAuth {
    async fn apply(&self, _headers: &mut HeaderMap) -> Result<(), AuthError> {
        Ok(())
    }
}

/// Static bearer credential supplied by the caller.
///
/// This provider only transports the supplied credential; issuance and validation
/// belong to the endpoint and its credential authority.
#[derive(Clone)]
pub struct BearerToken {
    /// Pre-formatted `Bearer <token>` value. Kept formatted so the
    /// hot path is a single header insert with no per-call string
    /// allocation.
    formatted: String,
}

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BearerToken")
            .field("formatted", &"[REDACTED]")
            .finish()
    }
}

impl BearerToken {
    /// Construct from a raw token. Returns an error if the token
    /// contains characters not valid in an HTTP header value —
    /// cheaper to reject at construction than on every request.
    pub fn new(token: &str) -> Result<Self, AuthError> {
        if token.is_empty() {
            return Err(AuthError::Misconfigured(
                "bearer token must not be empty".into(),
            ));
        }
        let formatted = format!("Bearer {token}");
        // Smoke-test that the formatted value is a valid header
        // value. Real validation happens at the reqwest layer, but
        // catching here lets a caller reject invalid configuration before
        // its first request.
        reqwest::header::HeaderValue::from_str(&formatted)
            .map_err(|e| AuthError::Misconfigured(format!("invalid bearer token value: {e}")))?;
        Ok(Self { formatted })
    }
}

#[async_trait]
impl AuthProvider for BearerToken {
    async fn apply(&self, headers: &mut HeaderMap) -> Result<(), AuthError> {
        // Safe: validated at construction.
        let value = reqwest::header::HeaderValue::from_str(&self.formatted)
            .map_err(|e| AuthError::Misconfigured(e.to_string()))?;
        headers.insert(reqwest::header::AUTHORIZATION, value);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_static_bearer_credentials() {
        let token = BearerToken::new("static-bearer-sentinel-593").expect("valid header");
        let text = format!("{token:?}");
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("static-bearer-sentinel-593"));
        assert!(!text.contains("Bearer static-bearer-sentinel-593"));
    }

    #[tokio::test]
    async fn noauth_leaves_headers_empty() {
        let mut h = HeaderMap::new();
        NoAuth.apply(&mut h).await.unwrap();
        assert!(h.is_empty());
    }

    #[tokio::test]
    async fn bearer_token_stamps_authorization() {
        let bearer = BearerToken::new("s3cret").unwrap();
        let mut h = HeaderMap::new();
        bearer.apply(&mut h).await.unwrap();
        assert_eq!(
            h.get(reqwest::header::AUTHORIZATION).unwrap(),
            "Bearer s3cret"
        );
    }

    #[test]
    fn bearer_token_rejects_empty_string() {
        let result = BearerToken::new("");
        assert!(matches!(result, Err(AuthError::Misconfigured(_))));
    }

    #[test]
    fn bearer_token_rejects_invalid_header_value() {
        // HTTP header values can't contain newlines — catches a
        // common "accidentally pasted a multi-line secret" failure
        // mode at boot.
        let result = BearerToken::new("line1\nline2");
        assert!(matches!(result, Err(AuthError::Misconfigured(_))));
    }
}
