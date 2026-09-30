---
title: 'Story 4.6: The Five Skills'
type: 'feature'
created: '2026-10-01'
status: 'done'
route: 'dispatch'
baseline_commit: '1f3270796e84ee30dafff957ff51a884ff907b23'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Developers and AI assistants lack rich, end-to-end slash-command skill implementations for `/qdev`, `/qdev-plan`, `/qdev-create-story`, `/qdev-develop`, and `/qdev-review`. Assistant skills need to expose the full lifecycle—rendering pulse, structured planning synthesis, story creation with constraints, preflight-guarded development with bound gates, and audit-driven review—while surfacing advisory model hints from workspace configuration.

**Approach:** Generate rich, comprehensive templates for the five core skills across Claude, Cursor, and Agent targets. Include the Structured Multi-Perspective Synthesis template in `/qdev-plan` covering the four architectural perspectives (Product & domain value, Architectural constraints, Safety & risk profile, Implementation directives). Surface `[models]` advisory hints in each skill's frontmatter (using `model_hint` and `model` keys, resolving from workspace config or defaulting to `reasoning`, `fast-coding`, and `strongest`). Update `install_skills` to read workspace model configuration when available and render skills with the resolved model hints.

## Boundaries & Constraints

**Always:**
- Keep all 5 skills (`qdev`, `qdev-plan`, `qdev-create-story`, `qdev-develop`, `qdev-review`) generated from `crates/qdev-core/src/skills.rs` stamped with binary version and advisory model tiering hints.
- In `/qdev`: instruct assistant to run `qdev --json` and render the pulse (workspace status, sprint state, gates, leases, next steps).
- In `/qdev-plan`: embed the Structured Multi-Perspective Synthesis template with the four mandatory headings: `Product & domain value`, `Architectural constraints`, `Safety & risk profile`, and `Implementation directives`; instruct creation of PRD, requirements, ADRs, and hazards via `qdev create` and relationships via `qdev relate`.
- In `/qdev-create-story`: instruct assistant to call `qdev context <epic-id> --phase specify --json`, draft acceptance criteria and negative constraints, execute `qdev create story` and `qdev constraint add`, and transition to `ready` via `qdev transition story <story-id> ready --json`.
- In `/qdev-develop`: instruct assistant to run `qdev preflight --story <story-id> --json`, `qdev claim <story-id> --json`, `qdev context <story-id> --phase develop --json`, append progress/notes to scratchpad via `qdev scratch append`, run `qdev gate run --for-transition review --story <story-id> --json`, and transition to review via `qdev transition story <story-id> review --json`. On any refusal, quote cited IDs (`constraint_id`, `gate_id`, `policy`, `blocking_ids`, `holder`).
- In `/qdev-review`: instruct assistant to fetch `qdev context <story-id> --phase review --json`, audit diff against acceptance criteria and constraints, run `qdev impact` and `qdev gate run --all`, and either transition to `done` (`qdev transition story <story-id> done --json`) or back to `in_progress` with justification (`qdev transition story <story-id> in_progress --justification "<justification>" --json`).
- Surface `[models]` hints in skill frontmatter: `specify` (`"reasoning"`) for `qdev-plan` and `qdev-create-story`, `develop` (`"fast-coding"`) for `qdev-develop`, `review` (`"strongest"`) for `qdev-review`, and `"reasoning"` / `"fast-coding"` for `qdev`.
- Allow `install_skills` and CLI `qdev install skills` to read configured `[models]` from `qdev.toml` or fallback to default tiers.

