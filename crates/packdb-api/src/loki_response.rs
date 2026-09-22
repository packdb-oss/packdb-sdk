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

//! Loki-compatible query API response types.
//!
//! Implements the JSON response format documented at
//! <https://grafana.com/docs/loki/latest/reference/loki-http-api/>.
//! Shared across PackDB's server-side Loki handler (which
//! serializes) and the alert evaluator's LogQL client (which
//! deserializes) so a wire-format change lands in one spot and
//! breaks both sides at compile time if they diverge.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Request header a Loki client uses to negotiate response encodings.
/// Grafana's Loki datasource sets it unconditionally on every data query
/// (`pkg/tsdb/loki/api.go`).
pub const ENCODING_FLAGS_HEADER: &str = "X-Loki-Response-Encoding-Flags";

/// The one encoding flag PackDB implements: keep structured metadata (and
/// parser-derived labels) *out* of the stream label set and categorize them
/// per entry instead. See [`LokiEntry`] for the two wire forms.
pub const ENCODING_FLAG_CATEGORIZE_LABELS: &str = "categorize-labels";

/// True iff an `X-Loki-Response-Encoding-Flags` header value negotiates
/// [`ENCODING_FLAG_CATEGORIZE_LABELS`]. The value is a comma-separated flag
/// list; unknown flags are ignored, matching Loki.
#[must_use]
pub fn negotiates_categorize_labels(header_value: &str) -> bool {
    header_value.split(',').any(|flag| {
        flag.trim()
            .eq_ignore_ascii_case(ENCODING_FLAG_CATEGORIZE_LABELS)
    })
}

/// Top-level Loki API response envelope.
///
/// `T` is typically [`LokiQueryData`] for query responses, or
/// the unit type `()` for error shapes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LokiResponse<T> {
    /// `"success"` or `"error"`, mirroring Loki's own envelope.
    pub status: String,
    // `Option<T>` deserializes from an absent JSON field as `None`
    // in serde_json without needing `#[serde(default)]` — and
    // `#[serde(default)]` on a generic `Option<T>` propagates a
    // `T: Default` bound onto the derived `Deserialize` impl,
    // which is why we omit it here. Same pattern as
    // [`crate::prometheus_response::PrometheusApiResponse`].
    /// The payload, present when `status` is `"success"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Human-readable failure description, present when `status` is
    /// `"error"`. Free text; `error_type` carries the classification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Loki's `errorType` classification for the failure in `error`, such as
    /// `"bad_data"` or `"internal"`. Clients branch on this rather than on
    /// the message text.
    #[serde(rename = "errorType", skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
}

impl<T> LokiResponse<T> {
    /// Create a success response.
    pub fn ok(data: T) -> Self {
        Self {
            status: "success".to_string(),
            data: Some(data),
            error: None,
            error_type: None,
        }
    }
}

impl LokiResponse<()> {
    /// Create an error response with an explicit Loki/Prometheus-style
    /// `errorType` classification (e.g. `bad_data`, `execution`,
    /// `timeout`, `too_many_requests`, `internal`). The HTTP status code
    /// is set separately by the caller; this only shapes the JSON body.
    pub fn err_typed(msg: String, error_type: &str) -> Self {
        Self {
            status: "error".to_string(),
            data: None,
            error: Some(msg),
            error_type: Some(error_type.to_string()),
        }
    }

    /// Create an error response with the Loki-documented `bad_data`
    /// error-type classification (caller-fixable request errors).
    pub fn err(msg: String) -> Self {
        Self::err_typed(msg, "bad_data")
    }

    /// Create an error response with the `internal` error-type
    /// classification.
    pub fn internal_err(msg: String) -> Self {
        Self::err_typed(msg, "internal")
    }
}

