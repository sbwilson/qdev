---
title: 'Story 3.2: Gate Result Contract & Failure Taxonomy'
type: 'feature'
created: '2026-09-22'
status: 'done'
baseline_commit: '07ff70f123bd2546866b2a0b166c4ecb24234b85'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Automated agents and developers need precise failure classification to take safe actions. Without a strict gate result contract and failure taxonomy, tooling cannot distinguish logical test regressions from broken infrastructure (timeouts, missing binaries, process kills), leading agents to hallucinate code fixes for environment faults.
**Approach:** Implement the Gate Result Contract schema validation, the three-way failure taxonomy (`pass`, `fail`, `infra`), output adapters (`cargo`, `xcodebuild`, `json`) extracting failing test locations and assertion diffs, fallback derivation from exit code and the last 40 lines of stderr, and receipts conforming to `docs/compliance-and-safety.md` §2.

## Boundaries & Constraints

**Always:**
- Validate gate result documents (read from `$QDEV_RESULT_FILE` or stdout) against the gate result schema (`status`, `summary`, `failures`, `metric`, `constraint_ids`).
- If `output_adapter = "json"` is declared, require a valid result document; treat missing output, invalid JSON, or schema validation failures as `status: infra` with `agent_instruction: "halt_and_alert"` and exit code 4 (`ExitCode::InfrastructureFailure`).
- If no result document is present and no adapter is declared, derive `pass`/`fail` from the exit code (exit 0 -> `pass`, non-zero -> `fail`) and summary from the last 40 lines of stderr (falling back to stdout or exit description if stderr is empty).
- Treat timeout, kill/signal termination, and missing executables as `status: infra` with exit code 4 and payload `agent_instruction: "halt_and_alert"`.
- Support built-in output adapters: `"json"`, `"cargo"`, and `"xcodebuild"`. Unknown adapter declarations must immediately fail with exit code 2 (`ExitCode::UsageError`).
- Format receipts according to `docs/compliance-and-safety.md` §2:
  - Pass: single line `[PASS] <gate_id> | <summary> | <sha> | <duration>`
  - Fail: `[FAIL] <gate_id> (exit <exit_code>) | <location> | <message>` (or `[FAIL] <gate_id> (exit <exit_code>) | <summary>` when no structured failures exist)
  - Infra: `[INFRA] <gate_id> | <summary> | halt and alert`
- Include `agent_instruction` in the gate run payload: `"continue"` for `pass`, `"fix_cited_failures"` for `fail`, and `"halt_and_alert"` for `infra`.

