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

//! Public-origin and mount-prefix validation for canonical endpoint URIs.
//!
//! Two protocol surfaces derive a canonical URI that doubles as an OAuth
//! resource audience. Both require the same thing of it — derive it in
//! exactly one place,
//! because a token bound to a URI the server computes differently is a token
//! the server rejects for the wrong reason. That rule is only worth as much as
//! the agreement between the surfaces, so the parsing lives here once rather
//! than once per surface.
//!
//! The validation is deliberately stricter than `http::Uri` alone. `Uri`
//! accepts URI-reference forms that are not valid HTTP origins, and its
//! `host()` accessor ignores bytes after a bracketed IPv6 literal — so a
//! loopback literal followed by attacker-controlled text would otherwise
//! inherit the literal's loopback classification and, with it, the permission
//! to serve without TLS.

use http::Uri;

/// A validated HTTP origin: scheme, host, and normalized `scheme://authority`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicOrigin {
    canonical: String,
    scheme: String,
    host: String,
}

impl PublicOrigin {
    /// Parse a complete HTTP origin rather than recognizing a URL-shaped
    /// prefix.
    ///
    /// Accepts an empty path or the conventional single trailing slash, but
    /// never a real path, query, or fragment. Returns `None` for anything
    /// that is not a well-formed `scheme://host[:port]`.
    #[must_use]
    pub fn parse(origin: &str) -> Option<Self> {
        let uri = origin.parse::<Uri>().ok()?;
        let scheme = uri.scheme_str()?;
        if scheme != "http" && scheme != "https" {
            return None;
        }
        let authority = uri.authority()?;
        if uri.query().is_some() || !matches!(uri.path(), "" | "/") {
            return None;
        }
        let host = validate_authority(authority.as_str())?;
        Some(Self {
            canonical: format!("{scheme}://{authority}"),
            scheme: scheme.to_string(),
            host,
        })
    }

    /// The normalized `scheme://authority`, without a trailing slash.
    #[must_use]
    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    /// The URI scheme, `http` or `https`.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The host, with any brackets and port stripped.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Whether the host names the loopback interface.
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }

    /// Whether this origin may be served without TLS.
    ///
    /// `https` always may; plaintext only on loopback. A surface that serves
    /// sensitive data calls this at construction time, so a plaintext public
    /// deployment fails at startup instead of serving over cleartext.
    #[must_use]
    pub fn is_transport_secure(&self) -> bool {
        self.scheme == "https" || self.is_loopback()
    }

    /// Consume the origin, returning its normalized string.
    #[must_use]
    pub fn into_canonical(self) -> String {
        self.canonical
    }
}