/// Query result data wrapper.
///
/// The `result_type` field tells clients how to interpret the
/// result:
/// - `"streams"` — log lines grouped by label set
/// - `"vector"` — instant metric samples
/// - `"matrix"` — range metric samples (time series)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LokiQueryData {
    /// `"streams"`, `"vector"` or `"matrix"` — which variant of
    /// [`LokiQueryResult`] `result` holds, as described on this type.
    ///
    /// A `String` rather than an enum so an unrecognised result type
    /// round-trips instead of failing deserialization.
    #[serde(rename = "resultType")]
    pub result_type: String,
    /// `data.encodingFlags` — the response encodings in force for this
    /// payload, echoing back the `X-Loki-Response-Encoding-Flags` request
    /// header (see [`ENCODING_FLAG_CATEGORIZE_LABELS`]). Empty ⇒ the field
    /// is omitted and the historical wire shape is unchanged.
    ///
    /// **Field order is load-bearing**: this must serialize *before*
    /// `result`. Grafana's streaming JSON reader
    /// (`promlib/converter.readResult`) picks the entry parser at the moment
    /// it reaches `result`, using the flags it has accumulated so far — a
    /// trailing `encodingFlags` arrives too late and the categorized
    /// third element blows up its 2-element parser with a `ReadArray` error.
    #[serde(
        rename = "encodingFlags",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub encoding_flags: Vec<String>,
    /// The result rows, in whichever shape `result_type` names.
    pub result: LokiQueryResult,
    /// Optional execution stats at `data.stats`. Absent
    /// from non-query surfaces and omitted entirely when not populated, so
    /// the existing wire shape is unchanged for clients that don't read
    /// it. `default` keeps deserialization of older stat-less payloads
    /// working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<LokiQueryStats>,
}

/// Execution stats surfaced under `data.stats`.
///
/// A PackDB extension (not Loki's `ingester`/`querier`/`summary` schema —
/// those values don't exist here): it explains how the query's stream
/// selector was split between columnar pushdown filters and residual
/// per-row evaluation, so operators can see when a query falls into a
/// residual scan. Loki/Grafana clients ignore the unknown keys.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct LokiQueryStats {
    /// Label names applied as columnar pushdown filters (the pushable
    /// log-schema columns: `service_name`, `severity`).
    #[serde(rename = "pushdownLabels")]
    pub pushdown_labels: Vec<String>,
    /// Label names evaluated as residual per-row filters — unpromoted
    /// attributes that cannot be pushed into the columnar scan, so each
    /// candidate row is checked individually.
    #[serde(rename = "residualLabels")]
    pub residual_labels: Vec<String>,
    /// `true` iff the selector required residual per-row evaluation
    /// (equivalently, `residualLabels` is non-empty) — the one-glance
    /// "this query did a residual scan" signal.
    #[serde(rename = "usesResidualScan")]
    pub uses_residual_scan: bool,
}

/// Polymorphic query result. Wire shape is an untagged union:
/// the server looks at the neighboring `resultType` field to
/// decide which variant it emits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum LokiQueryResult {
    /// `resultType: "streams"` — log lines grouped by stream label set.
    Streams(Vec<LokiStream>),
    /// `resultType: "vector"` — one instant metric sample per series.
    Vector(Vec<LokiVectorSample>),
    /// `resultType: "matrix"` — a range of metric samples per series.
    Matrix(Vec<LokiMatrixSeries>),
}

/// A stream of log entries sharing the same *stream label set*.
///
/// Wire shape: `{"stream": {labels}, "values": [<entry>, ...]}` where each
/// entry is a [`LokiEntry`] in one of the [`EntryEncoding`] forms.
///
/// Where unpromoted OTLP log-record attributes (Loki structured metadata)
/// live depends on what the client negotiated, exactly as in Loki:
///
/// * [`EntryEncoding::Categorized`] — `stream` carries only the indexed
///   labels that define stream identity (`service_name`, `severity`, and any
///   promoted labels), and the attributes travel per-entry in
///   [`LokiEntry::structured_metadata`]. Two lines differing only by an
///   attribute value stay in one stream.
/// * [`EntryEncoding::Plain`] — the attributes are merged into `stream`
///   instead, so those two lines split into two streams. Nothing is lost;
///   the client just cannot tell an indexed label from an attribute.
///
/// Both fields are required (no `#[serde(default)]`) so the
/// untagged [`LokiQueryResult`] dispatch can distinguish a stream
/// from a vector sample by *shape* rather than by falling into the
/// first variant whose defaults happen to match.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LokiStream {
    /// The label set defining this stream's identity.
    ///
    /// What it contains depends on the negotiated encoding, as described
    /// above: under [`EntryEncoding::Categorized`] only indexed labels,
    /// under [`EntryEncoding::Plain`] those plus the merged structured
    /// metadata.
    pub stream: HashMap<String, String>,
    /// The log entries in this stream.
    ///
    /// Ordering follows the query's `direction` parameter — `backward`
    /// (newest first) is the default, `forward` gives oldest first — so a
    /// consumer must not assume one or the other.
    pub values: Vec<LokiEntry>,
}

