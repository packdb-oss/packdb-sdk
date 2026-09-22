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

//! Process outcome handling shared by service binaries.

use std::fmt::Display;
use std::process::ExitCode;

/// A service's completed result, reduced to logging and process-exit behavior.
///
/// Returning this outcome's [`ExitCode`] from `main` preserves normal stack
/// unwinding, so shutdown guards can flush telemetry on failure. Calling
/// `std::process::exit` would skip every destructor in the process.
#[derive(Debug, PartialEq, Eq)]
pub struct ServiceOutcome {
    error: Option<String>,
}

impl ServiceOutcome {
    /// Preserve a service error for logging while normalizing the exit status.
    pub fn from_result<E: Display>(result: Result<(), E>) -> Self {
        Self {
            error: result.err().map(|error| error.to_string()),
        }
    }

    /// The rendered service error, if startup or serving failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Return success for `Ok(())` and failure for any service error.
    pub fn exit_code(&self) -> ExitCode {
        if self.error.is_none() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_has_no_error_and_exits_zero() {
        let outcome = ServiceOutcome::from_result::<&str>(Ok(()));

        assert_eq!(outcome.error(), None);
        assert_eq!(outcome.exit_code(), ExitCode::SUCCESS);
    }

    #[test]
    fn failure_preserves_its_message_and_exits_one() {
        let outcome = ServiceOutcome::from_result(Err("database unavailable"));

        assert_eq!(outcome.error(), Some("database unavailable"));
        assert_eq!(outcome.exit_code(), ExitCode::FAILURE);
    }
}
