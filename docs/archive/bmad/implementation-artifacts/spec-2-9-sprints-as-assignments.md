---
title: 'Story 2.9: Sprints as Assignments'
type: 'feature'
created: '2026-09-15'
status: 'in-review'
baseline_commit: '7eabcfcf2dce9fa9d7ad692e39cae63350ab5d12'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Autonomous agents and release managers need sprints to schedule stories without owning them or altering story identifiers, ensuring sprint carry-over preserves story IDs, patch sprints can operate concurrently, and commands can deterministically resolve sprint scopes.

**Approach:** Implement `qdev sprint` CLI commands (`open`, `assign`, `close`) and domain logic in `crates/qdev-core/src/sprint.rs`. Decouple story identity from sprint membership via `docs/state/sprints/sprint-{n}.md` frontmatter and `sprint_assignments` SQLite table, enforce that stories may not belong to multiple active sprints via `qdev validate`, provide deterministic `--sprint` fallback resolution (`default_sprint` -> single active sprint -> exit 2), and execute carry-over on sprint close without renaming stories.

## Boundaries & Constraints

**Always:**
- Sprint definitions live strictly in `docs/state/sprints/sprint-{n}.md` validated against `schemas/sprint.json` (`EntityKind::Sprint`).
- Story files and story identifiers (`id`, filename) are immutable during sprint assignment and carry-over; only assignment records change.
- Sprints are written atomically under the workspace advisory write lock, keeping markdown files and SQLite cache (`sprints`, `sprint_assignments`, `entities`) synchronized in lock-step.
- `qdev validate` reports an error finding if any story is assigned to more than one active sprint (`status == "active"`).
- Sprints queried via `qdev get sprint <id> --json` project assigned stories and per-status counts.
- Commands accepting `--sprint` resolve in strict order: explicit `--sprint` / argument -> `default_sprint` in `qdev.toml` -> unique single active sprint -> usage error (exit 2).