/// Which of Loki's two entry wire forms a [`LokiEntry`] serializes as.
///
/// The choice is per-response, not per-entry: it is dictated by whether the
/// client negotiated [`ENCODING_FLAG_CATEGORIZE_LABELS`], and the whole
/// payload must be consistent because clients pick one parser for the whole
/// `result` array.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryEncoding {
    /// `[ts_ns_string, line]` — the historical 2-element shape. Structured
    /// metadata is not on the entry at all; the builder has already merged it
    /// into the enclosing stream's label set (Loki's behaviour for clients
    /// that did not negotiate categorized labels).
    #[default]
    Plain,
    /// `[ts_ns_string, line, {"structuredMetadata": {...}}]` — Loki 3.x's
    /// `categorize-labels` shape, emitted only alongside
    /// `data.encodingFlags: ["categorize-labels"]`.
    ///
    /// The third element is emitted for **every** entry, including when the
    /// map is empty. Grafana's categorized reader
    /// (`promlib/converter.readCategorizedStream`) reads the third element
    /// unconditionally, so a 2-element entry mixed into a categorized
    /// response is a parse error there.
    Categorized,
}

/// A single log entry on the Loki `streams` wire.
///
/// Serializes as a heterogeneous JSON array whose shape is chosen by
/// [`EntryEncoding`]. Deserialization accepts either form — plus the flat
/// `[ts, line, {k: v}]` third element older PackDB builds emitted — and
/// ignores any further trailing elements for forward-compat.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LokiEntry {
    /// Nanosecond epoch timestamp as a decimal string (Loki wire form).
    pub timestamp: String,
    /// The raw log line.
    pub line: String,
    /// Per-line Loki structured metadata (unpromoted OTLP attributes).
    /// Only reaches the wire under [`EntryEncoding::Categorized`].
    pub structured_metadata: HashMap<String, String>,
    /// Wire form for this entry. Not itself serialized.
    pub encoding: EntryEncoding,
}

impl LokiEntry {
    /// Entry with no structured metadata (2-element wire form).
    pub fn new(timestamp: String, line: String) -> Self {
        Self {
            timestamp,
            line,
            structured_metadata: HashMap::new(),
            encoding: EntryEncoding::Plain,
        }
    }

    /// Entry in the `categorize-labels` wire form: structured metadata
    /// travels per-entry rather than in the stream label set.
    pub fn categorized(
        timestamp: String,
        line: String,
        structured_metadata: HashMap<String, String>,
    ) -> Self {
        Self {
            timestamp,
            line,
            structured_metadata,
            encoding: EntryEncoding::Categorized,
        }
    }
}

/// The categorized third element: `{"structuredMetadata": {...}}`.
///
/// Loki also categorizes parser-derived labels under `parsed`; PackDB's
/// LogQL engine surfaces no parser labels, so that key is never emitted
/// (Grafana treats an absent `parsed` as empty) but is accepted on the
/// deserialize side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
struct CategorizedLabels {
    #[serde(rename = "structuredMetadata", default)]
    structured_metadata: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    parsed: HashMap<String, String>,
}

/// The third element of an entry array, on the read side.
///
/// `Categorized` is tried first and carries `deny_unknown_fields`, so it only
/// matches an object whose keys are a subset of `{structuredMetadata,
/// parsed}`; anything else falls through to the flat map older PackDB builds
/// emitted.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EntryMetadata {
    Categorized(CategorizedLabels),
    Flat(HashMap<String, String>),
}

