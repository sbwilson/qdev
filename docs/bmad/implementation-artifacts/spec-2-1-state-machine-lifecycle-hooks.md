---
title: 'Story 2.1: State Machine & Lifecycle Hooks'
type: 'feature'
created: '2026-09-12'
baseline_commit: '23ca087881599ace0441221ab446732cc3a19e70'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Story status updates currently occur via general frontmatter mutations (`qdev update`) without enforcing lifecycle rules, readiness prerequisites, dependency blockers, or lifecycle hooks for downstream systems like leases and verification gates.

**Approach:** Introduce a dedicated story transition engine and CLI command (`qdev transition story <id> <target_status> [--justification <reason>]`) with registered synchronous pre- and post-transition hooks, validating legal forward edges, enforcing readiness gates on `draft → ready`, blocking transitions on `ready → in-progress` while unmet dependencies exist, requiring justification for terminal `superseded`/`abandoned` moves, and automatically resolving `closes_dw` targets on `done`.

## Boundaries & Constraints

**Always:**
- Execute status mutations exclusively through the atomic write path (`apply_entity_update`), acquiring the workspace write lock and bumping entity versions.
- Evaluate legal forward edges: `draft → ready`, `ready → in-progress`, `in-progress → review`, and `review → done`.
- Run registered `pre_transition` hooks synchronously in order; abort immediately on the first hook error and execute zero mutations.
- Enforce `draft → ready` prerequisites: refuse with exit 1 (`readiness_criteria_unmet`) if the story lacks an acceptance criterion heading section, a valid non-empty `appetite`, or at least one `target_modules` entry.
- Enforce `ready → in-progress` dependencies: refuse with exit 3 (`story_blocked`) if `blocked` is true, citing all blocking story IDs in the error message and details.
- Require `--justification` for `superseded` and `abandoned` transitions from any non-terminal state; refuse with exit 3 (`needs_justification`) if omitted.
- Disallow any transition out of terminal states (`done`, `superseded`, `abandoned`), exiting with exit 1 (`invalid_transition`).
- Automatically set all target entities listed under `relations.closes_dw` to `status: "done"` with `resolution: "<story_id>"` via the write path when a story reaches `done`.
- Emit standard AD-13 JSON envelopes with exit codes (0: success, 1: logical failure, 2: usage, 3: policy refusal, 4: infrastructure failure, 5: conflict).

