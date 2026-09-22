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

//! Single-node query and response wire types.

pub mod loki_response;
pub mod prometheus_response;
pub mod response;
pub use loki_response::{
    ENCODING_FLAG_CATEGORIZE_LABELS, ENCODING_FLAGS_HEADER, EntryEncoding, LokiIndexStats,
    LokiMatrixSeries, LokiQueryData, LokiQueryResult, LokiResponse, LokiStream, LokiVectorSample,
    MetricValue, negotiates_categorize_labels,
};
pub use prometheus_response::{PrometheusApiResponse, QueryData, QueryResult};
pub use response::{ApiError, ApiErrorCode, ApiResponse, StructuredResponse};
