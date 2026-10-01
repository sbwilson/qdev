---
title: 'Story 3.1: Gate Runner: Sub-process, Environment, Timeouts'
type: 'feature'
created: '2026-09-22'
status: 'done'
baseline_commit: 'e6c702336242321300fd03cd9a777a8b137dc49b'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Project-specific verification checks cannot be compiled into `qdev`, and running arbitrary gate scripts without resource constraints risks hanging development workflows and stalling AI agents indefinitely. Furthermore, unconstrained process streams can cause out-of-memory errors while simple tail-only buffers erase critical initial diagnostic context and structured outputs.
**Approach:** Implement an external gate execution runner in `qdev-core` and CLI command `qdev gate run <id>` that injects parent, configured, and execution environments (`QDEV_STORY`, `QDEV_GATE`, `QDEV_COMMIT`, `QDEV_RESULT_FILE`, `QDEV_MODULE_PATHS`), captures stdout and stderr in bounded 1 MB Head+Tail buffers (preserving a 64 KB fixed header and 960 KB rolling tail with truncation markers), terminates hanging process trees across platforms (Unix process groups, Windows job objects) upon timeout, classifies missing executables as `infra` failures with resolved paths, and respects local gate skip configurations with `skipped_locally` receipts.

## Boundaries & Constraints

**Always:**
- Execute gates as external subprocesses without embedding project-specific logic inside the binary (AD-5).
- Inherit the parent environment, overlay `[environment]` config entries, and inject `QDEV_STORY`, `QDEV_GATE`, `QDEV_COMMIT`, `QDEV_RESULT_FILE`, and `QDEV_MODULE_PATHS` (JSON string).
- Enforce `timeout_ms` (defaulting to 300,000 ms if unspecified) and terminate the entire child process tree on timeout using OS process groups on Unix (`libc::kill(-pgid, SIGKILL)`) and job objects on Windows (`TerminateJobObject`).
- Bound stdout and stderr capture to 1 MB (1,048,576 bytes) each using a Head+Tail ring buffer: retain the first 64 KB in an immutable head buffer and the most recent 960 KB in a rolling tail buffer, inserting `\n[... qdev: <N> bytes truncated ...]\n` between them when overflow occurs. If total output ≤ 1 MB, retain all bytes contiguously with no marker.
- Report missing executables as `status: infra` with the resolved binary path and exit code 4 (`ExitCode::InfrastructureFailure`).
- Honor local `[gates] skip` declarations by skipping process launch, returning exit code 0, and recording `skipped_locally` in receipts and JSON payloads.
- Format console receipts conforming to `docs/compliance-and-safety.md` §2 (`[PASS]`, `[FAIL]`, `[INFRA]`, `[SKIP]`).

