---
title: 'Story 4.8: Structured Multi-Perspective Synthesis Template'
type: 'feature'
created: '2026-10-01'
status: 'done'
baseline_commit: 'cd5500f16d85b2527fdc6976774edf9f02d863b0'
route: 'dispatch'
review_loop_iteration: 0
context:
  - '{project-root}/docs/architecture.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Planning proposals and story generation lack uniform multi-perspective structure across agent skills: `/qdev-create-story` does not include synthesis guidance, `/qdev-plan` does not enforce rejection of outputs missing required perspectives, and synthesis headings/templates cannot be customized per workspace (e.g. for clinical value or safety-critical domains).

**Approach:** Introduce a typed `[synthesis]` configuration section in `qdev.toml` with shipped defaults matching `docs/architecture.md` §4 (Product & domain value, Architectural constraints, Safety & risk profile, Implementation directives); embed the synthesis template and strict rejection directives into both `/qdev-plan` and `/qdev-create-story` skills (and unified Cursor rule); and support workspace overrides for custom headings and templates.

## Boundaries & Constraints

**Always:**
- Keep the shipped default synthesis template aligned with `docs/architecture.md` §4: `Product & domain value`, `Architectural constraints`, `Safety & risk profile`, and `Implementation directives`.
- Enforce strict rejection instructions in `/qdev-plan` and `/qdev-create-story`: prompts must explicitly instruct the assistant to reject any model response or proposal missing any required heading and re-prompt.
- Support `[synthesis]` in `qdev.toml` allowing custom `headings = [...]` (array of strings) and/or `template = "..."` (string override).
- When `headings` is customized (e.g., adding `"Clinical value"`), dynamic template rendering must generate sections for all configured headings while preserving the rejection rule.
- Provide a programmatic helper `SynthesisConfig::validate_output` that checks if an output string contains all required headings (case-insensitive).
- Validate the `[synthesis]` section strictly in `qdev.toml` using `validate_config_table`, exiting 2 on unknown keys or invalid types.
- Ensure `qdev install skills` and `qdev doctor --fix` pick up the workspace `qdev.toml` `[synthesis]` configuration.

