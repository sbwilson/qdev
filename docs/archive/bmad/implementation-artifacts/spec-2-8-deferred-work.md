---
title: 'Story 2.8: Deferred Work'
type: 'feature'
created: '2026-09-15'
status: 'done'
baseline_commit: '9b5c9cf2826a4c19e1e5d433a835b2423c07c746'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Autonomous agents and developers lack a discrete, schema-validated ledger for tracking deferred debt and residual anomalies with ISO 14971 safety risks, required rationales, and target module validation.

**Approach:** Implement `qdev dw` CLI commands (`add`, `list`, `close`) and domain logic in `crates/qdev-core/src/dw.rs`, enforcing registered module verification, risk rationale gates citing `regulatory.require_rationale_for`, story lease gating, atomic file creation under `docs/state/dw/DW-hhhh.md`, SQLite synchronization (`entities` and `deferred_work`), and automatic closure when stories declaring `closes_dw` reach `done`.

## Boundaries & Constraints

**Always:**
- Deferred work identifiers use `DW-` followed by 4+ lowercase hex characters (`DW-hhhh`) via `allocate_deferred_work_id_in_with_rng`.
- Records are stored in `docs/state/dw/<dw-id>.md` and committed to Git.
- Frontmatter strictly validates against `schemas/dw.json` (`EntityKind::DeferredWork`).
- Initial status of created deferred work is strictly `open`.
- Allowed `safety_risk` values: `negligible`, `acceptable_with_mitigation`, `unacceptable`. Unrecognized risk exits 2 (`UsageError`).
- `--module` must exist in `config.modules`; unregistered module exits 2 (`UsageError`).
- If `--story` is specified, it must resolve to an existing story in workspace; unresolvable story exits 2 (`UsageError`).
- Omitting `--rationale` when `safety_risk` is non-negligible (or listed in `config.regulatory.require_rationale_for`) exits 1 (`LogicalFailure`), citing `regulatory.require_rationale_for`.
- Creating deferred work with `--story <id>` requires holding a lease on that story or explicit `--override --justification` (governed by `check_governance_gate`).
- `qdev dw close <id>` transitions status to `done` by default, setting `resolution` if `--resolution` is provided.
- `qdev dw close <id> --status wont_fix` requires a non-empty `--justification` (or `--resolution`); omitting it exits 2 (`UsageError`). Close statuses other than `done` or `wont_fix` exit 2 (`UsageError`).
- `qdev dw list` supports `--module`, `--risk`, `--status`, and `--story` filters combined with AND, outputting formatted text or JSON envelope (`--json`).
- Writing or updating DW files requires the advisory write lock (`write.lock`).
- Creating or closing a DW record immediately synchronizes both `entities` and `deferred_work` tables in SQLite cache.
- When a story declaring `relations.closes_dw` reaches `done`, all referenced DW records are automatically closed with `status: done`, `resolution: "<story_id>"`, and cache synchronization.

