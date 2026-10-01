---
title: 'Story 3.3: Gate Dependencies & --all'
type: 'feature'
created: '2026-09-23'
status: 'done'
baseline_commit: 'ae98fe55477c42de9918e9f64159509f9a081e68'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Verification gates lack dependency awareness and batch execution. Prerequisite fast checks (lint, unit tests) cannot protect expensive gates (ratchets, integration tests) from running when prerequisites fail, and no command exists to run all gates or inspect gate configuration and status.
**Approach:** Implement dependency graph validation with cycle detection via `find_dependency_cycle`, topological execution order, cascade skip propagation on dependency failures, `qdev gate run --all` and `--for-transition <name>`, aggregate exit codes (1 for logical failure, 4 for infrastructure failure, 0 for success), and `qdev gate list` (with `--json`) displaying kind, transitions, dependencies, and evidence status.

## Boundaries & Constraints

**Always:**
- Execute gates in topological order according to `depends_on` so prerequisites always run before dependents.
- Detect cycles in `depends_on` and reject immediately with exit code 2 (`ExitCode::UsageError`).
- Reject dangling or unknown gate dependencies in `depends_on` with exit code 2 (`ExitCode::UsageError`).
- When a dependency fails (`fail` or `infra`) or was skipped due to an upstream failure, mark downstream dependent gates `status: skip` with summary `dependency failed: <dep_id>` without executing their commands.
- Compute aggregate exit code: 1 if any gate had `status: fail`; 4 if any gate had `status: infra` and none failed; otherwise 0.
- Print console receipts for every evaluated gate in text mode: `[PASS]`, `[FAIL]`, `[INFRA]`, and `[SKIP] <gate> | <summary>`.
- In `qdev gate list --json`, emit JSON envelope with `gates` listing each gate's `id`, `kind`, `transitions`, `dependencies`, and `last_status` (resolved from SQLite `gate_runs` and `docs/state/evidence/`).
- Validate `--json` output payloads against schemas `payload-gate-list.json` and `payload-gate-set.json`.

