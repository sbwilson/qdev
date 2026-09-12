---
title: 'An unreadable directory is a finding'
type: 'bugfix'
created: '2026-09-12'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '45acdac210dc30ece00b5e7df200dfbae68ca858'
context: [ '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md', '{project-root}/docs/bmad/implementation-artifacts/deferred-work.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `collect_markdown_files` and `collect_files_with_ext` swallow `fs::read_dir` failures, returning empty file lists for unreadable directories. During incremental sweep, this causes all entities under an unreadable directory to be treated as deleted on disk and silently purged from SQLite cache without any finding, causing `qdev validate` and `qdev doctor` to report a falsely clean workspace, and allowing the ID allocator to reallocate IDs that still exist in the unreadable directory (NEW-8, P3-3). In the write path, `write::find_file_in_dir` also swallows `read_dir` errors, falsely reporting "Entity file not found" (exit 2) instead of surfacing the directory read failure.

**Approach:** Record a `read_error` finding for any directory that cannot be read during sweep and rebuild; ensure incremental sweep retains cached entities under an unreadable directory flagged stale rather than purging them; clear directory `read_error` findings when the directory becomes readable again or is deleted; and propagate `io_error` infrastructure failures from `find_file_in_dir` when directory listing fails.

## Boundaries & Constraints

**Always:**
- A directory qdev cannot read is an error-severity `read_error` finding with path set to the relative directory path from workspace root, not an empty directory.
- Purge must not treat "not seen" as "deleted" when directory traversal failed. Entities under an unreadable directory must be retained in the cache flagged stale (`stale = 1`), preserving their rows in `entities` and child tables, and retaining their IDs in `ids_in_use`.
- An unreadable directory must cause `qdev validate` to exit 1 (`LogicalFailure`) and report the finding.
- `qdev doctor` must report the unreadable directory finding in `findings_by_code` under the `validation` section.
- When an unreadable directory is made readable again (or deleted), the next sweep must clear the directory's `read_error` finding and unstale/re-hydrate its entities (or purge them if deleted).
- `write::find_file_in_dir` must return an `infrastructure_failure` (`io_error`) when `fs::read_dir` fails on an existing directory, rather than returning `Ok(None)` ("Entity file not found").

**Never:**
- Never purge entity rows, sync state, or child rows for files that could not be scanned due to directory read failures.
- Never allocate an ID that is occupied by a file under an unreadable directory whose row is retained stale in the cache.
- Never treat a directory read failure on the write path as a missing entity file (exit 2).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Unreadable story subdirectory | `chmod 000 docs/specs/stories/sub` with existing cached `sub/E1S3.md` | Sweep records `read_error` finding for `docs/specs/stories/sub`; retains `sub/E1S3.md` flagged stale (`purged: 0`); `qdev validate` reports `read_error` and exits 1 | Exit 1 `LogicalFailure` |
| Unreadable top-level specs dir | `chmod 000 docs/specs/stories` with existing cached stories | Sweep records `read_error` finding for `docs/specs/stories`; all stories retained stale; `qdev validate` exits 1 | Exit 1 `LogicalFailure` |
| ID allocation with unreadable directory | `chmod 000 docs/specs/stories/sub` (`sub/E1S3.md` cached); `qdev create story E1` | Allocator checks `ids_in_use`, which includes stale rows; skips `E1S3` and allocates `E1S4` | Exit 0 on next available ID |
| Unreadable directory restored | Directory `chmod 755`; `qdev validate` | Next sweep visits directory, clears directory `read_error` finding, unstales entities; exits 0 | Clean exit 0 |
| Write path on unreadable directory | `chmod 000 docs/specs/stories`; `qdev update E1S1 --title New` | `find_file_in_dir` fails with infrastructure error naming unreadable directory | Exit 4 `InfrastructureFailure` (`io_error`) |
| Unreadable scratch/evidence dir | `chmod 000 .qdev/state/scratch` or `evidence` | Sweep records `read_error` for the unreadable directory; `qdev validate` exits 1 | Exit 1 `LogicalFailure` |
| Rebuild over unreadable directory | `qdev sync --rebuild` with unreadable directory | Rebuild records `read_error` finding for unreadable directory; `qdev validate` exits 1 | Exit 1 `LogicalFailure` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs:5658` (`collect_markdown_files`) and `sqlite.rs:5678` (`collect_files_with_ext`) -- record unreadable directories instead of swallowing `fs::read_dir` errors.
- `crates/qdev-core/src/store/sqlite.rs:2980` (`sweep_workspace`) -- record `read_error` findings for unreadable directories; retain files under unreadable directories flagged stale rather than purging them; clear resolved directory findings.
- `crates/qdev-core/src/store/sqlite.rs:410` (`rebuild_from_workspace`) -- record `read_error` findings for unreadable directories.
- `crates/qdev-core/src/write.rs:1405` (`find_file_in_dir`) -- return `infrastructure_failure("io_error", ...)` when `fs::read_dir` fails on an existing directory.
- `crates/qdev-core/src/validate.rs:50` (`scan_duplicate_planning_ids`) -- adapt to `collect_markdown_files` helper changes.
- `crates/qdev-core/tests/sweep_tests.rs` -- add tests for unreadable directory sweep retention, stale flagging, finding emission, recovery, and rebuild behavior.
- `crates/qdev-cli/tests/validate_cli_tests.rs` -- add end-to-end CLI tests for `qdev validate`, `qdev doctor`, `qdev create story`, and `qdev update` with unreadable directories.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- collect unreadable directories in `collect_markdown_files` and `collect_files_with_ext`; in `sweep_workspace`, record directory `read_error` findings, retain files under unreadable directories flagged stale, exclude them from purge, and clear resolved directory findings; in `rebuild_from_workspace`, record directory `read_error` findings.
- [x] `crates/qdev-core/src/write.rs` -- propagate `io_error` from `find_file_in_dir` when `fs::read_dir` fails on an existing directory.
- [x] `crates/qdev-core/src/validate.rs` -- adapt callers of `collect_markdown_files` to collect unreadable directories if needed.
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- add integration tests verifying sweep retention, stale flagging, directory `read_error` finding, and permission restoration recovery.
- [x] `crates/qdev-cli/tests/validate_cli_tests.rs` -- add end-to-end CLI tests verifying validate exit 1, doctor findings reporting, allocator collision avoidance, and write path error.

**Acceptance Criteria:**
- Given an unreadable specs or state directory, when `qdev validate` runs, it exits 1 `LogicalFailure` and reports a `read_error` finding naming the directory.
- Given an unreadable directory containing previously cached entities, when a sweep runs, the cached entities are retained flagged stale and not purged (`purged: 0`).
- Given an unreadable directory containing previously cached entities, when `qdev create story` runs, the allocator skips IDs belonging to the retained stale entities and allocates a fresh ID.
- Given an unreadable directory that is restored to readable permissions, when `qdev validate` runs again, the directory `read_error` finding is cleared and the entities are unstaled.
- Given an unreadable directory, when `qdev update` runs on an entity in that directory, it exits 4 `InfrastructureFailure` (`io_error`) naming the directory, rather than reporting entity not found.
- Given `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`, all checks pass cleanly.

## Implementation Notes

## Spec Change Log

## Review Triage Log

- `crates/qdev-core/src/write.rs:1401-1404` (BH-1, ECH-4, ECH-5): `high` — `find_file_in_dir` starts with `if !dir.exists() { return Ok(None); }`. If `dir` or an ancestor directory lacks read or execute permissions, `Path::exists` returns `false`, swallowing the permission error into `Ok(None)` and causing `qdev update` to exit 2 with `UsageError: Entity file not found` instead of exit 4 with `InfrastructureFailure: io_error`.
- `crates/qdev-core/src/store/sqlite.rs:3247-3257` (ECH-2, ECH-6): `medium` — During sweep finding resolution, `!unreadable_dirs_rel.contains(path)` checks exact string match rather than prefix containment, and `!abs.exists()` evaluates to true when ancestor directory stat fails with permission error. As a result, permission errors on an ancestor directory cause child directory `read_error` findings to be cleared as if deleted.
- `crates/qdev-core/src/store/sqlite.rs:3249-3256` (ECH-3): `low` — If an unreadable directory is removed and replaced by a non-directory file of the exact same name, the previous directory `read_error` finding is not cleared in the directory resolution loop. Rejected: users do not replace directory trees with a flat file of the same name in normal repository workflows.
- `crates/qdev-core/src/store/sqlite.rs:5803-5811, 5849-5858` (ECH-1, BH-5): `low` — If a directory has read permissions but lacks execute permissions (e.g. chmod 0444), `path.is_dir()` and `path.is_file()` return false without recursing or recording the entry. Rejected: chmod 0444 without +x is an esoteric mode not encountered in standard workflows, and handling it adds metadata inspection complexity.
- `crates/qdev-core/src/validate.rs:54-61` (BH-2): `false` — `scan_duplicate_planning_ids` ignores `unreadable_dirs`. Disproven: the architectural design of Story 1-28 explicitly preserves stale rows in `entities` so that `ids_in_use` (via `store.list_entities`) preserves IDs from unreadable directories, and `qdev validate` reports `read_error` findings via `ensure_cache` rather than aborting the ID scan.
- `crates/qdev-core/src/validate.rs:529-543` (BH-3): `false` — `off_convention_file_holding_id` discards `unreadable_dirs`. Disproven: this is a diagnostic hint helper on the missing-file fallback path and not an authority on directory findings.
- `crates/qdev-core/src/validate.rs:770-773` (BH-4): `medium` — In `filter_by_changed`, findings are filtered against `changed_paths.contains(&finding.path)`. Because `git diff --name-only` yields file paths and not directories, directory `read_error` findings are omitted in `qdev validate --changed`. Pre-existing behavior from story 1-11; routed to `defer`.
- `crates/qdev-core/src/store/sqlite.rs:435-443, 3003-3010, 5832-5838` (BH-6): `low` — Calling `collect_files_with_ext` on scratch and evidence subdirectories when `docs/state` is unreadable records redundant findings for child directories. Rejected: redundant error findings for subtrees do not impair validation correctness.
- `crates/qdev-core/src/store/sqlite.rs:459-491` (BH-7): `false` — Rebuild drops tables rather than retaining stale entities on unreadable dirs. Disproven: a full rebuild is by definition a total cache reconstruction from readable disk files; the spec explicitly requires sweep to retain stale entities while rebuild records `read_error` findings.
- `crates/qdev-core/src/store/sqlite.rs:487-497, 605-613` (BH-8): `false` — Rebuild returns `retained: 0` for files under unreadable dirs. Disproven: rebuild has no prior cached entities to retain; `retained: 0` is accurate.
- `crates/qdev-core/src/write.rs:1561-1568` (BH-9): `false` — Non-standard ID fallback loop fails if any directory is unreadable. Disproven: when resolving an ambiguous un-prefixed ID, if an entity directory cannot be read, returning `InfrastructureFailure` is the correct fail-safe behavior.
- `crates/qdev-cli/tests/validate_cli_tests.rs:59-94` (BH-10): `false` — Tests wrap assertions in `#[cfg(unix)]`. Disproven: octal file mode manipulation is POSIX-specific; tests properly gate platform-specific behavior.
- `crates/qdev-core/src/write.rs:1515-1522` (BH-11): `false` — `find_file_in_dir` does not recurse into subdirectories. Disproven: canonical entities reside in single top-level kind directories by specification and architecture.
