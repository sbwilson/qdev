---
title: 'Story 2.5: Constraints as Entities'
type: 'feature'
created: '2026-09-13'
status: 'done'
baseline_commit: '3b70ad1c7a3583b0aebb7128dbaa75e124f2f4c2'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Autonomous agents and developers lack a formal CLI and write-path mechanism to declare and enforce first-class negative constraints (appetites, rabbit holes, and no-gos), making rejections and scope limits ad-hoc, untracked, and prone to silent removal after specification.

**Approach:** Implement `qdev constraint add` and `qdev constraint remove` commands backed by atomic write-lock mutation, sequential scoped identifier allocation (`{owner}/{kind_prefix}-{n}`), JSON Schema validation, and immediate SQLite cache synchronization, enforcing exit code 3 (`needs_justification`) when removing constraints from non-draft entities.

## Boundaries & Constraints

**Always:**
- Scoped constraint IDs follow the canonical grammar: `{owner_id}/{kind_prefix}-{n}` (e.g. `E12S4/NG-1`, `E12/RH-2`, `E12S4/APP-1`).
- Map constraint kinds to canonical prefixes: `no_go` -> `NG`, `rabbit_hole` -> `RH`, `appetite` -> `APP`.
- Allocate constraint IDs monotonically within an owner and kind: `n = max(existing_n) + 1` (starting at 1 if none exist).
- Store constraints in the owning entity's frontmatter as a sequence of `{ id, kind, text }` records (preserving relative ID e.g. `NG-1` or full ID).
- Pass all constraint mutations through the workspace advisory write lock (`write.lock`), bump the entity `version`, validate against entity JSON schema, and write atomically via tempfile replacement.
- Immediately synchronize the `constraints` table and update the entity row in the SQLite cache within the write path transaction.
- When `qdev constraint remove` targets a story or epic whose status is not `draft`, require a non-empty `--justification` flag; refuse missing or whitespace-only justification with exit code 3 (`needs_justification`).
- Pass target entities through governance checks (`check_governance_gate`), recognizing child constraints as exempt when under a held lease on the parent story.

