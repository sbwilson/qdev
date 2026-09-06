use serde::{Deserialize, Serialize};

use crate::errors::QdevError;

pub const SCHEMA_VERSION: &str = "1";

/// Universal success JSON envelope per AD-13.
/// Carries schema_version = "1" and flattens data fields into the top-level object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonEnvelope<T> {
    pub schema_version: String,
    #[serde(flatten)]
    pub data: T,
}

impl<T> JsonEnvelope<T> {
    pub fn new(data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            data,
        }
    }

    pub fn with_schema_version(schema_version: impl Into<String>, data: T) -> Self {
        Self {
            schema_version: schema_version.into(),
            data,
        }
    }
}

/// Universal error JSON envelope per AD-13.
/// Emitted on stdout in JSON mode with a non-zero exit code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonErrorEnvelope {
    pub schema_version: String,
    pub error: ErrorPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl JsonErrorEnvelope {
    pub fn new(
        code: impl Into<String>,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            error: ErrorPayload {
                code: code.into(),
                message: message.into(),
                details,
            },
        }
    }

    pub fn from_error(error: &QdevError) -> Self {
        Self::new(error.code(), error.message(), error.details().cloned())
    }
}

impl From<&QdevError> for JsonErrorEnvelope {
    fn from(error: &QdevError) -> Self {
        Self::from_error(error)
    }
}

impl From<QdevError> for JsonErrorEnvelope {
    fn from(error: QdevError) -> Self {
        Self::from_error(&error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
    struct SampleData {
        version: String,
    }

    #[test]
    fn test_success_envelope_serialization() {
        let envelope = JsonEnvelope::new(SampleData {
            version: "0.1.0".to_string(),
        });
        let serialized = serde_json::to_string(&envelope).unwrap();
        let val: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(val["schema_version"], "1");
        assert_eq!(val["version"], "0.1.0");
    }

    #[test]
    fn test_success_envelope_with_json_value() {
        let val = serde_json::json!({
            "title": "Story",
            "type": "object"
        });
        let envelope = JsonEnvelope::new(val);
        let serialized = serde_json::to_string(&envelope).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed["schema_version"], "1");
        assert_eq!(parsed["title"], "Story");
        assert_eq!(parsed["type"], "object");
    }

    #[test]
    fn test_error_envelope_serialization_without_details() {
        let envelope = JsonErrorEnvelope::new("usage_error", "unrecognized subcommand", None);
        let serialized = serde_json::to_string(&envelope).unwrap();
        let val: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(val["schema_version"], "1");
        assert_eq!(val["error"]["code"], "usage_error");
        assert_eq!(val["error"]["message"], "unrecognized subcommand");
        assert!(val["error"].get("details").is_none());
    }

    #[test]
    fn test_error_envelope_serialization_with_details() {
        let details = json!({"key": "value"});
        let envelope = JsonErrorEnvelope::new("conflict", "lock timeout", Some(details));
        let serialized = serde_json::to_string(&envelope).unwrap();
        let val: serde_json::Value = serde_json::from_str(&serialized).unwrap();

        assert_eq!(val["schema_version"], "1");
        assert_eq!(val["error"]["code"], "conflict");
        assert_eq!(val["error"]["message"], "lock timeout");
        assert_eq!(val["error"]["details"]["key"], "value");
    }

    #[test]
    fn test_deserialization() {
        let raw = r#"{"schema_version":"1","error":{"code":"policy_refusal","message":"denied"}}"#;
        let env: JsonErrorEnvelope = serde_json::from_str(raw).unwrap();
        assert_eq!(env.schema_version, "1");
        assert_eq!(env.error.code, "policy_refusal");
        assert_eq!(env.error.message, "denied");
        assert!(env.error.details.is_none());
    }
}