**Never:**
- Never modify story file frontmatter or rename story Markdown files when assigning stories or executing sprint carry-over.
- Never allow sprint assignment writes to bypass the atomic write lock or SQLite cache update.
- Never prompt interactively for `--sprint` in non-interactive mode; fail closed with exit code 2.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Open sprint | `qdev sprint open 6 --title "Port" --release 0.1.0` | Creates `docs/state/sprints/sprint-6.md` with status `active`, release, started date, empty assignments; returns exit 0 | Exit 1 if sprint already exists or title empty |
| Assign stories | `qdev sprint assign 6 E12S4 E11S9` | Appends assignments with `assigned_at` to `sprint-6.md`, updates `sprint_assignments` table, exit 0 | Exit 2 if sprint not found or story does not exist |
| Get sprint projection | `qdev get sprint 6 --json` | Returns JSON envelope with sprint entity, `assignments`, and `status_counts` | Exit 2 if sprint not found |
| List stories in sprint | `qdev list stories --sprint 6` | Returns stories assigned to sprint 6 | Exit 0 (empty table if no stories assigned) |
| Sprint fallback (single active) | `qdev sprint close --carry-over 7` with 1 active sprint (6) | Automatically resolves sprint 6, marks 6 `completed`, carries open stories to 7 with `carried_from: 6` | N/A |
| Sprint fallback (ambiguous) | `qdev sprint close --carry-over 7` with 2 active sprints, no `default_sprint` | Fails closed asking for `--sprint` | Exit 2 with usage error |
| Sprint fallback (`default_sprint`) | Command with no `--sprint`, multiple active sprints, `default_sprint = 6` | Resolves sprint 6 from configuration | N/A |
| Duplicate active sprint validation | Story `E12S4` assigned to active sprint 5 and active sprint 6 | `qdev validate` reports error finding `duplicate_active_sprint_assignment` | Exit 1 from `qdev validate` |
| Close sprint carry-over | `qdev sprint close 5 --carry-over 6` with `E12S4` (ready) and `E12S5` (done) | Sprint 5 status `completed`; sprint 6 contains `E12S4` with `carried_from: 5`; `E12S5` is not carried | Exit 2 if sprint 5 not found |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/sprint.rs` -- New module implementing sprint lifecycle operations: `open_sprint`, `assign_to_sprint`, `close_sprint`, `resolve_sprint_selection`, and assignment projection helpers.
- `crates/qdev-core/src/lib.rs` -- Register and export `sprint` module and types.
- `crates/qdev-core/src/validate.rs` -- Implement `find_duplicate_active_sprint_assignments` and invoke it in `run_validation`.
- `crates/qdev-core/src/query.rs` -- Update `query_entity` and `build_entity_projection` to support sprint ID lookup (`6` or `sprint-6`), populating `assignments` and `status_counts` for `EntityKind::Sprint`.
- `crates/qdev-core/schemas/payload-story.json` -- Document optional `assignments` and `status_counts` on entity projections.
- `crates/qdev-core/tests/architecture_tests.rs` -- Register `crates/qdev-core/src/sprint.rs` in `IDENTITY_RULE_SITES` if path resolution is constructed.
- `crates/qdev-cli/src/cli.rs` -- Add `Sprint(SprintArgs)` command, subcommands `Open`, `Assign`, `Close`, and ensure `--sprint` flags are available.
- `crates/qdev-cli/src/main.rs` -- Implement `handle_sprint`, wire subcommands, format human-readable sprint output in `render_get_entity_text`, and register in `requires_workspace`.
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `sprint open` to `guarded_invocations()`.
- `crates/qdev-core/tests/sprint_tests.rs` -- Unit tests for sprint opening, story assignment, carry-over, multi-active sprint validation, and fallback resolution.
- `crates/qdev-cli/tests/sprint_cli_tests.rs` -- Integration tests for `qdev sprint open`, `assign`, `close`, `qdev get sprint 6 --json`, `list stories --sprint 6`, exit codes, and carry-over behavior.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/sprint.rs` -- Create sprint domain module with `open_sprint`, `assign_to_sprint`, `close_sprint`, `resolve_sprint_selection`, and assignment models.
- [x] `crates/qdev-core/src/lib.rs` -- Export sprint module and public API.
- [x] `crates/qdev-core/src/query.rs` -- Support numeric sprint ID lookups and populate `assignments` and `status_counts` on sprint entity projections.
- [x] `crates/qdev-core/schemas/payload-story.json` -- Add `assignments` and `status_counts` properties.
- [x] `crates/qdev-core/src/validate.rs` -- Implement `find_duplicate_active_sprint_assignments` and register in `run_validation`.
- [x] `crates/qdev-core/tests/architecture_tests.rs` -- Register sprint path joins in `IDENTITY_RULE_SITES` if needed.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `Sprint` command and `SprintArgs` with `Open`, `Assign`, `Close` subcommands.
- [x] `crates/qdev-cli/src/main.rs` -- Wire `Commands::Sprint`, implement handlers, format text output for get sprint, and register in `requires_workspace`.
- [x] `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add sprint command to `guarded_invocations()`.
- [x] `crates/qdev-core/tests/sprint_tests.rs` -- Unit tests for sprint domain logic, fallback resolution, validation checks, and carry-over.
- [x] `crates/qdev-cli/tests/sprint_cli_tests.rs` -- CLI integration tests for `sprint open`, `assign`, `close`, `get sprint`, and `list stories --sprint`.

**Acceptance Criteria:**
- Given a workspace, when running `qdev sprint open 6 --title "The Rust Core Port" --release 0.1.0`, then `docs/state/sprints/sprint-6.md` is created with status `active` and `sprints` cache table is hydrated.
- Given sprint 6 and existing stories `E12S4` and `E11S9`, when running `qdev sprint assign 6 E12S4 E11S9`, then `sprint-6.md` records assignments with `assigned_at` and `sprint_assignments` is hydrated.
- Given a story assigned to multiple active sprints, when running `qdev validate`, then validation reports error finding `duplicate_active_sprint_assignment` and exits 1.
- Given sprint 6 with assigned stories, when running `qdev list stories --sprint 6`, then only stories assigned to sprint 6 are returned.
- Given sprint 6 with assigned stories, when running `qdev get sprint 6 --json`, then the output includes `assignments` and `status_counts`.
- Given commands resolving a sprint without explicit flag, when `default_sprint` is configured, then `default_sprint` is selected.
- Given commands resolving a sprint without explicit flag or `default_sprint`, when exactly one active sprint exists, then that sprint is selected.
- Given commands resolving a sprint without explicit flag or `default_sprint`, when zero or multiple active sprints exist, then the command exits 2 with a usage error asking for `--sprint`.
- Given sprint 5 with non-done stories `E12S4` and done story `E12S5`, when running `qdev sprint close 5 --carry-over 6`, then sprint 5 is marked `completed`, sprint 6 records `E12S4` with `carried_from: 5`, `E12S5` is not carried over, and no story files or IDs are renamed.

## Implementation Notes

- Implemented `crates/qdev-core/src/sprint.rs` module providing `open_sprint`, `assign_to_sprint`, `close_sprint`, `resolve_sprint_selection`, and sprint identifier normalization.
- Sprints are written atomically under the workspace advisory write lock to `docs/state/sprints/sprint-{n}.md` and validated against `schemas/sprint.json` (`EntityKind::Sprint`), keeping SQLite cache tables (`sprints`, `sprint_assignments`, `entities`) updated in lock-step.
- Implemented `find_duplicate_active_sprint_assignments` in `crates/qdev-core/src/validate.rs`, reporting finding code `duplicate_active_sprint_assignment` with error severity when any story is assigned to multiple active sprints.
- Updated `query_entity` and `build_entity_projection` in `crates/qdev-core/src/query.rs` to support numeric sprint IDs (`6` or `sprint-6`), populating `assignments` (`SprintAssignmentProjection`) and `status_counts` breakdown for `EntityKind::Sprint`.
- Registered `crates/qdev-core/src/sprint.rs` path construction sites in `IDENTITY_RULE_SITES` in `crates/qdev-core/tests/architecture_tests.rs`.
- Added CLI subcommands `qdev sprint open`, `assign`, and `close` in `crates/qdev-cli/src/cli.rs` and `crates/qdev-cli/src/main.rs`, adding text rendering for assignments and status counts in `render_get_entity_text`.
- Registered `sprint` in `requires_workspace` and added to `guarded_invocations()` in `crates/qdev-cli/tests/workspace_guard_cli_tests.rs`.
- Created comprehensive test suites: 11 unit tests in `crates/qdev-core/tests/sprint_tests.rs` and 7 CLI integration tests in `crates/qdev-cli/tests/sprint_cli_tests.rs`. All tests pass cleanly.

## Spec Change Log

## Review Triage Log

| ID | Source | Location | Description | Verdict | Evidence | Route |
|---|---|---|---|---|---|---|
| REV-1 | Blind Hunter | `crates/qdev-core/src/sprint.rs:481` | Re-assigning an already assigned/carried story overwrites `carried_from` and `assigned_at` with None in SQLite | `medium` | `upsert_sprint_assignment` is called unconditionally on all input stories, resetting `carried_from` in SQLite while frontmatter preserves it. | `patch` |
| REV-2 | Blind Hunter | `crates/qdev-core/src/sprint.rs:776` | In `close_sprint`, if target sprint already has story, SQLite is upserted but frontmatter is skipped | `low` | Desynchronizes SQLite and target markdown file when target already has an assignment record. | `patch` |
| REV-3 | Blind Hunter | `crates/qdev-core/src/sprint.rs:646` | Carrying over stories in terminal states (`abandoned`, `superseded`) | `medium` | Only checked `status != "done"`; terminal non-done states should remain closed rather than carried forward. | `patch` |
| REV-4 | Blind Hunter | `crates/qdev-core/src/sprint.rs:646` | Missing or deleted story entities carried over into target sprint | `low` | If `get_entity` returns None, phantom story ID is carried over; should verify story exists. | `patch` |
| REV-5 | Blind Hunter | `crates/qdev-core/src/sprint.rs:320` | Modifying or closing completed sprints | `medium` | Completed sprints should be immutable; `assign_to_sprint` and `close_sprint` should refuse operations on completed sprints. | `patch` |
| REV-6 | Blind Hunter | `crates/qdev-core/src/sprint.rs:543` | Target sprint in carry-over can be completed | `medium` | Carrying over to an already completed sprint violates lifecycle; target must be active. | `patch` |
| REV-7 | Blind Hunter | `crates/qdev-core/src/validate.rs:709` | Incomplete sorting order in `find_duplicate_active_sprint_assignments` | `low` | Findings sorted by `(path, code)` instead of calling `sort_findings(&mut findings)`. | `patch` |
| REV-8 | Blind Hunter | `crates/qdev-core/src/validate.rs` | `filter_by_changed` drops participating active sprints | `false` | Findings are reported on the modified sprint file path, matching existing validator behavior. | `reject` |
| REV-9 | Blind Hunter | `crates/qdev-cli/src/cli.rs:304` | `ListArgs.sprint` does not accept string `sprint-<n>` | `low` | Typing as `Option<String>` and normalizing via `normalize_sprint_id` unifies CLI argument handling. | `patch` |
| REV-10 | Blind Hunter | `crates/qdev-core/src/query.rs:307` | `query_entity` sprint ID resolution handles empty string and capitalization inconsistently | `low` | Using `normalize_sprint_id` in `query_entity` standardizes resolution and prevents empty string edge cases. | `patch` |
| REV-11 | Blind Hunter | `crates/qdev-core/src/query.rs` | Missing story entities omitted from `status_counts` | `false` | Story entities assigned to sprints have defined statuses; filtering absent records is standard. | `reject` |
| REV-12 | Blind Hunter | `crates/qdev-core/src/query.rs` | `EntityProjection` omits sprint `started_at` and `release` | `false` | Outside scope of Story 2.9 AC which specifies `assignments` and `status_counts`. | `reject` |
| REV-13 | Blind Hunter | `crates/qdev-cli/src/main.rs:5518` | CLI argument conflict when positional sprint and `--sprint` are both passed | `low` | If `--sprint` is provided, don't misinterpret positional sprint argument as a story ID. | `patch` |
| REV-14 | Blind Hunter | `crates/qdev-core/src/sprint.rs:410` | No-op sprint assignments still patch frontmatter and bump version | `low` | When no new assignments are added, return existing result without rewriting file. | `patch` |
| REV-15 | Blind Hunter | `crates/qdev-core/src/sprint.rs:277` | Whitespace-only release option stored in SQLite | `low` | Trim and filter release option; only write `release` in frontmatter. | `patch` |
| REV-16 | Blind Hunter | `crates/qdev-core/src/sprint.rs` | Direct store mutations lack single transaction | `false` | SQLite store manages statement execution and locking consistently with existing entity stores. | `reject` |
| REV-17 | Blind Hunter | `crates/qdev-core/schemas/sprint.json` | Frontmatter schema defines `assignments` as untyped array | `low` | Specify item schema with `story` and `assigned_at`. | `patch` |
| REV-18 | Blind Hunter | `crates/qdev-core/src/sprint.rs:172` | `unwrap_or(true)` swallows database error | `low` | Propagate database errors via `?`. | `patch` |
| REV-19 | Verification Gap | `crates/qdev-cli/src/main.rs:5518` | `handle_sprint_assign` CLI `--sprint` flag and fallback resolution untested | `medium` | Pre-verified gap; add CLI test cases in `sprint_cli_tests.rs`. | `patch` |
| REV-20 | Verification Gap | `crates/qdev-cli/src/main.rs:2948` | `render_get_entity_text` text-mode sprint output untested | `low` | Pre-verified gap; add CLI test assertion in `sprint_cli_tests.rs`. | `patch` |
| REV-21 | Verification Gap | `crates/qdev-core/src/query.rs:246` | `SprintAssignmentProjection.carried_from` projection untested on carried-over stories | `low` | Pre-verified gap; add assertion in `sprint_tests.rs`. | `patch` |
| REV-22 | Verification Gap | `crates/qdev-core/src/sprint.rs:481` | Re-assigning a carried-over story overwrites `carried_from` in SQLite | `medium` | Grouped with REV-1. | `patch` |

## Design Notes

- Sprint frontmatter conforms to `schemas/sprint.json` (`EntityKind::Sprint`).
- Sprint IDs are normalized to `sprint-{n}` for filenames and entity IDs, while accepting numeric inputs (`6` or `sprint-6`) across CLI arguments and lookups.
- Assignments in `sprint-{n}.md` are stored in the frontmatter `assignments` sequence:
  ```yaml
  assignments:
    - story: E12S4
      assigned_at: "2026-09-15"
    - story: E11S9
      assigned_at: "2026-09-15"
      carried_from: 5
  ```
- Sprint carry-over inspects the status of each assigned story in the closing sprint. Any story whose status is not `done` is added to the target sprint with `carried_from` set to the closing sprint number. Done stories remain assigned to the completed sprint.
- `--sprint` fallback resolution helper `resolve_sprint_selection(store, config, explicit)` provides the single uniform resolution path for all current and future sprint-scoped commands.

## Verification

**Commands:**
- `cargo test --test sprint_tests` -- expected: Unit tests verify sprint creation, assignment persistence, carry-over logic, fallback resolution, and duplicate active sprint validation.
- `cargo test --test sprint_cli_tests` -- expected: CLI integration tests verify `open`, `assign`, `close`, `get sprint`, `list stories --sprint`, exit codes, JSON envelopes, and fallback behavior.
- `cargo test --test workspace_guard_cli_tests` -- expected: Verifies `qdev sprint` is guarded outside an initialized workspace.
- `cargo test --test architecture_tests` -- expected: Verifies identity rule sites and dependencies remain compliant.
- `cargo test` -- expected: Entire workspace build and test suite pass cleanly with zero warnings.
