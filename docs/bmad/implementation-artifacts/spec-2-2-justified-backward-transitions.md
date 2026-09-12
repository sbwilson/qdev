---
title: 'Story 2.2: Justified Backward Transitions'
type: 'feature'
created: '2026-09-12'
baseline_commit: '14a34d4ea296c31e80d4122b78e331bc1a927b34'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Story status can move backward during pivots or review rejections, but without recording a justification, audit trail, and scratchpad history, teams and automated agents lose the rationale for regressions and risk invalidating held leases or evidence.

**Approach:** Require a non-empty `--justification` for all backward story transitions (enforcing exit 3 `needs_justification` when absent), automatically append a scratchpad entry of kind `transition` to the story's dedicated ledger, generate a committed `DEC-` record of type `review_rejection` (for backward moves from `review`) or `pivot` (for other backward moves), and ensure that backward transitions release no active lease and preserve existing evidence.

## Boundaries & Constraints

**Always:**
- Require a non-empty `--justification` on every backward transition; refuse missing or whitespace-only justifications with exit 3 (`needs_justification`).
- Append a JSONL entry with `kind: "transition"`, `seq = max_seq + 1` (starting at 1), current ISO8601 timestamp, and author attribution to `docs/state/scratch/<story-id>.jsonl` under the advisory write lock.
- Generate a new `DEC-{hex4+}` record in `docs/state/decisions/` allocated via `allocate_decision_id_in_with_rng`, with `decision_type` set to `review_rejection` when transitioning from `review` and `pivot` when transitioning backward from other states (`in-progress` or `ready`).
- Validate generated decision frontmatter against `decision.json` schema before writing to disk.
- Upsert the new decision entity and scratchpad entry into the SQLite cache when the cache exists.
- Report the allocated `decision_id` in `TransitionPayload` and CLI outputs.
- Preserve any active story lease and existing evidence records across backward transitions.
- Emit standard AD-13 JSON envelopes with exit codes (0: success, 1: logical failure, 2: usage, 3: policy refusal, 4: infrastructure failure, 5: conflict).

**Never:**
- Never allow a backward transition without a justification flag or with an empty/whitespace justification.
- Never release an active lease or delete/truncate evidence records on a backward transition.
- Never mutate story specification files to record transition scratchpad notes or decision rulings.
- Never bypass the advisory write lock or atomic file write when appending to scratchpad or creating decision records.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Backward without justification | `qdev transition story E12S4 in-progress` (status: `review`, no `--justification`) | Refused with `needs_justification`, no files modified | Exit 3 `needs_justification` |
| Backward with whitespace justification | `qdev transition story E12S4 in-progress --justification "   "` | Refused with `needs_justification`, no files modified | Exit 3 `needs_justification` |
| Review rejection (`review -> in-progress`) | `qdev transition story E12S4 in-progress --justification "Failed AC-3"` | Status becomes `in-progress`, scratchpad entry `kind: transition` appended, `DEC-` of type `review_rejection` created | Exit 0 |
| Pivot (`in-progress -> ready`) | `qdev transition story E12S4 ready --justification "Re-scoping appetite"` | Status becomes `ready`, scratchpad entry `kind: transition` appended, `DEC-` of type `pivot` created | Exit 0 |
| Multiple backward transitions | Repeated backward moves on same story | Scratchpad increments `seq` sequentially (`1, 2, ...`), distinct `DEC-` IDs allocated | Exit 0 |
| Scratchpad dir/file absent | First backward move on story without existing scratchpad | Creates `docs/state/scratch/<story-id>.jsonl` with `seq: 1` | Exit 0 |
| Lease and evidence preservation | Story holds active lease in `.qdev/leases` and evidence in `docs/state/evidence` | Lease file remains intact; evidence records remain intact | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/transition.rs` -- Implement backward transition side effects: scratchpad appending (`append_scratchpad_entry`), decision record creation (`create_backward_transition_decision`), decision type mapping (`review_rejection` vs `pivot`), and updating `TransitionPayload` with `decision_id`.
- `crates/qdev-core/schemas/payload-transition.json` -- Add optional `decision_id` property to transition payload schema.
- `crates/qdev-core/src/id.rs` -- Reuse `allocate_decision_id_in_with_rng` for hex-suffixed decision IDs.
- `crates/qdev-core/src/store/mod.rs` & `crates/qdev-core/src/store/sqlite.rs` -- Reuse `upsert_scratchpad_entry`, `upsert_decision`, and `upsert_cache_and_mark_dirty` for SQLite synchronization.
- `crates/qdev-cli/src/main.rs` -- In `handle_transition`, format and emit `decision_id` in text mode when present.
- `crates/qdev-core/tests/transition_tests.rs` -- Unit tests for backward transitions, scratchpad sequence increments, decision creation, decision types, and lease/evidence preservation.
- `crates/qdev-cli/tests/transition_cli_tests.rs` -- CLI integration tests for `qdev transition` backward flows in text and `--json` modes.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/payload-transition.json` -- Add optional `decision_id` field to payload schema.
- [x] `crates/qdev-core/src/transition.rs` -- Implement scratchpad append, decision entity creation with type `review_rejection` / `pivot`, update `TransitionPayload`, and integrate with backward transition flow.
- [x] `crates/qdev-cli/src/main.rs` -- Update `handle_transition` text formatting to emit the created decision ID.
- [x] `crates/qdev-core/tests/transition_tests.rs` -- Add unit tests covering backward transitions, scratchpad entry format and sequencing, decision record schema and attributes, and lease/evidence preservation.
- [x] `crates/qdev-cli/tests/transition_cli_tests.rs` -- Add CLI integration tests verifying `qdev transition story <id> <status> --justification ...` in text and `--json` modes, error exit 3 on missing/empty justification, and inspection of created files.

