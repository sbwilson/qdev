---
title: 'Story 3.4: Ratchets & Baselines'
type: 'feature'
created: '2026-09-23'
status: 'done'
baseline_commit: '70a17b94210686172bad2e2defa29829dae2bfe9'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Quality metrics like compiler warnings, test counts, coverage, and benchmark timings can silently regress across commits when gates only check boolean exit codes rather than enforcing committed baselines.
**Approach:** Implement ratchet gates (`kind = "ratchet"`) comparing numeric output against branch-specific committed baselines (`docs/state/baselines/<branch>/<gate>.json`) using direction rules (`must_not_increase` or `must_not_decrease`), CLI baseline inspection and recording (`qdev gate baseline <id> [--set]`), and sprint close ratchet snapshotting into release frontmatter.

## Boundaries & Constraints

**Always:**
- Identify ratchet gates by `kind.as_deref() == Some("ratchet")` in gate configuration.
- Require `direction` for ratchet gates (`must_not_increase` or `must_not_decrease`).
- Store branch baselines in `docs/state/baselines/<integration-branch>/<gate>.json` under `config.storage.state_dir`.
- On gate execution, if the gate is a ratchet and produces a numeric metric:
  - If no baseline exists for `<integration-branch>/<gate>.json`, report `pass` with `baseline: null` and a warning.
  - If a baseline exists, compare current metric against baseline value by direction:
    - `must_not_increase`: if current > baseline, report `fail` with delta `+X`, otherwise `pass`.
    - `must_not_decrease`: if current < baseline, report `fail` with delta `-X`, otherwise `pass`.
- In `qdev gate baseline <id>`:
  - Without `--set`: display current baseline metadata (metric, value, commit, author, timestamp) or indicate absence.
  - With `--set`: write the baseline file atomically through the write path (`write_file_atomic`) with author attribution (`resolve_author`), commit SHA (`resolve_commit_sha`), and current timestamp.
  - Support setting via explicit `--value <num>` or by running the gate to capture the current metric.
- In `qdev sprint close <num>`:
  - If the sprint has an associated release, collect all configured ratchet baselines and record them into the release frontmatter `baseline_snapshot` through the write path.
- Provide JSON envelope and schema validation for `qdev gate baseline` via `payload-gate-baseline.json`.

