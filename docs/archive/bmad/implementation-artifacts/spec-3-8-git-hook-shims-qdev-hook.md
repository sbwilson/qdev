---
title: 'Story 3.8: Git Hook Shims & qdev hook'
type: 'feature'
created: '2026-09-25'
status: 'done'
baseline_commit: '01f4484e77bf1ededdd0de1ba80396f7a864f625'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Git operations (commit, push) can bypass local quality gates, commit secrets or PHI into scratchpads/evidence, or push out-of-scope/stale branches unless enforced automatically at the version control boundary by reliable, cross-platform Git hooks.
**Approach:** Implement `qdev install hooks` to deploy lightweight two-line POSIX shims (`pre-commit`, `pre-push`, `prepare-commit-msg`) calling `qdev hook <name>`, preserving non-qdev hooks by chaining (`<name>.legacy`), enforce hygiene diff, changed validation, and NFR-404 secret pattern scans on `pre-commit`, preflight on `pre-push`, and provide missing/outdated shim detection and remediation via `qdev doctor [--fix]`.

## Boundaries & Constraints

**Always:**
- Write hook shims (`pre-commit`, `pre-push`, `prepare-commit-msg`) in the Git hooks directory (`.git/hooks` or git common directory hooks) as two-line POSIX shims calling `exec qdev hook <name> "$@"` with LF (`\n`) newlines and executable permissions (`0o755` on Unix).
- Preserve existing non-qdev hooks when installing shims by renaming them to `<name>.legacy` (preserving permissions) and chaining their execution inside `qdev hook <name>`.
- In `qdev hook pre-commit`:
  1. Run `hygiene check --diff` (fail with exit 1 on any comment hygiene finding).
  2. Run `validate --changed` (fail with exit 1 on any error-severity validation finding in files changed against `config.git.integration_branch`).
  3. When `config.hygiene.secret_patterns` is non-empty, scan staged scratchpad files (`docs/state/scratch/**` or `.jsonl`) and staged evidence files (`docs/state/evidence/**` or `.json`) using `git diff --cached --name-only` and staged content inspection. If any regex matches, fail with exit 1 and report the violating file and pattern (NFR-404).
  4. If chained `.git/hooks/pre-commit.legacy` exists, execute it with forwarded arguments. If it exits non-zero, propagate its exit code.
- In `qdev hook pre-push`:
  1. Run preflight (`run_preflight`). If refusal, exit 3 (`PolicyRefusal`). If infrastructure failure, exit 4 (`InfrastructureFailure`).
  2. If chained `.git/hooks/pre-push.legacy` exists, execute it forwarding arguments and standard input.
- In `qdev hook prepare-commit-msg`:
  1. When `config.commit_messages.enabled` is `false` (default), treat as a no-op (exit 0).
  2. When `enabled` is `true`, draft message into target commit message file if message is empty/default.
  3. If chained `.git/hooks/prepare-commit-msg.legacy` exists, execute it with forwarded arguments.
- In `qdev doctor`:
  - Add a `"hooks"` diagnostic section reporting `status`, `unavailable_reason`, `all_installed`, `missing_hooks`, and `outdated_hooks`.
  - Report `status: "ok"` when all three shims exist, are executable, and contain `qdev hook <name>`.
  - Report `status: "mismatch"` when any shim is missing or does not match expected contents.
  - When `--fix` is passed to `qdev doctor`, rewrite missing or outdated shims (preserving legacy hooks) and report them as repaired.
- Add `secret_patterns: Vec<String>` to `[hygiene]` configuration in `qdev.toml` schema and config parser.