**Never:**
- Never bypass the advisory write lock or mutate Markdown frontmatter outside the canonical write path.
- Never run `post_transition` hooks if `pre_transition` hooks or the write path fail.
- Never transition entities other than `story` (refuse with exit 2 `usage_error`).
- Never allow backward transitions or terminal jumps without valid justification.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Legal forward transition | `qdev transition story E12S4 review` (status: `in-progress`) | State updated to `review`, version bumped, cache synced, post-hooks executed | Exit 0 |
| `draft → ready` missing AC section | Story lacks markdown heading `## Acceptance Criteria` | Transition rejected, entity unchanged | Exit 1 `readiness_criteria_unmet` |
| `draft → ready` missing appetite | Story has no `appetite` in frontmatter | Transition rejected, entity unchanged | Exit 1 `readiness_criteria_unmet` |
| `draft → ready` missing target modules | Story has empty or absent `target_modules` | Transition rejected, entity unchanged | Exit 1 `readiness_criteria_unmet` |
| `ready → in-progress` blocked | Story has unmet `depends_on` target (not `done`) | Transition refused citing blocking story IDs | Exit 3 `story_blocked` |
| Transition to terminal without justification | `qdev transition story E12S4 abandoned` (no `--justification`) | Transition refused | Exit 3 `needs_justification` |
| Transition to terminal with justification | `qdev transition story E12S4 abandoned --justification "Superseded by bet"` | Status updated to `abandoned`, version bumped | Exit 0 |
| Transition out of terminal state | Target is `done` / `abandoned` / `superseded` | Transition rejected | Exit 1 `invalid_transition` |
| Transition to `done` closes DW | Story with `relations.closes_dw: ["DW-7f3a"]` transitions to `done` | Story updated to `done`; `DW-7f3a` updated to `status: done`, `resolution: "<story_id>"` | Exit 0; DW write errors fail transition |
| Pre-transition hook refusal | A registered `pre_transition` hook returns error | Aborts transition immediately, frontmatter untouched | Exits with hook's error code and message |
| Lock contention | Lock file held by another process for > 5000ms | Aborts with lock timeout | Exit 5 `lock_timeout` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/workflow/mod.rs` (or `crates/qdev-core/src/transition.rs`) -- New module defining `StoryState`, `TransitionContext`, `PreTransitionHook`, `PostTransitionHook`, `TransitionEngine`, and transition execution.
- `crates/qdev-core/src/write.rs` -- Write path integration: reuse `apply_entity_update`, `resolve_entity_file`, `parse_heading_line`, and `acquire_write_lock`.
- `crates/qdev-core/src/query.rs` -- Dependency / `blocked` resolution: reuse `get_live_entity_for_derivation` and relation inspect routines.
- `crates/qdev-core/src/lib.rs` -- Re-export transition types and engine from `qdev_core`.
- `crates/qdev-cli/src/cli.rs` -- Add `Transition(TransitionArgs)` subcommand to `Commands`.
- `crates/qdev-cli/src/main.rs` -- Implement `handle_transition`, wiring CLI arguments, author resolution, and emitting text/JSON envelope outputs.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/transition.rs` -- Implement story state machine, readiness validator, blocking dependency checker, pre/post hook runner, and DW closure logic.
- [x] `crates/qdev-core/src/lib.rs` -- Export `transition` module types (`StoryState`, `TransitionEngine`, `TransitionPayload`, hooks).
- [x] `crates/qdev-cli/src/cli.rs` -- Define `TransitionArgs` (`kind`, `id`, `target_status`, `--justification`).
- [x] `crates/qdev-cli/src/main.rs` -- Add `Commands::Transition` dispatch, author attribution, and JSON/text formatting.
- [x] `crates/qdev-core/tests/transition_tests.rs` -- Unit tests for transition states, readiness checks, blocked checks, DW closing, and pre/post hooks.
- [x] `crates/qdev-cli/tests/transition_cli_tests.rs` -- CLI integration tests for `qdev transition` covering success, readiness failure (exit 1), blocked refusal (exit 3), missing justification (exit 3), and JSON envelopes.

**Acceptance Criteria:**
- Given a story in `in-progress`, when `qdev transition story E12S4 review` is run, then status becomes `review` and registered pre/post hooks run.
- Given a story in `draft`, when transitioning to `ready` without AC, appetite, or target modules, then command fails with exit 1.
- Given a story in `ready` with unmet dependencies, when transitioning to `in-progress`, then command refuses with exit 3 citing blocking IDs.
- Given any non-terminal story, when transitioning to `superseded` or `abandoned` without `--justification`, then command refuses with exit 3 `needs_justification`.
- Given a story with `relations.closes_dw` targets transitioning to `done`, all referenced DW entities are updated to `status: done` with resolution set to the story ID.

## Implementation Notes

- Implemented `StoryState` in `crates/qdev-core/src/transition.rs` covering all 7 states (`draft`, `ready`, `in-progress`, `review`, `done`, `superseded`, `abandoned`), with terminal state identification and forward ranking.
- Implemented `TransitionEngine` with pluggable `PreTransitionHook` and `PostTransitionHook` registries and synchronous execution.
- Enforced legal forward transitions (`draft -> ready`, `ready -> in-progress`, `in-progress -> review`, `review -> done`), rejecting forward skips with exit 1 `invalid_transition`.
- Enforced `draft -> ready` prerequisites (Acceptance Criteria section heading, non-empty appetite in enum, and non-empty `target_modules`), rejecting with exit 1 `readiness_criteria_unmet`.
- Enforced `ready -> in-progress` dependency blocking checks against live SQLite store and story frontmatter relations, rejecting blocked transitions with exit 3 `story_blocked` and citing blocking story IDs.
- Enforced `--justification` requirement for terminal jumps (`abandoned`, `superseded`) and backward transitions, rejecting missing justification with exit 3 `needs_justification`.
- Enforced terminal state immutability, rejecting transitions out of `done`, `superseded`, or `abandoned` with exit 1 `invalid_transition`.
- Implemented automatic closure of `relations.closes_dw` targets on transition to `done`, setting `status: done` and `resolution: "<story_id>"` via `apply_entity_update`.
- Integrated `Transition(TransitionArgs)` into `crates/qdev-cli` with `--justification`, `--author-type`, and `--author-id` support, emitting AD-13 JSON envelopes and clean human-readable text output.

