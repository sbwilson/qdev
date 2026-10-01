---
title: 'Story 4.5: MCP Server'
type: 'feature'
created: '2026-09-30'
status: 'done'
route: 'dispatch'
baseline_commit: 'c3ddd34e30b9ecc2730f825d49070ea6a5374192'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** AI agent hosts (such as Claude Code, Cursor, and IDE extensions) require native tool interfaces via the Model Context Protocol (MCP) to interact with workspace entities, context projections, and gates without relying on custom shell scripts or ad-hoc file parsing.

**Approach:** Implement `qdev mcp serve` as a stdio-based MCP JSON-RPC 2.0 server exposing the 12 core operations documented in CLI reference §7 with JSON Schemas derived from Story 1.13, executing strictly with `Interactivity::NonInteractive` so refusals surface as structured error envelopes. Implement `qdev install mcp --claude|--cursor` to register the server in `.claude/settings.json` and `.cursor/mcp.json`, and add an `mcp` diagnostic section to `qdev doctor` verifying registration and handshake.

## Boundaries & Constraints

**Always:**
- Expose the 12 tools defined in CLI reference §7: `get_entity`, `list_entities`, `context`, `next`, `claim`, `transition`, `scratch_append`, `scratch_read`, `dw_add`, `decision_log`, `gate_run`, and `validate`.
- Serve over `stdio` using newline-delimited JSON-RPC 2.0 protocol conforming to the MCP specification (`initialize`, `notifications/initialized`, `tools/list`, `tools/call`).
- Provide JSON Schemas for each tool's input arguments in `tools/list` (`inputSchema`), matching CLI options, and advertise output payload schemas generated from Story 1.13's `PayloadKind`.
- Run every tool invocation with `Interactivity::NonInteractive`. On command error or policy refusal, return `isError: true` with text containing the structured JSON error envelope conforming to `payload-error.json`.
- Return the exact same JSON payload for successful tool calls as the CLI emits with `--json`.
- Implement `qdev install mcp --claude|--cursor` to safely register the `qdev` MCP server command (`command: "qdev"`, `args: ["mcp", "serve"]`) in `.claude/settings.json` and `.cursor/mcp.json`, preserving existing keys. Require at least one target flag (`--claude` or `--cursor`), exiting with code 2 if none provided.
- Support `qdev install mcp --json` returning a JSON envelope conforming to `payload-mcp-install.json`.
- Add `McpDoctorSection` to `default_doctor_sections` in `qdev-core`, verifying both MCP registration in `.claude/settings.json` / `.cursor/mcp.json` and performing a handshake (`initialize` exchange).
- Keep `qdev doctor` exit code 0 regardless of MCP status (diagnostic report, never a gate).

