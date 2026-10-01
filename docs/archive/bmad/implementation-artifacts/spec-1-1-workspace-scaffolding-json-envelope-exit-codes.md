---
title: 'Workspace Scaffolding, JSON Envelope & Exit Codes'
type: 'feature'
created: '2026-09-06'
status: 'done'
baseline_revision: '8e49fa226009e7e96bd7479776bd976f764c92f1'
review_loop_iteration: 0
followup_review_recommended: false
context:
  - 'docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md'
  - 'docs/cli-reference.md'
warnings: []
deferred: []
---

<intent-contract>

## Intent

**Problem:** The repository currently lacks the foundational Rust workspace, CLI entry point, and core data contracts required to support all subsequent qdev features and tool integrations.

**Approach:** Establish a two-crate Cargo workspace (`qdev-core` and `qdev-cli`) implementing AD-1 strict layer separation, AD-12 interactivity parsing (`--non-interactive` and `QDEV_NONINTERACTIVE`), AD-13 universal JSON output envelopes and standardized exit codes (0–5), a multi-platform CI matrix, and offline execution verification.

## Boundaries & Constraints

**Always:**
- `qdev-core` must remain strictly independent of `clap`, terminal crates (`colored`, `crossterm`, `console`), stdout printing, and stdin reading per AD-1.
- All JSON output must carry `"schema_version": "1"` and adhere to the AD-13 envelope structure.
- Exit codes must strictly conform to AD-13: 0 for success, 1 for logical failure, 2 for usage error, 3 for policy/confirmation, 4 for infrastructure failure, 5 for conflict.
- Commands with `--json` must emit the error envelope on stdout; commands in text mode emit on stderr.

**Never:**
- Never add network client dependencies (`reqwest`, `hyper`, `curl`, `ureq`) to any crate in the workspace.
- Never make unsolicited outbound network requests.
- Never allow unhandled panics or raw clap error formats to bypass the AD-13 JSON error envelope when in JSON mode.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Version Text | `qdev --version` | Prints version string to stdout (e.g. `qdev 0.1.0`), exit code 0 | No error expected |
| Version JSON | `qdev --version --json` | JSON payload with `"schema_version": "1"`, `"version": "0.1.0"` to stdout, exit code 0 | No error expected |
| Version JSON Reversed | `qdev --json --version` | JSON payload with `"schema_version": "1"`, `"version": "0.1.0"` to stdout, exit code 0 | No error expected |
| Unknown Subcommand JSON | `qdev unknown-cmd --json` | JSON error envelope with `code: "usage_error"` to stdout, exit code 2 | Handled via AD-13 error envelope |
| Unknown Subcommand Text | `qdev unknown-cmd` | Error envelope or formatted usage error to stderr, exit code 2 | Usage error exit code 2 |
| Non-Interactive Flag | `qdev --non-interactive <cmd>` | Parses `Interactivity::NonInteractive` and passes to core | Invalid flag handled as usage error |
| Non-Interactive Env Var | `QDEV_NONINTERACTIVE=1 qdev <cmd>` | Parses `Interactivity::NonInteractive` and passes to core | Invalid value falls back safely |
| Offline Verification | Network disabled / proxies to invalid endpoints | All commands succeed locally without network attempts | Fails if any network call is initiated |

</intent-contract>

## Code Map

- `Cargo.toml` -- Root Cargo workspace declaring `crates/qdev-core` and `crates/qdev-cli`
- `crates/qdev-core/Cargo.toml` -- Core crate manifest with serde dependencies, strictly no clap or terminal crates
- `crates/qdev-core/src/lib.rs` -- Core crate root exporting envelope, errors, and interactivity modules
- `crates/qdev-core/src/envelope.rs` -- Universal JSON envelope (`JsonEnvelope`, `JsonErrorEnvelope`) with schema versioning
- `crates/qdev-core/src/errors.rs` -- Error taxonomy (`QdevError`, `ExitCode`, error codes) conforming to AD-13
- `crates/qdev-core/src/interactivity.rs` -- `Interactivity` domain enum (`Interactive`, `NonInteractive`) per AD-12
- `crates/qdev-core/tests/architecture_tests.rs` -- Automated test asserting `qdev-core` does not depend on clap or terminal crates
- `crates/qdev-cli/Cargo.toml` -- CLI crate manifest depending on `qdev-core`, `clap`, `serde_json`, and test utilities
- `crates/qdev-cli/src/main.rs` -- CLI binary entrypoint with global error handler and exit code translation
- `crates/qdev-cli/src/cli.rs` -- Clap command-line argument parser with `--json` and `--non-interactive` global flags
- `crates/qdev-cli/src/output.rs` -- Output emitter dispatching text or JSON to stdout/stderr based on flags
- `crates/qdev-cli/tests/cli_tests.rs` -- Integration tests for version, unknown subcommand, exit codes, and envelope schemas
- `crates/qdev-cli/tests/network_tests.rs` -- Integration tests verifying zero network calls under offline/isolated environments
- `.github/workflows/ci.yml` -- GitHub Actions CI matrix covering macOS, Linux, and Windows

