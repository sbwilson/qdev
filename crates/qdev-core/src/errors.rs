use std::fmt;

/// Standard exit codes conforming strictly to AD-13.
/// 0: Success
/// 1: Logical failure (gate failed, validation findings, hygiene violations)
/// 2: Usage error (invalid flag, unknown command, malformed input)
/// 3: Refused by policy (preflight, lease, governance, missing confirmation)
/// 4: Infrastructure failure (timeout, missing tool, cache corruption, internal panic)
/// 5: Conflict (version mismatch, lease held by another holder, lock timeout)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum ExitCode {
    Success = 0,
    LogicalFailure = 1,
    UsageError = 2,
    PolicyRefusal = 3,
    InfrastructureFailure = 4,
    Conflict = 5,
}

impl ExitCode {
    pub fn as_i32(&self) -> i32 {
        *self as i32
    }

    pub fn is_success(&self) -> bool {
        matches!(self, ExitCode::Success)
    }
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> Self {
        code.as_i32()
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> Self {
        std::process::ExitCode::from(code.as_i32() as u8)
    }
}

impl TryFrom<i32> for ExitCode {
    type Error = i32;

    fn try_from(val: i32) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(ExitCode::Success),
            1 => Ok(ExitCode::LogicalFailure),
            2 => Ok(ExitCode::UsageError),
            3 => Ok(ExitCode::PolicyRefusal),
            4 => Ok(ExitCode::InfrastructureFailure),
            5 => Ok(ExitCode::Conflict),
            other => Err(other),
        }
    }
}

impl fmt::Display for ExitCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_i32())
    }
}

/// Core error taxonomy for qdev conforming to AD-13.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QdevError {
    pub exit_code: ExitCode,
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl fmt::Display for QdevError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for QdevError {}

impl QdevError {
    pub fn new(exit_code: ExitCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            exit_code,
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    /// Construct a usage error (ExitCode 2)
    pub fn usage_error(message: impl Into<String>) -> Self {
        Self::new(ExitCode::UsageError, "usage_error", message)
    }

    /// Construct a usage error with custom code (ExitCode 2)
    pub fn usage_error_with_code(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::UsageError, code, message)
    }

    /// Construct a logical failure (ExitCode 1)
    pub fn logical_failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::LogicalFailure, code, message)
    }

    /// Construct a policy refusal / needs confirmation error (ExitCode 3)
    pub fn policy_refusal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::PolicyRefusal, code, message)
    }

    /// Construct an infrastructure failure error (ExitCode 4)
    pub fn infrastructure_failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::InfrastructureFailure, code, message)
    }

    /// Construct a conflict error (ExitCode 5)
    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::Conflict, code, message)
    }

    pub fn exit_code(&self) -> ExitCode {
        self.exit_code
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn details(&self) -> Option<&serde_json::Value> {
        self.details.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_exit_code_values() {
        assert_eq!(ExitCode::Success.as_i32(), 0);
        assert_eq!(ExitCode::LogicalFailure.as_i32(), 1);
        assert_eq!(ExitCode::UsageError.as_i32(), 2);
        assert_eq!(ExitCode::PolicyRefusal.as_i32(), 3);
        assert_eq!(ExitCode::InfrastructureFailure.as_i32(), 4);
        assert_eq!(ExitCode::Conflict.as_i32(), 5);

        assert_eq!(ExitCode::try_from(0), Ok(ExitCode::Success));
        assert_eq!(ExitCode::try_from(2), Ok(ExitCode::UsageError));
        assert_eq!(ExitCode::try_from(6), Err(6));
    }

    #[test]
    fn test_qdev_error_constructors() {
        let err = QdevError::usage_error("invalid command");
        assert_eq!(err.exit_code(), ExitCode::UsageError);
        assert_eq!(err.code(), "usage_error");
        assert_eq!(err.message(), "invalid command");
        assert!(err.details().is_none());

        let err_with_details = QdevError::conflict("lock_timeout", "failed to acquire lock")
            .with_details(json!({"lock_file": "/path/to/lock"}));
        assert_eq!(err_with_details.exit_code(), ExitCode::Conflict);
        assert_eq!(err_with_details.code(), "lock_timeout");
        assert_eq!(
            err_with_details.details().unwrap()["lock_file"],
            "/path/to/lock"
        );
    }
}
