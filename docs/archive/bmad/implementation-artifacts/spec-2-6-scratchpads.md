---
title: 'Story 2.6: Scratchpads'
type: 'feature'
created: '2026-09-13'
status: 'done'
baseline_commit: '19a6b35d821333930cbaa240c4646da55b46f719'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Autonomous agents and developers need to record reasoning, decisions, tradeoffs, and transitions during development without bloating the story specification markdown files or losing thinking across git clones.

**Approach:** Implement `qdev scratch append` and `qdev scratch read` commands backed by dedicated append-only committed JSONL files under `docs/state/scratch/<story-id>.jsonl`, advisory write locking, SQLite cache synchronization into `scratchpad_entries`, story lease authorization gates with `--override` support, and token-bounded `--summary` filtering.

## Boundaries & Constraints

**Always:**
- Scratchpad entries for story `<story-id>` are stored in `docs/state/scratch/<story-id>.jsonl` (or storage-configured state directory), committed to Git.
- Scratchpad operations (`append`, `read`) must never modify or touch the story's specification markdown file.
- `qdev scratch append` writes a single JSONL line containing `seq` (1-based integer monotonically incremented within the story), `at` (ISO 8601 timestamp), `author` (`{"type": ..., "id": ...}`), `kind`, and `text`.
- Supported entry kinds are `note`, `decision`, `tradeoff`, and `transition`. When `--kind` is omitted on append, default to `note`. Reject any invalid kind with exit code 2 `usage_error`.
- Refuse empty or whitespace-only scratchpad text on append with exit code 2 `usage_error`.
- Target story must resolve via `resolve_entity_file` with `EntityKind::Story`; non-existent story IDs fail with exit code 2 `entity_not_found`.
- Appending requires an active lease on the target story in the current workspace, or `--override`.
- When appending without an active lease on the target story and `--override` is not provided: in non-interactive mode (`--non-interactive`, `QDEV_NONINTERACTIVE=1`, or stdin not a TTY), refuse mutation with exit code 3 `needs_confirmation` naming `--override --justification`. On an interactive TTY, prompt with options: [1] Override with justification, [2] Abort.
- When `--override` is supplied, require a non-empty `--justification`; fail with exit code 3 `needs_justification` if missing or whitespace-only. A justified override records a committed `DEC-` audit entry of type `lease_override`.
- Append operations must acquire the workspace advisory write lock (`write.lock`) before reading existing sequences and appending to the JSONL file.
- Append operations immediately upsert the new entry into the SQLite `scratchpad_entries` table and update `sync_state` for the scratchpad file.
- `qdev scratch read <story-id>` prints entries in `seq` ascending order.
- When `--summary` is passed to `qdev scratch read`, output is filtered to include all entries of kind `decision` and `transition` plus the last N entries (default N=5, configurable via `--last`), sorted by `seq` ascending.
- When `--budget <tokens>` is passed to `qdev scratch read`, bound output using the deterministic token estimator `(chars + 3) / 4`, prioritizing key decisions, transitions, and recent entries while dropping lowest-priority entries when exceeding budget.
- Emit versioned JSON envelope when `--json` is supplied on both `scratch append` and `scratch read`.
- Enforce workspace initialization guard (`requires_workspace`) so `qdev scratch` commands fail with exit code 2 `usage_error` outside an initialized workspace without creating stray cache files.

