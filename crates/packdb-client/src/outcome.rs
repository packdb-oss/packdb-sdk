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

//! Process output shared by command-line clients.

use crate::error::ClientResult;

/// A completed client command reduced to its process-facing contract.
///
/// Query and administrative CLIs share the same rule: successful output is
/// forwarded byte-for-byte to stdout, while a client error is rendered on
/// stderr and retains its documented client exit code.
#[derive(Debug, PartialEq, Eq)]
pub struct ClientOutcome {
    stdout: String,
    stderr: String,
    exit_code: i32,
}

impl ClientOutcome {
    /// Convert a rendered client result into stream contents and an exit code.
    pub fn from_result(result: ClientResult<String>) -> Self {
        match result {
            Ok(output) => Self {
                stdout: output,
                stderr: String::new(),
                exit_code: 0,
            },
            Err(error) => Self {
                stdout: String::new(),
                stderr: format!("ERROR: {error}\n"),
                exit_code: error.legacy_exit_code(),
            },
        }
    }

    /// Bytes to write to standard output.
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    /// Bytes to write to standard error.
    pub fn stderr(&self) -> &str {
        &self.stderr
    }

    /// Process exit code defined by the shared client error contract.
    pub fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ClientError;

    #[test]
    fn success_is_forwarded_byte_for_byte_to_stdout() {
        let outcome = ClientOutcome::from_result(Ok("no-trailing-newline".to_string()));

        assert_eq!(outcome.stdout(), "no-trailing-newline");
        assert_eq!(outcome.stderr(), "");
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn every_client_error_keeps_its_message_and_exit_class() {
        for (error, expected_code) in [
            (ClientError::SqlError("bad sql".into()), 1),
            (ClientError::ConnectionError("refused".into()), 2),
            (ClientError::ProtocolError("garbled".into()), 3),
            (ClientError::ConfigError("bad flag".into()), 2),
        ] {
            let rendered = error.to_string();
            let outcome = ClientOutcome::from_result(Err(error));

            assert_eq!(outcome.stdout(), "");
            assert_eq!(outcome.stderr(), format!("ERROR: {rendered}\n"));
            assert_eq!(outcome.exit_code(), expected_code);
        }
    }
}
