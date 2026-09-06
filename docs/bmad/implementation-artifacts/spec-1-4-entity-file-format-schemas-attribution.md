---
title: 'Entity File Format, Schemas & Attribution'
type: 'feature'
created: '2026-09-06'
status: 'in-progress'
baseline_revision: '61fe14cb51064ababa7daa859ab042ba029840b1'
review_loop_iteration: 0
followup_review_recommended: false
context:
  - 'docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md'
  - 'docs/architecture.md'
  - 'docs/cli-reference.md'
warnings: []
deferred: []
---

<intent-contract>

## Intent

**Problem:** Entities in qdev are hand-editable Markdown files with YAML frontmatter, but without strict machine-enforceable JSON Schemas embedded in the binary, file structures drift, attribution metadata is omitted or corrupted, and downstream validation and hydration engines cannot reliably parse and index records.

**Approach:** Define strict JSON Schema documents for all 13 entity kinds (PRD, requirement, epic, story, ADR, hazard, sprint, release, deferred work, decision, scratchpad entry, SOUP dependency, and evidence record) embedded directly in `qdev-core`. Implement frontmatter extraction and validation in `qdev-core`, a test fixture suite with 1 valid and at least 2 invalid examples per kind, and a CLI subcommand `qdev schema <kind>` in `qdev-cli`.

## Boundaries & Constraints

**Always:**
- Embed all 13 JSON Schema files directly into `qdev-core` using compile-time `include_str!`.
- Every entity schema must require `id`, `title` (where applicable: PRD, requirement, epic, story, ADR, hazard, sprint, release, deferred work, decision), `status`, `version` (positive integer), and attribution fields `created_by` and `updated_by` objects with required `type` (`human` or `agent`) and `id` (non-empty string).
- Story frontmatter schema must express constraints (`id`, `kind` in `['no_go', 'rabbit_hole', 'appetite']`, `text`) and relations (`depends_on`, `traces_to`, `governed_by`, `extends`, `supersedes`, `mitigates`, `closes_dw`) exactly as illustrated in `docs/architecture.md` §9.
- Provide a test fixture directory containing 1 valid and at least 2 invalid examples per kind (minimum 39 fixture files total) and verify them in automated tests.
- `qdev schema <kind>` must print the formatted schema to stdout in text mode, or wrap the schema object inside an AD-13 `JsonEnvelope` with `schema_version: "1"` under `--json`.
- `qdev schema <kind>` must be operational without requiring an initialized workspace or existing `qdev.toml`.
- Keep `qdev-core` completely free of forbidden terminal and network dependencies (`clap`, `colored`, `reqwest`, etc.) and direct terminal I/O per AD-1.
- Never write to or revert `sprint-status.yaml`.

**Never:**
- Never add forbidden network dependencies (`reqwest`, `hyper`, `curl`, `ureq`) to the workspace (violates NFR-403).
- Never allow `qdev-core` to perform direct terminal I/O (`println!`, `eprintln!`, `std::io::stdout`).
- Never allow missing attribution (`created_by`, `updated_by`) or author types other than `human` or `agent`.
- Never fail on unrecognized optional markdown body content; schemas validate frontmatter metadata.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Schema text happy path | `qdev schema story` | Exit 0; pretty-printed JSON Schema for story entity on stdout | No error expected |
| Schema JSON happy path | `qdev schema story --json` | Exit 0; AD-13 `JsonEnvelope` with `schema_version: "1"` containing story schema on stdout | No error expected |
| Schema kind alias | `qdev schema dw` / `qdev schema deferred_work` / `qdev schema adrs` | Exit 0; resolves alias and prints corresponding JSON Schema | No error expected |
| Schema in uninitialized dir | In empty temp dir: `qdev schema prd` | Exit 0; prints schema without requiring `qdev.toml` or workspace | No error expected |
| Unknown schema kind | `qdev schema invalid_kind` | Exit 2; error message listing valid schema kinds | Exit code 2 usage error |
| Unknown schema kind with `--json` | `qdev schema invalid_kind --json` | Exit 2; AD-13 `JsonErrorEnvelope` with code `usage_error` on stdout | Exit code 2 usage error |
| Missing kind argument | `qdev schema` | Exit 2; clap usage error identifying missing `<KIND>` argument | Exit code 2 usage error |
| Valid fixture validation | Frontmatter matching all required schema fields | Validation succeeds with 0 errors | No error expected |
| Missing attribution validation | Frontmatter missing `created_by` or `updated_by` | Validation fails reporting missing required property | Returns validation error list |
| Invalid author type validation | Frontmatter with `created_by: { type: "system", id: "bot" }` | Validation fails reporting type must be one of `human`, `agent` | Returns validation error list |
| Invalid relation structure | Frontmatter with `relations: { depends_on: "E12S3" }` (string instead of array) | Validation fails reporting `depends_on` must be array | Returns validation error list |