**Never:**
- Never remove a constraint from an active, ready, in-progress, review, or done story without an explicit non-empty justification.
- Never allocate duplicate or non-sequential constraint numbers within an entity.
- Never corrupt or reorder unrelated frontmatter sections during constraint addition or removal.
- Never leave SQLite cache out of sync with frontmatter after successful constraint addition or removal.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Add constraint happy path | `qdev constraint add E12S4 --kind no_go -- "Do not touch frame buffers"` on draft story | Allocates `E12S4/NG-1`, appends to frontmatter, updates cache, exit 0 | Standard write errors |
| Add second constraint of same kind | `qdev constraint add E12S4 --kind no_go -- "Do not use unsafe"` when `NG-1` exists | Allocates `E12S4/NG-2`, appends, bumps version, exit 0 | Standard write errors |
| Add constraint to epic | `qdev constraint add E12 --kind rabbit_hole -- "len == 0 does not mean empty"` | Allocates `E12/RH-1`, writes to `docs/specs/epics/E12.md`, updates cache, exit 0 | Standard write errors |
| Add constraint invalid kind | `qdev constraint add E12S4 --kind invalid_kind -- "text"` | Refuses with exit 2 `usage_error` naming valid kinds (`no_go`, `rabbit_hole`, `appetite`) | Exit 2 `usage_error` |
| Add constraint empty text | `qdev constraint add E12S4 --kind no_go -- "   "` | Refuses with exit 2 `usage_error` indicating constraint text cannot be empty | Exit 2 `usage_error` |
| Remove constraint in draft | `qdev constraint remove E12S4/NG-1` on draft story | Removes constraint from frontmatter, drops cache row, bumps version, exit 0 | Standard errors |
| Remove constraint after draft without justification | `qdev constraint remove E12S4/NG-1` on `ready` or `in-progress` story (no `--justification`) | Refuses mutation with exit 3 `needs_justification`, files untouched | Exit 3 `needs_justification` |
| Remove constraint after draft with justification | `qdev constraint remove E12S4/NG-1 --justification "Design changed"` on `ready` story | Removes constraint, updates cache, exit 0 | Standard errors |
| Remove non-existent constraint | `qdev constraint remove E12S4/NG-99` | Refuses with exit 2 `entity_not_found` | Exit 2 `entity_not_found` |
| Optimistic concurrency mismatch | `qdev constraint add E12S4 --kind no_go --if-version 1 -- "text"` when version is 2 | Refuses with exit 5 `version_mismatch`, no files modified | Exit 5 `version_mismatch` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/id.rs` -- Extend `ConstraintKind` to support `Appetite` (`APP`), and provide allocator helper for sequential constraint IDs within an owner.
- `crates/qdev-core/src/write.rs` -- Implement `apply_constraint_add` and `apply_constraint_remove` with advisory write lock, schema validation, frontmatter patching, and SQLite cache updates.
- `crates/qdev-core/src/lib.rs` -- Re-export constraint options and write functions.
- `crates/qdev-cli/src/cli.rs` -- Define `ConstraintArgs`, `ConstraintCommands`, `ConstraintAddArgs`, and `ConstraintRemoveArgs` in clap command hierarchy.
- `crates/qdev-cli/src/main.rs` -- Implement `handle_constraint_add` and `handle_constraint_remove`, wire governance gate and `--justification` enforcement, and render JSON/text outputs.
- `crates/qdev-core/tests/constraint_tests.rs` -- Unit tests verifying constraint ID allocation, atomic writes, version bumping, and cache synchronization.
- `crates/qdev-cli/tests/constraint_cli_tests.rs` -- CLI integration tests verifying `qdev constraint add`, `qdev constraint remove`, non-draft justification gate (exit 3), `--if-version`, and JSON envelope output.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/id.rs` -- Add `ConstraintKind::Appetite` (`APP`) and monotonic constraint ID allocator helper.
- [x] `crates/qdev-core/src/write.rs` -- Implement `apply_constraint_add` and `apply_constraint_remove` with write-lock, schema validation, frontmatter patching, and cache upsert/delete.
- [x] `crates/qdev-core/src/lib.rs` -- Export constraint write functions and structs.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `Constraint` subcommand and arguments (`add`, `remove`).
- [x] `crates/qdev-cli/src/main.rs` -- Implement constraint handlers with governance integration, justification gating, and CLI output formatting.
- [x] `crates/qdev-core/tests/constraint_tests.rs` -- Add unit tests for constraint allocation, mutation, and cache sync.
- [x] `crates/qdev-cli/tests/constraint_cli_tests.rs` -- Add integration tests for CLI commands, exit codes, and envelope structure.

