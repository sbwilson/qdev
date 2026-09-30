//! Model Context Protocol (MCP) server implementation per Story 4.5.
//!
//! Provides stdio-based MCP JSON-RPC 2.0 server exposing the 12 core operations
//! documented in CLI reference §7, installation into editor settings (`.claude/settings.json`,
//! `.cursor/mcp.json`), and diagnostic inspection for `qdev doctor`.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::{load_config, AnnotatedConfig, Config};
use crate::context::{build_context, ContextOptions, ContextPhase};
use crate::decision::{log_decision_with_store, DecisionInput, VALID_DECISION_TYPES};
use crate::dw::{add_deferred_work_with_store, validate_safety_risk, AddDeferredWorkInput};
use crate::envelope::{JsonEnvelope, JsonErrorEnvelope};
use crate::errors::QdevError;
use crate::gate::{
    execute_gate_set, resolve_gate_execution_order, GateRunOptions,
};
use crate::interactivity::Interactivity;
use crate::lease::claim_story;
use crate::next::{select_next, NextOptions, NextOwnerFilter};
use crate::query::{query_entity, query_list, GetResult, ListQueryOptions, QueryOptions};
use crate::schema::{EntityKind, PayloadKind};
use crate::scratch::{append_scratch_entry, read_scratch_entries, ScratchAppendPayload, ScratchReadPayload};
use crate::store::sqlite::SqliteStore;
use crate::store::FindingRecord;
use crate::transition::{StoryState, TransitionEngine, TransitionGateHook, TransitionOptions};
use crate::validate::{filter_by_changed, run_validation, sort_findings};
use crate::write::{resolve_author, resolve_entity_file, write_file_atomic};

/// JSON-RPC 2.0 request or notification structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 2.0 response structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".to_string(),
            data: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {}", method.into()),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }
}

/// Single item in MCP tool call response content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolContentItem {
    pub r#type: String,
    pub text: String,
}

/// Result returned from an MCP `tools/call` invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolContentItem>,
    pub is_error: bool,
}

impl CallToolResult {
    pub fn success(text: String) -> Self {
        Self {
            content: vec![ToolContentItem {
                r#type: "text".to_string(),
                text,
            }],
            is_error: false,
        }
    }

    pub fn error(error: &QdevError) -> Self {
        let envelope = JsonErrorEnvelope::from(error);
        let text = serde_json::to_string_pretty(&envelope).unwrap_or_else(|_| {
            format!("{{\"error\": \"{}\"}}", error.message())
        });
        Self {
            content: vec![ToolContentItem {
                r#type: "text".to_string(),
                text,
            }],
            is_error: true,
        }
    }
}

/// Advertised MCP tool definition in `tools/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

/// Installation report emitted by `qdev install mcp`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpInstallReport {
    pub targets: Vec<String>,
    pub installed_files: Vec<String>,
    pub command: String,
    pub args: Vec<String>,
}

/// Status report returned by MCP doctor inspection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpDoctorStatus {
    pub status: String,
    pub unavailable_reason: Option<String>,
    pub registered: bool,
    pub handshake_ok: bool,
    pub registered_targets: Vec<String>,
}

/// Stdio MCP server state.
#[derive(Debug, Clone)]
pub struct McpServer {
    workspace_root: PathBuf,
    annotated_config: AnnotatedConfig,
}

impl McpServer {
    /// Creates a server instance from root and config.
    pub fn new(workspace_root: PathBuf, config: Config) -> Self {
        let annotated_config = load_config(&workspace_root)
            .unwrap_or_else(|_| AnnotatedConfig::new(config, BTreeMap::new()));
        Self {
            workspace_root,
            annotated_config,
        }
    }

    /// Creates a server instance with an explicit `AnnotatedConfig`.
    pub fn with_annotated_config(workspace_root: PathBuf, annotated_config: AnnotatedConfig) -> Self {
        Self {
            workspace_root,
            annotated_config,
        }
    }

    /// Creates a server instance by inspecting the workspace root.
    pub fn from_workspace(workspace_root: &Path) -> Result<Self, QdevError> {
        let annotated_config = load_config(workspace_root)?;
        Ok(Self {
            workspace_root: workspace_root.to_path_buf(),
            annotated_config,
        })
    }