**Never:**
- Never modify or overwrite story specification markdown files during scratchpad operations.
- Never allow an unleased or out-of-lease scratchpad append without explicit override and justification or interactive confirmation.
- Never write non-sequential or duplicate `seq` numbers in a story's scratchpad.
- Never bypass the workspace advisory write lock during scratchpad appends.
- Never leave the SQLite `scratchpad_entries` table out of sync with the appended JSONL file.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Append with held lease | `qdev scratch append E12S4 --kind tradeoff -- "AtomicBool over Mutex"` with active lease on E12S4 | Appends seq 1 to `docs/state/scratch/E12S4.jsonl`, syncs cache row, exit 0 | Standard write errors |
| Append second entry default kind | `qdev scratch append E12S4 -- "Checked frame latch"` with active lease on E12S4 | Appends seq 2 with kind `note`, syncs cache row, exit 0 | Standard write errors |
| Append without lease non-interactive | `qdev scratch append E12S4 -- "Note"` with no active lease, non-interactive | Refuses mutation with exit 3 `needs_confirmation` naming `--override --justification` | Exit 3 `needs_confirmation` |
| Append with wrong lease non-interactive | `qdev scratch append E12S4 -- "Note"` while holding lease on E12S3, non-interactive | Refuses mutation with exit 3 `needs_confirmation` | Exit 3 `needs_confirmation` |
| Append with override & justification | `qdev scratch append E12S4 -- "Emergency note" --override --justification "Lead signoff"` without lease | Creates `DEC-` record (`lease_override`), appends entry to scratchpad, exit 0 | Standard write errors |
| Append with override missing justification | `qdev scratch append E12S4 -- "Note" --override` without lease | Refuses with exit 3 `needs_justification` | Exit 3 `needs_justification` |
| Append invalid kind | `qdev scratch append E12S4 --kind bogus -- "Text"` with lease | Refuses with exit 2 `usage_error` listing valid kinds | Exit 2 `usage_error` |
| Append empty text | `qdev scratch append E12S4 --kind note -- "   "` with lease | Refuses with exit 2 `usage_error` stating text cannot be empty | Exit 2 `usage_error` |
| Append non-existent story | `qdev scratch append E99S99 -- "Note"` with override | Refuses with exit 2 `entity_not_found` | Exit 2 `entity_not_found` |
| Read full scratchpad | `qdev scratch read E12S4` with 10 entries | Prints all 10 entries in ascending seq order, exit 0 | Exit 2 if story not found |
| Read empty scratchpad | `qdev scratch read E12S4` with no scratchpad file | Prints `(empty)` (or empty list in JSON), exit 0 | Exit 2 if story not found |
| Read summary | `qdev scratch read E12S4 --summary` with 20 entries (including 2 decisions and 1 transition) | Prints 2 decisions, 1 transition, and last 5 entries (deduplicated), exit 0 | Exit 2 if story not found |
| Read summary with token budget | `qdev scratch read E12S4 --summary --budget 100` | Truncates entries to fit within 100 estimated tokens, exit 0 | Exit 2 if story not found |
| Outside workspace guard | `qdev scratch read E12S4` outside workspace | Refuses with exit 2 `usage_error`, no stray cache created | Exit 2 `usage_error` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/scratch.rs` -- New module implementing scratchpad domain logic: `ScratchpadEntry`, `ScratchpadAuthor`, `append_scratch_entry`, `read_scratch_entries`, `summarize_scratch_entries`, and `estimate_tokens`.
- `crates/qdev-core/src/lib.rs` -- Register and export `scratch` module types and functions.
- `crates/qdev-cli/src/cli.rs` -- Define `ScratchArgs`, `ScratchCommands`, `ScratchAppendArgs`, and `ScratchReadArgs` in clap command hierarchy.
- `crates/qdev-cli/src/main.rs` -- Add `requires_workspace` matching for `Commands::Scratch`, implement `handle_scratch_append` and `handle_scratch_read` with lease check, governance override handling, output formatting, and JSON envelopes.
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `scratch` subcommand invocation to `guarded_invocations()`.
- `crates/qdev-core/tests/scratch_tests.rs` -- Core unit tests verifying append sequencing, atomic file writes, cache upsert, lease checking, summary filtering, token budgeting, and story file immutability.
- `crates/qdev-cli/tests/scratch_cli_tests.rs` -- CLI integration tests verifying `qdev scratch append`, `qdev scratch read`, `--summary`, `--budget`, `--override --justification`, exit codes, and JSON envelopes.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/scratch.rs` -- Create module with `ScratchpadEntry`, `append_scratch_entry`, `read_scratch_entries`, `summarize_scratch_entries`, and token estimator.
- [x] `crates/qdev-core/src/lib.rs` -- Export scratchpad structures and functions.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `Scratch` command and `append` / `read` subcommands.
- [x] `crates/qdev-cli/src/main.rs` -- Implement `handle_scratch_append` and `handle_scratch_read` with lease gate, override logging, and JSON envelope output.
- [x] `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` -- Add `scratch` to `guarded_invocations()`.
- [x] `crates/qdev-core/tests/scratch_tests.rs` -- Unit tests for append, read, summary filtering, budget truncation, and spec immutability.
- [x] `crates/qdev-cli/tests/scratch_cli_tests.rs` -- CLI integration tests for `qdev scratch append`, `qdev scratch read`, lease gating (exit 3), `--override`, and JSON output.