**Acceptance Criteria:**
- Given a story in `review`, when running `qdev transition story E12S4 in-progress` without `--justification`, then the command exits 3 with error code `needs_justification`.
- Given a story in `review`, when running `qdev transition story E12S4 in-progress --justification "Failed AC-3"`, then the transition succeeds with exit 0, a scratchpad entry with `kind: transition` and `seq: 1` is appended to `docs/state/scratch/E12S4.jsonl`, a `DEC-` record with `decision_type: review_rejection` is created in `docs/state/decisions/`, and both are synced to cache.
- Given a story in `in-progress`, when running `qdev transition story E12S4 ready --justification "Re-scoping"`, then the transition succeeds with exit 0 and a `DEC-` record with `decision_type: pivot` is created.
- Given a story with an active lease and existing evidence, when a backward transition occurs, then no lease is released and all evidence records remain untouched.

## Implementation Notes

- Added optional `decision_id` field to `crates/qdev-core/schemas/payload-transition.json` and `TransitionPayload`.
- Implemented `append_scratchpad_entry`: reads `docs/state/scratch/<story-id>.jsonl`, parses sequence numbers with overflow checks, stages content in memory, writes atomically via `write_file_atomic`, and syncs to SQLite `scratchpad_entries`.
- Implemented `create_backward_transition_decision`: pre-validates SQLite cache open, allocates `DEC-{hex4+}` ID, assigns `review_rejection` for moves from `review` and `pivot` for other backward moves, embeds state trajectory into `context` and markdown body, validates frontmatter against `decision.json` schema, canonicalizes path via `workspace_rel_path`, writes atomically via `write_file_atomic`, and syncs entity and decision records to SQLite cache.
- Gated backward transitions on non-empty trimmed `--justification`, exiting with exit code 3 `needs_justification` on absence or whitespace-only inputs.
- Updated CLI text output in `crates/qdev-cli/src/main.rs` to emit `Recorded decision: <id>` when `decision_id` is present.
- Verified active story leases in `.qdev/leases` and evidence records in `docs/state/evidence` are preserved untouched across backward transitions.