    /// Returns the 12 core tool definitions with JSON Schemas derived from CLI options and `PayloadKind`.
    pub fn tool_definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "get_entity".to_string(),
                description: "Get projection and metadata for a workspace entity by ID".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Entity ID (e.g. E12S4, E12, AD-1) or constraint path"
                        },
                        "expand": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional projections to expand (e.g. ['scratch', 'evidence'])"
                        }
                    },
                    "required": ["id"],
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::Story.schema_json()),
            },
            ToolDefinition {
                name: "list_entities".to_string(),
                description: "List entities of a specific kind with optional filters".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "description": "Entity kind to list (story, epic, decision, dw, etc.)"
                        },
                        "epic": {
                            "type": "string",
                            "description": "Filter by owning epic ID (e.g. E12)"
                        },
                        "status": {
                            "type": "string",
                            "description": "Filter by status"
                        },
                        "owner": {
                            "type": "string",
                            "description": "Filter by owner ('me' resolves to current active identity)"
                        },
                        "module": {
                            "type": "string",
                            "description": "Filter by target module ID"
                        },
                        "sprint": {
                            "type": "integer",
                            "description": "Filter by assigned sprint number"
                        },
                        "subject": {
                            "type": "string",
                            "description": "Filter decisions by subject entity ID"
                        },
                        "type": {
                            "type": "string",
                            "description": "Filter decisions by decision type"
                        },
                        "risk": {
                            "type": "string",
                            "description": "Filter deferred work by safety risk level"
                        }
                    },
                    "required": ["kind"],
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::List.schema_json()),
            },
            ToolDefinition {
                name: "context".to_string(),
                description: "Retrieve assembled context projection for a story and workflow phase".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Story ID (e.g. E12S4)"
                        },
                        "phase": {
                            "type": "string",
                            "enum": ["specify", "develop", "review"],
                            "description": "Workflow phase ('specify', 'develop', 'review')"
                        },
                        "budget": {
                            "type": "integer",
                            "description": "Optional token budget limit"
                        },
                        "stats": {
                            "type": "boolean",
                            "description": "Include section and token budget statistics"
                        }
                    },
                    "required": ["id", "phase"],
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::Context.schema_json()),
            },
            ToolDefinition {
                name: "next".to_string(),
                description: "Select the next actionable story in active sprint or backlog".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "sprint": {
                            "type": "integer",
                            "description": "Filter by sprint number"
                        },
                        "owner": {
                            "type": "string",
                            "description": "Filter by owner ('me' resolves to current identity)"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::Next.schema_json()),
            },
            ToolDefinition {
                name: "claim".to_string(),
                description: "Claim exclusive story lease for current developer/session".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "story_id": {
                            "type": "string",
                            "description": "Story ID to lease (e.g. E12S4)"
                        },
                        "author_type": {
                            "type": "string",
                            "description": "Author type ('human' or 'agent')"
                        },
                        "author_id": {
                            "type": "string",
                            "description": "Author identifier"
                        }
                    },
                    "required": ["story_id"],
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::Claim.schema_json()),
            },
            ToolDefinition {
                name: "transition".to_string(),
                description: "Transition story lifecycle state".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "story_id": {
                            "type": "string",
                            "description": "Story ID to transition (e.g. E12S4)"
                        },
                        "target_status": {
                            "type": "string",
                            "enum": ["draft", "ready", "in-progress", "review", "done", "superseded", "abandoned"],
                            "description": "Target lifecycle state ('draft', 'ready', 'in-progress', 'review', 'done', 'superseded', 'abandoned')"
                        },
                        "justification": {
                            "type": "string",
                            "description": "Justification for override or transition"
                        },
                        "if_version": {
                            "type": "integer",
                            "description": "Precondition check for story version"
                        },
                        "skip_gates": {
                            "type": "boolean",
                            "description": "Skip pre-transition gate validation"
                        }
                    },
                    "required": ["story_id", "target_status"],
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::Transition.schema_json()),
            },
            ToolDefinition {
                name: "scratch_append".to_string(),
                description: "Append a note or decision entry to a story's scratchpad".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "story_id": {
                            "type": "string",
                            "description": "Story ID (e.g. E12S4)"
                        },
                        "content": {
                            "type": "string",
                            "description": "Entry text content"
                        },
                        "kind": {
                            "type": "string",
                            "description": "Entry kind ('note', 'decision', 'tradeoff', 'transition')"
                        },
                        "author_type": {
                            "type": "string",
                            "enum": ["human", "agent"],
                            "description": "Author type ('human' or 'agent')"
                        },
                        "author_id": {
                            "type": "string",
                            "description": "Author identifier"
                        }
                    },
                    "required": ["story_id", "content"],
                    "additionalProperties": false
                }),
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "schema_version": { "type": "string" },
                        "story_id": { "type": "string" },
                        "seq": { "type": "integer" },
                        "at": { "type": "string" },
                        "author": { "type": "object" },
                        "kind": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["schema_version", "story_id", "seq", "at", "author", "kind", "text"]
                })),
            },
            ToolDefinition {
                name: "scratch_read".to_string(),
                description: "Read scratchpad entries for a story with optional budget limit".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "story_id": {
                            "type": "string",
                            "description": "Story ID (e.g. E12S4)"
                        },
                        "budget": {
                            "type": "integer",
                            "description": "Token budget limit"
                        },
                        "kind": {
                            "type": "string",
                            "description": "Filter entries by kind"
                        }
                    },
                    "required": ["story_id"],
                    "additionalProperties": false
                }),
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "schema_version": { "type": "string" },
                        "story_id": { "type": "string" },
                        "entries": { "type": "array" }
                    },
                    "required": ["schema_version", "story_id", "entries"]
                })),
            },
            ToolDefinition {
                name: "dw_add".to_string(),
                description: "Record deferred work item with safety risk evaluation".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Title of the deferred work"
                        },
                        "module": {
                            "type": "string",
                            "description": "Target module identifier"
                        },
                        "risk": {
                            "type": "string",
                            "enum": ["negligible", "acceptable_with_mitigation", "unacceptable"],
                            "description": "Safety risk level ('negligible', 'acceptable_with_mitigation', 'unacceptable')"
                        },
                        "rationale": {
                            "type": "string",
                            "description": "Rationale explaining the deferral"
                        },
                        "origin_story": {
                            "type": "string",
                            "description": "Originating story ID"
                        },
                        "author_type": {
                            "type": "string",
                            "enum": ["human", "agent"],
                            "description": "Author type ('human' or 'agent')"
                        },
                        "author_id": {
                            "type": "string",
                            "description": "Author identifier"
                        }
                    },
                    "required": ["title", "module", "risk"],
                    "additionalProperties": false
                }),
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "schema_version": { "type": "string" },
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "status": { "type": "string" },
                        "target_module": { "type": "string" },
                        "safety_risk": { "type": "string" },
                        "origin_story_id": { "type": ["string", "null"] },
                        "rationale": { "type": ["string", "null"] },
                        "gate": { "type": ["string", "null"] },
                        "resolution": { "type": ["string", "null"] },
                        "owners": { "type": ["array", "null"] }
                    },
                    "required": ["schema_version", "id", "title", "status", "target_module", "safety_risk"]
                })),
            },
            ToolDefinition {
                name: "decision_log".to_string(),
                description: "Record technical or architectural decision".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "topic": {
                            "type": "string",
                            "description": "Topic or title of the decision"
                        },
                        "ruling": {
                            "type": "string",
                            "description": "The decision ruling or outcome"
                        },
                        "type": {
                            "type": "string",
                            "enum": ["human_ruling", "agent_assumption", "cross_team_override", "pivot", "review_rejection"],
                            "description": "Decision type ('human_ruling', 'agent_assumption', 'cross_team_override', 'pivot', 'review_rejection')"
                        },
                        "subject_id": {
                            "type": "string",
                            "description": "Subject entity ID"
                        },
                        "author_type": {
                            "type": "string",
                            "enum": ["human", "agent"],
                            "description": "Author type ('human' or 'agent')"
                        },
                        "author_id": {
                            "type": "string",
                            "description": "Author ID"
                        }
                    },
                    "required": ["topic", "ruling", "type", "subject_id"],
                    "additionalProperties": false
                }),
                output_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "schema_version": { "type": "string" },
                        "id": { "type": "string" },
                        "topic": { "type": "string" },
                        "ruling": { "type": "string" },
                        "decision_type": { "type": "string" }
                    },
                    "required": ["schema_version", "id", "topic", "ruling", "decision_type"]
                })),
            },
            ToolDefinition {
                name: "gate_run".to_string(),
                description: "Execute verification gates (by ID, for a transition, or all)".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "Specific gate ID to run"
                        },
                        "all": {
                            "type": "boolean",
                            "description": "Run all configured gates"
                        },
                        "for_transition": {
                            "type": "string",
                            "description": "Run gates required for specified transition"
                        },
                        "story_id": {
                            "type": "string",
                            "description": "Associated story ID"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: Some(json!({
                    "anyOf": [
                        PayloadKind::GateRun.schema_json(),
                        PayloadKind::GateSet.schema_json()
                    ]
                })),
            },
            ToolDefinition {
                name: "validate".to_string(),
                description: "Execute workspace-wide validation checks".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "changed": {
                            "type": "boolean",
                            "description": "Validate only changed files against git baseline"
                        }
                    },
                    "additionalProperties": false
                }),
                output_schema: Some(PayloadKind::Validate.schema_json()),
            },
        ]
    }

    /// Handles a single incoming JSON-RPC 2.0 text line and returns the response string (if any).
    pub fn handle_request_str(&self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        let req: JsonRpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => {
                let err_resp = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: serde_json::Value::Null,
                    result: None,
                    error: Some(JsonRpcError::parse_error()),
                };
                return Some(serde_json::to_string(&err_resp).unwrap());
            }
        };

        let resp = self.handle_request(req)?;
        Some(serde_json::to_string(&resp).unwrap())
    }

    /// Dispatches a parsed JSON-RPC request to the appropriate handler.
    pub fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = match req.id {
            Some(ref id_val) => id_val.clone(),
            None => {
                // In JSON-RPC 2.0, requests without an id are notifications; never reply to notifications.
                return None;
            }
        };

        match req.method.as_str() {
            "initialize" => {
                let result = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {
                            "listChanged": false
                        }
                    },
                    "serverInfo": {
                        "name": "qdev",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                Some(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(result),
                    error: None,
                })
            }
            "ping" => Some(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: Some(json!({})),
                error: None,
            }),
            "tools/list" => {
                let tools = Self::tool_definitions();
                let result = json!({ "tools": tools });
                Some(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(result),
                    error: None,
                })
            }
            "tools/call" => {
                let params = req.params.unwrap_or(serde_json::Value::Null);
                let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                let call_result = self.execute_tool(tool_name, &arguments);
                Some(JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id,
                    result: Some(serde_json::to_value(call_result).unwrap()),
                    error: None,
                })
            }
            other => Some(JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id,
                result: None,
                error: Some(JsonRpcError::method_not_found(other)),
            }),
        }
    }

    /// Serves JSON-RPC requests over standard I/O streams until EOF.
    pub fn serve<R: BufRead, W: Write>(&self, mut reader: R, mut writer: W) -> std::io::Result<()> {
        let mut line = String::new();
        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line)?;
            if bytes_read == 0 {
                break;
            }

            if let Some(resp_str) = self.handle_request_str(&line) {
                writer.write_all(resp_str.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
            }
        }
        Ok(())
    }

    fn open_store(&self) -> Result<SqliteStore, QdevError> {
        let root = &self.workspace_root;
        if !root.join("qdev.toml").is_file() {
            return Err(QdevError::usage_error(format!(
                "Workspace at '{}' is not an initialized qdev workspace",
                root.display()
            )));
        }
        let cache_db_path = root
            .join(&self.annotated_config.config.storage.cache_dir)
            .join("cache.sqlite");
        SqliteStore::open(&cache_db_path)
    }

    /// Executes one of the 12 core operations with `Interactivity::NonInteractive`.
    pub fn execute_tool(&self, name: &str, args: &serde_json::Value) -> CallToolResult {
        let res = match name {
            "get_entity" => self.execute_get_entity(args),
            "list_entities" => self.execute_list_entities(args),
            "context" => self.execute_context(args),
            "next" => self.execute_next(args),
            "claim" => self.execute_claim(args),
            "transition" => self.execute_transition(args),
            "scratch_append" => self.execute_scratch_append(args),
            "scratch_read" => self.execute_scratch_read(args),
            "dw_add" => self.execute_dw_add(args),
            "decision_log" => self.execute_decision_log(args),
            "gate_run" => self.execute_gate_run(args),
            "validate" => self.execute_validate(args),
            other => Err(QdevError::usage_error(format!("Unknown tool '{}'", other))),
        };

        match res {
            Ok(json_payload) => CallToolResult::success(json_payload),
            Err(err) => CallToolResult::error(&err),
        }
    }

    fn execute_get_entity(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let id_val = args
            .get("id")
            .or_else(|| args.get("target"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'id'"))?;

        let mut expand_scratch = false;
        let mut expand_evidence = false;
        if let Some(expand) = args.get("expand") {
            if let Some(arr) = expand.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        match s.trim() {
                            "scratch" => expand_scratch = true,
                            "evidence" => expand_evidence = true,
                            "relations" | "constraints" | "" => {}
                            other => {
                                return Err(QdevError::usage_error(format!(
                                    "Unknown --expand value '{}', expected one of: relations, constraints, scratch, evidence",
                                    other
                                )));
                            }
                        }
                    }
                }
            } else if let Some(s) = expand.as_str() {
                match s.trim() {
                    "scratch" => expand_scratch = true,
                    "evidence" => expand_evidence = true,
                    "relations" | "constraints" | "" => {}
                    other => {
                        return Err(QdevError::usage_error(format!(
                            "Unknown --expand value '{}', expected one of: relations, constraints, scratch, evidence",
                            other
                        )));
                    }
                }
            }
        }

        let store = self.open_store()?;
        let query_opts = QueryOptions {
            expand_scratch,
            expand_evidence,
        };

        let result = query_entity(&store, None, id_val, &query_opts)?;
        let serialized = match result {
            GetResult::Entity(projection) => {
                let envelope = JsonEnvelope::new(*projection);
                serde_json::to_string_pretty(&envelope).map_err(|e| {
                    QdevError::infrastructure_failure("serialization_error", e.to_string())
                })?
            }
            GetResult::Constraint(constraint) => {
                let envelope = JsonEnvelope::new(constraint);
                serde_json::to_string_pretty(&envelope).map_err(|e| {
                    QdevError::infrastructure_failure("serialization_error", e.to_string())
                })?
            }
        };
        Ok(serialized)
    }

    fn execute_list_entities(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let kind_str = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'kind'"))?;

        let kind = EntityKind::from_str_loose(kind_str)?;
        let store = self.open_store()?;

        let mut query_opts = ListQueryOptions::new(kind);
        query_opts.epic_id = args.get("epic").and_then(|v| v.as_str()).map(String::from);
        query_opts.status = args.get("status").and_then(|v| v.as_str()).map(String::from);
        query_opts.module = args.get("module").and_then(|v| v.as_str()).map(String::from);
        query_opts.sprint = args.get("sprint").and_then(|v| v.as_i64());
        query_opts.subject = args.get("subject").and_then(|v| v.as_str()).map(String::from);
        query_opts.decision_type = args.get("type").and_then(|v| v.as_str()).map(String::from);
        query_opts.safety_risk = args.get("risk").and_then(|v| v.as_str()).map(String::from);

        if let Some(owner_val) = args.get("owner").and_then(|v| v.as_str()) {
            if owner_val.trim() == "me" {
                let author = resolve_author(None, None, &self.annotated_config, &self.workspace_root)?;
                query_opts.owner = Some(author.id);
            } else {
                query_opts.owner = Some(owner_val.trim().to_string());
            }
        }

        let projection = query_list(&store, &query_opts)?;
        #[derive(Serialize)]
        struct ListPayload {
            kind: String,
            items: Vec<crate::query::ListEntryProjection>,
        }
        let envelope = JsonEnvelope::new(ListPayload {
            kind: kind.as_str().to_string(),
            items: projection,
        });
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_context(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let id = args
            .get("id")
            .or_else(|| args.get("story_id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'id'"))?;

        let phase_str = args
            .get("phase")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'phase'"))?;

        let phase = ContextPhase::from_str_loose(phase_str)?;
        let budget = args.get("budget").and_then(|v| v.as_u64()).map(|b| b as u32);
        let stats = args.get("stats").and_then(|v| v.as_bool()).unwrap_or(false);

        let store = self.open_store()?;
        let options = ContextOptions {
            phase,
            budget,
            stats,
        };

        let payload = build_context(
            &self.workspace_root,
            &store,
            &self.annotated_config.config,
            &options,
            id,
        )?;
        let envelope = JsonEnvelope::new(payload);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_next(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let store = self.open_store()?;
        let author = resolve_author(None, None, &self.annotated_config, &self.workspace_root)?;

        let owner = args.get("owner").and_then(|v| v.as_str()).map(|o| {
            if o.trim() == "me" {
                NextOwnerFilter::Current
            } else {
                NextOwnerFilter::Literal(o.trim().to_string())
            }
        });

        let sprint = args.get("sprint").and_then(|v| v.as_i64());

        let options = NextOptions {
            workspace_root: &self.workspace_root,
            store: &store,
            config: &self.annotated_config.config,
            sprint,
            owner,
            author,
        };

        let selection = select_next(&options)?;
        let envelope = JsonEnvelope::new(selection);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_claim(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let story_id = args
            .get("story_id")
            .or_else(|| args.get("id"))
            .or_else(|| args.get("target"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'story_id'"))?;

        let author_type = args.get("author_type").and_then(|v| v.as_str());
        let author_id = args.get("author_id").and_then(|v| v.as_str());
        let author = resolve_author(author_type, author_id, &self.annotated_config, &self.workspace_root)?;

        let opt_store = self.open_store().ok();
        let lease = claim_story(
            &self.workspace_root,
            story_id,
            &author,
            Some(&self.annotated_config.config.storage),
            opt_store.as_ref().map(|s| s as &dyn crate::store::Store),
        )?;

        let envelope = JsonEnvelope::new(lease);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_transition(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let story_id = args
            .get("story_id")
            .or_else(|| args.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'story_id'"))?;

        let target_status = args
            .get("target_status")
            .or_else(|| args.get("status"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'target_status'"))?;

        StoryState::parse(target_status)?;

        let author_type = args.get("author_type").and_then(|v| v.as_str());
        let author_id = args.get("author_id").and_then(|v| v.as_str());
        let author = resolve_author(author_type, author_id, &self.annotated_config, &self.workspace_root)?;

        let justification = args.get("justification").and_then(|v| v.as_str()).map(String::from);
        let if_version = args.get("if_version").and_then(|v| v.as_u64());
        let skip_gates = args.get("skip_gates").and_then(|v| v.as_bool()).unwrap_or(false);

        let options = TransitionOptions {
            workspace_root: self.workspace_root.clone(),
            storage: Some(self.annotated_config.config.storage.clone()),
            entity_kind: "story".to_string(),
            story_id: story_id.to_string(),
            target_status: target_status.to_string(),
            justification,
            author,
            if_version,
            skip_gates,
            interactivity: Interactivity::NonInteractive,
        };

        let mut engine = TransitionEngine::new();
        engine.add_pre_hook(TransitionGateHook::new(self.annotated_config.config.clone()));
        let res = engine.transition(&options)?;

        let envelope = JsonEnvelope::new(res);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_scratch_append(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let story_id = args
            .get("story_id")
            .or_else(|| args.get("story"))
            .or_else(|| args.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'story_id'"))?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'content'"))?;

        let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("note");

        let (_entity_kind, canonical_story_id, _story_path) = resolve_entity_file(
            &self.workspace_root,
            Some(EntityKind::Story),
            story_id,
            Some(&self.annotated_config.config.storage),
        )?;

        let author_type = args.get("author_type").and_then(|v| v.as_str());
        let author_id = args.get("author_id").and_then(|v| v.as_str());
        let author = resolve_author(author_type, author_id, &self.annotated_config, &self.workspace_root)?;
        let opt_store = self.open_store().ok();
        let entry = append_scratch_entry(
            &self.workspace_root,
            Some(&self.annotated_config.config.storage),
            &canonical_story_id,
            Some(kind),
            content,
            &author,
            opt_store.as_ref().map(|s| s as &dyn crate::store::Store),
        )?;

        let payload = ScratchAppendPayload {
            story_id: canonical_story_id,
            seq: entry.seq,
            at: entry.at,
            author: entry.author,
            kind: entry.kind,
            text: entry.text,
        };
        let envelope = JsonEnvelope::new(payload);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_scratch_read(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let story_id = args
            .get("story_id")
            .or_else(|| args.get("story"))
            .or_else(|| args.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'story_id'"))?;

        let budget = args.get("budget").and_then(|v| v.as_u64()).map(|b| b as usize);
        let kind = args.get("kind").and_then(|v| v.as_str());

        let (_entity_kind, canonical_story_id, _story_path) = resolve_entity_file(
            &self.workspace_root,
            Some(EntityKind::Story),
            story_id,
            Some(&self.annotated_config.config.storage),
        )?;

        let entries = read_scratch_entries(
            &self.workspace_root,
            Some(&self.annotated_config.config.storage),
            &canonical_story_id,
        )?;

        let filtered_entries = if let Some(k) = kind {
            entries.into_iter().filter(|e| e.kind == k).collect()
        } else {
            entries
        };

        let filtered_entries = if let Some(budget) = budget {
            crate::scratch::filter_scratch_entries_by_budget(&filtered_entries, budget)
        } else {
            filtered_entries
        };

        let payload = ScratchReadPayload {
            story_id: canonical_story_id,
            entries: filtered_entries,
        };

        let envelope = JsonEnvelope::new(payload);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_dw_add(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'title'"))?;
        if title.trim().is_empty() {
            return Err(QdevError::usage_error("Title cannot be empty"));
        }

        let module = args
            .get("module")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'module'"))?;
        if module.trim().is_empty() {
            return Err(QdevError::usage_error("Module cannot be empty"));
        }

        let risk = args
            .get("risk")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'risk'"))?;
        validate_safety_risk(risk)?;

        let rationale = args.get("rationale").and_then(|v| v.as_str()).map(String::from);
        let origin_story = args.get("origin_story").and_then(|v| v.as_str()).map(String::from);

        let author_type = args.get("author_type").and_then(|v| v.as_str());
        let author_id = args.get("author_id").and_then(|v| v.as_str());
        let author = resolve_author(author_type, author_id, &self.annotated_config, &self.workspace_root)?;
        let opt_store = self.open_store().ok();

        let input = AddDeferredWorkInput {
            title: title.to_string(),
            target_module: module.to_string(),
            safety_risk: risk.to_string(),
            rationale,
            origin_story_id: origin_story,
            gate: None,
            owners: Vec::new(),
            author,
        };

        let payload = add_deferred_work_with_store(
            &self.workspace_root,
            &self.annotated_config.config,
            &input,
            opt_store.as_ref().map(|s| s as &dyn crate::store::Store),
        )?;

        let envelope = JsonEnvelope::new(payload);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_decision_log(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let topic = args
            .get("topic")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'topic'"))?;
        if topic.trim().is_empty() {
            return Err(QdevError::usage_error("--topic cannot be empty or whitespace-only"));
        }

        let ruling = args
            .get("ruling")
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'ruling'"))?;
        if ruling.trim().is_empty() {
            return Err(QdevError::usage_error("--ruling cannot be empty or whitespace-only"));
        }

        let decision_type = args
            .get("type")
            .or_else(|| args.get("decision_type"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'type'"))?;
        let trimmed_type = decision_type.trim();
        if !VALID_DECISION_TYPES.contains(&trimmed_type) {
            return Err(QdevError::usage_error(format!(
                "Invalid decision type '{}'; allowed types are: {}",
                trimmed_type,
                VALID_DECISION_TYPES.join(", ")
            )));
        }

        let subject_id = args
            .get("subject_id")
            .or_else(|| args.get("subject"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| QdevError::usage_error("Missing required argument 'subject_id'"))?
            .trim()
            .to_string();
        if subject_id.is_empty() {
            return Err(QdevError::usage_error("'subject_id' cannot be empty or whitespace-only"));
        }
        let validate_subject = true;

        let author_type = args.get("author_type").and_then(|v| v.as_str());
        let author_id = args.get("author_id").and_then(|v| v.as_str());
        let author = resolve_author(author_type, author_id, &self.annotated_config, &self.workspace_root)?;

        let opt_store = self.open_store().ok();
        let input = DecisionInput {
            subject_id,
            decision_type: trimmed_type.to_string(),
            topic: Some(topic.to_string()),
            context: None,
            ruling: ruling.to_string(),
            author,
            title: None,
            timestamp: None,
            validate_subject,
        };

        let payload = log_decision_with_store(
            &self.workspace_root,
            Some(&self.annotated_config.config.storage),
            &input,
            opt_store.as_ref().map(|s| s as &dyn crate::store::Store),
        )?;

        let envelope = JsonEnvelope::new(payload);
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_gate_run(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let id_opt = args.get("id").and_then(|v| v.as_str()).map(String::from);
        let all_opt = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let trans_opt = args.get("for_transition").and_then(|v| v.as_str()).map(String::from);
        let story_opt = args.get("story_id").or_else(|| args.get("story")).and_then(|v| v.as_str()).map(String::from);

        let count = (id_opt.is_some() as usize) + (all_opt as usize) + (trans_opt.is_some() as usize);
        if count > 1 {
            return Err(QdevError::usage_error(
                "Conflicting invocation: specify only one of gate ID, all, or for_transition",
            ));
        }
        if count == 0 {
            return Err(QdevError::usage_error(
                "Must specify a gate ID, all: true, or for_transition",
            ));
        }

        let options = GateRunOptions {
            story: story_opt,
            timeout_ms: None,
            ..Default::default()
        };

        let target_ids: Option<Vec<String>> = if let Some(ref gate_id) = id_opt {
            Some(vec![gate_id.clone()])
        } else if let Some(ref trans) = trans_opt {
            let matching: Vec<String> = self
                .annotated_config
                .config
                .gates
                .iter()
                .filter(|g| g.on_transition.contains(trans))
                .map(|g| g.id.clone())
                .collect();
            if matching.is_empty() {
                return Err(QdevError::usage_error(format!(
                    "No gates configured for transition '{}'",
                    trans
                )));
            }
            Some(matching)
        } else {
            None
        };

        let execution_order = resolve_gate_execution_order(
            &self.annotated_config.config.gates,
            target_ids.as_deref(),
        )?;

        let set_outcome = execute_gate_set(
            &self.workspace_root,
            &self.annotated_config.config,
            &execution_order,
            &options,
        )?;

        if let Some(ref target_id) = id_opt {
            if let Some(target_outcome) = set_outcome.outcomes.iter().find(|o| &o.gate_id == target_id) {
                let envelope = JsonEnvelope::new(target_outcome.to_payload());
                return serde_json::to_string_pretty(&envelope).map_err(|e| {
                    QdevError::infrastructure_failure("serialization_error", e.to_string())
                });
            }
        }

        let envelope = JsonEnvelope::new(set_outcome.to_payload());
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }

    fn execute_validate(&self, args: &serde_json::Value) -> Result<String, QdevError> {
        let changed = args.get("changed").and_then(|v| v.as_bool()).unwrap_or(false);
        let store = self.open_store()?;

        let mut findings = run_validation(&store, &self.workspace_root, &self.annotated_config.config)?;
        if changed {
            let changed_paths = crate::validate::git_changed_files(
                &self.workspace_root,
                &self.annotated_config.config.git.integration_branch,
            )?;
            findings = filter_by_changed(findings, &changed_paths);
        }
        sort_findings(&mut findings);

        #[derive(Serialize)]
        struct ValidatePayload {
            findings: Vec<FindingRecord>,
        }

        let envelope = JsonEnvelope::new(ValidatePayload { findings });
        serde_json::to_string_pretty(&envelope)
            .map_err(|e| QdevError::infrastructure_failure("serialization_error", e.to_string()))
    }
}

/// Helper to update or create an MCP configuration file preserving existing settings.
fn update_mcp_config_file(path: &Path) -> Result<(), QdevError> {
    let mut root_val = if path.is_file() {
        let content = std::fs::read_to_string(path).map_err(|e| {
            QdevError::infrastructure_failure("io_error", format!("Failed to read {}: {}", path.display(), e))
        })?;
        serde_json::from_str::<serde_json::Value>(&content).map_err(|e| {
            QdevError::infrastructure_failure(
                "corrupted_config",
                format!("Failed to parse {}: invalid JSON ({})", path.display(), e),
            )
        })?
    } else {
        json!({})
    };

    if !root_val.is_object() {
        return Err(QdevError::infrastructure_failure(
            "corrupted_config",
            format!("Root of {} must be a JSON object", path.display()),
        ));
    }

    let map = root_val.as_object_mut().unwrap();
    let mcp_servers_entry = map.entry("mcpServers").or_insert_with(|| json!({}));
    if !mcp_servers_entry.is_object() {
        *mcp_servers_entry = json!({});
    }

    let mcp_servers_map = mcp_servers_entry.as_object_mut().unwrap();
    mcp_servers_map.insert(
        "qdev".to_string(),
        json!({
            "command": "qdev",
            "args": ["mcp", "serve"]
        }),
    );

    let serialized = serde_json::to_string_pretty(&root_val).map_err(|e| {
        QdevError::infrastructure_failure("serialization_error", e.to_string())
    })?;

    write_file_atomic(path, &format!("{}\n", serialized))
}

/// Installs MCP server entries into `.claude/settings.json` and/or `.cursor/mcp.json`.
pub fn install_mcp(
    workspace_root: &Path,
    claude: bool,
    cursor: bool,
) -> Result<McpInstallReport, QdevError> {
    if !claude && !cursor {
        return Err(QdevError::usage_error(
            "At least one target flag (--claude or --cursor) must be provided",
        ));
    }

    let mut targets = Vec::new();
    let mut installed_files = Vec::new();

    if claude {
        let claude_settings_path = workspace_root.join(".claude").join("settings.json");
        update_mcp_config_file(&claude_settings_path)?;
        targets.push("claude".to_string());
        installed_files.push(".claude/settings.json".to_string());
    }

    if cursor {
        let cursor_mcp_path = workspace_root.join(".cursor").join("mcp.json");
        update_mcp_config_file(&cursor_mcp_path)?;
        targets.push("cursor".to_string());
        installed_files.push(".cursor/mcp.json".to_string());
    }

    Ok(McpInstallReport {
        targets,
        installed_files,
        command: "qdev".to_string(),
        args: vec!["mcp".to_string(), "serve".to_string()],
    })
}

fn is_qdev_mcp_entry(qdev: &serde_json::Value) -> bool {
    let cmd_ok = qdev.get("command").and_then(|c| c.as_str()) == Some("qdev");
    let args_ok = qdev.get("args").and_then(|a| a.as_array()).map_or(false, |arr| {
        let has_mcp = arr.iter().any(|v| v.as_str() == Some("mcp"));
        let has_serve = arr.iter().any(|v| v.as_str() == Some("serve"));
        has_mcp && has_serve
    });
    cmd_ok && args_ok
}

/// Inspects MCP registration and performs an in-memory handshake for `qdev doctor`.
pub fn inspect_mcp(workspace_root: &Path) -> Result<McpDoctorStatus, QdevError> {
    inspect_mcp_impl(workspace_root, None)
}

#[doc(hidden)]
pub fn inspect_mcp_with_handshake_override(
    workspace_root: &Path,
    handshake_ok: bool,
) -> Result<McpDoctorStatus, QdevError> {
    inspect_mcp_impl(workspace_root, Some(handshake_ok))
}

fn inspect_mcp_impl(
    workspace_root: &Path,
    handshake_override: Option<bool>,
) -> Result<McpDoctorStatus, QdevError> {
    let mut registered_targets = Vec::new();

    // Check Claude
    let claude_path = workspace_root.join(".claude").join("settings.json");
    if claude_path.is_file() {
        match std::fs::read_to_string(&claude_path) {
            Ok(content) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(mcp_servers) = val.get("mcpServers").and_then(|s| s.as_object()) {
                        if let Some(qdev) = mcp_servers.get("qdev") {
                            if is_qdev_mcp_entry(qdev) {
                                registered_targets.push("claude".to_string());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                return Ok(McpDoctorStatus {
                    status: "unavailable".to_string(),
                    unavailable_reason: Some(format!("io_error: {}", e)),
                    registered: false,
                    handshake_ok: false,
                    registered_targets: Vec::new(),
                });
            }
        }
    }

    // Check Cursor
    let cursor_path = workspace_root.join(".cursor").join("mcp.json");
    if cursor_path.is_file() {
        match std::fs::read_to_string(&cursor_path) {
            Ok(content) => {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(mcp_servers) = val.get("mcpServers").and_then(|s| s.as_object()) {
                        if let Some(qdev) = mcp_servers.get("qdev") {
                            if is_qdev_mcp_entry(qdev) {
                                registered_targets.push("cursor".to_string());
                            }
                        }
                    }
                }
            }
            Err(e) => {
                return Ok(McpDoctorStatus {
                    status: "unavailable".to_string(),
                    unavailable_reason: Some(format!("io_error: {}", e)),
                    registered: false,
                    handshake_ok: false,
                    registered_targets: Vec::new(),
                });
            }
        }
    }

    // Perform handshake check
    let handshake_ok = if let Some(forced) = handshake_override {
        forced
    } else {
        let handshake_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"doctor","version":"1.0"}}}"#;
        let server = McpServer::new(workspace_root.to_path_buf(), Config::default());
        match server.handle_request_str(handshake_req) {
            Some(resp_str) => {
                if let Ok(resp) = serde_json::from_str::<serde_json::Value>(&resp_str) {
                    resp.get("result")
                        .and_then(|r| r.get("serverInfo"))
                        .and_then(|s| s.get("name"))
                        .and_then(|n| n.as_str())
                        == Some("qdev")
                } else {
                    false
                }
            }
            None => false,
        }
    };

    let registered = !registered_targets.is_empty();
    let (status, unavailable_reason) = if registered && handshake_ok {
        ("ok".to_string(), None)
    } else if registered && !handshake_ok {
        ("unavailable".to_string(), Some("handshake_failed".to_string()))
    } else {
        ("unregistered".to_string(), None)
    };

    Ok(McpDoctorStatus {
        status,
        unavailable_reason,
        registered,
        handshake_ok,
        registered_targets,
    })
}