</intent-contract>

## Code Map

- `crates/qdev-core/Cargo.toml` -- Add `serde_yaml = "0.9"` and `jsonschema = { version = "0.54", default-features = false }` -- Frontmatter deserialization and offline JSON Schema validation
- `crates/qdev-core/schemas/*.json` -- 13 JSON Schema files (`prd.json`, `requirement.json`, `epic.json`, `story.json`, `adr.json`, `hazard.json`, `sprint.json`, `release.json`, `dw.json`, `decision.json`, `scratchpad.json`, `soup.json`, `evidence.json`) -- Canonical entity frontmatter schemas
- `crates/qdev-core/src/schema.rs` -- `EntityKind` enum, schema string/value getters, embedded schemas via `include_str!`, frontmatter extraction, and validation engine -- Core schema API
- `crates/qdev-core/src/lib.rs` -- Re-export `schema` types (`EntityKind`, `extract_frontmatter`, `validate_frontmatter`, etc.) -- Public core interface
- `crates/qdev-core/tests/fixtures/{kind}/*` -- 13 directories, each with `valid.md`, `invalid_missing_fields.md`, and `invalid_attribution.md` -- Golden test fixtures
- `crates/qdev-core/tests/schema_tests.rs` -- Unit tests verifying all 13 schemas, embedded assets, fixture validation, and edge cases -- Core verification
- `crates/qdev-cli/src/cli.rs` -- Add `Schema(SchemaArgs)` subcommand taking `kind: String` -- CLI grammar
- `crates/qdev-cli/src/main.rs` -- Dispatch `Commands::Schema` before workspace config loading, handling text and JSON envelope emission -- CLI execution and I/O
- `crates/qdev-cli/tests/schema_cli_tests.rs` -- E2E CLI integration tests for `qdev schema` in text and JSON modes, aliases, error conditions, and uninitialized directory execution -- CLI contract verification

## Tasks & Acceptance

**Execution:**
- `crates/qdev-core/Cargo.toml` -- Add `serde_yaml` and `jsonschema` (no default features) dependencies -- AD-1 storage dependencies
- `crates/qdev-core/schemas/*.json` -- Create 13 strict JSON Schema documents matching `docs/architecture.md` §5 & §9 -- Domain entity schemas
- `crates/qdev-core/src/schema.rs` -- Implement `EntityKind`, schema lookup, alias resolution, frontmatter extraction, and validation logic -- Core schema engine
- `crates/qdev-core/src/lib.rs` -- Re-export schema API from `qdev_core` -- Public core API
- `crates/qdev-core/tests/fixtures/` -- Populate 1 valid and at least 2 invalid examples for each of the 13 entity kinds -- Test fixtures
- `crates/qdev-core/tests/schema_tests.rs` -- Write automated tests for all embedded schemas, fixture validations, and attribution constraints -- Core validation
- `crates/qdev-cli/src/cli.rs` -- Add `schema` subcommand to Clap definitions -- CLI grammar
- `crates/qdev-cli/src/main.rs` -- Implement schema handler before config loading, emitting raw pretty JSON or `JsonEnvelope` -- CLI interaction
- `crates/qdev-cli/tests/schema_cli_tests.rs` -- Add E2E tests for schema printing, JSON envelope wrapping, and error handling -- E2E verification

**Acceptance Criteria:**
- Given the entity catalog in `docs/architecture.md` §5, when schemas are defined, then JSON Schema documents exist for PRD, requirement, epic, story, ADR, hazard, sprint, release, deferred work, decision, scratchpad entry, SOUP dependency, and evidence record, and are embedded in the binary.
- Given any entity schema, it requires `id`, `title` (where applicable), `status`, `version`, `created_by {type, id}`, `updated_by {type, id}` with `type ∈ {human, agent}`.
- Given the story schema, constraints and relations are expressed in frontmatter exactly as in `docs/architecture.md` §9.
- Given the fixture directory, it contains 1 valid and at least 2 invalid examples per kind, and automated tests verify all 13 valid fixtures pass and all 26+ invalid fixtures fail.
- Given `qdev schema <kind>`, when executed in text mode, the corresponding schema is printed to stdout; when executed with `--json`, it is wrapped in an AD-13 `JsonEnvelope` with `schema_version: "1"`.
- Given an invalid kind, when `qdev schema <kind>` is executed, it exits with code 2 (`UsageError`).
- Given the full test suite, `cargo test` passes cleanly with zero failures and zero network/architecture violations.

## Spec Change Log

## Review Triage Log

## Verification

**Commands:**
- `cargo test --test schema_tests` -- expected: all core schema and fixture validation tests pass
- `cargo test --test schema_cli_tests` -- expected: all CLI schema subcommand tests pass
- `cargo test` -- expected: entire workspace test suite passes including architecture and network checks