impl Serialize for LokiEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let categorized = self.encoding == EntryEncoding::Categorized;
        let mut seq = serializer.serialize_seq(Some(if categorized { 3 } else { 2 }))?;
        seq.serialize_element(&self.timestamp)?;
        seq.serialize_element(&self.line)?;
        if categorized {
            // Always present, even when empty — see `EntryEncoding::Categorized`.
            seq.serialize_element(&CategorizedLabels {
                structured_metadata: self.structured_metadata.clone(),
                parsed: HashMap::new(),
            })?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for LokiEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntryVisitor;
        impl<'de> serde::de::Visitor<'de> for EntryVisitor {
            type Value = LokiEntry;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a Loki log entry [ts, line] or [ts, line, {metadata}]")
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<LokiEntry, A::Error> {
                let timestamp: String = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
                let line: String = seq
                    .next_element()?
                    .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
                let (structured_metadata, encoding) = match seq.next_element()? {
                    Some(EntryMetadata::Categorized(c)) => {
                        (c.structured_metadata, EntryEncoding::Categorized)
                    }
                    // Flat third element: the pre-negotiation PackDB shape,
                    // still on the wire from older servers. The metadata is
                    // read back, but the entry is marked `Plain` so this
                    // never-negotiated shape is not re-emitted.
                    Some(EntryMetadata::Flat(m)) => (m, EntryEncoding::Plain),
                    None => (HashMap::new(), EntryEncoding::Plain),
                };
                // Tolerate (and drop) any further elements for forward-compat.
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(LokiEntry {
                    timestamp,
                    line,
                    structured_metadata,
                    encoding,
                })
            }
        }
        deserializer.deserialize_seq(EntryVisitor)
    }
}

/// A `[unix_seconds, "value"]` tuple for metric results.
///
/// Loki API spec: timestamp is a JSON number (seconds since
/// epoch), value is a JSON string. Serializes as
/// `[1588889221.5, "42.0"]` and round-trips the same on the
/// deserialize side.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricValue(pub f64, pub String);

impl Serialize for MetricValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeTuple;
        let mut tup = serializer.serialize_tuple(2)?;
        tup.serialize_element(&self.0)?;
        tup.serialize_element(&self.1)?;
        tup.end()
    }
}

impl<'de> Deserialize<'de> for MetricValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (ts, val) = <(f64, String)>::deserialize(deserializer)?;
        Ok(Self(ts, val))
    }
}

/// An instant metric sample.
///
/// Wire shape: `{"metric": {labels}, "value": [unix_seconds, "value"]}`.
///
/// `metric` is required (no `#[serde(default)]`) so the untagged
/// [`LokiQueryResult`] dispatch can distinguish a vector sample
/// from a stream (which has `stream` instead of `metric`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LokiVectorSample {
    /// Label set identifying the series this sample belongs to.
    pub metric: HashMap<String, String>,
    /// The single `[unix_seconds, "value"]` sample.
    pub value: MetricValue,
}

/// A range metric series.
///
/// Wire shape: `{"metric": {labels}, "values": [[unix_seconds, "value"], ...]}`.
///
/// `metric` is required for the same untagged-dispatch reason as
/// [`LokiVectorSample`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LokiMatrixSeries {
    /// Label set identifying the series these samples belong to.
    pub metric: HashMap<String, String>,
    /// The `[unix_seconds, "value"]` samples across the queried range, one
    /// per step, in ascending time order.
    ///
    /// Ascending because the engine evaluates the range step by step from
    /// `start` to `end`; unlike [`LokiStream::values`], this is not affected
    /// by the query's `direction`, which applies to log entries only.
    pub values: Vec<MetricValue>,
}

