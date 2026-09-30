---
title: 'Story 4.7: Doctor: Environment, Skills, MCP, Hooks, Leases'
type: 'feature'
created: '2026-10-01'
status: 'done'
baseline_commit: 'aaf8e895e85cb3f4a89b2a5a884c5310c3479e0b'
route: 'dispatch'
review_loop_iteration: 0
context:
  - '{project-root}/docs/cli-reference.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev doctor` only reports a subset of diagnostic sections (`cache`, `validation`, `leases`, `hooks`, `skills`, `mcp`), omitting `git`, `modules`, and `gates` checks defined in CLI reference §7, and `--fix` only rewrites hook shims without regenerating outdated skills or rebuilding a corrupt cache after confirmation.

**Approach:** Add `GitDoctorSection`, `ModulesDoctorSection`, and `GatesDoctorSection` to `default_doctor_sections`, ensure each section returns `status` in JSON and updates the `payload-doctor.json` schema, render the 8-item status summary block alongside detailed section reports in text output matching CLI reference §7, and extend `--fix` (with `-y`/`--yes` support) to rewrite hook shims, regenerate outdated skills, and rebuild corrupt caches after confirmation or `--yes`.

## Boundaries & Constraints

**Always:**
- Keep `qdev doctor` as a diagnostic report, never a gate: findings never alter its exit code (exits 0 on findings, non-zero only for fatal invocation/infrastructure errors).
- Order sections deterministically in `default_doctor_sections` and JSON output: `cache`, `validation`, `leases`, `hooks`, `skills`, `mcp`, `git`, `modules`, `gates`.
- Provide a `status` field (`ok`, `mismatch`, `unregistered`, or `unavailable`) in every doctor section report in JSON.
- Match CLI reference §7 text summary block format `[✓]`, `[!]`, `[x]` for all 8 checks (`Git`, `Cache`, `Modules`, `Gates`, `Hooks`, `Skills`, `MCP`, `Leases`), preserving detailed section blocks for backward compatibility.
- Ensure `--fix` requires an interactive confirmation or `--yes` before rebuilding a corrupt cache. In non-interactive mode without `--yes`, refuse cache rebuild with exit code 3 (`needs_confirmation`).
- Update `crates/qdev-core/schemas/payload-doctor.json` and validate all doctor payloads against it.