**Never:**
- Never create DW files outside `docs/state/dw/`.
- Never allow creating a DW record with an empty title (exits 2 `UsageError`).
- Never allow closing a nonexistent DW entity (exits 2 `entity_not_found`).
- Never allow `qdev dw` commands outside an initialized workspace (guarded by `requires_workspace`, exit 2).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Happy Path: Add DW with Lease | Active lease on `E12S4`, `qdev dw add --story E12S4 --module bridge --risk acceptable_with_mitigation --title "Zero-copy bypass" --rationale "Benchmarked margin"` | Exit 0, writes `docs/state/dw/DW-hhhh.md`, syncs SQLite cache (`entities` and `deferred_work`), prints created summary | N/A |
| Happy Path: Negligible Risk without Rationale | Active lease on `E12S4`, `qdev dw add --story E12S4 --module bridge --risk negligible --title "Cleanup logs"` | Exit 0, writes `DW-hhhh.md` with status `open`, no rationale required | N/A |
| Missing Rationale for Non-negligible Risk | Active lease on `E12S4`, `qdev dw add --story E12S4 --module bridge --risk acceptable_with_mitigation --title "Zero-copy bypass"` | Exit 1 (`LogicalFailure`), citing `regulatory.require_rationale_for`, no files created | Emits logical failure citing `regulatory.require_rationale_for` |
| Unregistered Module | `qdev dw add --module unregistered_mod --risk negligible --title "T"` | Exit 2 (`UsageError`), citing unregistered module | Emits `usage_error` |
| Unresolvable Story | `qdev dw add --story NONEXISTENT --module bridge --risk negligible --title "T"` | Exit 2 (`UsageError`), citing unresolvable story | Emits `entity_not_found` |
| Invalid Risk Level | `qdev dw add --module bridge --risk high --title "T"` | Exit 2 (`UsageError`), lists valid risk levels | Emits `usage_error` |
| Out of Lease without Override | Held lease is on `E12S5`, running `qdev dw add --story E12S4 ...` non-interactively | Exit 3 (`PolicyRefusal`), refuses out-of-lease mutation | Emits `policy_refusal` |
| Close DW as Done | `qdev dw close DW-7f3a --resolution "Resolved in PR 42"` | Exit 0, updates `status: done` and `resolution: "Resolved in PR 42"` in file and SQLite | N/A |
| Close DW as Wont Fix with Justification | `qdev dw close DW-7f3a --status wont_fix --justification "Architecture changed"` | Exit 0, updates `status: wont_fix` and `resolution: "Architecture changed"` in file and SQLite | N/A |
| Close DW as Wont Fix without Justification | `qdev dw close DW-7f3a --status wont_fix` | Exit 2 (`UsageError`), requires non-empty `--justification` | Emits `usage_error` |
| Close Nonexistent DW | `qdev dw close DW-0000` | Exit 2 (`UsageError`), DW not found | Emits `entity_not_found` |
| List DW with Filters | `qdev dw list --module bridge --risk unacceptable --status open` | Exit 0, returns matching deferred work records | N/A |
| Story Closes DW on Done | Story `E12S4` with `relations.closes_dw: ["DW-7f3a"]` transitions to `done` | Exit 0, story transitions to `done`, `DW-7f3a` updated to `status: done`, `resolution: "E12S4"` in file and SQLite | Fails transition on DW write error |
| Outside Workspace | `qdev dw list` outside workspace | Exit 2 (`UsageError`), no cache touched | Guarded by `requires_workspace` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/dw.rs` -- New module implementing `add_deferred_work`, `close_deferred_work`, `list_deferred_work_records`, input/payload types, and ISO 14971 validation.
- `crates/qdev-core/src/lib.rs` -- Export dw module public API.
- `crates/qdev-core/src/store/mod.rs` -- Add `safety_risk: Option<String>` to `EntityFilter`.
- `crates/qdev-core/src/query.rs` -- Add `safety_risk: Option<String>` to `ListQueryOptions`, forwarding to `EntityFilter`.
- `crates/qdev-core/src/store/sqlite.rs` -- In `list_entities`, left join `deferred_work dw ON e.id = dw.id` and filter on `safety_risk` and `target_module`.
- `crates/qdev-core/src/write.rs` -- In `apply_entity_update`, synchronize `deferred_work` table when updating `EntityKind::DeferredWork`.
- `crates/qdev-cli/src/cli.rs` -- Add `Dw` command, `DwCommands`, `DwAddArgs`, `DwListArgs`, `DwCloseArgs`, and `--risk` to `ListArgs`.
- `crates/qdev-cli/src/main.rs` -- Wire `Commands::Dw`, implement handlers for `add`, `list`, `close`, register in `requires_workspace`, and handle governance checks.
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `dw list` to `guarded_invocations()`.
- `crates/qdev-core/tests/dw_tests.rs` -- Core unit tests for DW creation, risk rationale gates, registry checks, close transitions, and cache sync.
- `crates/qdev-cli/tests/dw_cli_tests.rs` -- CLI integration tests for `dw add`, `dw list`, `dw close`, exit codes, JSON envelopes, and story `closes_dw` integration.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/dw.rs` -- Create module with `add_deferred_work`, `close_deferred_work`, `list_deferred_work_records`, input structs, payload shapes, and ISO 14971 risk validation.
- [x] `crates/qdev-core/src/lib.rs` -- Export dw module public functions and structs.
- [x] `crates/qdev-core/src/store/mod.rs` -- Add `safety_risk` field to `EntityFilter`.
- [x] `crates/qdev-core/src/query.rs` -- Add `safety_risk` field to `ListQueryOptions`.
- [x] `crates/qdev-core/src/store/sqlite.rs` -- Update `list_entities` with left join to `deferred_work` and filtering on safety risk and target module.
- [x] `crates/qdev-core/src/write.rs` -- Update `apply_entity_update` to synchronize `deferred_work` cache table on DW updates.
- [x] `crates/qdev-cli/src/cli.rs` -- Define `DwArgs`, `DwCommands`, `DwAddArgs`, `DwListArgs`, `DwCloseArgs`, and add `--risk` to `ListArgs`.
- [x] `crates/qdev-cli/src/main.rs` -- Wire `Commands::Dw`, implement handlers for `add`, `list`, `close`, register in `requires_workspace`, and handle governance check.
- [x] `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `dw list` to `guarded_invocations()`.
- [x] `crates/qdev-core/tests/dw_tests.rs` -- Unit tests for DW creation, risk rationale enforcement, registry validation, close transitions, and cache sync.
- [x] `crates/qdev-cli/tests/dw_cli_tests.rs` -- Integration tests for `qdev dw add`, `qdev dw list`, `qdev dw close`, exit codes, JSON output, and `closes_dw` integration.

**Acceptance Criteria:**
- Given a lease on `E12S4`, when running `qdev dw add --story E12S4 --module bridge --risk acceptable_with_mitigation --title "Zero-copy bypass" --rationale "..."`, then a `DW-hhhh` file is written under `docs/state/dw/` with status `open`.
- Given a non-negligible risk (`acceptable_with_mitigation` or `unacceptable`), when running `qdev dw add` without `--rationale`, then the command exits 1 citing `regulatory.require_rationale_for` and no file is created.
- Given `--module` not registered in `config.modules`, when running `qdev dw add`, then the command exits 2 with a usage error citing the unregistered module.
- Given recorded DW entities, when running `qdev dw list --module bridge --risk unacceptable --status open`, then matching items are returned and filtered accurately.
- Given a DW record `DW-7f3a`, when running `qdev dw close DW-7f3a --resolution "..."`, then the record is updated with `status: done` and resolution recorded in frontmatter and cache.
- Given a DW record `DW-7f3a`, when running `qdev dw close DW-7f3a --status wont_fix --justification "..."`, then the record is updated with `status: wont_fix`.
- Given a story with `relations.closes_dw` referencing `DW-7f3a`, when the story transitions to `done`, then `DW-7f3a` is automatically updated to `status: done` and `resolution: "<story_id>"`.

## Implementation Notes

- Implemented `crates/qdev-core/src/dw.rs` module providing `add_deferred_work`, `close_deferred_work`, `list_deferred_work_records`, and validation logic for ISO 14971 safety risks and regulatory rationale requirements.
- Frontmatter strictly validates against `schemas/dw.json` (`EntityKind::DeferredWork`).
- In `apply_entity_update` (`write.rs`), added immediate cache synchronization for `deferred_work` table when updating a DW entity, ensuring story lifecycle transitions (`closes_dw`) and direct closures update SQLite cache in lock-step.
- Updated `EntityFilter` in `store/mod.rs` and `ListQueryOptions` in `query.rs` with `safety_risk`, and updated SQLite `list_entities` to join `deferred_work` on `e.id = dw.id`.
- Added CLI subcommands `qdev dw add`, `qdev dw list`, and `qdev dw close` in `crates/qdev-cli/src/cli.rs` and `main.rs`, adding `--risk` to `qdev list`.
- Integrated `check_governance_gate` for `--story` mutations, ensuring held story leases are respected and overrides produce structured `DEC-` audit records.
- Added `dw list` to `guarded_invocations()` in `workspace_guard_cli_tests.rs`.
- Created comprehensive test suites: 12 unit tests in `crates/qdev-core/tests/dw_tests.rs` and 10 CLI integration tests in `crates/qdev-cli/tests/dw_cli_tests.rs`. All tests pass cleanly.

## Spec Change Log

## Review Triage Log

| ID | Source | Location | Description | Verdict | Evidence | Route |
|---|---|---|---|---|---|---|
| REV-1 | Blind Hunter | `crates/qdev-core/src/store/sqlite.rs:757` | `json_array(dw.target_module)` evaluates to `"[null]"` when target_module is NULL | `high` | Corrupted `target_modules` in `get_entity` query for entities without modules; fixed with `CASE WHEN ... IS NOT NULL`. | `patch` |
| REV-2 | Blind Hunter | `crates/qdev-core/tests/architecture_tests.rs` | `crates/qdev-core/src/dw.rs:325` missing from `IDENTITY_RULE_SITES` | `medium` | `test_the_identity_rule_has_exactly_one_set_of_sites` fails if path joins are unregistered; registered site matching decision.rs. | `patch` |
| REV-3 | Blind Hunter | `crates/qdev-core/src/write.rs:2109` | `story_detail_fields` wiped `target_modules` on DW entity updates | `high` | `target_modules` in `entities` table was set to `None`; fixed by explicitly preserving `target_module` as JSON array string and propagating store errors. | `patch` |
| REV-4 | Blind Hunter | `crates/qdev-core/src/dw.rs:373` | In `add_deferred_work_with_store`, `cache_db_path` was re-opened when `store_ref` was passed | `low` | Redundant store opening when store reference already provided; fixed by using `store_ref` directly. | `patch` |
| REV-5 | Blind Hunter | `crates/qdev-cli/src/main.rs:5140` | `handle_dw_close` ignored global `cli.justification` | `medium` | Root CLI `--justification` flag was ignored; fixed with `.or(cli.justification.as_deref())`. | `patch` |
| REV-6 | Blind Hunter | `crates/qdev-core/src/dw.rs` | Claimed lack of pagination in `list_deferred_work_records` | `false` | `list_deferred_work_records` delegates to `store.list_deferred_work()` consistent with all other entity stores in `qdev`. | `reject` |
| REV-7 | Blind Hunter | `crates/qdev-cli/src/main.rs` | Claimed missing interactive prompt for justification on `dw close` | `false` | Spec defines non-interactive CLI behavior with ExitCode 2 on missing arguments. | `reject` |
| REV-8 | Edge Case Hunter | `crates/qdev-cli/tests/dw_cli_tests.rs` | Missing test for `qdev dw add` with empty/whitespace title | `low` | Edge case in test matrix; added `test_cli_dw_add_empty_title_exit_2` confirming exit 2 and `usage_error`. | `patch` |
| REV-9 | Edge Case Hunter | `crates/qdev-cli/tests/dw_cli_tests.rs` | Missing test for `qdev dw close DW-0000` | `low` | Edge case in test matrix; added `test_cli_dw_close_nonexistent_exit_2` confirming exit 2 and `entity_not_found`. | `patch` |
| REV-10 | Edge Case Hunter | `crates/qdev-cli/tests/dw_cli_tests.rs` | Missing test for `qdev dw close` with invalid status | `low` | Edge case in test matrix; added `test_cli_dw_close_invalid_status_exit_2` confirming exit 2 and `usage_error`. | `patch` |
| REV-11 | Edge Case Hunter | `crates/qdev-cli/src/main.rs` | Claimed unhandled lease check when `--story` specified without `--force` | `false` | Already implemented and tested in `test_cli_dw_add_out_of_lease_gate_and_override`. | `reject` |
| REV-12 | Verification Gap | `crates/qdev-cli/tests/dw_cli_tests.rs` | Pre-verified gap on matrix test cases for DW CLI exit codes | `low` | Test matrix gaps resolved via new tests in `dw_cli_tests.rs`. | `patch` |
| REV-13 | Verification Gap | `crates/qdev-core/tests/dw_tests.rs` | Claimed missing verification for story transition closing DW | `false` | Already verified in `test_story_transition_to_done_closes_referenced_dw` and `test_cli_story_done_closes_dw`. | `reject` |


## Design Notes

- Deferred work frontmatter uses schema `schemas/dw.json` (`EntityKind::DeferredWork`).
- Allowed safety risks: `negligible`, `acceptable_with_mitigation`, `unacceptable`.
- Rationale requirement: if `config.regulatory.require_rationale_for` is configured and non-empty, risks matching that list require rationale; otherwise, all non-negligible risks require rationale. Omitting rationale when required fails with ExitCode 1 (`LogicalFailure`) and error code `"regulatory.require_rationale_for"`.
- Allowed close statuses: `done`, `wont_fix`. Closing as `wont_fix` strictly requires `--justification` or `--resolution`.
- Leases: when `--story` is specified, `check_governance_gate` ensures the held lease matches the origin story, or requires explicit override and justification.
- Cache synchronization: both `entities` and `deferred_work` tables are kept in lock-step upon creation, close, and story transition `closes_dw`.

## Verification

**Commands:**
- `cargo test --test dw_tests` -- expected: Core unit tests verify creation, validation, risk rationale gate, close transitions, and cache sync.
- `cargo test --test dw_cli_tests` -- expected: CLI integration tests verify add, list, close, JSON envelopes, filter flags, and transition integration.
- `cargo test --test workspace_guard_cli_tests` -- expected: Verifies `qdev dw` is refused outside an initialized workspace.
- `cargo test --test transition_tests` -- expected: Verifies story lifecycle and `closes_dw` integration remain green.
- `cargo test` -- expected: Entire workspace build and test suite pass cleanly with zero warnings.