## Spec Change Log

## Review Triage Log

- `crates/qdev-core/src/write.rs:638`: `medium` -- `has_markdown_heading` matches lines without skipping frontmatter delimited by `---`, treating YAML comments matching `# Acceptance Criteria` as body headings.
- `crates/qdev-core/src/transition.rs:326`: `medium` -- `validate_dependencies` ignores `SqliteStore::open` failure with `if let Ok(store)`, silently allowing blocked stories to transition if cache open fails.
- `crates/qdev-core/src/transition.rs:599`: `medium` -- `close_deferred_work` runs after the story is committed to terminal `done` without pre-validating DW target existence, risking permanent lock in `done` if a DW write fails.
- `crates/qdev-cli/src/main.rs:1247`: `low` -- `handle_transition` text output uses `println!` instead of `output.emit_text`, bypassing broken-pipe error handling.
- `crates/qdev-core/src/schema.rs:149`: `medium` -- `TransitionPayload` lacks `payload-transition.json` and registration in `PayloadKind`, violating the invariant that every shipped `--json` payload has a schema.
- `crates/qdev-core/src/transition.rs:511`: `low` -- Corrupted status on disk parsed via `StoryState::from_str` returns usage error (exit 2) instead of logical failure (exit 1).
- `crates/qdev-cli/tests/transition_cli_tests.rs`: `low` -- Pre-verified gap: `--author-type` and `--author-id` overrides on `qdev transition` lack assertion verifying updated story frontmatter attribution.
- `crates/qdev-cli/tests/transition_cli_tests.rs`: `low` -- Pre-verified gap: Closed deferred work text emission unasserted in text-mode transition to `done`.
- `crates/qdev-core/tests/transition_tests.rs`: `low` -- Pre-verified gap: Dependency blocking when `cache.sqlite` is absent on disk unasserted.
- `crates/qdev-cli/src/cli.rs:230`: `low` -- `TransitionArgs` lacks `--if-version` flag for optimistic concurrency.
- `crates/qdev-core/src/transition.rs:518`: `false` -- Persisting `--justification` into `DEC-` records and scratchpad entries is explicitly scheduled in Story 2.2.
- `crates/qdev-cli/src/main.rs:878`: `false` -- `qdev update` retaining `--status` is existing behaviour; restricting update is outside Story 2.1 scope.
- `crates/qdev-core/src/transition.rs:320`: `false` -- Implicit sync before reading cache violates explicit-sync architecture across all commands.
- `crates/qdev-core/src/transition.rs:419`: `false` -- Per-entity write locking is standard across all multi-entity operations in qdev-core.
- `crates/qdev-cli/src/main.rs:1227`: `false` -- CLI hook configuration from external gates/plugins is Epic 3 scope.
- `crates/qdev-core/src/transition.rs:188`: `false` -- Post-hook signature receives context and update result; closed DW list is not required by Story 2.1.
- `crates/qdev-core/src/transition.rs:616`: `false` -- Post-transition hooks execute after write by definition; failures cannot revert atomic writes.
- `crates/qdev-core/src/transition.rs:248`: `false` -- Target module registry validation is explicitly scheduled in Story 2.13.
- `docs/bmad/implementation-artifacts/sprint-status.yaml`: `false` -- Sprint status is updated to `in-progress` in step-03 and `review` in step-05 as required by the workflow.

## Design Notes

The transition engine is structured around a pluggable pipeline:
1. Parse & validate requested state transition against the story lifecycle state machine graph.
2. Check core invariants (`draft → ready` requirements, terminal justification).
3. Check dynamic preconditions (querying blocking dependencies for `ready → in-progress`).
4. Execute `pre_transition` hooks in registered sequence. First error aborts.
5. Mutate story frontmatter through `apply_entity_update` under the advisory write lock.
6. If target state is `done`, resolve each `closes_dw` entity and set `status: done`, `resolution: "<story_id>"` via `apply_entity_update`.
7. Execute `post_transition` hooks in registered sequence.

## Verification

**Commands:**
- `cargo test --test transition_tests` -- expected: All core state machine and hook unit tests pass.
- `cargo test --test transition_cli_tests` -- expected: CLI integration tests verify exit codes, error envelopes, and DW closing.
- `cargo test` -- expected: Full test suite passes without regressions.