**Never:**
- Never hardcode vendor-specific model identifiers (e.g. `claude-3-5-sonnet`, `gpt-4o`) into core default templates; use tier hints (`reasoning`, `fast-coding`, `strongest`).
- Never instruct assistants to assemble context from raw markdown files in `docs/` directly; always use `qdev context ... --json`.
- Never bypass `qdev preflight`, lease claims, or transition-bound gates in skill workflows.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Generate Claude skills with defaults | `generate_claude_skills()` | Returns 5 skills with frontmatter containing version and default `model_hint` (`reasoning`, `fast-coding`, `strongest`) | Valid Markdown templates |
| Generate skills with custom `[models]` config | Config with `specify = "custom-reasoner"`, `develop = "custom-coder"` | Generated skills frontmatter surfaces custom model hints | Falls back to default tier if key omitted |
| Run `qdev install skills --claude` | Workspace with or without `qdev.toml` | Installs 5 skills in `.claude/skills/` with synthesis template and model hints | Exit 0 |
| Skill `/qdev` invocation instructions | Assistant reading `/qdev` skill | Instructed to execute `qdev --json` and render pulse | N/A |
| Skill `/qdev-plan` synthesis template | Assistant reading `/qdev-plan` skill | Instructed to fill all 4 perspective headings and create entities | N/A |
| Skill `/qdev-create-story` workflow | Assistant reading `/qdev-create-story` | Calls `qdev context <epic-id> --phase specify --json`, adds constraints, transitions to ready | N/A |
| Skill `/qdev-develop` workflow | Assistant reading `/qdev-develop` | Runs preflight, claim, context develop, scratch append, gate run review, transition review | N/A |
| Skill `/qdev-review` workflow | Assistant reading `/qdev-review` | Context review, audit diff against AC/constraints, gate run all, transition done or in_progress | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/skills.rs` -- Update skill content generation (`generate_skill_content_with_models`, `generate_claude_skills_with_models`, `generate_agent_skills_with_models`, `generate_cursor_rule_with_models`), Structured Multi-Perspective Synthesis template, workflow steps, and frontmatter model tier hints.
- `crates/qdev-cli/src/handlers/install.rs` -- Pass loaded `AnnotatedConfig`'s `models` section into `install_skills`.
- `crates/qdev-core/tests/skills_tests.rs` -- Unit tests verifying 5 skills contents, Structured Multi-Perspective Synthesis template headings, model tier frontmatter, and custom models config propagation.
- `crates/qdev-cli/tests/install_skills_cli_tests.rs` -- Integration tests verifying installed skills have synthesis template and model hints.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/skills.rs` -- Implement rich five skills templates, Structured Multi-Perspective Synthesis template, and `[models]` frontmatter generation -- Core skill content definition
- [x] `crates/qdev-cli/src/handlers/install.rs` -- Pass configuration models to `install_skills` -- CLI install wiring
- [x] `crates/qdev-core/tests/skills_tests.rs` -- Add unit tests for the five skills workflows, synthesis template headings, and model frontmatter hints -- Core verification
- [x] `crates/qdev-cli/tests/install_skills_cli_tests.rs` -- Add integration tests checking installed skill contents and frontmatter hints -- CLI verification