## Tasks & Acceptance

**Execution:**
- `Cargo.toml` -- Create workspace root manifest -- AD-1 workspace setup
- `crates/qdev-core/Cargo.toml` -- Create core manifest without CLI/terminal dependencies -- AD-1 layer boundary
- `crates/qdev-core/src/lib.rs` -- Expose envelope, errors, and interactivity modules -- Public core API
- `crates/qdev-core/src/envelope.rs` -- Implement `JsonEnvelope<T>` and `JsonErrorEnvelope` -- AD-13 universal JSON format
- `crates/qdev-core/src/errors.rs` -- Implement `ExitCode` (0..5) and `QdevError` -- AD-13 exit codes and error structures
- `crates/qdev-core/src/interactivity.rs` -- Implement `Interactivity` enum and methods -- AD-12 non-interactive model
- `crates/qdev-core/tests/architecture_tests.rs` -- Unit test verifying absence of forbidden dependencies in `qdev-core` -- Guard AD-1 against regressions
- `crates/qdev-cli/Cargo.toml` -- Create CLI manifest with binary named `qdev` -- Binary build definition
- `crates/qdev-cli/src/cli.rs` -- Define CLI grammar with `--json` and `--non-interactive` -- Global flag definition
- `crates/qdev-cli/src/output.rs` -- Output formatting helpers for stdout/stderr -- AD-13 stream separation
- `crates/qdev-cli/src/main.rs` -- Main runner catching all errors and formatting envelopes -- AD-13 exit handling
- `crates/qdev-cli/tests/cli_tests.rs` -- Comprehensive CLI integration tests for all acceptance criteria -- Verification of CLI contract
- `crates/qdev-cli/tests/network_tests.rs` -- Offline execution test asserting zero network access -- Verification of NFR-403 offline invariant
- `.github/workflows/ci.yml` -- CI workflow running build and tests across macOS, Linux, and Windows -- AD-14 multi-platform CI

**Acceptance Criteria:**
- Given a fresh checkout, when running `cargo build --release`, then the workspace contains `qdev-cli` and `qdev-core`, and `qdev-core` has no dependency on `clap` or any terminal crate.
- Given the built binary, when running `qdev --version`, then it prints the version string and exits with code 0.
- Given the built binary, when running `qdev --version --json`, then it outputs a JSON object containing `"schema_version": "1"` and `"version": "0.1.0"` to stdout and exits with code 0.
- Given the built binary, when invoking an unknown subcommand with `--json`, then it outputs a JSON error envelope with `code: "usage_error"` to stdout and exits with code 2.
- Given the built binary, when invoking an unknown subcommand without `--json`, then it outputs an error message or error envelope to stderr and exits with code 2.
- Given the built binary, when running with `--non-interactive` or `QDEV_NONINTERACTIVE=1`, then the CLI exposes `Interactivity::NonInteractive` to core routines.
- Given the test suite, when running `cargo test`, then all tests pass, including an offline test that asserts zero network access.
- Given the repository, when inspected, then `.github/workflows/ci.yml` defines build and test jobs for macOS, Linux, and Windows.

## Spec Change Log

## Review Triage Log