**Never:**
- Never perform network I/O or open network sockets; MCP transport is stdio-only with zero telemetry.
- Never prompt for interactive input during an MCP session; fail closed with structured error envelopes.
- Never overwrite unrelated configuration sections in `.claude/settings.json` or `.cursor/mcp.json`.
- Never bypass the `Store` or write path abstractions when executing mutations through MCP tools.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Initialize handshake | JSON-RPC `initialize` request on stdio | Returns protocol version, capabilities (`tools`), server info (`qdev` + version) | Exit 0 / Valid response |
| Tools listing | JSON-RPC `tools/list` request on stdio | Returns list of 12 advertised tools with input schemas and descriptions | Valid JSON-RPC response |
| Call `get_entity` success | `tools/call` with `{"name": "get_entity", "arguments": {"id": "E12S4"}}` | Returns entity payload in `content[0].text`, `isError: false` | Valid response |
| Call tool refusal | `tools/call` with `{"name": "claim", "arguments": {"story_id": "LOCKED"}}` | Returns JSON error envelope in `content[0].text`, `isError: true` | Structured error envelope |
| Unknown tool call | `tools/call` with `{"name": "unknown_tool", "arguments": {}}` | Returns JSON-RPC error code -32601 (Method not found) or tool error | `isError: true` |
| Malformed JSON input | Non-JSON text on stdio | Emits JSON-RPC parse error (-32700) | Handled gracefully without crash |
| Install Claude MCP | `qdev install mcp --claude` | Updates `.claude/settings.json` with `mcpServers.qdev` entry | Exit 0 |
| Install Cursor MCP | `qdev install mcp --cursor` | Updates `.cursor/mcp.json` with `mcpServers.qdev` entry | Exit 0 |
| Install both targets | `qdev install mcp --claude --cursor` | Updates both configuration files | Exit 0 |
| Install missing flag | `qdev install mcp` (no flags) | Refuses with usage error | Exit 2 usage error |
| Doctor with registered MCP | `qdev doctor` in workspace with `.claude/settings.json` registered | `mcp` section reports `status: "ok"`, `registered: true`, `handshake_ok: true` | Exit 0 |
| Doctor unregistered | `qdev doctor` in fresh workspace | `mcp` section reports `status: "unregistered"`, `registered: false`, `handshake_ok: true` | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/mcp.rs` -- Implement MCP JSON-RPC protocol types, tool catalog for the 12 operations (`get_entity`, `list_entities`, `context`, `next`, `claim`, `transition`, `scratch_append`, `scratch_read`, `dw_add`, `decision_log`, `gate_run`, `validate`), tool execution logic, installation logic (`install_mcp`), and inspection logic (`inspect_mcp`).
- `crates/qdev-core/src/doctor.rs` -- Add `McpDoctorSection` verifying registration and handshake; register in `default_doctor_sections` after `skills`.
- `crates/qdev-core/src/schema.rs` -- Add `PayloadKind::McpInstall` mapping to `payload-mcp-install.json` with alias `"mcp_install"`.
- `crates/qdev-core/schemas/payload-mcp-install.json` -- JSON Schema for `qdev install mcp --json` payload.
- `crates/qdev-core/schemas/payload-doctor.json` -- Update doctor payload schema to validate the `mcp` section fields.
- `crates/qdev-core/src/skills.rs` -- Add `mcp` command (`serve`) and update `install` subcommands in `CommandCatalog::all()` to keep catalog aligned with clap.
- `crates/qdev-core/src/lib.rs` -- Export `mcp` types (`McpServer`, `install_mcp`, `inspect_mcp`, `McpInstallReport`, `McpDoctorStatus`, `McpDoctorSection`).
- `crates/qdev-cli/src/cli.rs` -- Add `Mcp(McpArgs)` to `Commands`, and `Mcp(InstallMcpArgs)` to `InstallCommands`.
- `crates/qdev-cli/src/handlers/mcp.rs` -- Implement `handle_mcp` for `qdev mcp serve` listening on stdin and writing to stdout.
- `crates/qdev-cli/src/handlers/install.rs` -- Extend `handle_install` to dispatch `InstallCommands::Mcp`.
- `crates/qdev-cli/src/handlers/mod.rs` & `crates/qdev-cli/src/main.rs` -- Wire `mcp` subcommand handler.
- `crates/qdev-core/tests/mcp_tests.rs` -- Unit tests for MCP server handshake, tool list, tool schemas, non-interactive tool calls, and install logic.
- `crates/qdev-cli/tests/mcp_cli_tests.rs` -- Integration tests for `qdev mcp serve`, `qdev install mcp`, and doctor MCP checks.
- `docs/cli-reference.md` -- Document `qdev install mcp` and `qdev mcp serve` details.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/mcp.rs` -- Implement MCP server protocol, tool registry, tool execution, installation, and inspection -- Expose headless core operations via stdio JSON-RPC
- [x] `crates/qdev-core/src/doctor.rs` -- Add `McpDoctorSection` to doctor registry after `skills` -- Report MCP registration and handshake status in `qdev doctor`
- [x] `crates/qdev-core/src/schema.rs` & `schemas/` -- Add `payload-mcp-install.json` and update `payload-doctor.json` -- Define JSON schema contracts
- [x] `crates/qdev-core/src/skills.rs` -- Update `CommandCatalog` with `mcp` command and `install mcp` subcommand -- Maintain CLI and catalog bidirectional symmetry
- [x] `crates/qdev-core/src/lib.rs` -- Re-export MCP module types and doctor section -- Provide public API for CLI and tests
- [x] `crates/qdev-cli/src/cli.rs` & `handlers/` -- Add CLI subcommands and handlers for `mcp serve` and `install mcp` -- Expose CLI interface
- [x] `crates/qdev-core/tests/` & `crates/qdev-cli/tests/` -- Implement comprehensive unit and integration tests -- Verify MCP server protocol, tools, install, and doctor
- [x] `docs/cli-reference.md` -- Update CLI reference for `qdev install mcp` and `qdev mcp serve` -- Maintain accurate documentation