**Never:**
- Never let an infrastructure failure (missing binary, timeout, signal kill, or invalid JSON when `output_adapter = "json"` is declared) report `status: fail` or exit code 1; it must report `status: infra` and exit code 4.
- Never exit 0 when a gate fails logically or infrastructure fails.
- Never silently ignore unknown `output_adapter` values; always exit 2.
- Never modify existing `[gates] skip` behavior or Head+Tail ring buffering semantics established in Story 3.1.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Valid Result Document via `$QDEV_RESULT_FILE` | Gate writes valid JSON to `$QDEV_RESULT_FILE` | Uses `status`, `summary`, `failures`, `metric`, `constraint_ids` from JSON | Validated against schema; exit code matches status |
| Valid Result Document via stdout | Gate writes valid JSON to stdout | Uses `status`, `summary`, `failures`, `metric`, `constraint_ids` from JSON | Validated against schema; exit code matches status |
| Declared JSON Adapter Missing/Invalid | `output_adapter = "json"`, gate writes invalid JSON or nothing | `status: infra`, `exit_code: 4`, `agent_instruction: "halt_and_alert"` | Handled as infrastructure failure |
| Fallback Exit Code Derivation | No result doc, gate exits 101, stderr has lines | `status: fail`, `exit_code: 101`, summary extracted from last 40 stderr lines | `agent_instruction: "fix_cited_failures"` |
| Fallback Exit 0 Derivation | No result doc, gate exits 0, stdout has lines | `status: pass`, `exit_code: 0`, summary from stdout or "gate passed" | `agent_instruction: "continue"` |
| Cargo Output Adapter Failure | `output_adapter = "cargo"`, raw test failure output | Extracts test location `file:line` and assertion diff into `failures` | `status: fail`, exit code 1 or raw exit code |
| Xcodebuild Output Adapter Failure | `output_adapter = "xcodebuild"`, raw XCTest failure output | Extracts test location `file:line` and assertion message into `failures` | `status: fail`, exit code 1 or raw exit code |
| Unknown Output Adapter | `output_adapter = "unsupported"` in config | Command rejected with usage error message | Exit code 2 (`ExitCode::UsageError`) |
| Process Killed by Signal | Gate terminated by external signal / SIGKILL | `status: infra`, exit code 4, `agent_instruction: "halt_and_alert"` | Handled as infrastructure failure |
| Multiple Failures in Receipt | Gate result contains multiple failures | Receipt formats each failure or primary failure with location and message | Exit code matches status |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/schemas/gate-result.json` -- JSON Schema for gate result document per compliance & safety §2.
- `crates/qdev-core/schemas/payload-gate-run.json` -- Additive schema update supporting `failures`, `metric`, and `constraint_ids`.
- `crates/qdev-core/src/schema.rs` -- Register gate result schema and validation helpers.
- `crates/qdev-core/src/gate/mod.rs` -- Extend `GateRunPayload` and `GateRunOutcome` with `failures`, `metric`, and `constraint_ids`; update receipt formatting for failure locations.
- `crates/qdev-core/src/gate/result.rs` -- Gate result document deserialization, validation against `gate-result.json`, and schema enforcement.
- `crates/qdev-core/src/gate/adapter/mod.rs` -- Adapter trait and dispatcher for `json`, `cargo`, and `xcodebuild`, including unknown adapter validation.
- `crates/qdev-core/src/gate/adapter/cargo.rs` -- Parser extracting failing test locations (`file:line`), assertion diffs, and test counts from Cargo output.
- `crates/qdev-core/src/gate/adapter/xcodebuild.rs` -- Parser extracting failing test locations (`file:line`), assertion messages, and test counts from xcodebuild output.
- `crates/qdev-core/src/gate/runner.rs` -- Integrate adapter validation, signal termination detection, result document validation, and 40-line stderr extraction.
- `crates/qdev-core/tests/fixtures/cargo_test_failure.txt` -- Test fixture with raw cargo test failure output.
- `crates/qdev-core/tests/fixtures/xcodebuild_test_failure.txt` -- Test fixture with raw xcodebuild test failure output.
- `crates/qdev-core/tests/gate_adapter_tests.rs` -- Unit tests for cargo, xcodebuild, and json adapters with fixtures.
- `crates/qdev-core/tests/gate_runner_tests.rs` -- Runner tests for result document precedence, fallback derivation, and failure taxonomy.
- `crates/qdev-cli/tests/gate_cli_tests.rs` -- CLI integration tests for `qdev gate run` with adapters, JSON outputs, and receipts.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/gate-result.json` and `crates/qdev-core/schemas/payload-gate-run.json` -- Define gate result JSON schema and update payload schema -- Provides schema contracts for verification.
- [x] `crates/qdev-core/src/gate/result.rs` -- Implement `GateResultDocument` with schema validation, failures, metric, and constraint IDs -- Core result contract data model.
- [x] `crates/qdev-core/src/gate/adapter/cargo.rs` -- Implement `CargoAdapter` parsing panic locations (`file:line`), assertion diffs, and test summaries -- Cargo test failure extraction.
- [x] `crates/qdev-core/src/gate/adapter/xcodebuild.rs` -- Implement `XcodebuildAdapter` parsing XCTest locations (`file:line`), assertion text, and test summaries -- Xcodebuild test failure extraction.
- [x] `crates/qdev-core/src/gate/adapter/mod.rs` -- Implement adapter registry dispatching to `json`, `cargo`, `xcodebuild` and validating against unknown adapters -- Adapter architecture.
- [x] `crates/qdev-core/src/gate/mod.rs` -- Update `GateRunOutcome`, `GateRunPayload`, `GateFailure`, and `receipt()` formatting per §2 -- Public API and formatting contract.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Wire adapters, signal kill detection, result file/stdout validation, and 40-line stderr fallback into `execute_gate` -- Full gate execution pipeline.
- [x] `crates/qdev-core/tests/gate_adapter_tests.rs` -- Add unit tests with fixtures for cargo and xcodebuild parsing -- Verification of test adapters.
- [x] `crates/qdev-core/tests/gate_runner_tests.rs` -- Add unit tests for result document validation, `output_adapter = "json"` infra failure, and signal kills -- Verification of failure taxonomy.
- [x] `crates/qdev-cli/tests/gate_cli_tests.rs` -- Add CLI tests for receipts, unknown adapter exit code 2, and payload JSON shape -- CLI integration verification.

