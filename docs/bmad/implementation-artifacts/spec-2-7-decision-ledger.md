---
title: 'Story 2.7: Decision Ledger'
type: 'feature'
created: '2026-09-13'
status: 'done'
baseline_commit: '07f6d8a7e7b3b25d70c5df051fe1c8bf19ce9841'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Autonomous agents and developers lack a unified, citable, and queryable ledger for human rulings, agent assumptions, architectural pivots, and governance overrides against arbitrary subject entities.

**Approach:** Implement `qdev decision log` and unified `log_decision` domain logic in `qdev-core`, write schema-validated `DEC-hhhh` markdown records under `docs/state/decisions/` with advisory write locking, update SQLite cache entities and decision tables, extend `qdev list decisions` with `--subject` and `--type` filtering, and refactor all system-generated decision logging (cross-team overrides, lease overrides, review rejections, and pivots) through the shared decision path.

## Boundaries & Constraints

**Always:**
- Decision identifiers use `DEC-` followed by 4+ lowercase hexadecimal characters (`DEC-hhhh`) allocated via `allocate_decision_id_in_with_rng`.
- Decision records are stored in `docs/state/decisions/<dec-id>.md` (or storage-configured state directory) and committed to Git.
- Decision frontmatter must strictly validate against `schemas/decision.json` (`EntityKind::Decision`).
- Allowed `decision_type` values are strictly: `human_ruling`, `agent_assumption`, `cross_team_override`, `pivot`, `review_rejection`, `lease_override`.
- `qdev decision log` requires `--subject`, `--type`, `--topic`, and `--ruling`.
- `--ruling` cannot be empty or whitespace-only; an empty ruling fails with exit code 2 (`UsageError`).
- `--type` must match one of the allowed decision types; an unrecognized type fails with exit code 2 (`UsageError`).
- `--subject` must name an existing entity resolvable in the workspace; an unresolvable subject fails with exit code 2 (`UsageError`).
- If `--context` is omitted from `qdev decision log`, the context defaults to `format!("Decision on {}", subject_id)`.
- Decisions are exempt from story lease gating; logging a decision does not require holding a lease on the subject entity.
- Writing decision files must acquire the workspace advisory write lock (`write.lock`).
- Creating a decision must synchronize the SQLite cache (`EntityRecord` in `entities` and `DecisionRecord` in `decisions`).
- All system-generated decisions (in `governance.rs`, `lease.rs`, and `transition.rs`) must use the unified `qdev_core::log_decision` function.
- `qdev list decisions` supports `--subject <id>` and `--type <type>` filters that combine via boolean AND.
- `qdev list decisions` outputs a standard `ListPayload` envelope when invoked with `--json`.
- `qdev decision log` outputs `DecisionLogPayload` within a `JsonEnvelope` when invoked with `--json`.

