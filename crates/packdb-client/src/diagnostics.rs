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

//! Shared diagnostic helpers for PackDB HTTP clients.
//!
//! Transport-level utilities that every PackDB client (the CLI
//! SQL client, the alert-evaluator's PromQL / LogQL / SQL
//! adapters, and future consumers) needs to format error paths
//! consistently. Keeping them in one place avoids silent drift
//! — the PromQL, LogQL, and SQL clients each used to carry their
//! own private copy of the truncator below, which meant a bugfix
//! to one didn't reach the others.

/// Read a response body best-effort for an operator-facing error
/// message, capping output at ~4 KiB and respecting UTF-8 codepoint
/// boundaries.
///
/// Streams the response chunk-by-chunk and stops as soon as enough
/// bytes are accumulated to render the diagnostic. A misconfigured
/// or malicious upstream that returns a multi-GB body would
/// otherwise force `response.text().await` to buffer the entire
/// payload into memory before truncation — enough to OOM the
/// evaluator process, turning a diagnostic-path bug into a
/// liveness issue. Capping at the byte-stream level keeps this
/// path bounded regardless of upstream behavior.
///
/// The naïve truncator (`&text[..MAX]`) panics when byte `MAX`
/// lands inside a multibyte codepoint — which happens in practice
/// when a server returns an emoji-laden error page longer than
/// the cap. The char-boundary-safe walk below keeps the
/// diagnostic path alive long enough to log the actual upstream
/// status + body.
///
/// If reading the body itself fails (e.g. connection dropped
/// mid-response), returns a sentinel string describing the read
/// failure rather than an empty string, so logs don't silently
/// misreport the upstream error as "no body".
pub async fn best_effort_response_body(mut response: reqwest::Response) -> String {
    const MAX_BYTES: usize = 4096;
    // Read a few extra bytes past `MAX_BYTES` so the char-boundary
    // walk below has room to retreat past a trailing multi-byte
    // codepoint that straddles the cap (UTF-8 sequences are at
    // most 4 bytes). Still well short of the multi-GB floor where
    // buffering becomes an OOM concern.
    const READ_CAP: usize = MAX_BYTES + 3;

    let mut buf: Vec<u8> = Vec::with_capacity(READ_CAP);
    loop {
        match response.chunk().await {
            Ok(None) => break,
            Ok(Some(bytes)) => {
                let remaining = READ_CAP.saturating_sub(buf.len());
                if remaining == 0 {
                    // Already at the cap; drop the rest of the
                    // response rather than continuing to buffer.
                    break;
                }
                let take = bytes.len().min(remaining);
                buf.extend_from_slice(&bytes[..take]);
                if bytes.len() > take {
                    // This chunk had more bytes than we wanted;
                    // the body is longer than READ_CAP. Stop.
                    break;
                }
            }
            Err(err) => return format!("<body read failed: {err}>"),
        }
    }

    // Lossy decode so a garbled upstream error page doesn't also
    // panic the diagnostic path. Validated UTF-8 round-trips
    // verbatim (into_owned is a no-op borrow → allocation only
    // when replacement characters were inserted).
    let text = String::from_utf8_lossy(&buf).into_owned();
    if buf.len() <= MAX_BYTES {
        return text;
    }

    // Find the last char boundary at or before MAX_BYTES.
    // `is_char_boundary` is O(1); this loop iterates at most 3
    // times (max UTF-8 continuation bytes before the previous
    // boundary).
    let mut end = MAX_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... (truncated)", &text[..end])
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// ASCII body under the cap round-trips verbatim.
    #[tokio::test]
    async fn small_ascii_body_returned_verbatim() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;
        let response = reqwest::get(server.uri()).await.unwrap();
        let body = best_effort_response_body(response).await;
        assert_eq!(body, "boom");
    }

    /// Regression: a 4-byte emoji straddling byte 4096 used to panic
    /// under `&text[..4096]`. Now it truncates at the preceding
    /// codepoint boundary and appends the marker.
    #[tokio::test]
    async fn truncation_respects_utf8_char_boundaries() {
        // Pad with ASCII so byte 4094 is the first byte of a
        // 4-byte codepoint (U+1F600 '😀' encodes as 0xF0 0x9F 0x98 0x80).
        // The char boundary scan must walk back from 4096 past the
        // three continuation bytes to land at 4094.
        let mut body = "a".repeat(4094);
        body.push('😀'); // 4 bytes starting at index 4094
        body.push_str("rest"); // tail we expect truncated away
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string(body))
            .mount(&server)
            .await;
        let response = reqwest::get(server.uri()).await.unwrap();
        let out = best_effort_response_body(response).await;
        // The emoji at index 4094 straddles the 4096 cap — expect
        // the walk to retreat to the boundary at 4094 (before the
        // emoji's 4-byte sequence), producing 4094 `a`s.
        assert!(
            out.ends_with("... (truncated)"),
            "expected truncation marker, got len={}",
            out.len()
        );
        let prefix = out.trim_end_matches("... (truncated)");
        assert!(
            prefix.is_char_boundary(prefix.len()),
            "truncation landed inside a codepoint"
        );
        assert!(
            prefix.chars().all(|c| c == 'a'),
            "expected only ASCII `a` before the truncation point"
        );
    }

    /// Regression: a very large body must be stream-capped near
    /// `MAX_BYTES`, not buffered whole. A previous `response.text()`
    /// implementation buffered everything before truncation — fine
    /// for well-behaved servers, an OOM liability for misbehaving
    /// or malicious ones. This test sends a 1 MiB body and
    /// asserts the returned string is truncated at the ~4 KiB cap
    /// (i.e. the helper didn't materialize the whole 1 MiB into
    /// the output). A real OOM-simulation would need a multi-GiB
    /// body; 1 MiB is enough to fail if the stream cap is absent.
    #[tokio::test]
    async fn streaming_cap_bounds_oversized_body() {
        let body = "x".repeat(1024 * 1024);
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string(body))
            .mount(&server)
            .await;
        let response = reqwest::get(server.uri()).await.unwrap();
        let out = best_effort_response_body(response).await;
        assert!(
            out.ends_with("... (truncated)"),
            "oversized body must be truncated"
        );
        // The 4 KiB cap (+ truncation marker ~14 bytes) is the
        // operationally meaningful bound. Gate loosely so an
        // allocator-side rounding doesn't cause flakes.
        assert!(
            out.len() < 8192,
            "truncated body unexpectedly long: {} bytes",
            out.len()
        );
    }
}
