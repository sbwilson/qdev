use std::fmt;
use serde::{Deserialize, Serialize};

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

/// Structured rejection attribution payload conforming to AD-13 and FR-304.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RejectionAttribution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    pub rule: String,
}

impl RejectionAttribution {
    pub fn new(rule: impl Into<String>) -> Self {
        Self {
            constraint_id: None,
            gate_id: None,
            policy: None,
            blocking_ids: None,
            holder: None,
            rule: rule.into(),
        }
    }

    pub fn with_constraint_id(mut self, constraint_id: impl Into<String>) -> Self {
        self.constraint_id = Some(constraint_id.into());
        self
    }

    pub fn with_gate_id(mut self, gate_id: impl Into<String>) -> Self {
        self.gate_id = Some(gate_id.into());
        self
    }

    pub fn with_policy(mut self, policy: impl Into<String>) -> Self {
        self.policy = Some(policy.into());
        self
    }

    pub fn with_blocking_ids(mut self, blocking_ids: Vec<String>) -> Self {
        self.blocking_ids = Some(blocking_ids);
        self
    }

    pub fn with_holder(mut self, holder: impl Into<String>) -> Self {
        self.holder = Some(holder.into());
        self
    }

    /// Verifies that at least one attribution key is present in addition to `rule`.
    pub fn has_attribution(&self) -> bool {
        self.constraint_id.is_some()
            || self.gate_id.is_some()
            || self.policy.is_some()
            || self.blocking_ids.as_ref().is_some_and(|b| !b.is_empty())
            || self.holder.is_some()
    }

    /// Injects attribution fields into a JSON Value.
    pub fn apply_to_value(&self, value: &mut serde_json::Value) {
        if !value.is_object() {
            let mut obj = serde_json::Map::new();
            if !value.is_null() {
                obj.insert("details".to_string(), value.clone());
            }
            *value = serde_json::Value::Object(obj);
        }
        if let serde_json::Value::Object(map) = value {
            if let Some(cid) = &self.constraint_id {
                map.insert("constraint_id".to_string(), serde_json::Value::String(cid.clone()));
            }
            if let Some(gid) = &self.gate_id {
                map.insert("gate_id".to_string(), serde_json::Value::String(gid.clone()));
            }
            if let Some(pol) = &self.policy {
                map.insert("policy".to_string(), serde_json::Value::String(pol.clone()));
            }
            if let Some(bids) = &self.blocking_ids {
                map.insert("blocking_ids".to_string(), serde_json::json!(bids));
            }
            if let Some(h) = &self.holder {
                map.insert("holder".to_string(), serde_json::Value::String(h.clone()));
            }
            map.insert("rule".to_string(), serde_json::Value::String(self.rule.clone()));
        }
    }

    /// Attaches this attribution to a QdevError.
    pub fn apply_to_error(self, err: QdevError) -> QdevError {
        err.with_attribution(self)
    }
}

/// Catalog entry describing a standard refusal error code and its expected attribution fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusalCatalogEntry {
    pub code: &'static str,
    pub exit_code: ExitCode,
    pub default_rule: &'static str,
    pub required_fields: &'static [&'static str],
}