**Acceptance Criteria:**
- Given story E12S4 and an active lease on E12S4, when running `qdev scratch append E12S4 --kind tradeoff -- "AtomicBool over Mutex on frame drop latch"`, then a JSONL line with seq, timestamp, author type, author id, kind, and text is appended to `docs/state/scratch/E12S4.jsonl` under the write lock and hydrated into `scratchpad_entries`.
- Given story E12S4, when running `qdev scratch read E12S4`, then entries are printed; when running with `--summary`, the last N entries plus all entries of kind `decision` and `transition` are printed, bounded by `--budget`.
- Given scratchpad operations on story E12S4, then the story spec markdown file `docs/specs/stories/E12S4.md` is never modified or touched.
- Given story E12S4 and no active lease on E12S4, when running `qdev scratch append E12S4 -- "Note"` without `--override`, then the mutation is refused with exit code 3; when running with `--override --justification "Approved"`, the append succeeds and logs a `lease_override` decision.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Finding ID | Verdict | Evidence / Refutation |
|---|---|---|
| BH-1 | `false` | Core `append_scratch_entry` intentionally focuses on data operations; lease governance is enforced at the CLI handler boundary (`handle_scratch_append`) consistently across qdev (`handle_update`, `handle_relate`). |
| BH-2 | `false` | Duplicate of BH-1; CLI handler validates active lease or override before invoking core append. |
| BH-3 | `medium` (patch) | `create_governance_override_decision` was called before `append_scratch_entry`, risking orphaned `DEC-` files on append failure. Patched: staged override decision is executed only after append succeeds. |
| BH-4 | `medium` (patch) | `normalize_raw_args` previously scanned all tokens after `--`, eating note text starting with `--`. Patched: first operand after `--` is preserved as positional text. |
| BH-5 | `medium` (patch) | Corrupt JSONL lines were previously skipped silently. Patched: loud parse error returned immediately. |
| BH-6 | `medium` (patch) | Scratchpad file path in `sync_state` used platform-native backslashes on Windows. Patched: normalized to forward slashes. |
| BH-7 | `false` | `PayloadKind` is dedicated to `qdev schema payload <name>` for core entities; operation responses use `JsonEnvelope`. |
| BH-8 | `low` (patch) | Passing `--last` without `--summary` was silently ignored. Patched: added `requires = "summary"` on `last` in `ScratchReadArgs`. |
| BH-9 | `low` (patch) | `apply_token_budget` recalculated sum repeatedly. Patched: maintains running token sum. |
| BH-10 | `false` | `println!` for text output alongside `output.emit_envelope` for `--json` is the standard pattern across all qdev CLI handlers. |
| BH-11 | `low` (patch) | `resolve_author` was called multiple times in `handle_scratch_append`. Patched: resolved once at handler entry. |
| BH-12 | `false` | `resolve_entity_file` uniformly returns `usage_error` (exit code 2) for missing entities across all commands. |
| BH-13 | `false` | Automatic transition recording in scratchpad is out of scope for Story 2.6 core data layer. |
| BH-14 | `false` | Status update in `sprint-status.yaml` is governed by step-05 of BMAD build workflow. |
| BH-15 | `false` | Scratchpad file writes are guarded by workspace advisory write lock (`write.lock`). |
| BH-16 | `false` | `seq` calculation is serialized under advisory write lock and increments monotonically from existing lines. |
| BH-17 | `false` | Integration test `test_scratch_read_summary_and_budget` verifies `--budget` behavior. |
| ECH-1 | `medium` (patch) | Same root cause as BH-4: note text matching flag names after `--` was parsed as flags. Patched in `normalize_raw_args`. |
| ECH-2 | `low` (patch) | Same root cause as BH-8: `--last` without `--summary`. Patched with Clap `requires = "summary"`. |
| ECH-3 | `medium` (patch) | Same root cause as BH-6: path separator normalization. Patched using forward slashes in `sync_state`. |
| VG-1 | `low` (patch) | Missing test for `qdev scratch append --json` envelope. Patched: added `test_scratch_append_success_json_envelope`. |
| VG-2 | `low` (patch) | Missing test for trailing flags placed after `-- <text>`. Patched: added `test_scratch_append_with_trailing_flags_after_separator`. |
| VG-3 | `medium` (patch) | Same root cause as BH-4 / ECH-1: test gap for text containing flag-like syntax. Patched: tested in `test_scratch_append_with_trailing_flags_after_separator`. |

## Design Notes

- Scratchpad lines use JSONL format: each line is a self-contained JSON object with `seq`, `at`, `author: {"type": ..., "id": ...}`, `kind`, and `text`.
- Sequence numbers are computed under the workspace advisory write lock by scanning existing lines in `docs/state/scratch/<story-id>.jsonl` and incrementing `max(seq) + 1` (starting at 1 if the file does not exist).
- The lease check inspects workspace leases (`find_workspace_leases`): if any active lease matches `story_id`, append is authorized. Otherwise, `--override --justification` is required.
- Token budgeting uses the documented character-based heuristic `(chars + 3) / 4`.

## Verification

**Commands:**
- `cargo test --test scratch_tests` -- expected: Core unit tests pass for scratch append, read, summary, budget, and spec immutability.
- `cargo test --test scratch_cli_tests` -- expected: CLI integration tests verify `qdev scratch append`, `qdev scratch read`, `--summary`, `--budget`, `--override`, exit codes, and JSON envelopes.
- `cargo test --test workspace_guard_cli_tests` -- expected: Workspace guard test confirms `qdev scratch` is refused outside workspace.
- `cargo test` -- expected: Entire workspace build and test suite pass cleanly without regressions.