**Never:**
- Never execute a gate when any prerequisite has failed or suffered an infrastructure failure.
- Never exit 0 when any evaluated gate failed or suffered an infrastructure failure.
- Never ignore circular dependencies; fail fast with exit code 2 before executing any gate.
- Never mutate evidence files or disk state during gate inspection.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Topological Execution | B depends on A | Runs A then B; receipts printed in execution order | Standard exit code |
| Prerequisite Logical Failure | A fails (exit 1), B depends on A | A reports `[FAIL]`, B reports `[SKIP] B \| dependency failed: A` | Aggregate exit 1 |
| Prerequisite Infra Failure | A fails with timeout/missing bin (exit 4), B depends on A | A reports `[INFRA]`, B reports `[SKIP] B \| dependency failed: A` | Aggregate exit 4 |
| Multi-level Cascade Skip | A fails, B depends on A, C depends on B | A fails, B skipped (`dependency failed: A`), C skipped (`dependency failed: B`) | Aggregate exit 1 (or 4 if infra) |
| Local Skip vs Failed Dependency | A has `skip = true`, B depends on A | A reports `[SKIP] A \| skipped_locally`, B executes normally | B evaluates own outcome |
| Circular Dependency | A -> B -> A in `depends_on` | Refuses execution citing cycle path `A -> B -> A` | Exit code 2 (`ExitCode::UsageError`) |
| Filter by Transition | `qdev gate run --for-transition review` | Executes gates with `on_transition = ["review"]` and their transitive dependencies | Aggregate exit code |
| Run All Gates | `qdev gate run --all` | Executes all configured gates in topological order | Aggregate exit code |
| Gate List JSON | `qdev gate list --json` | Emits envelope with `gates` array containing `id`, `kind`, `transitions`, `dependencies`, `last_status` | Validated against schema |
| Conflicting CLI Flags | `qdev gate run my-gate --all` | Rejection citing conflicting invocation | Exit code 2 (`ExitCode::UsageError`) |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/schemas/payload-gate-list.json` -- JSON Schema for `qdev gate list --json` payload.
- `crates/qdev-core/schemas/payload-gate-set.json` -- JSON Schema for `qdev gate run --all --json`.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::GateList` and `PayloadKind::GateSet`.
- `crates/qdev-core/src/gate/mod.rs` -- Models `GateListItem`, `GateListPayload`, `GateRunSetOutcome`, `GateRunSetPayload`; update `receipt()` for `Skip` summary.
- `crates/qdev-core/src/gate/runner.rs` -- `validate_gate_dependencies`, `resolve_gate_execution_order`, topological sort, skip cascade, and `execute_gate_set`.
- `crates/qdev-core/src/lib.rs` -- Re-export gate execution order and list models.
- `crates/qdev-cli/src/cli.rs` -- Add `--all`, `--for-transition` to `GateRunArgs`; add `GateListArgs` and `GateCommands::List`.
- `crates/qdev-cli/src/handlers/gate.rs` -- Wire multi-gate execution and `qdev gate list` (text and JSON).
- `crates/qdev-core/tests/gate_runner_tests.rs` -- Unit tests for topological ordering, cycles, skip cascade, aggregate exit codes.
- `crates/qdev-cli/tests/gate_cli_tests.rs` -- CLI integration tests for `qdev gate run --all`, `--for-transition`, and `qdev gate list`.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/payload-gate-list.json` and `crates/qdev-core/schemas/payload-gate-set.json` -- Create schemas for gate list and multi-run set payloads -- Standardized output contracts.
- [x] `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::GateList` and `PayloadKind::GateSet` with schema lookup -- Payload schema validation.
- [x] `crates/qdev-core/src/gate/mod.rs` -- Define `GateListItem`, `GateListPayload`, `GateRunSetOutcome`, `GateRunSetPayload`, and update `receipt()` to format skip reasons -- Core models and receipts.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Implement dependency validation, cycle rejection, topological ordering, skip cascade propagation, and `execute_gate_set` -- Execution engine.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `--all`, `--for-transition` to `GateRunArgs`, and add `GateCommands::List` -- CLI interface.
- [x] `crates/qdev-cli/src/handlers/gate.rs` -- Wire multi-gate execution, aggregate exit codes, and gate list with evidence status -- CLI command handler.
- [x] `crates/qdev-core/tests/gate_runner_tests.rs` -- Unit tests covering topological sorting, cycle detection, cascade skips, and exit code precedence -- Core engine verification.
- [x] `crates/qdev-cli/tests/gate_cli_tests.rs` -- CLI tests for `qdev gate run --all`, `--for-transition`, receipt printing, and `qdev gate list [--json]` -- CLI integration verification.

**Acceptance Criteria:**
- Given gates with `depends_on`, when `qdev gate run --all` or `--for-transition review` runs, gates execute in topological order.
- When a dependency fails (status `fail` or `infra`), dependent gates are marked `skipped` with summary `dependency failed: <dep_id>` and do not execute.
- When a cycle in `depends_on` exists, qdev rejects execution with exit code 2 citing the cycle path.
- When any gate fails, aggregate exit code is 1; if any was `infra` and none failed, aggregate exit code is 4; otherwise 0.
- When `qdev gate list --json` runs, it outputs each gate's `id`, `kind`, `transitions`, `dependencies`, and `last_status` from evidence, validating against `payload-gate-list.json`.

## Implementation Notes

- Implemented `validate_gate_dependencies` and `resolve_gate_execution_order` in `runner.rs`, utilizing Kahn's algorithm with deterministic tie-breaking based on definition order in `config.gates`.
- Wired `crate::dag::find_dependency_cycle` for cycle detection with formatted error messages reporting the full cycle chain.
- Added cascade skip logic to `execute_gate_set`: when a dependency fails (`fail`, `infra`, or cascaded `skip`), downstream dependent gates are skipped without executing commands, emitting `[SKIP] <gate> | dependency failed: <dep_id>`. Local skips (`skipped_locally: true`) do not cascade.
- Implemented aggregate exit codes: `1` if any gate failed; `4` if any gate had an infra failure and none failed; `0` otherwise.
- Implemented `get_gate_list` and `qdev gate list` (with `--json`), resolving gate metadata and latest status from SQLite store `gate_runs` and `docs/state/evidence/`.
- Created schemas `payload-gate-list.json` and `payload-gate-set.json` and registered them in `PayloadKind`.
- All unit, integration, and schema validation tests pass cleanly with 0 clippy warnings.

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---------|----------|---------|----------------------|-------------|
| `--for-transition` specifies transition name matching no gates silently succeeds | edge-case-hunter / blind-hunter | `medium` | `matching.is_empty()` returns empty execution order and exits 0 instead of alerting user to typo or unconfigured transition | patch |
| `get_gate_list` hardcodes `docs/state/evidence` ignoring `config.storage.state_dir` | edge-case-hunter / blind-hunter | `medium` | Evidence directory lookup bypasses `config.storage.state_dir`, failing on custom state directories | patch |
| Incompatible timestamp representation causes flawed evidence ordering | edge-case-hunter / blind-hunter / verification-gap | `medium` | Fallback unix seconds formatted as `0000...` compares smaller than ISO 8601 strings in ASCII comparison | patch |
| Inconsistent JSON payload schema on `qdev gate run <id> --json` | blind-hunter | `medium` | Output schema dynamically switched to `GateRunSetPayload` when target had dependencies instead of adhering to `payload-gate-run.json` | patch |
| Targeted single-gate execution with dependencies unverified at CLI layer | verification-gap | `medium` | Integration tests only executed single gates without `depends_on` | patch |
| Transition-bound gate execution with `--json` unverified against schema | verification-gap | `medium` | CLI integration tests covered text receipts only for `--for-transition` | patch |
| SQLite store resolution of gate `last_status` unverified | verification-gap | `medium` | Existing tests tested flat file evidence only, omitting `cache.sqlite` query coverage | patch |
| Duplicate gate IDs in `config.gates` silently shadowed | edge-case-hunter / blind-hunter | `low` | `validate_gate_dependencies` omitted unique gate ID check | patch |
| Ambiguous text formatting `transitions: [none]` in `gate list` | blind-hunter | `low` | Formatted `none` inside brackets instead of empty array `[]` | patch |
| Missing enum constraint on `last_status` in `payload-gate-list.json` | blind-hunter | `low` | Schema allowed unrestricted string values instead of gate status taxonomy | patch |
| Unbounded recursion and symlink traversal in `scan_evidence_dir` | blind-hunter | `low` | Added `!path.is_symlink()` check during recursive scan | patch |
| Redundant git subprocess execution during cascade skips | blind-hunter | `low` | Cache `commit_sha` once per `execute_gate_set` invocation | patch |
| Direct `execute_gate_set` call with omitted prerequisite | edge-case-hunter / blind-hunter | `low` | Defensive check in `execute_gate_set` ensures prerequisite evaluation | patch |
| Generic error message missing cycle path on Kahn's algorithm exhaustion | blind-hunter | `false` | Unreachable; `validate_gate_dependencies` runs first and formats cycle path with `find_dependency_cycle` | reject |
| Runner errors abort multi-gate execution | blind-hunter | `false` | `execute_gate` only returns `Err` on configuration errors, which must abort execution with exit 2 | reject |
| Sprint status artifact mismatch | blind-hunter | `false` | Sprint status is advanced to `review` at step-05 per lifecycle contract | reject |
| $O(V^2)$ scan in Kahn's algorithm | blind-hunter | `false` | Gate sets are tiny ($N < 50$); execution time is dominated by subprocesses (~microseconds vs seconds) | reject |


## Design Notes

### Dependency Resolution & Topological Ordering
`validate_gate_dependencies` builds `(gate_id, dep_id)` directed edges, checks for unknown dependencies, and invokes `crate::dag::find_dependency_cycle` to verify acyclicity. Kahn's algorithm resolves topological execution order with ties broken by declaration order in `config.gates`.

### Skip Cascade
Sequential execution inspects direct dependencies in `gate.depends_on`. If any dependency resulted in `Fail`, `Infra`, or a non-local `Skip`, the gate is marked `status: GateStatus::Skip` with `summary: format!("dependency failed: {}", failed_dep_id)`.

### Gate List Evidence Status
`handle_gate_list` inspects `store.list_gate_runs()` (if `SqliteStore` is open) and scans `docs/state/evidence/` for `.json` files to find the latest run timestamp/mtime per gate, populating `last_status`.

## Verification

**Commands:**
- `cargo test --test gate_runner_tests` -- expected: All runner unit tests pass.
- `cargo test --test gate_cli_tests` -- expected: All CLI integration tests pass.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero warnings.