/// Validate every byte of an authority and return its host.
///
/// See the module docs for why this does not trust `Uri::host`.
fn validate_authority(authority: &str) -> Option<String> {
    if authority.contains('@') {
        return None;
    }
    if let Some(after_open) = authority.strip_prefix('[') {
        let (literal, remainder) = after_open.split_once(']')?;
        literal.parse::<std::net::Ipv6Addr>().ok()?;
        validate_optional_port(remainder)?;
        return Some(literal.to_string());
    }
    if authority.contains(['[', ']']) {
        return None;
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if host.is_empty() || host.contains(':') || !valid_dns_or_ipv4_host(host) {
        return None;
    }
    if let Some(port) = port {
        validate_port(port)?;
    }
    Some(host.to_string())
}

fn validate_optional_port(remainder: &str) -> Option<()> {
    if remainder.is_empty() {
        return Some(());
    }
    validate_port(remainder.strip_prefix(':')?)
}

fn validate_port(port: &str) -> Option<()> {
    if port.is_empty() || port.parse::<u16>().is_err() {
        return None;
    }
    Some(())
}

fn valid_dns_or_ipv4_host(host: &str) -> bool {
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

/// Whether a parsed host names the loopback interface.
fn is_loopback_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

/// Whether a mount prefix is a literal, normalized path prefix suitable for
/// both a URI and an Axum route.
///
/// An empty prefix is valid. A non-empty one must start with `/`, must not end
/// with one, and may not contain an empty, dot, dot-dot, or non-literal
/// segment — so concatenating it cannot produce `//`, and a configured prefix
/// cannot smuggle traversal into an advertised audience.
#[must_use]
pub fn valid_path_prefix(prefix: &str) -> bool {
    if prefix.is_empty() {
        return true;
    }
    if !prefix.starts_with('/') || prefix.ends_with('/') {
        return false;
    }
    prefix.split('/').skip(1).all(|segment| {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && segment.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_https_origin_parses_and_normalizes() {
        let origin = PublicOrigin::parse("https://packdb.example").expect("valid");
        assert_eq!(origin.canonical(), "https://packdb.example");
        assert_eq!(origin.scheme(), "https");
        assert_eq!(origin.host(), "packdb.example");
        assert!(!origin.is_loopback());
        assert!(origin.is_transport_secure());
    }

    #[test]
    fn a_single_trailing_slash_is_normalized_away() {
        let origin = PublicOrigin::parse("https://packdb.example/").expect("valid");
        assert_eq!(origin.canonical(), "https://packdb.example");
        assert_eq!(
            origin.clone().into_canonical(),
            "https://packdb.example".to_string()
        );
    }

    #[test]
    fn a_port_is_preserved_in_the_canonical_form_but_not_the_host() {
        let origin = PublicOrigin::parse("http://localhost:4000").expect("valid");
        assert_eq!(origin.canonical(), "http://localhost:4000");
        assert_eq!(origin.host(), "localhost");
        assert!(origin.is_loopback());
        assert!(origin.is_transport_secure());
    }

    #[test]
    fn plaintext_off_loopback_is_not_transport_secure() {
        let origin = PublicOrigin::parse("http://packdb.example").expect("valid");
        assert!(!origin.is_loopback());
        assert!(!origin.is_transport_secure());
    }

    #[test]
    fn loopback_literals_are_recognized_in_both_families() {
        for origin in [
            "http://127.0.0.1:4000",
            "http://127.5.5.5",
            "http://[::1]:4000",
            "http://LOCALHOST:4000",
        ] {
            let parsed = PublicOrigin::parse(origin).expect(origin);
            assert!(parsed.is_loopback(), "{origin}");
            assert!(parsed.is_transport_secure(), "{origin}");
        }
    }

    #[test]
    fn malformed_authorities_cannot_masquerade_as_loopback() {
        // `Uri::host` stops at the closing bracket, so without byte-exact
        // authority validation each of these would classify as loopback and
        // earn the right to serve over cleartext.
        for origin in [
            "http://[::1]evil.example",
            "http://[::1]:notaport",
            "http://[::1",
            "http://[not-an-ipv6]",
            "http://user@127.0.0.1",
            "http://127.0.0.1:99999",
            "http://127.0.0.1:",
            "http://local]host",
        ] {
            assert!(
                PublicOrigin::parse(origin).is_none_or(|parsed| !parsed.is_loopback()),
                "{origin} classified as loopback"
            );
        }
    }

    #[test]
    fn non_origin_urls_are_refused() {
        for origin in [
            "not-a-url",
            "",
            "ftp://packdb.example",
            "packdb.example",
            "https://packdb.example/api",
            "https://packdb.example?a=b",
            "https://",
            "https://-packdb.example",
            "https://packdb-.example",
            "https://packdb..example",
            "https://pack db.example",
        ] {
            assert!(PublicOrigin::parse(origin).is_none(), "{origin} accepted");
        }
    }

    #[test]
    fn an_ipv4_authority_is_a_valid_host() {
        let origin = PublicOrigin::parse("https://203.0.113.10:8443").expect("valid");
        assert_eq!(origin.host(), "203.0.113.10");
        assert!(!origin.is_loopback());
    }

    #[test]
    fn prefixes_must_be_literal_and_normalized() {
        for prefix in ["", "/packdb", "/a/b", "/a-b_c.d~e"] {
            assert!(valid_path_prefix(prefix), "{prefix} rejected");
        }
        for prefix in [
            "packdb", "/packdb/", "/", "//a", "/a//b", "/.", "/..", "/a/../b", "/a b", "/a%2Fb",
        ] {
            assert!(!valid_path_prefix(prefix), "{prefix} accepted");
        }
    }
}
