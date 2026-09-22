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

//! Authentication configuration for client connections

use crate::error::{ClientError, ClientResult};

/// Authentication method for client connections
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// No authentication
    None,

    /// API key authentication (X-API-Key header)
    ApiKey(String),

    /// Bearer token authentication (Authorization: Bearer header)
    BearerToken(String),

    /// Basic authentication (Authorization: Basic header)
    BasicAuth {
        /// Username sent in the `Authorization: Basic` credential.
        username: String,
        /// Password sent in the `Authorization: Basic` credential.
        password: String,
    },
}

/// Authentication configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    /// Authentication method
    pub method: AuthMethod,
}

impl AuthConfig {
    /// Create auth config with no authentication
    pub fn none() -> Self {
        Self {
            method: AuthMethod::None,
        }
    }

    /// Create auth config with API key
    pub fn api_key(key: String) -> Self {
        Self {
            method: AuthMethod::ApiKey(key),
        }
    }

    /// Create auth config with bearer token
    pub fn bearer_token(token: String) -> Self {
        Self {
            method: AuthMethod::BearerToken(token),
        }
    }

    /// Create auth config with basic auth
    pub fn basic_auth(username: String, password: String) -> Self {
        Self {
            method: AuthMethod::BasicAuth { username, password },
        }
    }

    /// Parse basic auth string (format: "user:password")
    ///
    /// # Errors
    ///
    /// Returns `ClientError::ConfigError` if the auth string is not in "user:password" format
    pub fn parse_basic_auth(auth_str: &str) -> ClientResult<Self> {
        let parts: Vec<&str> = auth_str.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(ClientError::ConfigError(
                "Basic auth must be in format 'user:password'".into(),
            ));
        }
        Ok(Self::basic_auth(
            parts
                .first()
                .ok_or_else(|| ClientError::ConfigError("Missing username in basic auth".into()))?
                .to_string(),
            parts
                .get(1)
                .ok_or_else(|| ClientError::ConfigError("Missing password in basic auth".into()))?
                .to_string(),
        ))
    }

    /// Build from CLI options (priority: api_key > token > basic_auth > none)
    ///
    /// # Errors
    ///
    /// Returns `ClientError::ConfigError` if basic_auth is provided but malformed
    pub fn from_cli_options(
        api_key: Option<String>,
        token: Option<String>,
        basic_auth: Option<String>,
    ) -> ClientResult<Self> {
        if let Some(key) = api_key {
            Ok(Self::api_key(key))
        } else if let Some(tok) = token {
            Ok(Self::bearer_token(tok))
        } else if let Some(auth) = basic_auth {
            Self::parse_basic_auth(&auth)
        } else {
            Ok(Self::none())
        }
    }

    /// Add authentication headers to an HTTP request builder
    pub fn apply_to_http_request(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> reqwest::RequestBuilder {
        match &self.method {
            AuthMethod::None => builder,
            AuthMethod::ApiKey(key) => builder.header("X-API-Key", key),
            AuthMethod::BearerToken(token) => builder.bearer_auth(token),
            AuthMethod::BasicAuth { username, password } => {
                builder.basic_auth(username, Some(password))
            }
        }
    }

    /// Stamp authentication headers onto a `HeaderMap`.
    ///
    /// Parallel to [`Self::apply_to_http_request`] but works at the
    /// header level so callers that build requests via pre-composed
    /// `HeaderMap`s (e.g. shared transport functions like
    /// [`crate::sql::execute_sql_http`]) can stamp credentials
    /// without going through `RequestBuilder`.
    ///
    /// All [`AuthMethod`] variants are supported, including
    /// [`AuthMethod::BasicAuth`] — `reqwest::RequestBuilder::basic_auth`
    /// defers base64 encoding to `base64`, and this method does the
    /// same encoding explicitly so the two entry points produce
    /// byte-identical `Authorization` headers.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::ConfigError`] if the stored credential
    /// cannot be rendered as a valid HTTP header value (e.g. a
    /// bearer token containing a newline).
    pub fn apply_to_headers(&self, headers: &mut reqwest::header::HeaderMap) -> ClientResult<()> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        use reqwest::header::{AUTHORIZATION, HeaderName, HeaderValue};
        let invalid = |e: reqwest::header::InvalidHeaderValue| {
            ClientError::ConfigError(format!("invalid auth header value: {e}"))
        };
        match &self.method {
            AuthMethod::None => {}
            AuthMethod::ApiKey(key) => {
                let value = HeaderValue::from_str(key).map_err(invalid)?;
                headers.insert(HeaderName::from_static("x-api-key"), value);
            }
            AuthMethod::BearerToken(token) => {
                let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(invalid)?;
                headers.insert(AUTHORIZATION, value);
            }
            AuthMethod::BasicAuth { username, password } => {
                // Same wire format as `reqwest::RequestBuilder::basic_auth`
                // so CLI (--basic-auth) users keep byte-compatible behavior
                // across the RequestBuilder and HeaderMap paths.
                let encoded = STANDARD.encode(format!("{username}:{password}"));
                let value = HeaderValue::from_str(&format!("Basic {encoded}")).map_err(invalid)?;
                headers.insert(AUTHORIZATION, value);
            }
        }
        Ok(())
    }

    /// Adapt supplied static credentials into the transport's shared
    /// [`crate::credentials::AuthProvider`] interface.
    ///
    /// HTTP and Flight SQL use the same provider so credential precedence is
    /// consistent across transports.
    pub fn into_provider(self) -> std::sync::Arc<dyn crate::credentials::AuthProvider> {
        std::sync::Arc::new(AuthConfigProvider { config: self })
    }
}

