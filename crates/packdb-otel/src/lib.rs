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

//! Protocol-neutral W3C trace-context propagation.
//!
//! Applications own exporter setup, credentials, and routing policy. This
//! crate only adapts OpenTelemetry's global propagator to HTTP headers and
//! gRPC metadata.

use http::HeaderMap;
use opentelemetry::Context;
use opentelemetry::global;
use opentelemetry::propagation::{Extractor, Injector, TextMapCompositePropagator};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use tonic::metadata::{Ascii, MetadataKey, MetadataMap, MetadataValue};

struct HeaderMapExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderMapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|key| key.as_str()).collect()
    }
}

struct HeaderMapInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderMapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            http::HeaderName::from_bytes(key.as_bytes()),
            http::HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

struct MetadataExtractor<'a>(&'a MetadataMap);

impl Extractor for MetadataExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0
            .keys()
            .filter_map(|key| match key {
                tonic::metadata::KeyRef::Ascii(key) => Some(key.as_str()),
                tonic::metadata::KeyRef::Binary(_) => None,
            })
            .collect()
    }
}

struct MetadataInjector<'a>(&'a mut MetadataMap);

impl Injector for MetadataInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            MetadataKey::<Ascii>::from_bytes(key.as_bytes()),
            MetadataValue::<Ascii>::try_from(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

/// Install W3C trace-context and baggage propagation globally.
pub fn install_w3c_propagator() {
    global::set_text_map_propagator(TextMapCompositePropagator::new(vec![
        Box::new(TraceContextPropagator::new()),
        Box::new(BaggagePropagator::new()),
    ]));
}

/// Extract a W3C trace context from inbound HTTP headers.
pub fn extract_http_context(headers: &HeaderMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderMapExtractor(headers)))
}

/// Inject a W3C trace context into outbound HTTP headers.
pub fn inject_http_context(context: &Context, headers: &mut HeaderMap) {
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(context, &mut HeaderMapInjector(headers));
    });
}

/// Extract a W3C trace context from inbound gRPC metadata.
pub fn extract_grpc_context(metadata: &MetadataMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&MetadataExtractor(metadata)))
}

/// Inject a W3C trace context into outbound gRPC metadata.
pub fn inject_grpc_context(context: &Context, metadata: &mut MetadataMap) {
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(context, &mut MetadataInjector(metadata));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TraceContextExt;
    use opentelemetry::trace::{SpanContext, SpanId, TraceFlags, TraceId, TraceState};

    fn context() -> Context {
        Context::new().with_remote_span_context(SpanContext::new(
            TraceId::from_hex("4bf92f3577b34da6a3ce929d0e0e4736").expect("valid trace id"),
            SpanId::from_hex("00f067aa0ba902b7").expect("valid span id"),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        ))
    }

    #[test]
    fn http_round_trip_preserves_trace_context() {
        install_w3c_propagator();
        let mut headers = HeaderMap::new();
        inject_http_context(&context(), &mut headers);
        assert_eq!(
            extract_http_context(&headers)
                .span()
                .span_context()
                .trace_id(),
            context().span().span_context().trace_id()
        );
    }

    #[test]
    fn grpc_round_trip_preserves_trace_context() {
        install_w3c_propagator();
        let mut metadata = MetadataMap::new();
        inject_grpc_context(&context(), &mut metadata);
        assert_eq!(
            extract_grpc_context(&metadata)
                .span()
                .span_context()
                .trace_id(),
            context().span().span_context().trace_id()
        );
    }
}
