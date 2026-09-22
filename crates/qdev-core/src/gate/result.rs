use serde::{Deserialize, Serialize};

use crate::gate::{GateFailure, GateStatus};
use crate::schema::validate_gate_result_detailed;

/// Structured Gate Result Document conforming to compliance & safety §2.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateResultDocument {
    pub status: GateStatus,
    pub summary: String,
    #[serde(default)]
    pub failures: Vec<GateFailure>,
    #[serde(default)]
    pub metric: Option<f64>,
    #[serde(default)]
    pub constraint_ids: Vec<String>,
}

impl GateResultDocument {
    /// Validates a parsed JSON value against `gate-result.json` and deserializes it.
    pub fn from_json_value(value: &serde_json::Value) -> Result<Self, Vec<String>> {
        let errors = validate_gate_result_detailed(value).err();
        if let Some(errs) = errors {
            return Err(errs.into_iter().map(|e| e.to_string()).collect());
        }

        serde_json::from_value(value.clone()).map_err(|e| {
            vec![format!(
                "Failed to deserialize validated gate result document: {}",
                e
            )]
        })
    }

    /// Parses a JSON string, validates it against `gate-result.json`, and deserializes it.
    pub fn from_json_str(json_str: &str) -> Result<Self, Vec<String>> {
        let value: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| vec![format!("Invalid JSON in gate result document: {}", e)])?;
        Self::from_json_value(&value)
    }
}
