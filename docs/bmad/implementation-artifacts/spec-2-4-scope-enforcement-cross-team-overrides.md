---
title: 'Story 2.4: Scope Enforcement & Cross-Team Overrides'
type: 'feature'
created: '2026-09-13'
status: 'done'
baseline_commit: 'f4c35b9e823dd64d4c85422144c5af8e5c6be9ee'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Autonomous agents and developers operating under a story lease or within a team boundary risk mutating unleased epics, sibling stories, or foreign-team assets without authorization or audit trails, enabling accidental scope creep and silent requirement drift.

**Approach:** Implement a scope and governance classification engine that verifies active leases and team ownership before executing entity mutations, enforcing policy refusal (exit 3 `needs_confirmation` naming `--override --justification`) in non-interactive environments, providing interactive TTY choice prompts, logging structured `DEC-` audit records (`cross_team_override` or `lease_override`) upon justified override, while exempting decisions, scratchpad entries, constraints, and originating deferred work.

## Boundaries & Constraints

**Always:**
- Classify entity mutations against active story leases in the workspace and declared team ownership before executing writes in `update`, `transition`, `relate`, and `unrelate`.
- Refuse unauthorized cross-team or out-of-lease mutations in non-interactive mode with exit code 3 (`policy_refusal`, code `needs_confirmation`) naming `--override --justification`.
- On interactive TTYs, present the 3-option menu from `docs/governance-and-teams.md` §3: [1] Override with justification, [2] Add user's team to owners, [3] Abort.
- Require non-empty `--justification` when `--override` is supplied; refuse missing or whitespace-only justification with exit code 3 (`needs_justification`).
- Create and cache a committed `DEC-` record of type `cross_team_override` (for cross-team edits) or `lease_override` (for scope-only edits) with subject, author, ruling, and context when an override is executed.
- Exempt decisions (`DEC-`), scratchpads of the leased story, constraints of the leased story, and deferred work originating from the leased story from requiring lease overrides.
- Preserve entity file formatting, frontmatter ordering, and atomic write-lock semantics during all mutation and decision-recording operations.

**Never:**
- Never allow an out-of-lease or cross-team mutation to proceed without an explicit override and justification or interactive confirmation.
- Never require an override when mutating the currently leased story itself (unless owned exclusively by another team).
- Never fail or corrupt existing entity files when an override is refused or aborted.
- Never bypass the workspace advisory write lock when writing override decision records or updating entity owners.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Mutate owned, leased story | `qdev update E12S4 --title "New"` while holding lease on E12S4 as owner | Updates entity without prompt or decision, exit 0 | Standard update errors |
| Out-of-lease & cross-team non-interactive | `qdev update E12 --title "New"` while holding lease on E12S4 (team ui-shell) targeting E12 (owned by core-platform) | Refuses with exit 3 `needs_confirmation`, stderr/json names `--override --justification` | Exit 3 `needs_confirmation` |
| Out-of-lease with override & justification | `qdev update E12 --title "New" --override --justification "Approved"` | Creates `DEC-` record of type `cross_team_override`, updates E12, exit 0 | Exit 3 `needs_justification` if justification empty |
| Out-of-lease only (same team) | `qdev update E12 --title "New" --override --justification "Lead signoff"` when E12 owned by user's team but lease is on E12S4 | Creates `DEC-` record of type `lease_override`, updates E12, exit 0 | Exit 3 `needs_justification` if justification empty |
| Cross-team interactive option 1 | `qdev update E12 ...` on TTY, user selects `1` and inputs `"Architect approval"` | Logs `DEC-` record of type `cross_team_override`, applies update, exit 0 | Exit 3 if input justification empty |
| Cross-team interactive option 2 | `qdev update E12 ...` on TTY, user selects `2` and inputs `"Transferring component"` | Appends user's team to E12 `owners`, logs `DEC-` record, applies update, exit 0 | Exit 3 if input justification empty |
| Cross-team interactive option 3 (abort) | `qdev update E12 ...` on TTY, user selects `3` | Prints `Aborted.`, does not modify E12 or create decision, exit 0 | N/A |
| Exempt entities | `qdev relate E12S4 depends_on DEC-01a2` or updating scratchpad on leased story | Proceeds without lease override prompt or refusal, exit 0 | Standard relate errors |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/governance.rs` -- New module implementing `ScopeClassification`, `classify_mutation`, `create_governance_override_decision`, `add_team_to_entity_owners`, and owner-matching rules.
- `crates/qdev-core/src/lib.rs` -- Register and re-export `governance` module types.
- `crates/qdev-cli/src/cli.rs` -- Add `--override` and `--justification` global CLI flags to `Cli` struct.
- `crates/qdev-cli/src/main.rs` -- Implement `check_governance_gate` and integrate before `handle_update`, `handle_transition`, `handle_relate`, and `handle_unrelate`.
- `crates/qdev-core/tests/governance_tests.rs` -- Unit tests verifying classification, team matching, decision creation, and exemption rules.
- `crates/qdev-cli/tests/scope_cli_tests.rs` -- Integration tests testing CLI behavior for out-of-lease, cross-team, interactive TTY options, non-interactive refusal, and override flag execution.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/governance.rs` -- Implement `ScopeClassification`, `classify_mutation`, `create_governance_override_decision`, and `add_team_to_entity_owners`.
- [x] `crates/qdev-core/src/lib.rs` -- Export governance module and types from `qdev_core`.
- [x] `crates/qdev-cli/src/cli.rs` -- Add global `--override` and `--justification` flags to `Cli`.
- [x] `crates/qdev-cli/src/main.rs` -- Implement `check_governance_gate` with interactive TTY menu handling and wire into `handle_update`, `handle_transition`, `handle_relate`, and `handle_unrelate`.
- [x] `crates/qdev-core/tests/governance_tests.rs` -- Add unit tests for scope classification, team resolution, decision creation, and exemptions.
- [x] `crates/qdev-cli/tests/scope_cli_tests.rs` -- Add integration tests for scope enforcement, cross-team overrides, TTY interaction, and error envelopes.