**Never:**
- Never execute gate commands during `doctor` — gates doctor section only checks for configuration, executable existence via `$PATH`/relative paths, and local skip settings.
- Never rebuild an uncorrupt cache during `doctor --fix`.
- Never prompt in non-interactive mode.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
| --- | --- | --- | --- |
| Healthy workspace | `qdev doctor` | Prints `[✓]` for all 8 checks and section details; exit 0 | N/A |
| Healthy JSON | `qdev doctor --json` | Valid envelope with 9 sections, each having `status: "ok"` | N/A |
| Dirty Git state | Uncommitted files in workspace | Git section reports `status: "mismatch"`, `clean: false`, text displays `[!] Git: ...` | Exit 0 |
| Non-Git directory | `doctor` outside git repository | Git and hooks sections report `status: "unavailable"`, `unavailable_reason: "not_a_git_repository"`, text shows `[x] Git: not a git repository` | Exit 0 |
| Unmatched module glob | Module in `qdev.toml` matches 0 files | Modules section reports `status: "mismatch"`, text displays `[!] Modules: ...` | Exit 0 |
| Missing gate executable | Gate configured with non-existent binary | Gates section reports `status: "mismatch"`, text displays `[!] Gates: ...` | Exit 0 |
| Gate skipped locally | Gate configured with `skip = true` | Gates section reports `skipped_locally_count > 0`, text displays `N skipped locally` | Exit 0 |
| Outdated skills + `--fix` | Skill version != binary version | `--fix` regenerates outdated skills with current binary version | Exit 0 |
| Corrupt cache + `--fix --yes` | Corrupt or mismatched SQLite cache | Rebuilds SQLite cache from specs Markdown files | Exit 0 |
| Corrupt cache + `--fix` non-interactive no `--yes` | Non-interactive shell, corrupt cache | Fails closed with exit code 3 `needs_confirmation` specifying `--yes` | Exit 3 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/doctor.rs` -- Implement `GitDoctorSection`, `ModulesDoctorSection`, `GatesDoctorSection`, add `status` field to `CacheDoctorSection`, wire all into `default_doctor_sections`.
- `crates/qdev-core/schemas/payload-doctor.json` -- Add schema definitions for `git`, `modules`, `gates` sections and add `status` to `cache` section.
- `crates/qdev-cli/src/cli.rs` -- Add `#[arg(short = 'y', long = "yes")] pub yes: bool` to `DoctorArgs`.
- `crates/qdev-cli/src/main.rs` -- Update `render_doctor_text` to format CLI reference §7 summary lines, update `handle_doctor` to implement skills regeneration and corrupt cache rebuild with confirmation/`--yes`.
- `crates/qdev-core/tests/doctor_tests.rs` -- Unit tests for new doctor sections and updated section order.
- `crates/qdev-cli/tests/doctor_cli_tests.rs` -- Integration tests for CLI output, `--json` validation against schema, and `--fix` behavior.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/doctor.rs` -- Implement `GitDoctorSection`, `ModulesDoctorSection`, and `GatesDoctorSection`, add `status` to `CacheDoctorSection`, and register them in `default_doctor_sections` -- Core diagnostics additions.
- [x] `crates/qdev-core/schemas/payload-doctor.json` -- Update doctor payload schema to declare `git`, `modules`, and `gates` section shapes and `cache.status` -- Payload contract update.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `-y`/`--yes` to `DoctorArgs` -- CLI argument parsing.
- [x] `crates/qdev-cli/src/main.rs` -- Update `render_doctor_text` to include CLI reference §7 summary lines, and extend `handle_doctor` `--fix` for skills and cache recovery -- CLI handler and renderer.
- [x] `crates/qdev-core/tests/doctor_tests.rs` -- Add unit tests for `git`, `modules`, and `gates` doctor sections and updated default order -- Core unit test coverage.
- [x] `crates/qdev-cli/tests/doctor_cli_tests.rs` -- Update CLI integration tests for 8-check summary text, `--json` schema validation, and `--fix` with `--yes` -- CLI integration test coverage.

**Acceptance Criteria:**
- Given a healthy initialized workspace, when `qdev doctor` runs, then text output begins with the 8 status lines matching CLI reference §7 and exits 0.
- Given `qdev doctor --json`, when it runs, then every section provides a `status` field and passes validation against `qdev schema payload doctor`.
- Given outdated skills or missing hook shims, when `qdev doctor --fix` runs, then hook shims are rewritten and outdated skills are regenerated.
- Given a corrupt cache, when `qdev doctor --fix --yes` runs, then the cache is rebuilt from markdown files and subsequent doctor check passes.
- Given a corrupt cache, when `qdev doctor --fix` runs in non-interactive mode without `--yes`, then it exits 3 with code `needs_confirmation`.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Finding | Verdict | Evidence | Disposition |
|---|---|---|---|
| Skip boot ensure for bare `qdev doctor` on outdated cache | `false` | `test_doctor_reports_ok_after_a_stale_cache_is_healed_at_boot` explicitly tests and requires that bare `doctor` heals an older cache during boot ensure. | reject |
| Declining rebuild causes crash in doctor | `false` | `DoctorSection::run` is designed to handle corrupt cache gracefully and report `mismatch` without panicking. | reject |
| Locally skipped gates counted as missing executables | `high` | Gates with `skip = true` in local config should not trigger missing executable errors in `GatesDoctorSection`. | patch |
| Hooks installation error silently swallowed | `medium` | If `install_hooks(&root)` fails during `doctor --fix`, the error should be emitted and returned. | patch |
| Skill regeneration during `--fix` ignores custom models | `medium` | `handle_doctor` called `install_skills` with `None` models instead of `Some(&annotated_config.config.models)`. | patch |
| Git summary reports feature branch tracking origin/develop or dirty tree on clean diverged | `medium` | Git summary formatting in `render_doctor_text` misidentified branch tracking and diverged status description. | patch |
| Git porcelain status execution failure defaults to clean tree | `medium` | If `run_git` status command fails, `GitDoctorSection` should report `unavailable` with reason instead of `clean = true`. | patch |
| Cache summary line displays `[✓]` when `val_findings > 0` | `medium` | Cache summary block in `render_doctor_text` should display `[!]` when workspace has validation findings. | patch |
| Non-existent cache treated as corrupt requiring confirmation | `medium` | Missing cache file is a fresh state, not a corrupt database requiring destructive confirmation. | patch |
| Summary block glyphs unasserted and clean git test missing | `medium` | Pre-verified gap: `doctor_cli_tests.rs` should assert status glyph prefixes `[✓]`, `[!]`, `[x]` and test a clean git repo. | patch |
| Cache summary fallback assumes schema mismatch on missing tables | `low` | Distinguish missing tables and unreadable database from version mismatch in text rendering. | patch |
| Hooks summary hardcoded to 3 shims | `low` | Render actual missing/outdated hook shim names when reporting mismatch. | patch |
| MCP summary defaults to claude in else branch | `low` | Correctly handle unrecognized or empty targets in MCP text formatting. | patch |
| ModulesDoctorSection reimplements file collection loop | `low` | Reuse `validate::find_unmatched_module_globs` directly in `ModulesDoctorSection`. | patch |
| Git refs_missing should report mismatch | `low` | Flag missing integration refs as `mismatch` in `GitDoctorSection`. | patch |


## Verification

**Commands:**
- `cargo test --test doctor_tests` -- expected: all core doctor tests pass.
- `cargo test --test doctor_cli_tests` -- expected: all doctor CLI tests pass.
- `cargo test` -- expected: entire test suite passes without regression.