**Acceptance Criteria:**
- Given an epic E12 with `E12/RH-1` and a story E12S4 with `E12S4/NG-1`, when running `qdev get story E12S4 --json`, then `constraints` contains both with the epic constraint marked `inherited_from: "E12"`.
- Given an entity E12S4, when running `qdev constraint add E12S4 --kind no_go -- "text"`, then it allocates the next `NG-n` within the owner, writes it via the atomic write path, bumps version, and synchronizes the cache.
- Given a story that has left `draft` (e.g. `ready`, `in-progress`, `review`, `done`), when running `qdev constraint remove E12S4/NG-1` without `--justification`, then the command is refused with exit code 3 (`needs_justification`).
- Given a story that has left `draft`, when running `qdev constraint remove E12S4/NG-1 --justification "Approved"`, then the constraint is removed, cache row is deleted, and version is bumped.
- Given an allocated constraint `E12S4/NG-1`, when running `qdev get E12S4/NG-1`, then it resolves and returns the constraint details.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Source | File:Line | Verdict | Route | Summary / Evidence |
|---|---|---|---|---|
| Blind Hunter | `write.rs:3420` | high | patch | Frontmatter parsing and `remove_idx` resolution was done before acquiring write lock; patched to parse under advisory write lock. |
| Blind Hunter | `cli.rs:60` | low | patch | Missing `--justification` in `ConstraintRemoveArgs`; patched to expose `--justification` on subcommand so help displays it. |
| Blind Hunter | `main.rs:304` | medium | patch | Constraint ID passed to `check_governance_gate` instead of entity owner; patched to parse owner before governance check. |
| Blind Hunter | `main.rs:302` | low | patch | Missing syntax pre-validation on constraint ID before governance check; patched to check syntax early. |
| Blind Hunter | `write.rs:3388` | medium | patch | Permissive constraint relative suffix matching ignored owner prefix; patched to check owner prefix when slash is present. |
| Blind Hunter | `id.rs:768` | low | patch | `parse_constraint_seq` permitted leading zeroes; patched to reject leading zeroes. |
| Blind Hunter | `id.rs:811` | false | rejected | SQLite query in allocator already filtered by `prefix == kind.as_str()` in `parse_constraint_seq`. |
| Blind Hunter | `write.rs:1203` | false | rejected | Non-existent constraint removal correctly fails with `entity_not_found` per spec. |
| Blind Hunter | `schema.rs:151` | false | rejected | `PayloadKind` only registers specifically designated commands; `relate`/`unrelate` are likewise not registered. |
| Blind Hunter | `sprint-status.yaml:67` | false | rejected | Sprint-status rules state `review` status is set at step-05 upon completion, not during step-04. |
| Blind Hunter | `write.rs:512` | false | rejected | Shared behavior of `patch_frontmatter` across all entity mutations. |
| Blind Hunter | `write.rs:965` | low | patch | Owner ID parsing should precede lock acquisition; patched to validate before lock. |
| Blind Hunter | `main.rs:321` | false | rejected | Forwarding interactive governance justification is established pattern from Story 2.4. |
| Blind Hunter | `constraint_cli_tests.rs:200` | low | patch | Missing test coverage for `APP` constraint removal and `get` resolution; patched with test. |
| Blind Hunter | `main.rs:260` | false | rejected | Governance interactive menus live in CLI layer by architectural design. |
| Edge Case Hunter | `write.rs:3420` | high | patch | Duplicate of BH-1: concurrent constraint removal lock safety. |
| Edge Case Hunter | `main.rs:2674` | medium | patch | Duplicate of BH-3: interactive add-team prompt fails on slash in target id. |
| Edge Case Hunter | `write.rs:3389` | medium | patch | Duplicate of BH-5: foreign constraint suffix collision. |
| Verification Gap | `workspace_guard_cli_tests.rs:15` | low | patch | Workspace guard test omitted `constraint` subcommand; patched `guarded_invocations()`. |
| Verification Gap | `constraint_cli_tests.rs:268` | low | patch | Missing CLI test for optimistic concurrency mismatch (`--if-version`) on constraint remove; patched with test. |

## Design Notes

- Constraint IDs in frontmatter can be relative (`NG-1`) or absolute (`E12S4/NG-1`); both are accepted and indexed consistently in SQLite as `E12S4/NG-1`.
- Sequential allocation scans existing constraints in both the entity frontmatter and the SQLite cache, computing `max(n) + 1` for the specified kind prefix.
- Constraint removal requires checking the entity status in frontmatter: if `status != "draft"`, require `--justification` (non-empty trimmed string).
- Governance gate (`check_governance_gate`) treats `{story_id}/...` as child constraint exempt from story lease violations.

## Verification

**Commands:**
- `cargo test --test constraint_tests` -- expected: All core unit tests pass for constraint allocation, addition, removal, and cache sync.
- `cargo test --test constraint_cli_tests` -- expected: CLI integration tests verify `qdev constraint add`, `qdev constraint remove`, justification check, and JSON envelopes.
- `cargo test` -- expected: Workspace build and entire test suite pass cleanly without regressions.