/// `GET|POST /loki/api/v1/index/stats` response.
///
/// Grafana Explore calls this to size a query before running it (the
/// "this query will process N bytes" guardrail). Unlike every other Loki
/// query route, the response is a **bare object** — it is NOT wrapped in the
/// `{"status": ..., "data": ...}` envelope — so it is serialized directly
/// rather than through [`LokiResponse`].
///
/// Values are **approximate and index-derived**, matching upstream: all four
/// fields come from PackDB's persisted fragment manifest, never from scanning
/// log rows or consulting active-only catalogs. Upstream excludes data still
/// in ingesters; PackDB's equivalent exclusion is the hot tier — rows still
/// in memory have no fragment yet and are not counted.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LokiIndexStats {
    /// Distinct persisted service partitions matching the selector window.
    /// This is PackDB's currently available durable stream approximation.
    pub streams: u64,
    /// Matching persisted log fragments — PackDB's analogue of a Loki chunk.
    pub chunks: u64,
    /// Time-overlap-adjusted stored rows across the matching fragments.
    pub entries: u64,
    /// Time-overlap-adjusted stored payload bytes across the matching
    /// fragments, mirroring upstream's stored chunk-byte accounting.
    pub bytes: u64,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Round-trip a vector result through serde to lock in the
    /// wire-format: client and server must both agree on this
    /// exact JSON shape.
    #[test]
    fn vector_round_trips() {
        let data = LokiQueryData {
            result_type: "vector".into(),
            encoding_flags: Vec::new(),
            result: LokiQueryResult::Vector(vec![LokiVectorSample {
                metric: [("app".into(), "api".into())].into_iter().collect(),
                value: MetricValue(1_700_000_000.0, "42".into()),
            }]),
            stats: None,
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["resultType"], "vector");
        assert_eq!(json["result"][0]["metric"]["app"], "api");
        assert_eq!(json["result"][0]["value"][0], 1_700_000_000.0);
        assert_eq!(json["result"][0]["value"][1], "42");

        let parsed: LokiQueryData = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, data);
    }

    /// Round-trip a streams result: label set + log lines.
    #[test]
    fn streams_round_trip() {
        let data = LokiQueryData {
            result_type: "streams".into(),
            encoding_flags: Vec::new(),
            result: LokiQueryResult::Streams(vec![LokiStream {
                stream: [("app".into(), "api".into())].into_iter().collect(),
                values: vec![LokiEntry::new(
                    "1700000000000000000".into(),
                    "ERROR: upstream 500".into(),
                )],
            }]),
            stats: None,
        };
        let json = serde_json::to_string(&data).unwrap();
        let parsed: LokiQueryData = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, data);
    }

    /// A `Plain` entry is the historical 2-element `[ts, line]` array — never
    /// a third element, whatever metadata the value happens to hold, because
    /// a client that did not negotiate `categorize-labels` parses exactly two.
    #[test]
    fn plain_entry_is_always_two_elements() {
        let bare = LokiEntry::new("1700000000000000000".into(), "hello".into());
        let json = serde_json::to_value(&bare).unwrap();
        let arr = json.as_array().expect("entry is a JSON array");
        assert_eq!(arr.len(), 2, "no-metadata entry must be a 2-element array");
        assert_eq!(arr[0], "1700000000000000000");
        assert_eq!(arr[1], "hello");
        assert_eq!(serde_json::from_value::<LokiEntry>(json).unwrap(), bare);
    }

    /// A `Categorized` entry is Loki 3.x's
    /// `[ts, line, {"structuredMetadata": {...}}]` — and the third element is
    /// emitted even when the map is empty, since Grafana's categorized reader
    /// consumes it unconditionally.
    #[test]
    fn categorized_entry_wire_shape_round_trips() {
        let with_meta = LokiEntry::categorized(
            "1700000000000000001".into(),
            "auth refresh failed".into(),
            [("error_code".to_string(), "auth-invalid".to_string())]
                .into_iter()
                .collect(),
        );
        let json = serde_json::to_value(&with_meta).unwrap();
        let arr = json.as_array().expect("entry is a JSON array");
        assert_eq!(arr.len(), 3, "categorized entry must be a 3-element array");
        assert_eq!(arr[2]["structuredMetadata"]["error_code"], "auth-invalid");
        assert!(
            arr[2].get("parsed").is_none(),
            "no parser labels to categorize, so the key stays off the wire"
        );
        assert_eq!(
            serde_json::from_value::<LokiEntry>(json).unwrap(),
            with_meta
        );

        let empty =
            LokiEntry::categorized("1700000000000000002".into(), "line".into(), HashMap::new());
        let json = serde_json::to_value(&empty).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 3);
        assert!(json[2]["structuredMetadata"].is_object());
    }

    /// The read side accepts every third-element shape PackDB has emitted:
    /// absent, categorized, and the flat map from pre-negotiation builds.
    #[test]
    fn entry_deserialize_accepts_every_third_element_shape() {
        let categorized: LokiEntry = serde_json::from_value(serde_json::json!([
            "1", "line", {"structuredMetadata": {"k": "v"}, "parsed": {"p": "q"}}
        ]))
        .unwrap();
        assert_eq!(categorized.encoding, EntryEncoding::Categorized);
        assert_eq!(
            categorized.structured_metadata.get("k").map(String::as_str),
            Some("v")
        );

        // Flat map: an older PackDB server's shape. Metadata is still read,
        // but the entry is marked `Plain` so it is not re-emitted that way.
        let flat: LokiEntry =
            serde_json::from_value(serde_json::json!(["1", "line", {"k": "v"}])).unwrap();
        assert_eq!(flat.encoding, EntryEncoding::Plain);
        assert_eq!(
            flat.structured_metadata.get("k").map(String::as_str),
            Some("v")
        );

        let two: LokiEntry = serde_json::from_value(serde_json::json!(["1", "line"])).unwrap();
        assert_eq!(two.encoding, EntryEncoding::Plain);
        assert!(two.structured_metadata.is_empty());
    }

    /// `data.encodingFlags` round-trips and is omitted when empty, so a
    /// response to a client that negotiated nothing is byte-identical to the
    /// pre-negotiation shape.
    #[test]
    fn encoding_flags_round_trip_and_omitted_when_empty() {
        let data = LokiQueryData {
            result_type: "streams".into(),
            encoding_flags: vec![ENCODING_FLAG_CATEGORIZE_LABELS.to_string()],
            result: LokiQueryResult::Streams(Vec::new()),
            stats: None,
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["encodingFlags"][0], "categorize-labels");
        assert_eq!(serde_json::from_value::<LokiQueryData>(json).unwrap(), data);

        let bare = LokiQueryData {
            result_type: "streams".into(),
            encoding_flags: Vec::new(),
            result: LokiQueryResult::Streams(Vec::new()),
            stats: None,
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert!(
            json.get("encodingFlags").is_none(),
            "encodingFlags omitted when nothing was negotiated"
        );
    }

    #[test]
    fn categorize_labels_negotiation_parsing() {
        assert!(negotiates_categorize_labels("categorize-labels"));
        assert!(negotiates_categorize_labels(" categorize-labels "));
        assert!(negotiates_categorize_labels(
            "future-flag,categorize-labels"
        ));
        assert!(!negotiates_categorize_labels("future-flag"));
        assert!(!negotiates_categorize_labels(""));
    }

    /// Envelope `ok` round-trips. Guards against a future `data`
    /// field serde change breaking both server and client.
    #[test]
    fn envelope_ok_round_trip() {
        let env = LokiResponse::ok(LokiQueryData {
            result_type: "vector".into(),
            encoding_flags: Vec::new(),
            result: LokiQueryResult::Vector(Vec::new()),
            stats: None,
        });
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["status"], "success");
        let parsed: LokiResponse<LokiQueryData> = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.status, "success");
    }

    /// `data.stats` wire shape round-trips, and is omitted entirely when
    /// `None` so older stat-less payloads still deserialize.
    #[test]
    fn query_stats_round_trip_and_omitted_when_none() {
        let data = LokiQueryData {
            result_type: "streams".into(),
            encoding_flags: Vec::new(),
            result: LokiQueryResult::Streams(Vec::new()),
            stats: Some(LokiQueryStats {
                pushdown_labels: vec!["service_name".into()],
                residual_labels: vec!["error_code".into()],
                uses_residual_scan: true,
            }),
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["stats"]["pushdownLabels"][0], "service_name");
        assert_eq!(json["stats"]["residualLabels"][0], "error_code");
        assert_eq!(json["stats"]["usesResidualScan"], true);
        let parsed: LokiQueryData = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, data);

        // None → no `stats` key emitted, and a stat-less payload parses.
        let bare = LokiQueryData {
            result_type: "vector".into(),
            encoding_flags: Vec::new(),
            result: LokiQueryResult::Vector(Vec::new()),
            stats: None,
        };
        let json = serde_json::to_value(&bare).unwrap();
        assert!(json.get("stats").is_none(), "stats omitted when None");
        let parsed: LokiQueryData =
            serde_json::from_value(serde_json::json!({"resultType": "vector", "result": []}))
                .unwrap();
        assert_eq!(parsed.stats, None);
    }
}
