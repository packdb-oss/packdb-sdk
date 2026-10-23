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

//! Completeness across HTTP and Flight, and the bounded Flight-to-HTTP fallback.
#![cfg(feature = "sql")]

use packdb_client::{SqlClient, SqlClientConfig, SqlTransport, credentials::NoAuth};
use reqwest::header::HeaderMap;
use serde_json::json;
use std::{sync::Arc, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn client(url: String, flight_url: Option<String>) -> SqlClient {
    SqlClient::new(SqlClientConfig {
        url,
        flight_url,
        auth: Arc::new(NoAuth),
        timeout: Duration::from_secs(2),
        transport: SqlTransport::Auto,
    })
}

#[tokio::test]
async fn http_rows_keep_the_completeness_carrier() {
    for (carrier, expected) in [
        (None, vec![]),
        (
            Some("tier_scan_failed, cold_tier_unavailable"),
            vec!["tier_scan_failed", "cold_tier_unavailable"],
        ),
        (Some(""), vec!["empty_partial_causes"]),
        (Some("  "), vec!["empty_partial_causes"]),
        (Some(", ,"), vec!["empty_partial_causes"]),
    ] {
        let server = MockServer::start().await;
        let mut response =
            ResponseTemplate::new(200).set_body_json(json!({"rows": [{"count": 7}]}));
        if let Some(value) = carrier {
            response = response.insert_header("x-packdb-partial-causes", value);
        }
        Mock::given(method("POST"))
            .and(path("/api/v1/sql"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let result = client(server.uri(), None)
            .execute_query_rows_with_completeness("SELECT count", HeaderMap::new())
            .await
            .unwrap();
        assert_eq!(result.data, [json!({"count": 7})]);
        assert_eq!(result.partial_causes, expected);
    }
}

#[cfg(feature = "flight-sql")]
mod flight {
    use super::*;
    use arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    };
    use arrow_flight::{
        FlightDescriptor, FlightEndpoint, FlightInfo, Ticket,
        encode::FlightDataEncoderBuilder,
        flight_service_server::{FlightService, FlightServiceServer},
        sql::{
            CommandStatementQuery, ProstMessageExt, SqlInfo, TicketStatementQuery,
            server::FlightSqlService,
        },
    };
    use futures::{TryStreamExt, stream};
    use prost::Message;
    use tonic::{Request, Response, Status};

    struct Stub {
        carrier: Option<&'static str>,
        fail_stream: bool,
        refuse_info: Option<tonic::Code>,
    }

    #[tonic::async_trait]
    impl FlightSqlService for Stub {
        type FlightService = Self;
        async fn get_flight_info_statement(
            &self,
            _: CommandStatementQuery,
            _: Request<FlightDescriptor>,
        ) -> Result<Response<FlightInfo>, Status> {
            if let Some(code) = self.refuse_info {
                return Err(Status::new(code, "refused by the stub"));
            }
            let ticket = Ticket {
                ticket: TicketStatementQuery {
                    statement_handle: "test".into(),
                }
                .as_any()
                .encode_to_vec()
                .into(),
            };
            Ok(Response::new(
                FlightInfo::new().with_endpoint(FlightEndpoint::new().with_ticket(ticket)),
            ))
        }
        async fn do_get_statement(
            &self,
            _: TicketStatementQuery,
            _: Request<Ticket>,
        ) -> Result<Response<<Self as FlightService>::DoGetStream>, Status> {
            let schema = Arc::new(Schema::new(vec![Field::new(
                "count",
                DataType::Int64,
                false,
            )]));
            let batch =
                RecordBatch::try_new(schema.clone(), vec![Arc::new(Int64Array::from(vec![7]))])
                    .unwrap();
            let stream = FlightDataEncoderBuilder::new()
                .with_schema(schema)
                .build(stream::iter(vec![Ok(batch)]))
                .map_err(|error| Status::internal(error.to_string()));
            let data: <Self as FlightService>::DoGetStream = if self.fail_stream {
                Box::pin(stream.chain(stream::iter(vec![Err(Status::unavailable(
                    "stream interrupted",
                ))])))
            } else {
                Box::pin(stream)
            };
            let mut response = Response::new(data);
            if let Some(carrier) = self.carrier {
                response
                    .metadata_mut()
                    .insert("x-packdb-partial-causes", carrier.parse().unwrap());
            }
            Ok(response)
        }
        async fn register_sql_info(&self, _: i32, _: &SqlInfo) {}
    }

    use futures::StreamExt;

    async fn start(
        stub: Stub,
    ) -> (
        String,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let incoming = stream::unfold(listener, |listener| async move {
            let connection = listener.accept().await.map(|(socket, _)| socket);
            Some((connection, listener))
        });
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(FlightServiceServer::new(stub))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        (url, shutdown, task)
    }

    #[tokio::test]
    async fn flight_initial_metadata_carries_completeness() {
        for carrier in [
            None,
            Some("tier_scan_failed, cold_tier_unavailable"),
            Some(""),
            Some(" , "),
        ] {
            let http = MockServer::start().await;
            let (url, shutdown, task) = start(Stub {
                carrier,
                fail_stream: false,
                refuse_info: None,
            })
            .await;
            let result = client(http.uri(), Some(url))
                .execute_query_rows_with_completeness("SELECT count", HeaderMap::new())
                .await
                .unwrap();
            assert_eq!(result.data, [json!({"count": 7})]);
            let expected = match carrier {
                None => vec![],
                Some("tier_scan_failed, cold_tier_unavailable") => {
                    vec!["tier_scan_failed", "cold_tier_unavailable"]
                }
                Some(_) => vec!["empty_partial_causes"],
            };
            assert_eq!(result.partial_causes, expected);
            assert!(
                http.received_requests().await.unwrap().is_empty(),
                "Flight must answer without fallback"
            );
            shutdown.send(()).unwrap();
            task.await.unwrap();
        }
    }

    /// Once Flight SQL has accepted the query, nothing falls back to HTTP: a
    /// stream that fails part-way and a request the server refuses are both
    /// returned as errors, with their gRPC meaning kept.
    #[tokio::test]
    async fn no_fallback_after_submission() {
        for (stub, expected) in [
            (
                Stub {
                    carrier: Some("failed_attempt_only"),
                    fail_stream: true,
                    refuse_info: None,
                },
                "ConnectionError",
            ),
            (
                Stub {
                    carrier: None,
                    fail_stream: false,
                    refuse_info: Some(tonic::Code::Unauthenticated),
                },
                "Unauthenticated",
            ),
            (
                Stub {
                    carrier: None,
                    fail_stream: false,
                    refuse_info: Some(tonic::Code::PermissionDenied),
                },
                "PermissionDenied",
            ),
            (
                Stub {
                    carrier: None,
                    fail_stream: false,
                    refuse_info: Some(tonic::Code::InvalidArgument),
                },
                "SqlError",
            ),
        ] {
            let http = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"rows": []})))
                .expect(0)
                .mount(&http)
                .await;
            let (url, shutdown, task) = start(stub).await;
            let err = client(http.uri(), Some(url))
                .execute_query_rows_with_transport("SELECT count", HeaderMap::new())
                .await
                .expect_err("no fallback after submission");
            let kind = match &err {
                packdb_client::ClientError::SqlError(_) => "SqlError",
                packdb_client::ClientError::Unauthenticated(_) => "Unauthenticated",
                packdb_client::ClientError::PermissionDenied(_) => "PermissionDenied",
                packdb_client::ClientError::ConnectionError(_) => "ConnectionError",
                packdb_client::ClientError::ProtocolError(_) => "ProtocolError",
                _ => "other",
            };
            assert_eq!(kind, expected, "{err:?}");
            assert!(
                http.received_requests().await.unwrap().is_empty(),
                "HTTP must not be contacted after Flight SQL accepted the query"
            );
            shutdown.send(()).unwrap();
            task.await.unwrap();
        }
    }

    /// A refused Flight SQL connection falls back to HTTP once, and the answer
    /// says so.
    #[tokio::test]
    async fn a_refused_flight_connection_falls_back_and_reports_it() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let flight = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let http = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/sql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"rows": [{"count": 8}]}))
                    .insert_header("x-packdb-partial-causes", "cold_tier_unavailable"),
            )
            .expect(1)
            .mount(&http)
            .await;
        let answer = client(http.uri(), Some(flight))
            .execute_query_rows_with_transport("SELECT count", HeaderMap::new())
            .await
            .unwrap();
        assert_eq!(answer.transport, packdb_client::TransportUsed::Http);
        assert!(answer.fallback_reason.is_some());
        assert_eq!(answer.response.data, [json!({"count": 8})]);
        assert_eq!(answer.response.partial_causes, ["cold_tier_unavailable"]);
    }
}
