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

//! PackDB client library.
//!
//! Provides caller-supplied credential transport and error types, plus an
//! optional SQL client behind the `sql` feature. Credential issuance,
//! validation, tenant resolution, and control-plane access are outside this
//! public crate.

pub mod auth;
pub mod diagnostics;
pub mod error;
pub mod outcome;

pub mod completeness;
pub use completeness::{QueryResponse, partial_causes_of};

pub mod credentials;

#[cfg(feature = "sql")]
pub mod sql;

pub use auth::{AuthConfig, AuthMethod};
pub use diagnostics::best_effort_response_body;
pub use error::{ClientError, ClientResult};
pub use outcome::ClientOutcome;

#[cfg(feature = "sql")]
pub use sql::{
    OutputFormat, QueryFailure, RowsAnswer, SqlClient, SqlClientConfig, SqlJsonResponse,
    SqlTransport, TransportUsed, execute_sql_http,
};