**Acceptance Criteria:**
- Given installed skills, when `/qdev` is run, then it runs `qdev --json` and renders the pulse.
- Given installed skills, when `/qdev-plan` is run, then it uses the Structured Multi-Perspective Synthesis template (with headings: Product & domain value, Architectural constraints, Safety & risk profile, Implementation directives) to draft PRD, requirements, ADRs, and hazards via `qdev create`.
- Given installed skills, when `/qdev-create-story E12` is run, then it calls `qdev context E12 --phase specify`, drafts a story with acceptance criteria and constraints, creates it via `qdev create story` and `qdev constraint add`, and transitions it to `ready`.
- Given installed skills, when `/qdev-develop E12S4` is run, then it runs `qdev preflight`, `qdev claim`, `qdev context --phase develop`, works within scope appending to the scratchpad, runs `qdev gate run --for-transition review`, and transitions to `review`.
- Given installed skills, when `/qdev-review E12S4` is run, then it uses `qdev context --phase review`, audits the diff against acceptance criteria and constraints, and either transitions to `done` or back to `in-progress` with a justification.
- Given any installed skill, when its frontmatter is inspected, then `[models]` advisory hints (`model_hint` and `model`) are surfaced.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| # | Reviewer Layer | Location | Claim / Finding | Verdict | Route | Resolution |
|---|---|---|---|---|---|---|
| 1 | Edge Case Hunter / Blind Hunter | `crates/qdev-core/src/skills.rs:971` | `qdev impact --story <id>` fails because `story` is a positional arg in clap | `medium` | `patch` | Update `qdev impact` invocation to use positional `<story-id>`. |
| 2 | Verification Gap Reviewer | `crates/qdev-core/src/skills.rs:1021` | Pre-verified gap: Cursor rule generation and installation with custom models configuration is unverified | `low` | `patch` | Add test assertion for custom models Cursor rule and include `--cursor` in CLI custom models test. |
| 3 | Verification Gap Reviewer | `crates/qdev-core/src/skills.rs:1072` | Pre-verified gap: Agent skills generation and installation with custom models configuration is unverified | `low` | `patch` | Include `--agents` in CLI custom models integration test. |
| 4 | Verification Gap Reviewer | `crates/qdev-core/src/skills.rs:739` | Pre-verified gap: Non-plan Agent skills lack content and workflow verification on installation | `low` | `patch` | Verify workflow and frontmatter contents across all 5 installed agent skills. |
| 5 | Blind Hunter / Verification Gap | `crates/qdev-core/src/skills.rs:826` | `qdev create prd|requirement|adr|hazard` subcommands do not exist in clap | `medium` | `patch` | Clarify planning workflow to create stories via `qdev create story` and link via `qdev relate`. |
| 6 | Blind Hunter | `crates/qdev-core/src/skills.rs:1197` | `install_skills` changed signature breaking 2-argument callers | `low` | `patch` | Retain 2-argument `install_skills` and export `install_skills_with_models`. |
| 7 | Blind Hunter | `crates/qdev-core/src/skills.rs:556` | Missing `--story` option definition in `CommandCatalog` for `gate run` | `low` | `patch` | Add `--story` option definition to `CommandCatalog`. |
| 8 | Blind Hunter | `crates/qdev-core/src/skills.rs:697` | Inline ATX headers in numbered list items in synthesis template | `low` | `patch` | Format synthesis template headings as clean Markdown headings. |
| 9 | Blind Hunter | `crates/qdev-core/src/skills.rs:717` | Unescaped string interpolation for frontmatter model hints | `low` | `patch` | Add `escape_yaml()` for frontmatter model hints. |
| 10 | Blind Hunter | `sprint-status.yaml` | `sprint-status.yaml` is in-progress while spec is in-review | `false` | `reject` | BMAD specification explicitly rules that sprint-status transitions to review at step-05. |
| 11 | Blind Hunter | `crates/qdev-core/src/skills.rs:846` | `qdev context <epic-id>` requires story ID | `low` | `patch` | Add note indicating `qdev get epic <epic-id> --json` alongside `qdev context`. |

## Design Notes

- **Model Tiering in Frontmatter:**
  Each skill's YAML frontmatter contains:
  ```yaml
  model_hint: "{tier}"
  model: "{tier}"
  ```
  For `/qdev`, the tier hint is `reasoning` (or `models.specify.unwrap_or("reasoning")`).
  For `/qdev-plan`, the tier hint is `models.specify.unwrap_or("reasoning")`.
  For `/qdev-create-story`, the tier hint is `models.specify.unwrap_or("reasoning")`.
  For `/qdev-develop`, the tier hint is `models.develop.unwrap_or("fast-coding")`.
  For `/qdev-review`, the tier hint is `models.review.unwrap_or("strongest")`.

- **Structured Multi-Perspective Synthesis Template:**
  Embedded in `/qdev-plan` and the command catalog / cursor rule:
  ```markdown
  ### Structured Multi-Perspective Synthesis Template
  Every planning proposal must provide answers under each of the four headings:
  1. #### Product & domain value: Problem statement, user persona, measurable success metrics, appetite.
  2. #### Architectural constraints: Boundaries, target modules, dependencies, cross-crate interfaces, invariants.
  3. #### Safety & risk profile: ISO 14971 hazards, regulatory compliance (IEC 62304), negative constraints (no-gos and rabbit holes).
  4. #### Implementation directives: Specific CLI commands to create and relate entities (`qdev create prd|requirement|adr|hazard`, `qdev relate`).
  ```

## Verification

**Commands:**
- `cargo test --package qdev-core --test suite skills_tests` -- expected: all skills tests pass
- `cargo test --package qdev-cli --test suite install_skills_cli_tests` -- expected: install skills CLI tests pass
- `cargo test --package qdev-core --package qdev-cli` -- expected: full suite clean pass