**Acceptance Criteria:**
- Given `qdev mcp serve`, when a client connects over stdio, then the 12 tools listed in CLI reference §7 are advertised with JSON Schemas generated from Story 1.13 and return the same payloads as the CLI.
- Given any MCP tool call that fails or is refused, when executed, then it runs with `Interactivity::NonInteractive` and surfaces a structured error envelope conforming to `payload-error.json` with `isError: true`.
- Given `qdev install mcp --claude|--cursor`, when executed, then it registers the `qdev` MCP server in `.claude/settings.json` and/or `.cursor/mcp.json` without overwriting other settings.
- Given `qdev install mcp` with no flags, when executed, then it exits 2 with a usage error.
- Given `qdev doctor`, when executed, then it reports the `mcp` section verifying server registration and handshake status.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| # | Reviewer Layer | Location | Claim / Finding | Verdict | Route | Resolution |
|---|---|---|---|---|---|---|
| 1 | Blind Hunter / Edge Case Hunter / Verif Gap | `crates/qdev-core/src/mcp.rs:2230` | `update_mcp_config_file` catches `from_str` with `.unwrap_or_else(|_| json!({}))`, wiping out invalid JSON in existing config files | `high` | `patch` | Return error refusing to overwrite corrupted settings files. |
| 2 | Blind Hunter / Edge Case Hunter | `crates/qdev-core/src/mcp.rs:1563` | `handle_request` emits response for notifications without `"id"` when method name lacks `notifications/` prefix | `medium` | `patch` | If `req.id.is_none()`, suppress response for all notifications. |
| 3 | Blind Hunter / Verif Gap | `crates/qdev-core/src/mcp.rs:1242,1313,1417,1460` | Tool schema descriptions and enums conflict with runtime validation (`context` phase, `dw_add` risk, `decision_log` type, `transition` state) | `medium` | `patch` | Align schema descriptions, allowed enum values, and require `subject_id` for `decision_log`. |
| 4 | Blind Hunter | `crates/qdev-core/src/mcp.rs:1417` | `dw_add` output schema specifies `module` and `risk` instead of `target_module` and `safety_risk` | `medium` | `patch` | Update `dw_add` output schema to match `DeferredWorkPayload`. |
| 5 | Blind Hunter / Verif Gap | `crates/qdev-core/src/mcp.rs:1515` | `gate_run` output schema advertises `GateRun` but returns `GateRunSetPayload` on `all: true` or `for_transition` | `low` | `patch` | Use `anyOf` with both `GateRun` and `GateSet` schemas. |
| 6 | Blind Hunter / Edge Case Hunter / Verif Gap | `crates/qdev-core/src/mcp.rs:2375` | `inspect_mcp` does not check `args == ["mcp", "serve"]` and maps handshake failure to `"unregistered"` when registered | `medium` | `patch` | Verify `args` and set `status: "unavailable"` with `unavailable_reason: Some("handshake_failed")`. |
| 7 | Blind Hunter | `crates/qdev-core/src/mcp.rs:1550` | `initialize` capabilities omits `"listChanged": false` for tools | `low` | `patch` | Add `"listChanged": false` to capabilities. |
| 8 | Edge Case Hunter | `crates/qdev-core/src/mcp.rs:1990` | `scratch_read` applies budget before filtering by kind | `low` | `patch` | Filter by kind before applying budget truncation. |
| 9 | Blind Hunter | `crates/qdev-core/src/mcp.rs:1880,2050` | `scratch_append` and `dw_add` omit attribution options (`author_type` and `author_id`) | `low` | `patch` | Support optional attribution parameters in schema and execution. |
| 10 | Blind Hunter | `crates/qdev-core/schemas/payload-doctor.json:237` | `registered_targets` is defined in `properties` but omitted from `required` | `low` | `patch` | Add `registered_targets` to `required` array. |
| 11 | Blind Hunter | `crates/qdev-cli/src/main.rs:134,143` | `requires_workspace` and `ensure_cache_with_summary` exempt `Commands::Mcp(_)` | `medium` | `patch` | Require workspace and cache hydration for `McpCommands::Serve`. |
| 12 | Verification Gap Reviewer | `crates/qdev-core/src/mcp.rs:1664` | Pre-verified gap: MCP tool call execution and argument dispatch untested for 10 of 12 advertised tools | `medium` | `patch` | Add unit tests in `mcp_tests.rs` exercising `tools/call` for the 10 tools. |
| 13 | Verification Gap Reviewer | `crates/qdev-cli/src/handlers/mcp.rs:26` | Pre-verified gap: `qdev mcp serve` CLI process lacks stdio integration test for `tools/call` execution | `medium` | `patch` | Add a test in `mcp_cli_tests.rs` executing `tools/call` over stdio. |
| 14 | Verification Gap Reviewer | `crates/qdev-core/src/mcp.rs:2374` | Pre-verified gap: Doctor inspection logic lacks test coverage for loopback handshake failure and invalid settings | `medium` | `patch` | Add tests covering handshake failure and invalid settings. |


## Design Notes

- **MCP Stdio Server Architecture:**
  The server runs a synchronous event loop over `std::io::stdin().lock().lines()`. Each incoming line is parsed as a JSON-RPC 2.0 request or notification:
  - `initialize`: Responds with server capabilities and info.
  - `notifications/initialized`: Handshake completion notification.
  - `tools/list`: Emits the 12 advertised tools with `name`, `description`, `inputSchema`, and `outputSchema` derived from `PayloadKind`.
  - `tools/call`: Dispatches to `qdev-core` APIs with `Interactivity::NonInteractive`.
- **Install File Mutation:**
  When registering in `.claude/settings.json` or `.cursor/mcp.json`, the existing file is read if present. The `mcpServers` JSON object is created or updated with `"qdev": { "command": "qdev", "args": ["mcp", "serve"] }`, and the file is atomically written back.

## Verification

**Commands:**
- `cargo test --package qdev-core --test suite mcp_tests` -- expected: all core MCP tests pass
- `cargo test --package qdev-core --test suite doctor_tests` -- expected: doctor tests including MCP pass
- `cargo test --package qdev-cli --test suite mcp_cli_tests` -- expected: CLI integration tests pass
- `cargo test --package qdev-cli --test suite install_skills_cli_tests` -- expected: CommandCatalog alignment test passes
- `cargo run --bin qdev -- install mcp --claude --cursor --json` -- expected: exit 0 and valid JSON envelope
- `cargo run --bin qdev -- doctor --json` -- expected: exit 0 and includes valid `mcp` doctor section
