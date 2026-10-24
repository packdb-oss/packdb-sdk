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

//! Query responses and the shared completeness carrier.

use reqwest::header::HeaderMap;

/// Carrier used by HTTP responses and Flight SQL initial metadata.
pub const PARTIAL_CAUSES_HEADER: &str = "x-packdb-partial-causes";

/// A successful query's data together with upstream incompleteness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryResponse<T> {
    /// The data returned by the answering transport.
    pub data: T,
    /// Cause tokens reported by the upstream. An empty list means complete.
    pub partial_causes: Vec<String>,
}

impl<T> QueryResponse<T> {
    /// Project the data without discarding its completeness.
    pub fn map<U>(self, project: impl FnOnce(T) -> U) -> QueryResponse<U> {
        QueryResponse {
            data: project(self.data),
            partial_causes: self.partial_causes,
        }
    }
}

/// Read every carrier value, preserving unknown tokens and failing closed.
///
/// Absence means no cause was reported; whether that establishes a complete
/// result depends on the answering API. A present value without usable tokens is partial
/// with `empty_partial_causes`; non-ASCII values use
/// `unparseable_partial_causes`. Duplicate tokens are returned only once.
pub fn partial_causes_of(headers: &HeaderMap) -> Vec<String> {
    parse_partial_causes(
        headers
            .get_all(PARTIAL_CAUSES_HEADER)
            .iter()
            .map(|value| value.to_str().ok()),
    )
}

pub(crate) fn parse_partial_causes<'a>(
    values: impl Iterator<Item = Option<&'a str>>,
) -> Vec<String> {
    let mut causes = Vec::new();
    for value in values {
        let tokens: Vec<&str> = match value {
            Some(text) => {
                let tokens: Vec<_> = text
                    .split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .collect();
                if tokens.is_empty() {
                    vec!["empty_partial_causes"]
                } else {
                    tokens
                }
            }
            None => vec!["unparseable_partial_causes"],
        };
        for token in tokens {
            if !causes.iter().any(|cause| cause == token) {
                causes.push(token.to_string());
            }
        }
    }
    causes
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn presence_never_becomes_complete() {
        assert!(partial_causes_of(&HeaderMap::new()).is_empty());
        for value in ["", "   ", ", ,,,"] {
            let mut headers = HeaderMap::new();
            headers.insert(PARTIAL_CAUSES_HEADER, HeaderValue::from_static(value));
            assert_eq!(partial_causes_of(&headers), ["empty_partial_causes"]);
        }
        let mut headers = HeaderMap::new();
        headers.append(
            PARTIAL_CAUSES_HEADER,
            HeaderValue::from_static(" tier_scan_failed, new_cause, tier_scan_failed "),
        );
        headers.append(
            PARTIAL_CAUSES_HEADER,
            HeaderValue::from_bytes(b"\xff").unwrap(),
        );
        headers.append(
            PARTIAL_CAUSES_HEADER,
            HeaderValue::from_static("cold_tier_unavailable"),
        );
        assert_eq!(
            partial_causes_of(&headers),
            [
                "tier_scan_failed",
                "new_cause",
                "unparseable_partial_causes",
                "cold_tier_unavailable"
            ]
        );
    }

    #[test]
    fn projection_preserves_causes() {
        let response = QueryResponse {
            data: vec![1, 2],
            partial_causes: vec!["tier_scan_failed".to_string()],
        }
        .map(|rows| rows.len());
        assert_eq!(response.data, 2);
        assert_eq!(response.partial_causes, ["tier_scan_failed"]);
    }
}