### 2026-09-06 — Review pass
- verdicts: 26 findings — high 0, medium 2, low 18, false 6, maybe-false 0
- findings:
  - `[medium]` `[patch]` `is_json_requested` matches `args[0]` if binary path contains `--json` — skip `args[0]` via `args.iter().skip(1)`
  - `[low]` `[patch]` `emit_envelope` and `emit_error_envelope` use `println!` panicking on broken pipe — handle `io::ErrorKind::BrokenPipe` safely via `writeln!`
  - `[false]` `[reject]` `output.emit_envelope` error prints to stderr instead of stdout JSON — if stdout write fails, writing JSON to stdout is impossible; stderr diagnostic is standard
  - `[false]` `[reject]` `DisplayHelp` errors from clap print plain text instead of JSON in JSON mode — `--help` is standard human help text inspection
  - `[low]` `[patch]` `emit_error` in text mode drops `error.code()` — updated to print `eprintln!("{}", error)` to include both code and message
  - `[low]` `[patch]` `architecture_tests.rs` only shallowly reads `src/` — recursively scan all `.rs` files in subdirectories
  - `[low]` `[patch]` `architecture_tests.rs` uses `--no-deps` — inspect full workspace dependency graph to ensure no transitive forbidden crates
  - `[low]` `[patch]` `network_tests.rs` listener probe delays test run 500ms — signaled probe with atomic stop flag to exit immediately when CLI finishes
  - `[low]` `[patch]` `test_no_forbidden_network_dependencies_in_workspace` silently passes if `Cargo.lock` is missing — added `assert!(root_cargo_lock.exists())`
  - `[low]` `[reject]` `JsonEnvelope<T>` flattening limitation on non-struct collections — all qdev payloads in schema are structured maps/objects
  - `[low]` `[patch]` CI workflow lacks formatting and clippy checks — added `cargo fmt --check` and `cargo clippy -- -D warnings`
  - `[false]` `[reject]` `qdev status` in text mode prints only version — full pulse formatting is scheduled for Story 2.11 per roadmap
  - `[low]` `[patch]` `parse_env_value` unused in `interactivity.rs` — refactored `Interactivity::resolve` to delegate directly to `parse_env_value`
  - `[false]` `[reject]` CI caching target directory across OSes without isolation — cache key already isolates by runner OS: `${{ runner.os }}-cargo-...`
  - `[low]` `[patch]` Standard output stream broken or closed panics — handled `ErrorKind::BrokenPipe` in output emitter
  - `[medium]` `[patch]` Positional argument after `--` activates JSON output mode — stopped argument scan at `--` delimiter
  - `[false]` `[reject]` Clap help text prints to stdout instead of JSON envelope — `--help` is standard terminal help documentation
  - `[false]` `[reject]` Failed envelope emission prints to stderr instead of stdout — stdout I/O failure cannot be handled by writing to stdout
  - `[low]` `[patch]` Invalid `QDEV_NONINTERACTIVE` fallback bypassed by `--version` in test — updated test to run `status --json` reaching interactivity resolution
  - `[low]` `[patch]` Non-TTY stdin in test runner causes vacuous pass for interactivity tests — asserted and verified flag and env resolution across all combinations
  - `[low]` `[patch]` Text mode execution of `qdev` and `qdev status` completely untested — added unit and integration tests for text mode execution
  - `[low]` `[patch]` `PulseStatus` `name` and `version` fields unasserted across all JSON integration and unit tests — added explicit assertions for `name` and `version`
  - `[low]` `[patch]` `test_no_forbidden_network_dependencies_in_workspace` silent pass risk — added assertion for lockfile existence
  - `[low]` `[patch]` `architecture_tests.rs` shallow directory scan — updated to recursive directory walk
  - `[false]` `[reject]` `DisplayHelp` in JSON mode — help output is formatted for terminal display
  - `[low]` `[patch]` `emit_error` drops error code in text mode — formatted error with code and message

## Design Notes

Clap argument parsing will parse raw arguments with `try_get_matches_from`. If clap returns an error (e.g. `InvalidSubcommand` or `UnknownArgument`), the CLI checks if `--json` is present in the arguments. If `--json` is present, it constructs a `QdevError::usage_error` and formats it using the `JsonErrorEnvelope` directly to stdout, exiting with code 2. This prevents clap from bypassing the AD-13 error envelope.

## Verification

**Commands:**
- `cargo build --release` -- expected: successful release build of `qdev` binary and `qdev-core` crate
- `cargo test` -- expected: all unit, integration, architecture, and network isolation tests pass
- `cargo run --bin qdev -- --version` -- expected: exit 0, text output
- `cargo run --bin qdev -- --version --json` -- expected: exit 0, JSON payload with schema_version "1"
- `cargo run --bin qdev -- unknown --json` -- expected: exit 2, AD-13 JSON error envelope on stdout
- `cargo test --test architecture_tests` -- expected: pass, confirms no clap or terminal crates in `qdev-core`

## Auto Run Result

Status: done

### Implemented Changes
Established the two-crate Rust workspace (`qdev-core` and `qdev-cli`) forming the architectural substrate of qdev. Implemented the universal JSON output envelope (`JsonEnvelope<T>` and `JsonErrorEnvelope`) carrying `schema_version: "1"`, the standardized exit code taxonomy (0..5) per AD-13, and the interactivity domain model per AD-12 (`--non-interactive` flag and `QDEV_NONINTERACTIVE` environment variable). Added multi-platform GitHub Actions CI matrix for macOS, Linux, and Windows per AD-14, and verified offline execution with zero network access per NFR-403.

