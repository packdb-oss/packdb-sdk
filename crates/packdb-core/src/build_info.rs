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

//! Build information for PackDb
//!
//! This module provides access to build-time information such as version,
//! git commit, build date, etc. This information is collected at compile
//! time and embedded in the binary.

use serde::Serialize;
use std::sync::OnceLock;

/// Build information captured at compile time
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    ///   Version from Cargo.toml
    pub version: String,
    ///   Git commit hash
    pub revision: String,
    ///   Git branch
    pub branch: String,
    ///   Build user or CI system
    pub build_user: String,
    ///   Build date in RFC3339 format
    pub build_date: String,
    ///   Rust compiler version (reported as goVersion for Prometheus compatibility)
    pub go_version: String,
}

// Static instance of build info
static BUILD_INFO: OnceLock<BuildInfo> = OnceLock::new();

impl BuildInfo {
    ///   Get the build information
    pub fn get() -> &'static BuildInfo {
        BUILD_INFO.get_or_init(|| BuildInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            revision: option_env!("GIT_COMMIT").unwrap_or("unknown").to_string(),
            branch: option_env!("GIT_BRANCH").unwrap_or("unknown").to_string(),
            build_user: option_env!("BUILD_USER").unwrap_or("unknown").to_string(),
            build_date: option_env!("BUILD_DATE").unwrap_or("unknown").to_string(),
            // Report Rust version as goVersion for Prometheus compatibility
            go_version: format!("rustc-{}", env!("RUSTC_VERSION")),
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::float_cmp
)]
mod tests {
    use super::*;

    #[test]
    fn test_build_info_singleton() {
        let info1 = BuildInfo::get();
        let info2 = BuildInfo::get();
        assert!(std::ptr::eq(info1, info2));
    }

    #[test]
    fn test_build_info_fields() {
        let info = BuildInfo::get();
        assert!(!info.version.is_empty());
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.go_version.starts_with("rustc-"));
    }

    #[test]
    fn test_build_info_serialization() {
        let info = BuildInfo::get();
        let json = serde_json::to_string(info).expect("Failed to serialize");

        assert!(json.contains("\"version\":"));
        assert!(json.contains("\"revision\":"));
        assert!(json.contains("\"branch\":"));
        assert!(json.contains("\"buildUser\":"));
        assert!(json.contains("\"buildDate\":"));
        assert!(json.contains("\"goVersion\":"));
    }

    #[test]
    fn test_build_info_clone() {
        let info = BuildInfo::get();
        let cloned = info.clone();

        assert_eq!(cloned.version, info.version);
        assert_eq!(cloned.revision, info.revision);
        assert_eq!(cloned.branch, info.branch);
        assert_eq!(cloned.build_user, info.build_user);
        assert_eq!(cloned.build_date, info.build_date);
        assert_eq!(cloned.go_version, info.go_version);
    }

    #[test]
    fn test_build_info_debug_format() {
        let info = BuildInfo::get();
        let debug_str = format!("{info:?}");

        assert!(debug_str.contains("BuildInfo"));
        assert!(debug_str.contains("version"));
        assert!(debug_str.contains("revision"));
        assert!(debug_str.contains("branch"));
    }
}