/// `AuthProvider` adapter for the CLI-style static `AuthConfig`.
/// Kept private — callers construct via [`AuthConfig::into_provider`].
#[derive(Debug, Clone)]
struct AuthConfigProvider {
    config: AuthConfig,
}

#[async_trait::async_trait]
impl crate::credentials::AuthProvider for AuthConfigProvider {
    async fn apply(
        &self,
        headers: &mut reqwest::header::HeaderMap,
    ) -> Result<(), crate::credentials::AuthError> {
        self.config
            .apply_to_headers(headers)
            .map_err(|e| crate::credentials::AuthError::Misconfigured(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_methods() {
        let auth = AuthConfig::none();
        assert!(matches!(auth.method, AuthMethod::None));

        let auth = AuthConfig::api_key("test-key".into());
        assert!(matches!(auth.method, AuthMethod::ApiKey(_)));

        let auth = AuthConfig::bearer_token("test-token".into());
        assert!(matches!(auth.method, AuthMethod::BearerToken(_)));

        let auth = AuthConfig::basic_auth("user".into(), "pass".into());
        assert!(matches!(auth.method, AuthMethod::BasicAuth { .. }));
    }

    #[test]
    fn test_parse_basic_auth() {
        let auth = AuthConfig::parse_basic_auth("user:password");
        assert!(auth.is_ok());

        let auth = AuthConfig::parse_basic_auth("invalid");
        assert!(auth.is_err());

        let auth = AuthConfig::parse_basic_auth("user:pass:extra");
        assert!(auth.is_ok());
    }

    #[test]
    fn test_from_cli_options() {
        let auth = AuthConfig::from_cli_options(
            Some("key".into()),
            Some("token".into()),
            Some("user:pass".into()),
        );
        assert!(auth.is_ok());
        assert!(matches!(
            auth.ok().map(|a| a.method),
            Some(AuthMethod::ApiKey(_))
        ));

        let auth =
            AuthConfig::from_cli_options(None, Some("token".into()), Some("user:pass".into()));
        assert!(matches!(
            auth.ok().map(|a| a.method),
            Some(AuthMethod::BearerToken(_))
        ));

        let auth = AuthConfig::from_cli_options(None, None, Some("user:pass".into()));
        assert!(matches!(
            auth.ok().map(|a| a.method),
            Some(AuthMethod::BasicAuth { .. })
        ));

        let auth = AuthConfig::from_cli_options(None, None, None);
        assert!(matches!(
            auth.ok().map(|a| a.method),
            Some(AuthMethod::None)
        ));
    }

    /// Regression guard: `apply_to_headers` and
    /// `apply_to_http_request` must produce byte-identical
    /// `Authorization` headers for BasicAuth. The shared HTTP
    /// transport (`execute_sql_http`) routes through
    /// `apply_to_headers`; CLIs that still build a
    /// `RequestBuilder` directly route through
    /// `apply_to_http_request`. Drift between the two paths
    /// would break `packdb-query --basic-auth` silently.
    #[test]
    fn apply_to_headers_basic_auth_matches_request_builder_path() {
        use reqwest::header::AUTHORIZATION;
        let auth = AuthConfig::basic_auth("alice".into(), "hunter2".into());

        // Path A: HeaderMap.
        let mut headers = reqwest::header::HeaderMap::new();
        auth.apply_to_headers(&mut headers)
            .expect("BasicAuth must stamp via apply_to_headers");
        let via_headers = headers
            .get(AUTHORIZATION)
            .expect("authorization header present")
            .to_str()
            .expect("header value is ascii")
            .to_string();

        // Path B: RequestBuilder. Extract the header from the built
        // request without actually sending it.
        let client = reqwest::Client::new();
        let req = auth
            .apply_to_http_request(client.get("http://example/"))
            .build()
            .expect("request builds");
        let via_builder = req
            .headers()
            .get(AUTHORIZATION)
            .expect("authorization header present")
            .to_str()
            .expect("header value is ascii")
            .to_string();

        assert_eq!(
            via_headers, via_builder,
            "BasicAuth Authorization header must match between paths"
        );
        assert!(
            via_headers.starts_with("Basic "),
            "expected Basic auth prefix, got: {via_headers}"
        );
    }

    /// Parallel guard for BearerToken. Primarily to lock in the
    /// `Bearer <tok>` wire format — `reqwest`'s bearer_auth uses
    /// the same spelling, so drift would show up here first.
    #[test]
    fn apply_to_headers_bearer_matches_request_builder_path() {
        use reqwest::header::AUTHORIZATION;
        let auth = AuthConfig::bearer_token("s3cret".into());
        let mut headers = reqwest::header::HeaderMap::new();
        auth.apply_to_headers(&mut headers).unwrap();
        let via_headers = headers.get(AUTHORIZATION).unwrap().to_str().unwrap();

        let client = reqwest::Client::new();
        let req = auth
            .apply_to_http_request(client.get("http://example/"))
            .build()
            .unwrap();
        let via_builder = req.headers().get(AUTHORIZATION).unwrap().to_str().unwrap();

        assert_eq!(via_headers, via_builder);
        assert_eq!(via_headers, "Bearer s3cret");
    }
}