**Never:**
- Never accept planning or story proposals that omit any required heading.
- Never hardcode vendor model names in synthesis templates or skills.
- Never break backward compatibility for existing `generate_skill_content` and `install_skills_with_models` callers.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
| --- | --- | --- | --- |
| Default synthesis template | Default `SynthesisConfig` | Contains the 4 §4 headings and rejection directive | N/A |
| Custom headings in `qdev.toml` | `[synthesis]` with `headings = ["...", "Clinical value"]` | Skill templates generated with all configured headings including Clinical value | N/A |
| Custom template in `qdev.toml` | `[synthesis]` with `template = "..."` | Custom template rendered verbatim with resolved headings | N/A |
| Output validation complete | Text containing all 4 headings | `SynthesisConfig::validate_output` returns `Ok(())` | N/A |
| Output validation missing heading | Text missing `Safety & risk profile` | `SynthesisConfig::validate_output` returns `Err(vec!["Safety & risk profile"])` | Non-fatal programmatic error |
| Unknown key in `[synthesis]` | `[synthesis] foo = "bar"` in `qdev.toml` | Schema validation error exiting 2 naming key and file | Exit 2 |
| Invalid type in `[synthesis]` | `[synthesis] headings = 123` | Schema validation error exiting 2 naming type mismatch | Exit 2 |
| Install skills with custom synthesis | Run `qdev install skills --claude` in workspace with custom headings | Installed `/qdev-plan` and `/qdev-create-story` skills contain custom headings and rejection rule | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/config/types.rs` -- Define `SynthesisConfig`, `default_synthesis_headings`, implement `Default`, `resolved_headings`, `render_template`, and `validate_output`. Add `pub synthesis: SynthesisConfig` to `Config`.
- `crates/qdev-core/src/config/mod.rs` -- Add `"synthesis"` to `ALLOWED_TOP_LEVEL_SECTIONS`, implement `validate_synthesis_section`, update `validate_config_table`, and wire parsing in `merge_and_validate_configs`.
- `crates/qdev-core/src/config/source.rs` -- Render `[synthesis]` in configuration formatting.
- `crates/qdev-core/src/skills.rs` -- Update `STRUCTURED_SYNTHESIS_TEMPLATE` to include the explicit rejection rule; embed synthesis template and rejection instructions into `qdev-create-story`; update `qdev-plan` instructions; update cursor rule; provide `generate_skill_content_configured`, `generate_claude_skills_configured`, `generate_agent_skills_configured`, and `install_skills_configured`. Wire `install_skills_with_models` to load workspace `synthesis` config if not explicitly supplied.
- `crates/qdev-core/src/lib.rs` -- Export `SynthesisConfig` and `default_synthesis_headings`.
- `crates/qdev-cli/src/handlers/install.rs` -- Pass `annotated_config.config.synthesis` into skills installation.
- `crates/qdev-cli/src/main.rs` -- Pass `annotated_config.config.synthesis` into `install_skills_configured` during `doctor --fix`.
- `crates/qdev-core/tests/skills_tests.rs` -- Unit tests for synthesis template, headings, rejection rules, custom workspace headings, and output validation.
- `crates/qdev-core/tests/config_tests.rs` -- Unit tests for `[synthesis]` configuration parsing, defaults, custom headings, custom templates, and schema error handling.
- `crates/qdev-cli/tests/install_skills_cli_tests.rs` -- CLI integration tests verifying installed `/qdev-plan` and `/qdev-create-story` skills include the synthesis template, rejection rule, and respond to workspace `qdev.toml` `[synthesis]` overrides.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/config/types.rs` -- Add `SynthesisConfig` struct with `template` and `headings`, implement `Default`, `resolved_headings`, `render_template`, and `validate_output`, and add `synthesis` field to `Config` -- Core configuration model.
- [x] `crates/qdev-core/src/config/mod.rs` & `source.rs` -- Register `"synthesis"` in `ALLOWED_TOP_LEVEL_SECTIONS`, implement strict validation in `validate_synthesis_section`, wire into `merge_and_validate_configs`, and format in `source.rs` -- Config loading and validation.
- [x] `crates/qdev-core/src/skills.rs` -- Add rejection directive to synthesis template; update `qdev-plan` and `qdev-create-story` content generation to embed template, require all configured headings, and instruct rejecting output missing any heading; update Cursor rule; wire workspace synthesis configuration into skill generation and installation -- Core skills synthesis integration.
- [x] `crates/qdev-core/src/lib.rs` -- Export `SynthesisConfig` and `default_synthesis_headings` from `qdev-core` root -- Core exports.
- [x] `crates/qdev-cli/src/handlers/install.rs` & `main.rs` -- Pass workspace `synthesis` configuration to skills installer in `install` command and `doctor --fix` -- CLI integration.
- [x] `crates/qdev-core/tests/config_tests.rs` -- Add unit tests for `[synthesis]` parsing, defaults, custom headings, custom templates, and unknown key/type validation -- Core config test coverage.
- [x] `crates/qdev-core/tests/skills_tests.rs` -- Add unit tests for `/qdev-plan` and `/qdev-create-story` synthesis templates, rejection directives, custom headings, and `validate_output` -- Core skills test coverage.
- [x] `crates/qdev-cli/tests/install_skills_cli_tests.rs` -- Add integration tests asserting installed skills contain synthesis template and rejection directives, and that workspace `[synthesis]` headings are reflected -- CLI integration test coverage.

**Acceptance Criteria:**
- Given the `/qdev-plan` and `/qdev-create-story` skills, when they prompt a model, then the prompt requires the four headings in `docs/architecture.md` §4 (`Product & domain value`, `Architectural constraints`, `Safety & risk profile`, `Implementation directives`) and rejects output missing any heading.
- Given `qdev.toml` with `[synthesis]` containing custom headings (such as `Clinical value`) or a custom template, when skills are generated or installed, then `/qdev-plan` and `/qdev-create-story` reflect the configured headings and template.
- Given an output missing any required heading, `SynthesisConfig::validate_output` detects the missing headings and returns an error.
- Given a `qdev.toml` with invalid keys or types under `[synthesis]`, `qdev` fails validation with exit code 2 naming the invalid key/type and file.

## Implementation Notes