**Acceptance Criteria:**
- Given a lease on E12S4 held by `sally` (team ui-shell), when a mutating command (`update`, `transition`, `relate`, `unrelate`) targets E12 (owned by team core-platform), then it is classified as both out-of-lease and cross-team.
- Given out-of-lease or cross-team classification in non-interactive mode without `--override`, then the command exits 3 with error code `needs_confirmation` naming `--override --justification`.
- Given out-of-lease or cross-team classification with `--override --justification "<rationale>"`, then the mutation succeeds and a committed `DEC-` record of type `cross_team_override` (or `lease_override`) is created and indexed in the cache.
- Given an interactive TTY, when a cross-team edit is attempted, then the 3 options from `docs/governance-and-teams.md` §3 are offered, allowing override with justification, adding team to owners, or aborting cleanly.
- Given decisions, scratchpad entries on the leased story, or originating deferred work, when mutated, then no lease override is required.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Source | File:Line | Verdict | Route | Summary / Evidence |
|---|---|---|---|---|
| Blind Hunter | `governance.rs:352` | false | rejected | Cross-worktree lease detection via `get_lease` was already implemented for workspaces without leases. |
| Blind Hunter | `governance.rs:55` | high | patch | Direct constraint ID without frontmatter lacks owner inheritance; patched to inherit from parent story owners. |
| Blind Hunter | `governance.rs:184` | medium | patch | Case-insensitive team normalization required on `owners` frontmatter comparison; patched using `normalize_team_name`. |
| Blind Hunter | `main.rs:913` | low | patch | Adding owner for developer without team should use raw `author.id` instead of `team:` prefix; patched in interactive flow and owner insertion. |
| Edge Case Hunter | `main.rs:840` | high | patch | Staging DEC creation before mutation risks orphaned records on write failure; patched to defer DEC creation until after mutation succeeds. |
| Edge Case Hunter | `governance.rs:400` | medium | patch | Stale cache DB probe; patched to check file existence before attempting sqlite open. |
| Edge Case Hunter | `main.rs:1585` | medium | patch | Interactive transition on out-of-lease story loses prompt justification; patched to forward `effective_justification = cli.justification.clone().or(gov_just)`. |
| Edge Case Hunter | `governance.rs:540` | medium | patch | Optimistic concurrency in `add_team_to_entity_owners`; patched to accept `if_version` and return early if team is already present. |
| Verification Gap | `governance_tests.rs:460` | high | patch | Missing test for originating deferred work (`DW-`) exemption; patched with `test_originating_deferred_work_exemption`. |
| Verification Gap | `governance_tests.rs:500` | medium | patch | Missing test for cross-worktree lease detection; patched with `test_cross_worktree_lease_detection`. |
| Verification Gap | `scope_cli_tests.rs:730` | medium | patch | Missing test for 2-option menu for same-team out-of-lease mutation; patched with `test_cli_out_of_lease_same_team_interactive_option_1_override` and abort test. |
| Verification Gap | `scope_cli_tests.rs:655` | low | patch | Missing explicit DEC count and relation removal assertion in `test_cli_unrelate_scope_enforcement`; patched. |

## Design Notes

- Governance evaluation:
  1. Leases: check workspace leases (`find_workspace_leases`). If active lease exists, target must match leased story ID, child constraint ID (`{story_id}/...`), scratchpad, or decision. If not, mark `is_out_of_lease = true`. If no workspace lease, check if target is leased by another holder/worktree (`get_lease`).
  2. Owners: target entity frontmatter `owners` is inspected. If present and non-empty, current user ID (`author.id` / config / git email) and teams (`config.identity.teams` + `config.teams` mapping) must match at least one owner string (normalizing `team:` prefix). If no match, mark `is_cross_team = true`.
  3. Overrides create `DEC-` files under `docs/state/decisions/` using atomic write and update SQLite cache.

## Verification

**Commands:**
- `cargo test --test governance_tests` -- expected: All core unit tests pass for classification, owner matching, and decision generation.
- `cargo test --test scope_cli_tests` -- expected: CLI integration tests verify non-interactive exit 3, TTY interactive options, `--override --justification`, and exemptions.
- `cargo test` -- expected: Entire workspace test suite passes with zero regressions.
