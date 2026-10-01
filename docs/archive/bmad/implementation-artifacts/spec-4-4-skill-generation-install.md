---
title: 'Story 4.4: Skill Generation & Install'
type: 'feature'
created: '2026-09-30'
status: 'done'
route: 'dispatch'
baseline_commit: 684c37dfdd7e28208b426b4d5920bd5990a3d3fe
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** AI editor integrations and skills frequently drift from CLI commands and flags when authored by hand. Additionally, developers lack an automated way to verify that installed assistant skills match the active binary version.

**Approach:** Implement `qdev install skills --claude|--cursor|--agents` to generate and install editor and agent skill bundles from a centralized command catalog aligned with `clap`. Stamp each generated skill file with the binary version, ensure all generated skills invoke `qdev ... --json` and `qdev context` rather than parsing spec files directly, and add a `skills` diagnostic section to `qdev doctor` that reports any installed skill whose version stamp differs from the binary.

## Boundaries & Constraints

**Always:**
- Generate skills from a centralized `CommandCatalog` in `qdev-core` that mirrors the CLI commands, options, and descriptions driving `clap`.
- Support `--claude`, `--cursor`, and `--agents` flags on `qdev install skills`; require at least one target flag to be specified (exit code 2 usage error if none provided).
- Install Claude skills to `.claude/skills/<skill>/SKILL.md` for the five core skills (`qdev`, `qdev-plan`, `qdev-create-story`, `qdev-develop`, `qdev-review`).
- Install Cursor rule to `.cursor/rules/qdev.mdc`.
- Install Agent skills to `.agents/skills/<skill>/SKILL.md` for the five core skills (`qdev`, `qdev-plan`, `qdev-create-story`, `qdev-develop`, `qdev-review`).
- Stamp each generated file with the binary version (`env!("CARGO_PKG_VERSION")`) in its frontmatter (`version: "<version>"` and `qdev_version: "<version>"`).
- Generate skills that instruct the agent to always use `qdev ... --json` and `qdev context`, and never read, parse, or write raw entity Markdown files directly.
- Add `SkillsDoctorSection` to `default_doctor_sections` in `qdev-core`, reporting `status: "ok"` when all installed skills match the binary version (or when none are installed) and `status: "mismatch"` when any installed skill's version stamp differs from the binary.
- Register `payload-skill-install.json` in `qdev schema payload` under `skill_install` / `skills_install`.