**Never:**
- Never clobber or delete an existing non-qdev hook without preserving it as `<name>.legacy`.
- Never use CRLF line endings in shims; shims must remain compatible with Git for Windows MSYS bash.
- Never let `doctor` exit non-zero due to missing or outdated hooks (`doctor` is a diagnostic report).
- Never scan working tree unstaged files for the pre-commit secret check; only inspect staged changes (`git diff --cached`).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Install clean | `.git/hooks` has no existing hooks | Writes `pre-commit`, `pre-push`, `prepare-commit-msg` shims (0o755); exit 0 | N/A |
| Install with legacy hook | Existing custom `.git/hooks/pre-commit` | Renames to `pre-commit.legacy`, writes new shim; exit 0 | Preserves permissions |
| Reinstall existing qdev shims | Hooks already contain `qdev hook <name>` | Updates shims in-place; does NOT rename qdev shim to `.legacy` | Idempotent |
| Pre-commit clean | Clean diff, 0 validate errors, 0 secret matches | Exit 0 | N/A |
| Pre-commit secret found | Staged scratchpad contains string matching `secret_patterns` | Exit 1; outputs matched pattern and file path | LogicalFailure (exit 1) |
| Pre-commit validate fails | Staged changes cause error finding in `validate --changed` | Exit 1; prints validation error findings | LogicalFailure (exit 1) |
| Pre-push dirty scope | Uncommitted changes outside leased module | Exit 3; prints preflight refusal with stash command | PolicyRefusal (exit 3) |
| Pre-commit chained hook | `pre-commit.legacy` exists and fails (exit 42) | Exit 42 (propagates legacy hook exit code) | Propagated exit code |
| Doctor without shims | No shims in `.git/hooks` | Reports `status = mismatch`, `all_installed = false`, `missing_hooks = [...]` | Exit 0 (doctor report) |
| Doctor with `--fix` | Missing or outdated shims present | Rewrites shims to `.git/hooks/`, reports `all_installed = true` | Exit 0 |
| Non-git directory | Workspace is not a git repository | Doctor hooks section reports `status = unavailable`, reason `not_a_git_repository` | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/config/types.rs` -- Add `pub secret_patterns: Vec<String>` to `HygieneConfig`.
- `crates/qdev-core/src/config/mod.rs` -- Validate and extract `secret_patterns` string array under `[hygiene]` section.
- `crates/qdev-core/src/config/source.rs` -- Emit `secret_patterns` in annotated config output.
- `crates/qdev-core/src/hook.rs` -- New module: hook constants (`EXPECTED_HOOKS`, shim template), `install_hooks`, `inspect_hooks`, secret pattern scanner for staged scratchpad/evidence files, and hook execution dispatchers (`run_pre_commit`, `run_pre_push`, `run_prepare_commit_msg`).
- `crates/qdev-core/src/doctor.rs` -- Implement `HooksDoctorSection` inspecting hook status; wire into `default_doctor_sections`.
- `crates/qdev-core/schemas/payload-doctor.json` -- Add JSON Schema definition for `hooks` section under `allOf`.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::HookInstall` (payload `hook_install`).
- `crates/qdev-core/schemas/payload-hook-install.json` -- JSON Schema for `qdev install hooks` output envelope.
- `crates/qdev-core/src/lib.rs` -- Export `hook` module and `HooksDoctorSection`.
- `crates/qdev-cli/src/cli.rs` -- Add `Commands::Install(InstallArgs)`, `Commands::Hook(HookArgs)`, `Commands::Hygiene(HygieneArgs)`, and `DoctorArgs` with `--fix` flag.
- `crates/qdev-cli/src/handlers/install.rs` -- CLI handler for `qdev install hooks`.
- `crates/qdev-cli/src/handlers/hook.rs` -- CLI handler for `qdev hook <name>`.
- `crates/qdev-cli/src/handlers/hygiene.rs` -- CLI handler for `qdev hygiene check [--diff]`.
- `crates/qdev-cli/src/handlers/doctor.rs` or `main.rs` -- Handle `--fix` flag to rewrite hook shims before running doctor sections.
- `crates/qdev-core/tests/hook_tests.rs` -- Unit tests for hook installation, shim generation, legacy chaining, and staged secret scanning.
- `crates/qdev-cli/tests/hook_cli_tests.rs` -- Integration tests for `qdev install hooks`, `qdev hook pre-commit`, `qdev hook pre-push`, `qdev doctor` reporting and `--fix`.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/config/types.rs` -- Add `secret_patterns` field to `HygieneConfig` -- Configuration data model.
- [x] `crates/qdev-core/src/config/mod.rs` -- Add `secret_patterns` validation and parsing in `validate_hygiene_section` and config loading -- Config validation.
- [x] `crates/qdev-core/src/config/source.rs` -- Add `secret_patterns` tracking in config source emitter -- Config provenance.
- [x] `crates/qdev-core/schemas/payload-hook-install.json` -- Define JSON Schema for hook installation envelope -- Hook install contract.
- [x] `crates/qdev-core/schemas/payload-doctor.json` -- Add `hooks` section definition to doctor payload schema -- Doctor schema extension.
- [x] `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::HookInstall` and embed schema -- Schema payload registry.
- [x] `crates/qdev-core/src/hook.rs` -- Implement shim installation, legacy chaining, hook inspection, staged secret scanner, and execution runners -- Core hook engine.
- [x] `crates/qdev-core/src/doctor.rs` -- Implement `HooksDoctorSection` and add to `default_doctor_sections` -- Doctor diagnostics integration.
- [x] `crates/qdev-core/src/lib.rs` -- Export hook module and types -- Core library exports.
- [x] `crates/qdev-cli/src/cli.rs` -- Define CLI grammar for `install`, `hook`, `hygiene`, and update `Doctor` with `DoctorArgs` -- CLI argument parsing.
- [x] `crates/qdev-cli/src/handlers/install.rs` -- Implement `handle_install_hooks` -- CLI install handler.
- [x] `crates/qdev-cli/src/handlers/hook.rs` -- Implement `handle_hook` -- CLI hook dispatcher.
- [x] `crates/qdev-cli/src/handlers/hygiene.rs` -- Implement `handle_hygiene` -- CLI hygiene stub handler.
- [x] `crates/qdev-cli/src/handlers/mod.rs` -- Expose install, hook, and hygiene handlers -- Handler module exports.
- [x] `crates/qdev-cli/src/main.rs` -- Route CLI commands and handle `doctor --fix` -- CLI main routing.
- [x] `crates/qdev-core/tests/hook_tests.rs` -- Unit tests for shim generation, legacy chaining, and staged secret scanner -- Core test coverage.
- [x] `crates/qdev-cli/tests/hook_cli_tests.rs` -- CLI integration tests for `install hooks`, `hook pre-commit`, `hook pre-push`, and `doctor --fix` -- CLI test coverage.

**Acceptance Criteria:**
- Given `qdev install hooks`, when it runs, then `pre-commit`, `pre-push`, and `prepare-commit-msg` shims are written as in `docs/compliance-and-safety.md` §6, existing non-qdev hooks are preserved by chaining, and the shims work under Git for Windows.
- `qdev hook pre-commit` runs `hygiene check --diff`, `validate --changed`, and, when `[hygiene] secret_patterns` is configured, a pattern scan of staged scratchpad and evidence files (NFR-404).
- `qdev hook pre-push` runs `preflight`.
- `qdev doctor` reports missing or outdated shims and `--fix` rewrites them.

## Implementation Notes

- Added `pub secret_patterns: Vec<String>` to `HygieneConfig` in `crates/qdev-core/src/config/types.rs`, supported in `crates/qdev-core/src/config/mod.rs` and provenance in `source.rs`.
- Implemented `crates/qdev-core/src/hook.rs` providing:
  - `EXPECTED_HOOKS`: `pre-commit`, `pre-push`, `prepare-commit-msg`.
  - `shim_content`: Writes two-line POSIX shims calling `exec qdev hook <name> "$@"` with LF newlines and `0o755` permissions on Unix.
  - `install_hooks`: Writes shims, preserves non-qdev hooks by renaming to `<name>.legacy` (guarding against clobbering existing legacy hooks), and idempotently updates existing qdev shims.
  - `inspect_hooks`: Examines hook directory and reports `HookStatus` with `all_installed`, `missing_hooks`, and `outdated_hooks`.
  - `scan_staged_secrets`: Inspects staged changes via `git diff --cached --name-only -c core.quotePath=false --relative`, strips quotes, and matches regex patterns against staged files in scratchpad (`docs/state/scratch/**` / `.jsonl`) and evidence (`docs/state/evidence/**` / `.json`) directories (NFR-404).
  - `run_legacy_hook`: Executes `<name>.legacy` in `workspace_root`, redirects stdin to `Stdio::null()` unless `forward_stdin` is true, invokes via `sh` on Windows for shell scripts, and propagates exit codes.
  - `run_pre_commit`: Runs `qdev hygiene check --diff`, `validate --changed` (falling back to staged files if integration branch diff is unavailable), staged secret pattern scan, and chained legacy hook.
  - `run_pre_push`: Runs preflight guard and chained legacy hook with forwarded stdin.
  - `run_prepare_commit_msg`: Handles draft messages if enabled and invokes chained legacy hook.
- Implemented `HooksDoctorSection` in `crates/qdev-core/src/doctor.rs` and added to `default_doctor_sections`.
- Extended `crates/qdev-core/schemas/payload-doctor.json` with the `"hooks"` section under `allOf`.
- Defined `payload-hook-install.json` and registered `PayloadKind::HookInstall` in `crates/qdev-core/src/schema.rs`.
- Implemented CLI commands:
  - `qdev install hooks [--json]`: `crates/qdev-cli/src/handlers/install.rs`.
  - `qdev hook <name> [args]`: `crates/qdev-cli/src/handlers/hook.rs`.
  - `qdev hygiene check [--diff]`: `crates/qdev-cli/src/handlers/hygiene.rs`.
  - `qdev doctor [--fix]`: Rewrites missing or outdated shims and reports repaired status.
- Added comprehensive unit tests in `crates/qdev-core/tests/hook_tests.rs` and integration tests in `crates/qdev-cli/tests/hook_cli_tests.rs`.

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---|---|---|---|---|
| Unverified execution and legacy chaining for `prepare-commit-msg` hook | verification-gap | `medium` | Pre-existing tests only checked shim presence, not execution; added tests verifying default no-op and legacy exit code propagation | patch |
| Standard input forwarding to `pre-push.legacy` is unverified | verification-gap | `medium` | Test did not verify stdin lines forwarded to legacy script; updated test to pipe refs on stdin and assert legacy hook reads them | patch |
| `HooksDoctorSection` diagnostic reporting on non-git workspaces is unverified | verification-gap | `low` | `HooksDoctorSection::run` on non-git folder was not covered; added unit test in `doctor_tests.rs` asserting `status == "unavailable"` | patch |
| Pre-existing `<name>.legacy` file collision during shim install | edge-case-hunter / blind-hunter | `low` | Renaming over existing legacy hook could fail or clobber; added guard checking `legacy_path.exists()` before renaming | patch |
| Git quotePath and non-relative path in `scan_staged_secrets` | edge-case-hunter / blind-hunter | `medium` | Staged paths with spaces/non-ASCII could be quoted by Git and miss path checks; added `-c core.quotePath=false` and `--relative` and quote stripping | patch |
| Over-broad file matching in `scan_staged_secrets` | blind-hunter | `medium` | Matching any `.jsonl` or `.json` outside scratchpad/evidence dirs could false-positive; restricted strictly to scratchpad/evidence directories | patch |
| `run_pre_commit` omitted hygiene execution | blind-hunter / edge-case-hunter | `medium` | Pre-commit had comment placeholder instead of invoking hygiene check; added explicit subprocess execution of `qdev hygiene check --diff` | patch |
| `git_changed_files` fallback in `run_pre_commit` | blind-hunter / edge-case-hunter | `medium` | When integration branch was missing, whole repo was validated; added fallback to staged files (`git diff --cached --name-only`) | patch |
| `std::process::exit` directly in `run()` inside `main.rs` | blind-hunter | `medium` | Bypassed runtime unwinding in tests; refactored `run` to return `i32` and let `main()` convert to `StdExitCode` | patch |
| `run_legacy_hook` working dir and windows script execution | blind-hunter | `low` | Legacy hooks executed in invocation directory without null stdin; set `workspace_root` as `current_dir`, set `Stdio::null()` when stdin not forwarded, and invoke `sh` on Windows | patch |
| Discarded errors in `handle_doctor --fix` | blind-hunter | `low` | `install_hooks(&root)` result was discarded; handled result and emitted error on failure | patch |


## Design Notes

### Shim Content
```sh
#!/bin/sh
exec qdev hook <name> "$@"
```
Written with standard POSIX shell syntax and LF line endings so Git for Windows executes it cleanly via MSYS bash.

### Legacy Hook Chaining
When `qdev install hooks` encounters an existing `.git/hooks/<name>` that does not contain `qdev hook <name>`, it renames it to `<name>.legacy`.
When `qdev hook <name>` runs:
1. Core checks execute first (`preflight` for pre-push, `hygiene` + `validate` + secret scan for pre-commit).
2. If core checks fail, execution aborts immediately with the appropriate exit code.
3. If core checks succeed and `<name>.legacy` exists, it executes `<name>.legacy "$@"` and propagates its exit code.

### NFR-404 Secret Scanning
Inspects staged files (`git diff --cached --name-only`) that belong to scratchpad or evidence directories:
- `docs/state/scratch/**` (or `.jsonl` files)
- `docs/state/evidence/**` (or `.json` files)
Staged content is extracted via `git show :<staged-path>` and matched against compiled regex patterns from `config.hygiene.secret_patterns`.

## Verification

**Commands:**
- `cargo test --test hook_tests` -- expected: All core hook unit tests pass.
- `cargo test --test hook_cli_tests` -- expected: CLI integration tests for `install hooks`, `hook pre-commit`, `hook pre-push`, and `doctor --fix` pass.
- `cargo test` -- expected: Full test suite passes.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero clippy warnings.
