---
title: 'Fix-ids leaves every entity resolvable and reports what it did'
type: 'bugfix'
created: '2026-09-12'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '1b43ed8b7bbd1319eccdf7444faec2890afc5762'
context: [ '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md', '{project-root}/docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-24-identity-answers-from-the-rule.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev validate --fix-ids` has four related contract and identity defects: (1) its keeper guard checks only the directory half of the identity rule, so an early-sorting copy like `A-copy.md` is kept, the convention-compliant file is renumbered, and the id is left owned by a file no writer can resolve (P3-4); (2) early `return ExitCode::Success` when no duplicates exist causes it to exit 0 on a workspace where plain `qdev validate` exits 1; (3) when surviving error findings cause exit 1, `payload-fix-ids.json` emits neither findings nor error, leaving callers unable to see what failed; and (4) an off-convention relation source causes a mid-repair abort after the renumber has landed, dropping the promised relation redirect and leaving edges pointing to what is now a different entity.

**Approach:** Enforce the complete identity rule (directory and filename) on group keepers; pre-flight relation sources before renumbering so a duplicate with unresolvable relation sources is refused before any file is rewritten; always run validation before exiting so exit codes reflect surviving findings; and surface surviving findings in both `FixIdsPayload` and text mode.

## Boundaries & Constraints

**Always:**
- The identity rule has two halves: the file must be in the kind directory, and its filename must carry the id (`filename_carries_id`). A keeper must satisfy both halves to be kept.
- **Decision (2026-09-12, Simon): Option A for keeper selection.** The keeper is strictly the first path in lexicographical sort order (`paths.first()`). If the keeper fails either half of the identity rule (kind directory or filename carrying `old_id`), the duplicate group is refused whole and skipped, with all paths added to `skipped` and a descriptive refusal message printed to stderr.
- **Decision (2026-09-12, Simon): Option A for payload findings.** `FixIdsPayload` and `payload-fix-ids.json` specify an optional `findings` field (`#[serde(skip_serializing_if = "Vec::is_empty", default)]`), emitted only when surviving findings exist. Clean runs omit the field.
- What `--fix-ids` reports is what it did: if a renumber is reported as completed, its references have been redirected, not dropped.
- Pre-flight relation rewrites: do not rewrite an entity's id on disk if redirecting incoming edges targeting that id will fail. An unresolvable relation source causes the candidate renumber to be refused and skipped before touching disk.
- Exit code symmetry: `qdev validate --fix-ids` returns `ExitCode::LogicalFailure` (1) if and only if error-severity validation findings survive, matching plain `qdev validate`.
- When exiting 1 due to surviving validation findings, both JSON mode (`FixIdsPayload.findings`) and text mode (rendered findings) report the findings.

**Never:**
- Never renumber a file without renaming it to carry the new id.
- Never clobber an existing file or overwrite an occupied rename target.
- Never silently drop relation rewrites or leave references pointing at a keeper when they were intended for the renumbered entity.
- Do not mutate frontmatter or filesystem paths for duplicate groups that cannot be safely and completely repaired.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Keeper filename off-convention | Duplicate group with `A-copy.md` (sorts first, does not carry id) and `E1S1.md` (compliant) | Refuses group whole; both paths placed in `skipped`; id is not left owned by unresolvable file | Both paths in `skipped`, exit per validation |
| Keeper outside kind directory | `Archive/E1S1.md` and `docs/specs/stories/E1S1.md` | Keeper is outside kind directory; group refused whole, both paths in `skipped` | Stderr explains keeper is outside kind directory |
| No duplicates, but error findings exist | Clean duplicates, but workspace has unregistered module or schema violation | Emits payload/output; runs validation; exits 1 `LogicalFailure` | Exit 1, findings reported |
| Surviving error findings after fix | Duplicate fixed, but schema violation remains on another file | `renumbered` reports fix; payload includes surviving `findings`; text mode displays findings | Exit 1 `LogicalFailure` |
| Unresolvable relation source | `E1S2` duplicated; incoming edge from `misnamed.md` (declares `E1S8`, filename does not carry it) | Pre-flight detects unresolvable source `E1S8`; refuses renumber before write; path skipped | `skipped` includes duplicate; no file rewritten on disk |
| Clean repair | `E1S1.md` and `E1S1-copy.md`, valid relations | `E1S1-copy.md` renumbered and renamed; relations redirected; exits 0 | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-cli/src/main.rs:2175` (`handle_fix_ids`) -- the entrypoint for `--fix-ids`.
- `crates/qdev-cli/src/main.rs:2202` -- empty duplicate scan early-exit; must run validation instead of returning `ExitCode::Success`.
- `crates/qdev-cli/src/main.rs:2255` -- keeper validation check; currently only calls `off_convention_directory_target`; must enforce filename half (`filename_carries_id`) as well.
- `crates/qdev-cli/src/main.rs:2283` -- planning loop; must preflight relation sources targeting `old_id` using `resolve_entity_file` before committing to renumber.
- `crates/qdev-cli/src/main.rs:2534` (`FixIdsPayload`) -- payload struct; needs `findings` field.
- `crates/qdev-cli/src/main.rs:2548` -- text mode output; must render surviving validation findings when present.
- `crates/qdev-core/schemas/payload-fix-ids.json` -- JSON schema for `--fix-ids` payload; needs `error` and `findings` property definitions.
- `crates/qdev-core/src/write.rs:1269` (`filename_carries_id`) -- identity rule filename check.
- `crates/qdev-core/src/write.rs:1492` (`resolve_entity_file`) -- write-path resolver used to check relation sources.
- `crates/qdev-cli/tests/validate_cli_tests.rs` -- test suite covering `--fix-ids` CLI interactions.
- `crates/qdev-cli/tests/schema_payload_cli_tests.rs` -- schema validation round-trip tests.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-cli/src/main.rs` -- enforce identity rule (directory and filename) on group keepers; pre-flight incoming relation sources before writing; run validation on empty scan; surface findings in `FixIdsPayload` and text mode.
- [x] `crates/qdev-core/schemas/payload-fix-ids.json` -- add `findings` and `error` properties to the schema to match payload.
- [x] `crates/qdev-cli/tests/validate_cli_tests.rs` -- add tests for P3-4 keeper filename compliance, empty duplicate scan with error findings, surviving findings payload/text output, and off-convention relation source pre-flight refusal.
- [x] `crates/qdev-cli/tests/schema_payload_cli_tests.rs` -- update and verify round-trip tests against updated payload schema.

**Acceptance Criteria:**
- Given a duplicate group whose early-sorting file does not carry the id in its filename, when `--fix-ids` runs, the id is never left owned by an unresolvable file and the group is refused whole.
- Given a workspace with error-severity findings but no duplicate IDs, when `qdev validate --fix-ids` runs, it exits 1 `LogicalFailure` and reports the findings.
- Given a workspace with error-severity findings after renumbering, when `qdev validate --fix-ids --json` runs, it exits 1 `LogicalFailure` and the payload includes the findings.
- Given a duplicate entity with an incoming relation from an off-convention file, when `qdev validate --fix-ids` runs, the duplicate is skipped without renumbering or mutating files on disk, and relations are not corrupted.
- Given `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --check`, all checks pass cleanly.

## Implementation Notes

- **Keeper Identity Rule Enforcement (Option A):** Group keepers are strictly `paths.first()`. In `main.rs:handle_fix_ids`, keepers are checked against both directory convention (`off_convention_directory_target`) and filename convention (`qdev_core::filename_carries_id`). If the keeper fails either check, the duplicate group is refused whole, skipped, and all group paths added to `skipped`. Descriptive refusal messages are written to stderr.
- **Relation Pre-flight:** During planning in `handle_fix_ids`, before any renumber is confirmed or written to disk, incoming relation edges targeting `old_id` are pre-flighted using `qdev_core::resolve_entity_file`. If any incoming source cannot be resolved (e.g. an off-convention relation source file like `misnamed.md`), the candidate renumber is refused and added to `skipped` without writing or mutating files on disk.
- **Validation Before Exit & Exit Code Symmetry:** In both the empty duplicate scan branch and the post-write branch, `qdev_core::run_validation` is run before emitting output. When error findings survive, `handle_fix_ids` exits 1 `LogicalFailure`, matching plain `qdev validate`.
- **Surfacing Findings (Option A):** `FixIdsPayload` gained an optional `findings: Vec<FindingRecord>` field with `#[serde(skip_serializing_if = "Vec::is_empty", default)]`. Clean runs omit the field; surviving findings are included. In text mode, surviving findings are rendered via `render_validate_text`.
- **Schema Update:** `crates/qdev-core/schemas/payload-fix-ids.json` was updated with optional `error` and `findings` property definitions matching runtime payloads.

## Spec Change Log

- 2026-09-12 — Implemented and verified: keeper identity rule enforcement, relation pre-flight refusal, validation on empty scan, surviving findings in `FixIdsPayload` and text mode, schema updates, and comprehensive test suite additions.

## Review Triage Log

Pass 1 (2026-09-12) — three layers (blind-hunter, edge-case-hunter, verification-gap):

- **low / patch** — verification-gap: omission of `findings` in `FixIdsPayload` on clean runs was unpinned against regression. Patched with assertion in `schema_payload_cli_tests.rs`.
- **low / patch** — verification-gap: clean workspace with zero duplicate IDs was unverified in JSON mode (`--fix-ids --json`). Patched with integration test in `validate_cli_tests.rs`.
- **low / patch** — blind-hunter: `handle_fix_ids` omitted calling `qdev_core::sort_findings` before output emission in empty-scan and post-write branches. Patched by sorting findings on both paths.
- **low / patch** — blind-hunter: `store.list_relations()` was repeatedly queried inside candidate loop in `handle_fix_ids`. Patched by querying once before planning loop.
- **low / patch** — blind-hunter: line 2234 used raw `println!` for "No duplicate planning ids found." bypassing `OutputEmitter`. Patched to use `output.emit_text`.
- **false** — blind-hunter: candidate pre-flight refusal was claimed to leave keeper out of `skipped`. Refuted: `skipped` records paths offered a renumber and declined/refused; keepers are never offered a renumber, consistent across all candidate skip sites.
- **false** — blind-hunter: post-write branch was claimed to improperly suppress `findings` when `aborted` is set. Refuted: an aborted run failed mid-stream or with a specific error; emitting `error` and returning the error exit code is the documented abort contract.
- **false** — blind-hunter: pre-flight refusals were claimed to improperly omit setting `aborted` and `error`. Refuted: pre-flight refusals are plan-phase skips recorded in `skipped`, an established and intentional distinction from in-flight aborts.
- **false** — blind-hunter: relation pre-flight was claimed to improperly refuse candidates when relation sources have duplicate files. Refuted: `resolve_entity_file` is the exact resolver `apply_relation_change` uses; if a source has duplicates it cannot resolve, so refusing pre-flight prevents an in-flight abort.
- **false** — blind-hunter: empty duplicate scan branch was claimed to improperly emit raw `JsonErrorEnvelope` on DB errors. Refuted: infrastructure errors during startup emit `JsonErrorEnvelope` uniformly across all commands; `FixIdsPayload.error` exists solely to preserve already-committed renames during write loop aborts.
- **false** — blind-hunter: claimed diff left no integration test for in-flight aborts. Refuted: `test_fix_ids_reports_what_it_wrote_when_a_later_file_fails` and `test_round_trip_fix_ids_payload_with_error_against_schema` directly test in-flight abort reporting and reconciliation.
- **false** — blind-hunter: claimed spec tasks omitted making `filename_carries_id` public and payload schema descriptions still state renumber report. Refuted: spec edits during review are rejected by rule, and payload schema description accurately describes the renumber report purpose.

## Verification

**Commands:**
- `cargo test --test validate_cli_tests` -- passed (37 tests passed, 0 failed).
- `cargo test --test schema_payload_cli_tests` -- passed (20 tests passed, 0 failed).
- `cargo test --workspace` -- passed (all workspace tests green across 30+ suites).
- `cargo clippy --workspace --all-targets -- -D warnings` -- passed (0 warnings).
- `cargo fmt --check` -- passed (clean).