## Spec Change Log

## Review Triage Log

- `crates/qdev-core/src/transition.rs:508`: `medium` -- `append_scratchpad_entry` sequential writes were non-atomic; patched with in-memory staging and `write_file_atomic`.
- `crates/qdev-core/src/transition.rs:474`: `medium` -- Scratchpad sequence parsing used unchecked casting and swallowed errors; patched with `u32::try_from`, `checked_add(1)`, and parse errors on malformed lines.
- `crates/qdev-core/src/transition.rs:609`: `medium` -- `create_backward_transition_decision` ignored cache open failure during ID allocation; patched by pre-opening cache upfront.
- `crates/qdev-core/src/transition.rs:675`: `low` -- Decision relative path used string replacement instead of `workspace_rel_path`; patched with `workspace_rel_path`.
- `crates/qdev-core/src/transition.rs:595`: `low` -- Decision entity omitted state trajectory metadata; patched by populating `context` and markdown body with `<from> -> <to>`.
- `crates/qdev-core/tests/transition_tests.rs`: `low` -- Test suite omitted `ready -> draft`, multi-step backward jumps, and CLI `--json` missing justification refusal; patched with new tests in both suites.
- `crates/qdev-core/src/transition.rs:640`: `false` -- Story mutation occurs under write lock with preconditions checked; pre-validating cache avoids partial failures.
- `crates/qdev-core/src/transition.rs:669`: `false` -- Per-entity write locking is standard across all multi-entity operations in qdev-core.
- `crates/qdev-core/src/transition.rs:669`: `false` -- Skipping lock on absent cache parent directory is deliberate design in write.rs to prevent stray test directories.
- `crates/qdev-core/src/transition.rs:567`: `false` -- Incremental cache sweeps routinely re-check file mtimes and hashes; sync_state update is managed during sweeps.
- `crates/qdev-core/src/transition.rs:609`: `false` -- Connection opening in rusqlite WAL mode is lightweight; separate transactions for entity and decision details are standard.
- `docs/bmad/implementation-artifacts/sprint-status.yaml`: `false` -- Sprint status advances sequentially per workflow rules; spec review triage log is populated in step-04.

## Design Notes

Backward transition execution order:
1. Validate requested transition: classify as `TransitionKind::Backward`.
2. Check justification: must be non-empty after trimming, else exit 3 `needs_justification`.
3. Execute `pre_transition` hooks.
4. Mutate story frontmatter via `apply_entity_update` under the advisory write lock.
5. Record scratchpad entry:
   - Read `docs/state/scratch/<story-id>.jsonl` if present to calculate `next_seq`.
   - Append JSON entry `{"seq": next_seq, "at": "<iso>", "author": {"type": "...", "id": "..."}, "kind": "transition", "text": "<justification>"}`.
   - Upsert row into `scratchpad_entries` table in SQLite cache if cache exists.
6. Record decision entity:
   - Allocate ID using `allocate_decision_id_in_with_rng`.
   - Determine `decision_type`: `review_rejection` if `from_state == StoryState::Review`, else `pivot`.
   - Build frontmatter and markdown document with title, status `active`, `subject_id: <story_id>`, `decision_type`, `ruling: <justification>`, and author attribution.
   - Validate frontmatter against `decision.json` schema.
   - Atomically write `docs/state/decisions/<dec_id>.md`.
   - Upsert entity and decision rows into SQLite cache if cache exists.
7. Execute `post_transition` hooks.
8. Return `TransitionPayload` with story ID, status change, new version, empty closed_dw, and `decision_id: Some(dec_id)`.

## Verification

**Commands:**
- `cargo test --test transition_tests` -- expected: All transition unit tests pass including backward transitions, scratchpad sequencing, and decision records.
- `cargo test --test transition_cli_tests` -- expected: CLI integration tests verify backward transitions, error envelopes, and decision outputs.
- `cargo test` -- expected: Full test suite passes without regressions.