/// Canonical catalog of refusal error codes used across the binary.
pub static REFUSAL_CATALOG: &[RefusalCatalogEntry] = &[
    RefusalCatalogEntry {
        code: "gate_failed",
        exit_code: ExitCode::LogicalFailure,
        default_rule: "Verification gates must pass before proceeding",
        required_fields: &["gate_id", "rule"],
    },
    RefusalCatalogEntry {
        code: "gate_infra_failure",
        exit_code: ExitCode::InfrastructureFailure,
        default_rule: "Gate execution infrastructure must succeed without timeouts or system errors",
        required_fields: &["gate_id", "rule"],
    },
    RefusalCatalogEntry {
        code: "already_leased",
        exit_code: ExitCode::Conflict,
        default_rule: "Story leases are exclusive and may only be held by a single user or session",
        required_fields: &["holder", "policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "lease_not_found",
        exit_code: ExitCode::LogicalFailure,
        default_rule: "Active story lease required to perform lease operations",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "needs_justification",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Actions requiring override or non-linear transition require explicit justification",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "needs_confirmation",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Modifications outside active lease or across team ownership boundaries require confirmation and override",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "story_blocked",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Stories may not start until all blocking dependencies are done",
        required_fields: &["blocking_ids", "policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "unacceptable_deferred_work",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Sprints cannot close with open unacceptable safety risk deferred work lacking rationale",
        required_fields: &["blocking_ids", "policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "active_story_lease",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Sprints cannot close while stories in progress hold active leases",
        required_fields: &["holder", "policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "sprint_already_closed",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Terminal sprints cannot be closed again",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "sprint_not_active",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Only active sprints can be closed",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "carry_over_not_allowed",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Story carry-over is only permitted when completing a sprint",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "tty_required",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Gate skipping requires an interactive TTY session",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "human_required",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Gate skipping is restricted to human authors",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "preflight_refusal",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Working tree modifications must remain within active story scope before push",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "scope_violation",
        exit_code: ExitCode::LogicalFailure,
        default_rule: "Modified files must be within story target modules or covered by approved no-go constraints",
        required_fields: &["gate_id", "rule"],
    },
    RefusalCatalogEntry {
        code: "policy_refusal",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Action refused by governance policy",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "close_reason_required",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Closing a sprint as paused or abandoned requires a non-empty reason",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "lease_held",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Chore execution is refused when worktree holds an active story lease",
        required_fields: &["policy", "rule"],
    },
    RefusalCatalogEntry {
        code: "out_of_allowlist",
        exit_code: ExitCode::PolicyRefusal,
        default_rule: "Chore execution modified paths outside configured chore allowlist in strict mode",
        required_fields: &["policy", "rule"],
    },
];

pub fn get_refusal_entry(code: &str) -> Option<&'static RefusalCatalogEntry> {
    REFUSAL_CATALOG.iter().find(|e| e.code == code)
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

    /// Attaches structured rejection attribution fields into `details`.
    /// Existing fields in `details` are preserved.
    pub fn with_attribution(mut self, attribution: RejectionAttribution) -> Self {
        let mut map = match self.details {
            Some(serde_json::Value::Object(map)) => map,
            Some(other) => {
                let mut map = serde_json::Map::new();
                map.insert("details".to_string(), other);
                map
            }
            None => serde_json::Map::new(),
        };
        if let Some(cid) = &attribution.constraint_id {
            map.insert("constraint_id".to_string(), serde_json::Value::String(cid.clone()));
        }
        if let Some(gid) = &attribution.gate_id {
            map.insert("gate_id".to_string(), serde_json::Value::String(gid.clone()));
        }
        if let Some(pol) = &attribution.policy {
            map.insert("policy".to_string(), serde_json::Value::String(pol.clone()));
        }
        if let Some(bids) = &attribution.blocking_ids {
            map.insert("blocking_ids".to_string(), serde_json::json!(bids));
        }
        if let Some(h) = &attribution.holder {
            map.insert("holder".to_string(), serde_json::Value::String(h.clone()));
        }
        map.insert("rule".to_string(), serde_json::Value::String(attribution.rule));
        self.details = Some(serde_json::Value::Object(map));
        self
    }

    /// Extracts attribution fields if present in `details`.
    pub fn attribution(&self) -> Option<RejectionAttribution> {
        let details = self.details.as_ref()?.as_object()?;
        let rule = details.get("rule")?.as_str()?.to_string();
        let constraint_id = details
            .get("constraint_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let gate_id = details
            .get("gate_id")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let policy = details
            .get("policy")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let blocking_ids = details
            .get("blocking_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            });
        let holder = details
            .get("holder")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        Some(RejectionAttribution {
            constraint_id,
            gate_id,
            policy,
            blocking_ids,
            holder,
            rule,
        })
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

    /// Construct an attributed logical failure (ExitCode 1)
    pub fn logical_failure_attributed(
        code: impl Into<String>,
        message: impl Into<String>,
        attribution: RejectionAttribution,
    ) -> Self {
        Self::logical_failure(code, message).with_attribution(attribution)
    }

    /// Construct a policy refusal / needs confirmation error (ExitCode 3)
    pub fn policy_refusal(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::PolicyRefusal, code, message)
    }

    /// Construct an attributed policy refusal (ExitCode 3)
    pub fn policy_refusal_attributed(
        code: impl Into<String>,
        message: impl Into<String>,
        attribution: RejectionAttribution,
    ) -> Self {
        Self::policy_refusal(code, message).with_attribution(attribution)
    }

    /// Construct an infrastructure failure error (ExitCode 4)
    pub fn infrastructure_failure(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::InfrastructureFailure, code, message)
    }

    /// Construct an attributed infrastructure failure error (ExitCode 4)
    pub fn infrastructure_failure_attributed(
        code: impl Into<String>,
        message: impl Into<String>,
        attribution: RejectionAttribution,
    ) -> Self {
        Self::infrastructure_failure(code, message).with_attribution(attribution)
    }

    /// Construct a conflict error (ExitCode 5)
    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(ExitCode::Conflict, code, message)
    }

    /// Construct an attributed conflict error (ExitCode 5)
    pub fn conflict_attributed(
        code: impl Into<String>,
        message: impl Into<String>,
        attribution: RejectionAttribution,
    ) -> Self {
        Self::conflict(code, message).with_attribution(attribution)
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

    #[test]
    fn test_rejection_attribution_builder_and_merge() {
        let attr = RejectionAttribution::new("Lease is exclusive")
            .with_holder("amelia")
            .with_policy("single_lease_holder");

        assert!(attr.has_attribution());

        let err = QdevError::conflict("already_leased", "Story is already leased")
            .with_details(json!({ "story_id": "E12S4" }))
            .with_attribution(attr.clone());

        assert_eq!(err.exit_code(), ExitCode::Conflict);
        let details = err.details().expect("details present");
        assert_eq!(details["story_id"], "E12S4");
        assert_eq!(details["holder"], "amelia");
        assert_eq!(details["policy"], "single_lease_holder");
        assert_eq!(details["rule"], "Lease is exclusive");

        let extracted = err.attribution().expect("attribution extracted");
        assert_eq!(extracted.holder.as_deref(), Some("amelia"));
        assert_eq!(extracted.policy.as_deref(), Some("single_lease_holder"));
        assert_eq!(extracted.rule, "Lease is exclusive");
    }

    #[test]
    fn test_refusal_catalog_contains_expected_codes() {
        let codes: Vec<&str> = REFUSAL_CATALOG.iter().map(|e| e.code).collect();
        assert!(codes.contains(&"gate_failed"));
        assert!(codes.contains(&"gate_infra_failure"));
        assert!(codes.contains(&"already_leased"));
        assert!(codes.contains(&"lease_not_found"));
        assert!(codes.contains(&"needs_justification"));
        assert!(codes.contains(&"needs_confirmation"));
        assert!(codes.contains(&"story_blocked"));
        assert!(codes.contains(&"unacceptable_deferred_work"));
        assert!(codes.contains(&"active_story_lease"));
        assert!(codes.contains(&"sprint_already_closed"));
        assert!(codes.contains(&"sprint_not_active"));
        assert!(codes.contains(&"carry_over_not_allowed"));
        assert!(codes.contains(&"tty_required"));
        assert!(codes.contains(&"human_required"));
        assert!(codes.contains(&"preflight_refusal"));
        assert!(codes.contains(&"scope_violation"));
    }
}