- Added `SynthesisConfig` struct with `template` and `headings`, implementing `Default`, `resolved_headings`, `render_template` (with dynamic perspective interpolation for default/custom headings including Clinical value and verbatim template support with `{headings}` interpolation), and `validate_output` (case-insensitive heading detection returning `Err(Vec<String>)` of missing headings).
- Added `default_synthesis_headings()` returning the four architecture §4 headings: "Product & domain value", "Architectural constraints", "Safety & risk profile", and "Implementation directives".
- Added `synthesis` field to `Config`, registered `"synthesis"` in `ALLOWED_TOP_LEVEL_SECTIONS`, implemented strict schema validation in `validate_synthesis_section`, wired into `merge_configs`, and rendered in `AnnotatedConfig::to_text_report`.
- Updated `STRUCTURED_SYNTHESIS_TEMPLATE` to include the explicit rejection directive.
- Embedded synthesis template and explicit rejection directive in `/qdev-create-story`, updated `/qdev-plan` and Cursor rule to require configured headings and reject incomplete outputs.
- Provided `generate_skill_content_configured`, `generate_claude_skills_configured`, `generate_agent_skills_configured`, `generate_cursor_rule_configured`, `generate_cursor_rule_content_configured`, and `install_skills_configured`, wiring `install_skills_with_models` to auto-load workspace `synthesis` config if not explicitly supplied.
- Updated `qdev install skills` CLI handler and `qdev doctor --fix` in `main.rs` to pass workspace `synthesis` configuration to `install_skills_configured`.
- Added unit and integration tests across `config_tests.rs`, `skills_tests.rs`, and `install_skills_cli_tests.rs`. All 796 tests passing cleanly.

## Spec Change Log

## Review Triage Log

| Finding | Verdict | Evidence | Disposition |
|---|---|---|---|
| Auto-loading of workspace synthesis in install_skills unverified | `medium` | Pre-verified gap: programmatic callers of `install_skills` did not have test coverage for auto-loading `[synthesis]` from `qdev.toml`. | patch |
| Regeneration of outdated skills via doctor --fix with synthesis unverified | `medium` | Pre-verified gap: `doctor_cli_tests.rs` did not verify that `doctor --fix` regenerates outdated skills with custom workspace synthesis headings. | patch |
| Rejection directives in cursor rule unasserted | `low` | Pre-verified gap: `.cursor/rules/qdev.mdc` lacked assertions checking for the rejection directive in `skills_tests.rs`. | patch |
| Rendering of synthesis in to_text_report unasserted | `low` | Pre-verified gap: `test_to_text_report_comprehensive` did not verify `[synthesis]` section emission in config text report. | patch |
| Empty array or empty/whitespace string allowed in headings or template | `medium` | `validate_synthesis_section` permitted `headings = []`, `headings = [""]`, or `template = ""` without usage error. | patch |
| Custom template without {headings} enforces default headings | `medium` | `resolved_headings` defaulted to 4 architecture headings when custom template omitted `{headings}`, causing validation mismatches. | patch |
| Custom template can omit mandatory rejection directive | `medium` | Custom `template` was returned verbatim without guaranteeing that the mandatory rejection rule was appended. | patch |
| Missing blank line preceding ## Workflow headers in skills | `low` | `render_template` ended with single `\n` instead of `\n\n`, putting level 2 headers against body text without blank line. | patch |
| Status mismatch between sprint tracking ledger and spec | `false` | `sprint-status.yaml` explicitly documents that story status remains `in-progress` until step-05 sets `review` after review completes. | reject |
| Leading and trailing whitespace in headings | `low` | Unstripped whitespace prevented matching standard heading descriptions and created uneven Markdown indentation. | patch |
| Substring search in validate_output | `low` | Basic substring check is suitable for programmatic validation, improved by whitespace trimming on headings. | patch |
| Missing CLI tests for custom template installation and schema errors | `low` | `install_skills_cli_tests.rs` only tested custom headings; added tests for custom template and invalid schema. | patch |

## Design Notes

The synthesis template replaces multi-persona role-play with a single high-reasoning prompt covering four orthogonal concerns:
1. Product & domain value: Problem statement, user persona, measurable success metrics, appetite.
2. Architectural constraints: Boundaries, target modules, dependencies, cross-crate interfaces, invariants.
3. Safety & risk profile: ISO 14971 hazards, regulatory compliance (IEC 62304), negative constraints (no-gos and rabbit holes).
4. Implementation directives: Specific CLI commands to create stories (`qdev create story`) and establish relations (`qdev relate`).

When domain-specific perspectives like `Clinical value` are configured under `[synthesis].headings`, the generator dynamically synthesizes additional headings into the template with appropriate prompts while enforcing the rejection rule on all headings.

## Verification

**Commands:**
- `cargo test --test config_tests` -- expected: all configuration tests pass including new synthesis tests.
- `cargo test --test skills_tests` -- expected: all skills tests pass including synthesis template and rejection assertions.
- `cargo test --test install_skills_cli_tests` -- expected: all CLI installation tests pass including synthesis assertions.
- `cargo test` -- expected: complete test suite passes without regressions.