**Never:**
- Never assemble or inspect context from raw files directly in generated skill templates.
- Never overwrite non-qdev skills or rules residing in `.claude/skills/`, `.cursor/rules/`, or `.agents/skills/`.
- Never fail or return non-zero exit code from `qdev doctor` due to mismatched skill version stamps (`doctor` is a diagnostic report, not a gate).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Install Claude skills | `qdev install skills --claude` | Writes 5 `SKILL.md` files into `.claude/skills/` with binary version stamp | Exit 0; emits report payload |
| Install Cursor rule | `qdev install skills --cursor` | Writes `.cursor/rules/qdev.mdc` with binary version stamp | Exit 0; emits report payload |
| Install Agent skills | `qdev install skills --agents` | Writes 5 `SKILL.md` files into `.agents/skills/` with binary version stamp | Exit 0; emits report payload |
| Install multiple targets | `qdev install skills --claude --cursor --agents` | Writes all 11 skill/rule files across all three targets | Exit 0; emits combined report |
| Missing target flag | `qdev install skills` (no target flag) | Refuses execution with usage error | Exit 2 usage error: "specify at least one of --claude, --cursor, or --agents" |
| Doctor clean / up to date | `qdev doctor` with up-to-date skills | `skills` section reports `status: "ok"`, `up_to_date: true`, `outdated_count: 0` | Exit 0 |
| Doctor detects outdated skill | `qdev doctor` with a skill stamped `0.0.9` | `skills` section reports `status: "mismatch"`, `up_to_date: false`, lists path in `outdated_skills` | Exit 0 (diagnostic report) |
| Doctor no skills installed | `qdev doctor` in workspace with no skills directories | `skills` section reports `status: "ok"`, `installed_count: 0`, `up_to_date: true` | Exit 0 |
| JSON output mode | `qdev install skills --claude --json` | Emits JSON envelope conforming to `payload-skill-install.json` | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/skills.rs` -- Implement `CommandCatalog` defining CLI commands, skill generation logic (`generate_claude_skills`, `generate_cursor_rule`, `generate_agent_skills`), `install_skills`, and `inspect_skills`.
- `crates/qdev-core/src/doctor.rs` -- Implement `SkillsDoctorSection` calling `inspect_skills`, reporting `status`, `binary_version`, `installed_count`, `outdated_count`, `up_to_date`, and `outdated_skills`; register in `default_doctor_sections` after `hooks`.
- `crates/qdev-core/src/lib.rs` -- Export `skills` module types (`CommandCatalog`, `install_skills`, `inspect_skills`, `SkillsInstallReport`, `SkillsStatus`, `SkillsDoctorSection`).
- `crates/qdev-core/src/schema.rs` -- Add `PayloadKind::SkillInstall` mapping to `payload-skill-install.json` with aliases `"skill_install"`, `"skills_install"`.
- `crates/qdev-core/schemas/payload-skill-install.json` -- JSON Schema for `qdev install skills --json` payload.
- `crates/qdev-core/schemas/payload-doctor.json` -- Update doctor payload schema to validate the `skills` section properties.
- `crates/qdev-cli/src/cli.rs` -- Add `Skills(InstallSkillsArgs)` subcommand to `InstallCommands` with `--claude`, `--cursor`, `--agents` flags.
- `crates/qdev-cli/src/handlers/install.rs` -- Handle `InstallCommands::Skills`, validate flags, invoke `install_skills`, and format text/JSON output.
- `crates/qdev-cli/src/main.rs` -- Ensure `render_doctor_text` formats the `skills` section cleanly.
- `crates/qdev-core/tests/doctor_tests.rs` -- Add unit tests for `SkillsDoctorSection` inspecting clean, outdated, and uninstalled states, and update `test_default_doctor_sections_order`.
- `crates/qdev-core/tests/skills_tests.rs` -- Unit tests for `CommandCatalog`, skill template generation, version stamping, and installation logic.
- `crates/qdev-cli/tests/install_skills_cli_tests.rs` -- Integration tests for `qdev install skills`, `--json` envelope, usage error on missing flag, and `qdev doctor` detection of outdated skills.
- `crates/qdev-cli/tests/doctor_cli_tests.rs` -- Update doctor section order test to assert `skills` follows `hooks`.
- `docs/cli-reference.md` -- Document `qdev install skills` options, generated file layout, and doctor skills diagnostics.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/skills.rs` -- Create `skills` module with `CommandCatalog`, generators for Claude, Cursor, and Agents, `install_skills`, and `inspect_skills` -- Centralize command definitions and skill generation logic
- [x] `crates/qdev-core/src/doctor.rs` -- Add `SkillsDoctorSection` to doctor registry -- Report installed and outdated skills in `qdev doctor`
- [x] `crates/qdev-core/src/schema.rs` & `schemas/` -- Add `payload-skill-install.json` and update `payload-doctor.json` -- Maintain schema contracts for CLI payloads
- [x] `crates/qdev-core/src/lib.rs` -- Re-export skill types and functions -- Provide public API for CLI and tests
- [x] `crates/qdev-cli/src/cli.rs` & `handlers/install.rs` -- Implement `InstallCommands::Skills` with `--claude`, `--cursor`, `--agents` flags and dispatch -- Expose CLI interface for skill installation
- [x] `crates/qdev-cli/src/main.rs` -- Update doctor text rendering for `skills` section -- Clean CLI output formatting
- [x] `crates/qdev-core/tests/` & `crates/qdev-cli/tests/` -- Implement comprehensive unit and integration tests -- Verify skill generation, installation, version stamping, and doctor detection
- [x] `docs/cli-reference.md` -- Update documentation for `qdev install skills` and doctor output -- Keep user documentation accurate