**Never:**
- Never modify or mutate the subject entity's markdown specification when logging a decision against it.
- Never allow unvalidated or arbitrary strings in `decision_type`.
- Never execute decision logging without an initialized workspace (guarded by `requires_workspace`).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| Happy Path: Manual Decision | `qdev decision log --subject E12S4 --type human_ruling --topic "Buffer sizing" --ruling "Fixed 4 MB pool"` | Exit 0, writes `docs/state/decisions/DEC-hhhh.md`, syncs SQLite, prints log summary | N/A |
| Happy Path: Decision with Context | `qdev decision log --subject AD-43 --type agent_assumption --topic "Serialization" --ruling "Use rmp-serde" --context "Benchmarked 30% faster"` | Exit 0, frontmatter and markdown body record provided context | N/A |
| JSON Output | `qdev decision log --subject E12S4 ... --json` | Exit 0, outputs `JsonEnvelope<DecisionLogPayload>` on stdout | N/A |
| Missing Subject Entity | `qdev decision log --subject NONEXISTENT ...` | Exit 2 (`UsageError`), no file or cache row created | Emits `usage_error` / `entity_not_found` |
| Invalid Decision Type | `qdev decision log --subject E12S4 --type invalid_type ...` | Exit 2 (`UsageError`), lists valid types | Emits `usage_error` |
| Empty Ruling | `qdev decision log --subject E12S4 --type human_ruling --topic "T" --ruling "   "` | Exit 2 (`UsageError`) | Emits `usage_error` |
| Missing Required Flags | `qdev decision log --subject E12S4` (missing `--type`, `--topic`, `--ruling`) | Exit 2 (Clap usage error) | CLI prints usage error |
| List Filter by Subject | `qdev list decisions --subject E12S4` | Exit 0, returns only decisions where `subject_id = 'E12S4'` | N/A |
| List Filter by Subject and Type | `qdev list decisions --subject E12S4 --type cross_team_override` | Exit 0, returns only decisions matching both filters | N/A |
| List Filter No Matches | `qdev list decisions --subject E12S4 --type pivot` (none exist) | Exit 0, prints `(no matching entities)` or empty items array in JSON | N/A |
| Outside Workspace | `qdev decision log ...` run outside workspace | Exit 2 (`UsageError`), no cache touched | Guarded by `requires_workspace` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/decision.rs` -- New module implementing `log_decision`, `DecisionInput`, `DecisionLogPayload`, `VALID_DECISION_TYPES`, markdown serialization, and cache synchronization.
- `crates/qdev-core/src/lib.rs` -- Re-export decision module structures and functions.
- `crates/qdev-core/src/store/mod.rs` -- Add `subject: Option<String>` and `decision_type: Option<String>` to `EntityFilter`.
- `crates/qdev-core/src/query.rs` -- Add `subject: Option<String>` and `decision_type: Option<String>` to `ListQueryOptions`, propagating to `EntityFilter`.
- `crates/qdev-core/src/store/sqlite.rs` -- Update `list_entities` SQL query to `LEFT JOIN decisions d ON e.id = d.id` and filter on `d.subject_id` and `d.decision_type`.
- `crates/qdev-core/src/governance.rs` -- Refactor `create_governance_override_decision` to delegate to `crate::decision::log_decision`.
- `crates/qdev-core/src/lease.rs` -- Refactor `create_lease_override_decision` to delegate to `crate::decision::log_decision`.
- `crates/qdev-core/src/transition.rs` -- Refactor `record_backward_transition_decision` to delegate to `crate::decision::log_decision`.
- `crates/qdev-cli/src/cli.rs` -- Add `Decision(DecisionArgs)` with `Log(DecisionLogArgs)` subcommand; add `--subject` and `--type` to `ListArgs`.
- `crates/qdev-cli/src/main.rs` -- Implement `handle_decision_log`, wire `Commands::Decision` in dispatch, update `requires_workspace`, wire `--subject` and `--type` in `handle_list`.
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `decision log` to `guarded_invocations()`.
- `crates/qdev-core/tests/decision_tests.rs` -- New unit test suite testing `log_decision`, validation rules, and cache sync.
- `crates/qdev-cli/tests/decision_cli_tests.rs` -- New CLI integration test suite verifying `qdev decision log`, `qdev list decisions --subject ... --type ...`, JSON envelopes, system-generated decision parity, and error cases.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/decision.rs` -- Create module with `log_decision`, `DecisionInput`, `DecisionLogPayload`, `VALID_DECISION_TYPES`, schema validation, atomic file creation, and store synchronization.
- [x] `crates/qdev-core/src/lib.rs` -- Export `log_decision`, `DecisionInput`, `DecisionLogPayload`, and `VALID_DECISION_TYPES`.
- [x] `crates/qdev-core/src/store/mod.rs` -- Add `subject` and `decision_type` fields to `EntityFilter`.
- [x] `crates/qdev-core/src/query.rs` -- Add `subject` and `decision_type` fields to `ListQueryOptions`.
- [x] `crates/qdev-core/src/store/sqlite.rs` -- Update `list_entities` with left join to `decisions` and filtering on subject and decision type.
- [x] `crates/qdev-core/src/governance.rs` -- Refactor `create_governance_override_decision` to call `crate::decision::log_decision`.
- [x] `crates/qdev-core/src/lease.rs` -- Refactor `create_lease_override_decision` to call `crate::decision::log_decision`.
- [x] `crates/qdev-core/src/transition.rs` -- Refactor `record_backward_transition_decision` to call `crate::decision::log_decision`.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `Decision` command, `DecisionLogArgs`, and add `--subject` and `--type` to `ListArgs`.
- [x] `crates/qdev-cli/src/main.rs` -- Implement `handle_decision_log`, update `handle_list`, and register `Commands::Decision` in `requires_workspace`.
- [x] `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `decision log` to `guarded_invocations()`.
- [x] `crates/qdev-core/tests/decision_tests.rs` -- Unit tests for `log_decision`, input validation, subject kinds, and store updates.
- [x] `crates/qdev-cli/tests/decision_cli_tests.rs` -- CLI integration tests for `qdev decision log`, `qdev list decisions --subject ... --type ...`, JSON envelopes, system-generated decision parity, and error cases.

**Acceptance Criteria:**
- Given a subject `E12S4`, `E12`, or `AD-43`, when running `qdev decision log --subject E12S4 --type human_ruling --topic "Buffer sizing" --ruling "Fixed 4 MB pool"`, then a `DEC-hhhh` file is created under `docs/state/decisions/` with context, ruling, author, and type from the allowed set in the schema.
- Given decisions recorded for `E12S4`, when running `qdev list decisions --subject E12S4 --type cross_team_override`, then the list output filters correctly by both subject and type.
- Given system-generated decisions (pivot, review rejection, lease override, cross-team override), then they use the same `DEC-hhhh` path, schema validation, and storage synchronization as manual decisions.
- Given an invalid decision type or empty ruling or non-existent subject entity, when running `qdev decision log`, then the command exits 2 with a descriptive usage error.
- Given invocation outside an initialized workspace, then `qdev decision log` exits 2 without touching any cache file.

## Implementation Notes

- Implemented `qdev_core::decision` module containing `log_decision`, `log_decision_with_store`, `DecisionInput`, and `VALID_DECISION_TYPES`.
- Frontmatter validates against `schemas/decision.json` (`EntityKind::Decision`).
- Refactored `governance.rs`, `lease.rs`, and `transition.rs` to delegate decision logging to `log_decision`.
- Updated SQLite query engine in `sqlite.rs` and `query.rs` with `subject` and `decision_type` filters.
- Added `qdev decision log` CLI command and updated `qdev list` with `--subject` and `--type` filter flags.
- Registered `decision log` in `requires_workspace` and `workspace_guard_cli_tests.rs`.
- Removed stale `IDENTITY_RULE_SITES` entries in `architecture_tests.rs`.
- All unit and CLI integration tests pass cleanly with zero failures and zero clippy warnings.

## Spec Change Log

## Review Triage Log

| Finding ID | Verdict | Evidence / Refutation |
|---|---|---|
| VG-1 | `low` (patch) | Stale entries for `transition.rs`, `lease.rs`, and `governance.rs` in `IDENTITY_RULE_SITES` in `architecture_tests.rs` should be removed now that path creation is consolidated in `decision.rs`. |
| VG-2 | `low` (patch) | Added CLI test case verifying `--topic ""` and `--topic "   "` are rejected with exit code 2. |
| VG-3 | `low` (patch) | Added CLI test case verifying `qdev list decisions --type human_ruling` filters across subjects without requiring `--subject`. |
| VG-4 | `medium` (patch) | Body markdown in `decision.rs` emitted both `Context: <context>` and `Transition: <context>` redundantly for pivots/rejections; simplified to standard `Context: <context>\n\n<ruling>\n` per design notes. |
| BH-1 | `medium` (patch) | Duplicate of VG-4. |
| BH-2 | `low` (patch) | Duplicate of VG-1. |
| BH-3 | `low` (patch) | Removed needless borrow `if let Some(ref store)` in `decision.rs`. |
| BH-4 | `false` | SQLite `LEFT JOIN` returning 0 rows for non-matching kind-specific filters is the established, uniform design across `qdev list` (identical to `--epic`, `--sprint`, and `--module`). |
| BH-5 | `false` | `qdev list` query filters uniformly return empty results for non-matching criteria rather than failing as usage errors (identical to `--status`, `--owner`, `--module`). |
| BH-6 | `false` | `ListPayload` schema is fixed by project-wide `payload-list.json`; modifying it for decisions would break schema conformance. |
| BH-7 | `low` (patch) | `resolve_entity_file` error in `decision.rs` was masked under `entity_not_found`; return `Err(e)` directly to preserve exact error code. |
| BH-8 | `false` | Logging decisions against embedded constraint sub-items is outside the scope of Story 2.7. |
| BH-9 | `false` | Dropping write lock before `log_decision` avoids recursive file locking deadlocks on non-reentrant platforms and is verified by 26 transition tests. |
| BH-10 | `false` | Entity record plus detail record upsert is the standard architectural pattern across all qdev cache writes. |
| BH-11 | `false` | Multi-line ruling from stdin or file is a feature request not specified in Story 2.7 acceptance criteria. |
| BH-12 | `false` | `qdev decision show` is not part of Story 2.7 requirements. |
| BH-13 | `false` | Lifecycle documentation is populated during review triage. |
| BH-14 | `low` (patch) | Added test cases for attribution override flags (`--author-type`, `--author-id`) and formatted table output in CLI tests. |


## Design Notes

- Decision records use markdown frontmatter with fields: `id`, `title` (from topic or type), `status` ("active"), `version` (1), `created_by`, `updated_by`, `subject_id`, `decision_type`, `topic`, `context`, `ruling`, and `created_at`.
- Body content format: `---\n<yaml>---\n\n# <title>\n\nContext: <context>\n\n<ruling>\n`.
- The six allowed decision types are: `human_ruling`, `agent_assumption`, `cross_team_override`, `pivot`, `review_rejection`, `lease_override`.
- Default context when `--context` is omitted is `format!("Decision on {}", subject_id)`.
- `qdev list decisions` filtering executes directly in SQLite query via `LEFT JOIN decisions d ON e.id = d.id` where `d.subject_id = ?` and `d.decision_type = ?`.

## Verification

**Commands:**
- `cargo test --test decision_tests` -- expected: Core unit tests pass for decision logging, validation, schema compliance, and store upsert.
- `cargo test --test decision_cli_tests` -- expected: CLI integration tests verify logging, filtering, JSON envelopes, system decision parity, and exit codes.
- `cargo test --test workspace_guard_cli_tests` -- expected: Workspace guard verifies `qdev decision log` is refused outside a workspace.
- `cargo test --test governance_tests --test lease_tests --test transition_tests` -- expected: Existing system-generated decision tests continue to pass without regressions.
- `cargo test` -- expected: Entire workspace build and test suite pass cleanly.