### Files Changed
- `Cargo.toml`: Root Cargo workspace configuration with `crates/qdev-core` and `crates/qdev-cli`.
- `crates/qdev-core/Cargo.toml`: Core domain crate manifest free of clap, terminal crates, and network clients.
- `crates/qdev-core/src/lib.rs`: Exports core modules (`envelope`, `errors`, `interactivity`) and pulse status routine.
- `crates/qdev-core/src/envelope.rs`: Implements `JsonEnvelope<T>` and `JsonErrorEnvelope` with `schema_version = "1"`.
- `crates/qdev-core/src/errors.rs`: Implements `ExitCode` (0..5) and `QdevError` taxonomy.
- `crates/qdev-core/src/interactivity.rs`: Implements `Interactivity` domain model and resolution logic.
- `crates/qdev-core/tests/architecture_tests.rs`: Tests enforcing AD-1 crate separation and absence of terminal I/O in core.
- `crates/qdev-cli/Cargo.toml`: CLI crate manifest producing binary `qdev`.
- `crates/qdev-cli/src/main.rs`: CLI binary entrypoint with error interception, panic hook, and exit translation.
- `crates/qdev-cli/src/cli.rs`: Clap argument parser with global `--json` and `--non-interactive` flags.
- `crates/qdev-cli/src/output.rs`: Stream formatting helpers for stdout/stderr with broken-pipe safety.
- `crates/qdev-cli/tests/cli_tests.rs`: Integration tests covering version, unknown subcommands, exit codes, and interactivity.
- `crates/qdev-cli/tests/network_tests.rs`: Tests asserting zero network calls under proxy blackholes and active listeners.
- `.github/workflows/ci.yml`: GitHub Actions CI matrix covering macOS, Linux, and Windows with test, clippy, and fmt checks.
- `docs/bmad/implementation-artifacts/epic-1-context.md`: Compiled developer-ready context for Epic 1.
- `docs/bmad/implementation-artifacts/spec-1-1-workspace-scaffolding-json-envelope-exit-codes.md`: Story specification, triage log, and run results.

### Review Findings Breakdown
- Patches applied:
  - Fixed `is_json_requested` bounds (`args.iter().skip(1).take_while(|a| *a != "--")`).
  - Added broken-pipe handling in `output.rs` to avoid panics when piping stdout.
  - Formatted text errors with both error code and message.
  - Refactored `Interactivity::resolve` to use `parse_env_value`.
  - Upgraded architecture tests to recursively inspect all subdirectories and full dependency graph.
  - Added atomic stop flag to network test listener probe, reducing test execution time by 500ms.
  - Added explicit lockfile existence assertion in dependency checks.
  - Added text mode tests for `qdev` and `qdev status`, and asserted `PulseStatus` `name` and `version` fields.
  - Added `cargo fmt` and `cargo clippy` to CI workflow.
- Items deferred: None (`deferred: []`).
- Rejected findings:
  - `output.emit_envelope` error prints to stderr: Rejected because stdout write failures cannot be recovered by writing to stdout.
  - `DisplayHelp` in JSON mode: Rejected because `--help` is intended as human terminal documentation.
  - `JsonEnvelope<T>` non-struct flattening: Rejected because all qdev schema entities are structured objects.
  - `qdev status` text output brevity: Rejected because full pulse output is scheduled for Story 2.11.
  - CI cross-OS cache collision: Rejected because cache key already segments by `${{ runner.os }}`.

### Follow-up Review Recommendation
`followup_review_recommended: false`. All patched issues were localized code hygiene and test coverage items (1 medium group, 7 low groups, 0 high groups). The implementation fully converged.

### Verification Performed
- `cargo build --release`: Passed, producing release `qdev` binary.
- `cargo test`: All 30 tests passed across workspace.
- `cargo clippy --workspace --all-targets -- -D warnings`: Passed with 0 warnings.
- `cargo fmt --all --check`: Passed with 0 diffs.
- `cargo run --bin qdev -- --version`: Exits 0 with `qdev 0.1.0`.
- `cargo run --bin qdev -- --version --json`: Exits 0 with `{"schema_version": "1", "version": "0.1.0"}`.
- `cargo run --bin qdev -- unknown --json`: Exits 2 with AD-13 JSON error envelope on stdout.
- `cargo test --test architecture_tests`: Passed, verifying AD-1 compliance.
- `cargo test --test network_tests`: Passed, verifying NFR-403 offline execution.

### Residual Risks
None. The crate boundary, exit codes, JSON envelope, and non-interactive handling are thoroughly covered by unit, integration, and architecture tests.