**Acceptance Criteria:**
- Given a gate writing valid result JSON to stdout or `$QDEV_RESULT_FILE`, when it exits, then qdev validates the document against `gate-result.json` and populates `status`, `summary`, `failures`, `metric`, and `constraint_ids`.
- Given a gate without a result document, when it exits, then qdev derives `pass`/`fail` from the exit code and summary from the last 40 lines of stderr.
- Given a gate where `output_adapter = "json"` is declared, when the result document is missing or fails schema validation, then status is `infra`, exit code is 4, and payload includes `agent_instruction: "halt_and_alert"`.
- Given a gate that is killed by a signal, when it terminates, then status is `infra`, exit code is 4, and payload includes `agent_instruction: "halt_and_alert"`.
- Given a gate configured with an unknown `output_adapter`, when run, then qdev exits with exit code 2.
- Given `output_adapter = "cargo"`, when cargo test fails, then failing test locations (`file:line`) and assertion text are extracted into `failures`.
- Given `output_adapter = "xcodebuild"`, when xcodebuild test fails, then failing test locations (`file:line`) and assertion text are extracted into `failures`.
- Receipts are printed conforming to `docs/compliance-and-safety.md` §2 with a single line on pass and detailed location/assertion on fail.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---------|----------|---------|----------------------|-------------|
| Windows drive letter path with line number in `normalize_location` | edge-case-hunter / blind-hunter | `medium` | `s.split(':')` treating `C:\...\test.rs:142` as path `C` and line `\...\test.rs` | patch |
| Quoted panic message containing internal apostrophe | edge-case-hunter / verification-gap | `low` | `find('\'')` prematurely splitting at apostrophe in message | patch |
| Gate result document `status: pass` with non-zero exit code | edge-case-hunter / blind-hunter | `false` | In compliance-and-safety.md §2 table, pass is explicitly defined as "Exit 0 or result pass"; result doc is authoritative | reject |
| Result file touched but empty while stdout has valid JSON document | edge-case-hunter / blind-hunter | `low` | `result_file_path.exists()` was evaluated without checking stdout when file content is empty | patch |
| Passing `#[should_panic]` test panic output collected as failure | edge-case-hunter | `medium` | `parse_cargo_output` checked `panicked at` outside `failures:` block | patch |
| `cargo test` non-zero exit with 0 failed tests summary | edge-case-hunter | `low` | Gate summary reported "X tests passed" despite non-zero exit code | patch |
| `gate-result.json` status enum omits `skip` | blind-hunter | `low` | `GateStatus::Skip` exists in core and payload schema; external gate writing `status: skip` was rejected by schema | patch |
| `relativize_path` prefix stripping ignoring path separator | blind-hunter | `low` | `strip_prefix` on `/workspace/app` against `/workspace/app_lib` strips subpart without checking boundary | patch |
| Unquoted panic header path with spaces truncated | blind-hunter | `low` | `split_whitespace().next()` cuts path containing spaces | patch |
| `cargo test` compiler errors not parsed into test failures | blind-hunter | `low` | AC specifically targets test failure locations and assertion text; compiler diagnostics occur pre-test | reject |
| `xcodebuild` test counts double-counted across suites | blind-hunter | `medium` | Accumulating `Executed X tests` across root and nested suites inflates summary counts | patch |
| Fallback 40-line stderr flattened in single-line receipt | blind-hunter | `false` | Receipts in §2 are explicitly single/concise lines, while `GateRunOutcome.summary` and payload retain multiline stderr | reject |
| Stdout result document extraction fails on surrounding logs | blind-hunter | `false` | Gates emitting JSON to stdout must emit the result doc; `$QDEV_RESULT_FILE` is provided for mixed logging | reject |
| Receipt format trailing delimiter on empty failure message | blind-hunter | `low` | `[FAIL] ... | loc | ` leaves trailing delimiter if message is empty | patch |
| `gate-result.json` omitted from `PayloadKind` | blind-hunter | `false` | `PayloadKind` is strictly for CLI output payloads, while gate-result is an input verification contract | reject |
| `output_adapter = "json"` valid result document handling unverified | verification-gap | `medium` | Pre-verified gap: no test executed `output_adapter = "json"` with a valid document | patch |
| Passing `cargo` and `xcodebuild` gate runner caller integration unverified | verification-gap | `medium` | Pre-verified gap: tests tested `parse_with_adapter` directly but never `execute_gate` or CLI success | patch |
| `parse_with_adapter("json", ...)` usage error untested | verification-gap | `low` | Usage error return on stream parsing was uncovered | patch |


## Design Notes

### Schema Validation
Gate results written to `$QDEV_RESULT_FILE` or captured on stdout are parsed into `serde_json::Value` and validated against `gate-result.json` using `jsonschema::validator_for`. If validation succeeds, fields are extracted into `GateResultDocument`. If `output_adapter = "json"` was configured and validation fails, the runner sets `GateStatus::Infra` and `agent_instruction: "halt_and_alert"`.

### Adapter Extraction
`CargoAdapter` inspects stdout/stderr for `panicked at <path>:<line>:<col>` lines inside `failures:` sections, extracting relative file paths and line numbers, plus subsequent lines up to `note:` or empty lines for assertion diffs.
`XcodebuildAdapter` inspects lines matching `<path>:<line>: error: <assertion>`, extracting relative paths and message content.
Both adapters parse summary lines (e.g. `test result: FAILED. X passed; Y failed`) to synthesize human-readable summaries.

### Receipts
On pass, `[PASS] <gate> | <summary> | <sha> | <duration>`.
On fail with failures, `[FAIL] <gate> (exit <code>) | <location> | <message_first_line>`.
On infra, `[INFRA] <gate> | <summary> | halt and alert`.
