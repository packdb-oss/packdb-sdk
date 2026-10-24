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

//! The PromQL, LogQL and TraceQL clients against a mock server.

use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use packdb_client::{
    AuthConfig, ClientError, Direction, LogOptions, QueryApiClient, QueryApiConfig,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::json;
use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(url: String) -> QueryApiClient {
    QueryApiClient::new(QueryApiConfig {
        url,
        auth: AuthConfig::api_key("secret-key".into()).into_provider(),
        timeout: Duration::from_secs(2),
    })
    .unwrap()
}

fn tenant() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-scope-orgid", HeaderValue::from_static("tenant-a"));
    headers
}

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

#[tokio::test]
async fn promql_instant_and_range_send_their_parameters_and_credentials() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": {"resultType": "vector", "result": []}});
    Mock::given(method("GET"))
        .and(path("/api/v1/query"))
        .and(query_param("query", "up"))
        .and(query_param("time", "1700000000"))
        .and(header("x-api-key", "secret-key"))
        .and(header("x-scope-orgid", "tenant-a"))
        .and(header("accept", "application/json"))
        .respond_with(
            ok(body.clone()).insert_header("x-packdb-partial-causes", "cold_tier_unavailable"),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query_range"))
        .and(query_param("query", "up"))
        .and(query_param("start", "1"))
        .and(query_param("end", "2"))
        .and(query_param("step", "15s"))
        .respond_with(ok(body.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(server.uri());
    let instant = client
        .promql_instant("up", Some("1700000000"), tenant())
        .await
        .unwrap();
    assert_eq!(instant.data, body);
    assert_eq!(instant.partial_causes, ["cold_tier_unavailable"]);
    let range = client
        .promql_range("up", "1", "2", "15s", HeaderMap::new())
        .await
        .unwrap();
    assert!(range.partial_causes.is_empty());
}

#[tokio::test]
async fn an_absent_time_is_not_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/query"))
        .and(query_param_is_missing("time"))
        .respond_with(ok(json!({"status": "success"})))
        .expect(1)
        .mount(&server)
        .await;
    client(server.uri())
        .promql_instant("up", None, HeaderMap::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn logql_queries_send_limit_and_direction() {
    let server = MockServer::start().await;
    let body = json!({"status": "success", "data": {"resultType": "streams", "result": []}});
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query_range"))
        .and(query_param("query", "{job=\"a\"}"))
        .and(query_param("start", "10"))
        .and(query_param("end", "20"))
        .and(query_param("step", "30s"))
        .and(query_param("limit", "5"))
        .and(query_param("direction", "forward"))
        .respond_with(ok(body.clone()))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/loki/api/v1/query"))
        .and(query_param("query", "count_over_time({job=\"a\"}[1m])"))
        .and(query_param_is_missing("limit"))
        .and(query_param_is_missing("direction"))
        .respond_with(ok(body.clone()))
        .expect(1)
        .mount(&server)
        .await;
    let client = client(server.uri());
    let options = LogOptions {
        limit: Some(5),
        direction: Some(Direction::Forward),
    };
    let range = client
        .logql_range(
            "{job=\"a\"}",
            "10",
            "20",
            Some("30s"),
            options,
            HeaderMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(range.data, body);
    client
        .logql_instant(
            "count_over_time({job=\"a\"}[1m])",
            None,
            LogOptions::default(),
            HeaderMap::new(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn traceql_search_and_trace_reads_use_tempo_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/search"))
        .and(query_param("q", "{ .service.name = \"api\" }"))
        .and(query_param("start", "100"))
        .and(query_param("end", "200"))
        .and(query_param("limit", "3"))
        .respond_with(ok(json!({"traces": [], "metrics": {}})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/traces/abc123"))
        .respond_with(
            ok(json!({"trace": {"batches": []}}))
                .insert_header("x-packdb-partial-causes", "cold_tier_unavailable"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let client = client(server.uri());
    client
        .traceql_search(
            "{ .service.name = \"api\" }",
            Some(100),
            Some(200),
            Some(3),
            HeaderMap::new(),
        )
        .await
        .unwrap();
    let trace = client
        .trace("abc123", None, None, HeaderMap::new())
        .await
        .unwrap();
    assert_eq!(trace.partial_causes, ["cold_tier_unavailable"]);
}

#[tokio::test]
async fn a_non_hex_trace_id_is_refused_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ok(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    for id in ["", "../api/search", "abc 123"] {
        let error = client(server.uri())
            .trace(id, None, None, HeaderMap::new())
            .await
            .expect_err("refused");
        assert!(
            matches!(error, ClientError::ConfigError(_)),
            "{id:?}: {error:?}"
        );
    }
}

#[tokio::test]
async fn failures_are_classified_with_the_server_message() {
    let cases = [
        (
            "/api/v1/query",
            ResponseTemplate::new(400).set_body_json(
                json!({"status": "error", "errorType": "bad_data", "error": "parse error at 3"}),
            ),
            "query",
            "parse error at 3",
        ),
        (
            "/api/v1/query",
            ResponseTemplate::new(422).set_body_json(
                json!({"status": "error", "errorType": "execution", "error": "boom"}),
            ),
            "protocol",
            "boom",
        ),
        (
            "/api/v1/query",
            ResponseTemplate::new(401).set_body_string("missing credentials"),
            "unauthenticated",
            "missing credentials",
        ),
        (
            "/api/v1/query",
            ResponseTemplate::new(403).set_body_string("not yours"),
            "permission",
            "not yours",
        ),
        (
            "/api/v1/query",
            ResponseTemplate::new(429).set_body_string("query plane at capacity"),
            "protocol",
            "query plane at capacity",
        ),
    ];
    for (route, response, expected, text) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .mount(&server)
            .await;
        let error = client(server.uri())
            .promql_instant("up", None, HeaderMap::new())
            .await
            .expect_err("a failure");
        let kind = match &error {
            ClientError::QueryError(_) => "query",
            ClientError::Unauthenticated(_) => "unauthenticated",
            ClientError::PermissionDenied(_) => "permission",
            ClientError::ProtocolError(_) => "protocol",
            _ => "other",
        };
        assert_eq!(kind, expected, "{error:?}");
        assert!(error.to_string().contains(text), "{error}");
    }
}

#[tokio::test]
async fn tempo_failures_keep_their_meaning() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/search"))
        .respond_with(
            ResponseTemplate::new(422).set_body_json(json!({"error": "unsupported: pipeline"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/traces/ff"))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(json!({"error": "trace not found: ff"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v2/traces/ee"))
        .respond_with(ResponseTemplate::new(500).set_body_json(
            json!({"data": null, "errors": [{"code": 500, "msg": "store unavailable"}]}),
        ))
        .mount(&server)
        .await;
    let client = client(server.uri());
    let search = client
        .traceql_search("{ | count() > 1 }", None, None, None, HeaderMap::new())
        .await
        .expect_err("unsupported");
    assert!(
        matches!(&search, ClientError::QueryError(m) if m == "unsupported: pipeline"),
        "{search:?}"
    );
    let missing = client
        .trace("ff", None, None, HeaderMap::new())
        .await
        .expect_err("not found");
    assert!(
        matches!(&missing, ClientError::NotFound(m) if m == "trace not found: ff"),
        "{missing:?}"
    );
    let broken = client
        .trace("ee", None, None, HeaderMap::new())
        .await
        .expect_err("server failure");
    assert!(
        matches!(&broken, ClientError::ProtocolError(m) if m.contains("store unavailable")),
        "{broken:?}"
    );
}

#[tokio::test]
async fn only_the_servers_trace_miss_is_not_found() {
    let server = MockServer::start().await;
    let miss = json!({"error": "trace not found: ab12"});
    Mock::given(method("GET"))
        .and(path("/api/v2/traces/ab12"))
        .respond_with(ResponseTemplate::new(404).set_body_json(miss.clone()))
        .mount(&server)
        .await;
    let other_404s = [
        (
            "a1",
            ResponseTemplate::new(404).set_body_string("404 page not found"),
        ),
        (
            "a2",
            ResponseTemplate::new(404)
                .insert_header("content-type", "text/html")
                .set_body_string("<html><body><h1>404 Not Found</h1></body></html>"),
        ),
        ("a3", ResponseTemplate::new(404)),
        (
            "a4",
            ResponseTemplate::new(404).set_body_json(json!({"error": "trace not found: ab12"})),
        ),
        (
            "a5",
            ResponseTemplate::new(404)
                .set_body_json(json!({"error": "trace not found: a5"}))
                .insert_header("x-packdb-partial-causes", "cold_tier_unavailable"),
        ),
    ];
    for (id, response) in &other_404s {
        Mock::given(method("GET"))
            .and(path(format!("/api/v2/traces/{id}")))
            .respond_with(response.clone())
            .mount(&server)
            .await;
    }
    let client = client(server.uri());
    let missing = client
        .trace("ab12", None, None, HeaderMap::new())
        .await
        .expect_err("absent");
    assert!(
        matches!(&missing, ClientError::NotFound(m) if m == "trace not found: ab12"),
        "{missing:?}"
    );
    for (id, _) in &other_404s {
        let error = client
            .trace(id, None, None, HeaderMap::new())
            .await
            .expect_err("a 404");
        assert!(
            matches!(&error, ClientError::ProtocolError(m) if m.contains("404")),
            "{id}: {error:?}"
        );
    }
}

/// An endpoint that refuses connections. Its port is the local end of an open
/// loopback connection: nothing listens on it, and while this is held the
/// system does not assign the port to another socket bound to port 0, as every
/// test server here is. The refusal is checked here, not assumed.
struct Refusing {
    endpoint: String,
    _held: (TcpListener, TcpStream, TcpStream),
}

fn refusing_endpoint() -> Refusing {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let client =
        TcpStream::connect(listener.local_addr().expect("listener addr")).expect("connect");
    let (server, _) = listener.accept().expect("accept");
    let addr = client.local_addr().expect("local addr");
    let refused = TcpStream::connect(addr).expect_err("nothing listens");
    assert_eq!(refused.kind(), ErrorKind::ConnectionRefused, "{refused}");
    Refusing {
        endpoint: format!("http://{addr}"),
        _held: (listener, client, server),
    }
}

#[tokio::test]
async fn an_unreachable_server_is_a_connection_error() {
    let refusing = refusing_endpoint();
    let error = client(refusing.endpoint.clone())
        .promql_instant("up", None, HeaderMap::new())
        .await
        .expect_err("refused");
    assert!(
        matches!(error, ClientError::ConnectionError(_)),
        "{error:?}"
    );
}