**Acceptance Criteria:**
- Given `qdev install skills --claude|--cursor|--agents`, when it runs, then files are written to `.claude/skills/<skill>/SKILL.md`, `.cursor/rules/qdev.mdc`, or `.agents/skills/<skill>/SKILL.md`, generated from the centralized command catalog driving `clap`, and stamped with the binary version.
- Given installed skills whose version stamp differs from the binary, when `qdev doctor` runs, then the `skills` section reports `status: "mismatch"` and lists the outdated skills.
- Given generated skills, when inspected, then all commands invoke `qdev … --json` and `qdev context`, and never assemble context from files directly.
- Given `qdev install skills` with no target flags, when it runs, then it exits 2 with a usage error requiring at least one target flag.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| # | Reviewer Layer | Location | Claim / Finding | Verdict | Route | Resolution |
|---|---|---|---|---|---|---|
| 1 | Verification Gap Reviewer | `crates/qdev-cli/src/main.rs:2979` | Pre-verified gap: `qdev doctor` text mode output formatting for array fields and `[skills]` section is unverified | `medium` | `patch` | Add CLI tests running `qdev doctor` in text mode asserting `[skills]`, `outdated_skills = none`, and `outdated_skills = <path>`. |
| 2 | Edge Case Hunter | `crates/qdev-core/src/skills.rs:762` | `generate_skill_content` panics if called with unknown skill name | `low` | `patch` | Return a fallback/empty string rather than panicking on unexpected input. |
| 3 | Edge Case Hunter | `crates/qdev-core/src/skills.rs:871` | `write_managed_file` overwrites unreadable non-qdev files if `fs::read_to_string` fails | `medium` | `patch` | Return `skill_conflict` error if an existing file cannot be read and verified as qdev-managed. |
| 4 | Edge Case Hunter | `crates/qdev-core/src/skills.rs:960` | `inspect_skills` counts non-qdev files at managed paths as outdated qdev skills | `medium` | `patch` | Skip foreign files lacking a qdev version stamp instead of counting them as outdated qdev skills. |
| 5 | Edge Case Hunter / Blind Hunter | `crates/qdev-core/src/skills.rs:160` | `CommandCatalog` and generated skills use invalid flags `--to` and `--relation` rejected by clap | `high` | `patch` | Align with clap positional arguments: `qdev transition story <id> <status> --json` and `qdev relate <source> <relation> <target> --json`. |
| 6 | Blind Hunter / Edge Case Hunter | `crates/qdev-core/src/skills.rs:645` | Skills instruct running `qdev context <id> --phase plan` which is rejected as an invalid phase | `high` | `patch` | Change to `qdev context <story-id> --phase specify --json` which matches valid `ContextPhase`. |
| 7 | Blind Hunter / Edge Case Hunter | `crates/qdev-core/src/skills.rs:653` | `qdev-create-story` invokes `qdev constraint add` with nonexistent `--story` and `--text` flags | `high` | `patch` | Change to `qdev constraint add <target> --kind <no_go|rabbit_hole|appetite> "<text>" --json`. |
| 8 | Blind Hunter | `crates/qdev-core/src/skills.rs:670` | `qdev-develop` omits mandatory instruction to quote cited IDs when reporting refusals | `medium` | `patch` | Add explicit refusal instruction to quote cited IDs (`constraint_id`, `gate_id`, `policy`, `blocking_ids`, `holder`). |
| 9 | Blind Hunter | `crates/qdev-core/src/skills.rs:720` | `render_command_catalog_markdown` omits command options and subcommands from markdown table | `low` | `patch` | Format command options and subcommands in the generated catalog markdown. |
| 10 | Blind Hunter | `crates/qdev-core/src/skills.rs:873` | `write_managed_file` does not recognize `version` alongside `qdev_version` | `low` | `patch` | Check for both `qdev_version` and `version` frontmatter keys. |
| 11 | Blind Hunter | `crates/qdev-core/src/skills.rs:899` | `write_managed_file` uses `fs::write` directly instead of atomic file writing | `medium` | `patch` | Write via temporary file and atomic rename. |
| 12 | Blind Hunter | `crates/qdev-cli/tests/install_skills_cli_tests.rs:25` | `test_command_catalog_aligned_with_clap_cli` does not check bidirectionally | `medium` | `patch` | Assert that all commands in `CommandCatalog` exist in `clap`, and all clap subcommands exist in `CommandCatalog`. |
| 13 | Blind Hunter | `docs/bmad/implementation-artifacts/sprint-status.yaml` | Story 4.4 key is `in-progress` instead of `review` | `false` | `reject` | Workflow rule: `sprint-status.yaml` advances to `review` at step-05, not step-04. |

## Design Notes

- **Centralized Command Catalog:**
  `CommandCatalog::all()` defines the list of core commands, subcommands, arguments, and summaries. A test in `qdev-cli` verifies that every command in `CommandCatalog` exists in clap's `Cli::command()`, ensuring the catalog and CLI parser never drift.
- **Skill Structure & Content:**
  Each generated skill specifies:
  1. Frontmatter with `name`, `description`, `version`, and `qdev_version`.
  2. Clear instructions that all workspace state reads and mutations must be executed through `qdev <subcommand> ... --json`.
  3. Context instructions explicitly directing the agent to run `qdev context <id> --phase <phase> --json` rather than reading markdown files.
  4. Workflows for:
     - `/qdev`: pulse inspection and next step recommendation.
     - `/qdev-plan`: planning entities (`epic`, `requirement`, `adr`, `hazard`).
     - `/qdev-create-story`: context specification, story creation, constraints, transition to ready.
     - `/qdev-develop`: preflight, lease claim, develop context, scratchpad updates, gate verification, transition to review.
     - `/qdev-review`: review context, diff audit against AC and constraints, transition to done or in-progress.

## Verification

**Commands:**
- `cargo test --package qdev-core --test suite skills_tests` -- expected: all unit tests pass
- `cargo test --package qdev-core --test suite doctor_tests` -- expected: all doctor tests pass
- `cargo test --package qdev-cli --test suite install_skills_cli_tests` -- expected: CLI integration tests pass
- `cargo test --package qdev-cli --test suite doctor_cli_tests` -- expected: doctor integration tests pass
- `cargo run --bin qdev -- install skills --claude --cursor --agents --json` -- expected: exit 0 and valid JSON envelope