**Never:**
- Never let child or grandchild processes orphan or survive when a gate times out.
- Never block stdout or stderr pipe reads; process streams concurrently in background reader threads.
- Never exit 0 when a gate fails or encounters an infrastructure error.
- Never require external network access or external daemon services to run gates.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Successful Gate Execution | `qdev gate run lint` (exit 0) | `[PASS] lint \| <summary> \| <sha> \| <duration>` | Exit code 0 |
| Gate Logical Failure | `qdev gate run test` (exit 101) | `[FAIL] test (exit 101) \| <summary>` | Exit code 1 (`ExitCode::LogicalFailure`) |
| Gate Timeout | Gate command hangs past `timeout_ms` | Process tree terminated; `[INFRA] <id> \| timeout after <ms>ms \| halt and alert` | Exit code 4 (`ExitCode::InfrastructureFailure`) |
| Missing Executable | Gate command names nonexistent binary | `[INFRA] <id> \| missing executable: <path> \| halt and alert` | Exit code 4 (`ExitCode::InfrastructureFailure`) |
| Local Skip | Gate marked `skip = true` in local config | Gate command not spawned; `[SKIP] <id> \| skipped_locally` | Exit code 0 (`ExitCode::Success`) |
| Unknown Gate ID | `qdev gate run unknown-gate` | Error: gate `unknown-gate` not found in configuration | Exit code 2 (`ExitCode::UsageError`) |
| Process Output Exceeds 1 MB | Gate generates >1 MB on stdout/stderr | Retains initial 64 KB header + `[... qdev: N bytes truncated ...]` + final 960 KB tail | No error; bounded 1 MB buffer preserved |
| Process Output ≤ 1 MB | Gate generates small output | Contiguous full output retained with no truncation marker | Exit code matches status |
| JSON Output Mode | `qdev gate run <id> --json` | Valid JSON envelope containing `status`, `summary`, `duration_ms`, `exit_code`, `skipped_locally` | Exit code matches status |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/Cargo.toml` -- Dependencies for `libc` (unix process groups) and `windows-sys` (windows job objects).
- `crates/qdev-core/src/gate/mod.rs` -- Module root for gate execution models, options, outcomes, and coordinator.
- `crates/qdev-core/src/gate/ring_buffer.rs` -- Bounded Head+Tail ring buffer implementation (64 KB fixed head + 960 KB rolling tail with truncation tracking).
- `crates/qdev-core/src/gate/runner.rs` -- Gate subprocess execution, environment assembly, timeout monitoring, and tree killing.
- `crates/qdev-core/src/gate/process.rs` -- Cross-platform process group creation (Unix) and job object attachment (Windows).
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::GateRun`.
- `crates/qdev-core/schemas/payload-gate-run.json` -- JSON Schema for `gate_run` output envelope.
- `crates/qdev-core/src/lib.rs` -- Export gate runner structures and entrypoints.
- `crates/qdev-cli/src/cli.rs` -- Define `GateArgs` and `GateCommands::Run`.
- `crates/qdev-cli/src/handlers/gate.rs` -- Handler for `qdev gate run <id>`.
- `crates/qdev-cli/src/handlers/mod.rs` -- Export gate handler.
- `crates/qdev-cli/src/main.rs` -- Route `Commands::Gate` to gate handler.
- `crates/qdev-core/tests/gate_runner_tests.rs` -- Unit tests for gate runner environment, Head+Tail buffer rollover, and timeouts.
- `crates/qdev-cli/tests/gate_cli_tests.rs` -- CLI integration tests for `qdev gate run` scenarios.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/Cargo.toml` -- Add `libc` under `target.'cfg(unix)'.dependencies` and `windows-sys` under `target.'cfg(windows)'.dependencies` -- Enables cross-platform process tree signals.
- [x] `crates/qdev-core/src/gate/ring_buffer.rs` -- Implement `HeadTailBuffer` with 64 KB immutable head and 960 KB rolling tail that tracks truncated byte count and inserts truncation banner -- Preserves critical startup context and recent output within 1 MB bound.
- [x] `crates/qdev-core/src/gate/process.rs` -- Implement cross-platform process group isolation (Unix) and Job Object isolation (Windows) with kill-tree primitives -- Guarantees clean child/grandchild termination on timeout.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Implement `execute_gate` constructing environment, spawning subprocess, capturing streams via Head+Tail buffers, handling timeouts, and missing binary resolution -- Core Story 3.1 execution engine.
- [x] `crates/qdev-core/schemas/payload-gate-run.json` -- Create JSON Schema for gate run payload and wire into `crates/qdev-core/src/schema.rs` -- Fulfills output contract and deferred work.
- [x] `crates/qdev-core/src/gate/mod.rs` and `crates/qdev-core/src/lib.rs` -- Expose gate runner API -- Public core interface.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `GateArgs` with `Run(GateRunArgs)` subcommand accepting `<id>` and optional `--story` -- CLI command parsing.
- [x] `crates/qdev-cli/src/handlers/gate.rs` and `crates/qdev-cli/src/handlers/mod.rs` -- Implement gate run handler formatting receipts and handling `--json` -- User-facing CLI behavior.
- [x] `crates/qdev-cli/src/main.rs` -- Connect `Commands::Gate` to `handlers::gate::handle_gate` -- CLI entrypoint wiring.
- [x] `crates/qdev-core/tests/gate_runner_tests.rs` -- Add unit tests for environment propagation, Head+Tail buffer retention/truncation banner, timeout termination, and missing executable -- Test verification.
- [x] `crates/qdev-cli/tests/gate_cli_tests.rs` -- Add CLI integration tests for `qdev gate run`, receipt format, local skip, and timeout tree kill -- End-to-end verification.

**Acceptance Criteria:**
- Given a gate in `qdev.toml`, when running `qdev gate run <id>`, then parent env, `[environment]`, `QDEV_STORY`, `QDEV_GATE`, `QDEV_COMMIT`, `QDEV_RESULT_FILE`, and `QDEV_MODULE_PATHS` are passed to the subprocess.
- Given a gate that outputs >1 MB of data, when it executes, then stdout and stderr Head+Tail buffers retain the first 64 KB, the most recent 960 KB, and report total truncated bytes between them without exceeding 1 MB memory limit.
- Given a gate that exceeds `timeout_ms`, when the timeout fires, then the process and all descendant processes are terminated via process groups (Unix) or Job Objects (Windows), returning status `infra` and exit code 4.
- Given a gate pointing to a missing executable, when run, then status `infra` is returned with the resolved path and exit code 4.
- Given a gate configured with `skip = true` in local config, when run, then the process is not spawned, exit code is 0, and the receipt contains `skipped_locally`.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---------|----------|---------|----------------------|-------------|
| Reader thread join hang on background grandchild | edge-case-hunter / blind-hunter | `medium` | If child exits but spawned grandchildren keep stdout/stderr open, `join()` blocks indefinitely without killing remaining process tree | patch |
| Windows `resolve_binary` missing `PATHEXT` probing | edge-case-hunter / blind-hunter | `high` | Checking `candidate.is_file()` without appending `.exe`/`.cmd`/`.bat` causes standard Windows tools like `cargo` to be marked missing | patch |
| `Command::new(binary_str)` vs `resolved_path` | blind-hunter / verification-gap | `medium` | Bare filenames in workspace root resolve to local files in `resolve_binary` but `Command::new` fails to locate them via PATH | patch |
| Subprocess stdin unconfigured (triggers SIGTTIN) | blind-hunter | `medium` | Stdin defaults to inherit; reading from stdin inside a non-foreground process group triggers SIGTTIN | patch |
| Empty quoted argument dropped in `parse_command_args` | edge-case-hunter / blind-hunter | `low` | Quoted empty strings `""` are dropped because `!current.is_empty()` check excludes them | patch |
| Commit SHA slicing char boundary defense | edge-case-hunter | `low` | `s[..7]` could panic on non-ASCII commit metadata; use char iterator | patch |
| Single-line receipt broken by multi-line summary | blind-hunter | `low` | Summaries containing newlines break the single-line receipt contract; sanitize to single line | patch |
| Temporary result JSON file leak on error | edge-case-hunter / blind-hunter | `low` | Result JSON file may leak on unexpected early return | patch |
| Schema `payload-gate-run.json` `additionalProperties: false` | blind-hunter | `low` | Blocks additive evolution of schema across subsequent Epic 3 stories; remove false restriction | patch |
| Dead constant `DEFAULT_MAX_CAPACITY` | blind-hunter | `low` | Unused constant in `ring_buffer.rs` | patch |
| Workspace guard test omits `gate run` | verification-gap | `low` | Pre-verified gap: `guarded_invocations()` in `workspace_guard_cli_tests.rs` did not include `gate run` | patch |
| Active story resolution from lease untested | verification-gap | `low` | Pre-verified gap: lease-based derivation when `--story` is None was unexercised | patch |
| Multi-token argument parsing untested | verification-gap | `low` | Pre-verified gap: all test fixtures used single-token commands without arguments | patch |
| Missing CLI `--timeout-ms` flag | blind-hunter | `false` | Story 3.1 AC specifies timeout configuration via `qdev.toml`; CLI flag is not part of the requirement | reject |
| `execute_gate` ignores result JSON status | blind-hunter | `false` | Story 3.2 is explicitly responsible for Gate Result Contract validation and failure taxonomy | reject |


## Design Notes

### Process Tree Termination
On Unix platforms, the child process is placed in a new process group via `.process_group(0)`. On timeout, `libc::kill(-(pid as i32), libc::SIGKILL)` is dispatched to the negative process group ID, terminating the child and all grandchildren. On Windows, the process handle is assigned to a Win32 Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, ensuring immediate process tree destruction when closed or terminated via `TerminateJobObject`.

### Head+Tail Buffer Architecture
A `HeadTailBuffer` holds:
1. An immutable `head: Vec<u8>` capped at 64 KB (`65_536` bytes) that stores the stream prefix.
2. A rolling `tail: VecDeque<u8>` capped at 960 KB (`983_040` bytes) that preserves the most recent bytes.
3. A counter `truncated_bytes: usize`.

When total stream bytes ≤ 1 MB (`1_048_576` bytes), bytes flow directly into `head` until 64 KB, then into `tail`, assembling contiguously with no markers. When total stream bytes exceed 1 MB, each byte entering `tail` pushes the oldest non-head byte out and increments `truncated_bytes`. On readout, the buffer renders:
`[head bytes] + "\n[... qdev: <truncated_bytes> bytes truncated ...]\n" + [tail bytes]`.
Background reader threads concurrently read stdout/stderr until EOF.

## Verification

**Commands:**
- `cargo check --workspace` -- expected: Clean compilation of all crates.
- `cargo test -p qdev-core --test gate_runner_tests` -- expected: All unit tests pass.
- `cargo test -p qdev-cli --test gate_cli_tests` -- expected: All integration tests pass.
- `cargo test --workspace` -- expected: Entire test suite passes.