**Never:**
- Never mark a ratchet gate `pass` when a regression occurs against the committed baseline.
- Never crash or fail with an unhandled error when a baseline file is missing; report `pass` with `baseline: null` and a warning.
- Never write baseline files outside the configured `state_dir/baselines/<branch>/` directory.
- Never overwrite release snapshots or sprint files without acquiring workspace write locks and adhering to schema validation.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Missing Baseline | Ratchet gate runs, no baseline file on branch | `[PASS]` receipt, summary indicates `baseline: null` with warning, exit code 0 | Warning emitted |
| Metric No Regression | `must_not_increase`, metric 10, baseline 12 | `[PASS]`, summary reports metric and non-regression delta (-2) | N/A |
| Metric Regression | `must_not_increase`, metric 15, baseline 10 | `[FAIL] (exit 1)`, summary reports regression with delta `+5` | Exit code 1 (`LogicalFailure`) |
| Direction `must_not_decrease` | Metric 80, baseline 85 | `[FAIL] (exit 1)`, summary reports regression with delta `-5` | Exit code 1 (`LogicalFailure`) |
| Set Baseline via Run | `qdev gate baseline warning-count --set` | Executes gate, captures metric, writes `docs/state/baselines/<branch>/warning-count.json` | Rejects if gate run fails or yields no metric |
| Set Baseline via `--value` | `qdev gate baseline warning-count --set --value 10` | Writes baseline with value 10 without re-running gate | Rejects if invalid number |
| Inspect Baseline | `qdev gate baseline warning-count` | Displays baseline value, commit, author, timestamp | Reports missing if absent |
| Inspect Baseline JSON | `qdev gate baseline warning-count --json` | Validated JSON envelope matching `payload-gate-baseline.json` | Standard error envelope on failure |
| Sprint Close Snapshot | `qdev sprint close 5` with release `0.1.0` | Records all ratchet values in `docs/state/releases/0.1.0.md` under `baseline_snapshot` | Skips if no release linked |
| Unknown Gate ID | `qdev gate baseline nonexistent` | Refuses with unknown gate error | Exit code 2 (`UsageError`) |
| Non-ratchet Gate Baseline | `qdev gate baseline lint --set` | Rejects setting baseline for gate with `kind != "ratchet"` | Exit code 2 (`UsageError`) |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/schemas/payload-gate-baseline.json` -- JSON Schema for `qdev gate baseline [--json]` payload.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::GateBaseline` and schema lookup.
- `crates/qdev-core/src/gate/mod.rs` -- Models `RatchetBaseline`, `GateBaselinePayload`, helper methods for baseline resolution and ratchet evaluation.
- `crates/qdev-core/src/gate/runner.rs` -- Wire ratchet baseline loading, direction comparison, regression delta reporting, and warning on missing baseline in `execute_gate`.
- `crates/qdev-core/src/sprint.rs` -- Update `close_sprint` to record ratchet values in `release.baseline_snapshot`.
- `crates/qdev-core/src/lib.rs` -- Re-export ratchet baseline models and functions.
- `crates/qdev-cli/src/cli.rs` -- Add `GateCommands::Baseline(GateBaselineArgs)`.
- `crates/qdev-cli/src/handlers/gate.rs` -- Implement `handle_gate_baseline` supporting inspect, `--set`, `--value`, and `--json`.
- `crates/qdev-cli/src/handlers/sprint.rs` -- Pass gates configuration into `SprintCloseOptions`.
- `crates/qdev-core/tests/ratchet_tests.rs` -- Unit tests for ratchet comparisons, direction handling, missing baselines, and delta calculation.
- `crates/qdev-cli/tests/gate_baseline_cli_tests.rs` -- CLI integration tests for `qdev gate baseline`, `--set`, `--value`, `--json`, and regression execution.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/payload-gate-baseline.json` -- Create schema for gate baseline CLI output -- Standardized JSON contract.
- [x] `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::GateBaseline` -- Schema registration and validation.
- [x] `crates/qdev-core/src/gate/mod.rs` -- Define `RatchetBaseline`, `GateBaselinePayload`, baseline path helper, and direction comparison logic -- Core data models.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Wire ratchet comparison in `execute_gate`: load baseline, evaluate direction, calculate delta, handle missing baseline warning -- Gate execution engine.
- [x] `crates/qdev-core/src/sprint.rs` -- Update `close_sprint` to record ratchet values in the associated release `baseline_snapshot` -- Sprint close integration.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `GateCommands::Baseline` and `GateBaselineArgs` -- CLI command parsing.
- [x] `crates/qdev-cli/src/handlers/gate.rs` -- Implement baseline handler for inspection and setting -- CLI handler.
- [x] `crates/qdev-cli/src/handlers/sprint.rs` -- Pass gate config to `SprintCloseOptions` -- Handler integration.
- [x] `crates/qdev-core/tests/ratchet_tests.rs` -- Comprehensive unit tests for ratchet regression and baseline evaluation -- Verification.
- [x] `crates/qdev-cli/tests/gate_baseline_cli_tests.rs` -- Integration tests for baseline CLI commands and sprint close release snapshotting -- Verification.

**Acceptance Criteria:**
- Given a gate with `kind = "ratchet"`, `metric`, and `direction`, when it runs and returns a numeric metric, qdev loads `docs/state/baselines/<integration-branch>/<gate>.json`, compares by direction, and reports `fail` with the delta on regression, `pass` otherwise; with no baseline it reports `pass` with `baseline: null` and a warning.
- `qdev gate baseline <id> --set` writes the current value with author, commit, and timestamp through the write path.
- `qdev gate baseline <id>` inspects and formats current baseline details in text and JSON.
- `qdev sprint close <id>` records all current ratchet values in the release snapshot when a release is linked.

## Implementation Notes

- Implemented `RatchetBaseline`, `RatchetDirection`, `RatchetEvaluation`, and `GateBaselinePayload` in `crates/qdev-core/src/gate/mod.rs`.
- Created schema `payload-gate-baseline.json` and registered `PayloadKind::GateBaseline` with schema validation in `crates/qdev-core/src/schema.rs`.
- Implemented `read_baseline`, `write_baseline`, and `evaluate_ratchet` supporting atomic file writes under workspace locks, formatting metric numbers, and computing directional deltas.
- Integrated ratchet evaluation into `execute_gate` in `crates/qdev-core/src/gate/runner.rs`: on regression, reports failure exit code 1 with regression delta; on missing baseline, reports pass with warning and `baseline: null`.
- Added `qdev gate baseline <id> [--set] [--value <num>]` command with human-readable text and JSON envelope outputs in `crates/qdev-cli/src/handlers/gate.rs`.
- Updated `close_sprint` in `crates/qdev-core/src/sprint.rs` to snapshot all configured ratchet values into the associated release frontmatter `baseline_snapshot`.
- Added unit tests in `crates/qdev-core/tests/ratchet_tests.rs` and CLI integration tests in `crates/qdev-cli/tests/gate_baseline_cli_tests.rs`. All tests pass cleanly with 0 clippy warnings.

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---------|----------|---------|----------------------|-------------|
| Silent pass when ratchet gate produces no numeric metric | blind-hunter / verification-gap | `high` | Ratchet gate with command exiting 0 without numeric output bypassed ratchet baseline checks | patch |
| Missing verification for `--set` baseline capture failure when command yields no numeric metric | verification-gap | `medium` | No test asserted failure on `qdev gate baseline <id> --set` when command yields no metric | patch |
| Missing verification for `qdev gate baseline <id> --value` invocation without `--set` | verification-gap | `medium` | No test asserted failure on `--value` without `--set` | patch |
| Ratchet regression does not populate `failures` in `GateRunOutcome` | blind-hunter | `medium` | `failures` was empty when regression occurred despite instruction to fix cited failures | patch |
| `sprint close` completely overwrites `baseline_snapshot` instead of merging | blind-hunter | `medium` | Pre-existing snapshot entries in release frontmatter were discarded | patch |
| `sprint close` silently swallows release resolution failures | blind-hunter | `medium` | Non-existent release linked to sprint was ignored instead of returning an error | patch |
| Missing validation for non-finite values (`NaN` and `Infinity`) | blind-hunter | `medium` | Non-finite values broke JSON serialization and comparison ordering | patch |
| Ignored result of updating release entity in store during sprint close | blind-hunter | `low` | `store.upsert_entity` error was swallowed | patch |
| Redundant directory creation outside workspace lock in `write_baseline` | blind-hunter | `low` | Directory created before lock; `write_file_atomic` already handles parent directories | patch |
| Committed baseline JSON files lack trailing newline | blind-hunter | `low` | Missing trailing `\n` causes git diff warnings | patch |
| Narrow stdout metric parsing | blind-hunter | `low` | Did not match `<metric>: <num>` or `<metric> = <num>` | patch |
| Invalid `Eq` trait implementation on `GateBaselineArgs` | blind-hunter | `low` | Manual empty `Eq` for struct containing `Option<f64>`; floats are finite so reflexivity holds | reject |
| Missing `--branch` argument in `qdev gate baseline` CLI | blind-hunter | `false` | Specification and architecture bind baselines strictly to configured integration branch | reject |
| Missing schema validation and consistency checks against stored baseline | blind-hunter | `false` | `qdev.toml` is the authoritative definition of gate evaluation rules | reject |
| Status tracking discrepancy in `sprint-status.yaml` | blind-hunter | `false` | BMAD workflow advances status to `review` at step-05 per process | reject |

## Design Notes

### Baseline Storage Format
Stored under `<state_dir>/baselines/<integration_branch>/<gate_id>.json`:
```json
{
  "gate": "warning-count",
  "metric": "warnings",
  "direction": "must_not_increase",
  "value": 42.0,
  "commit": "8f1b2c4d5e...",
  "author": {
    "type": "human",
    "id": "simon"
  },
  "timestamp": "2026-09-23T06:30:00Z"
}
```

### Ratchet Comparison Logic
- If `must_not_increase` and `metric > baseline.value`: regression failure with summary `ratchet regression: <metric> increased from <baseline> to <metric> (delta: +<delta>)`.
- If `must_not_decrease` and `metric < baseline.value`: regression failure with summary `ratchet regression: <metric> decreased from <baseline> to <metric> (delta: -<delta>)`.
- If no baseline: status `pass`, summary `ratchet passed with no baseline (<metric>: <val>, baseline: null) [warning: no baseline recorded for branch '<branch>']`.

## Verification

**Commands:**
- `cargo test --test ratchet_tests` -- expected: All ratchet unit tests pass.
- `cargo test --test gate_baseline_cli_tests` -- expected: All gate baseline CLI tests pass.
- `cargo test --test gate_runner_tests` -- expected: Existing gate runner tests pass.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero warnings.
