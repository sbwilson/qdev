---
title: 'Story 2.3: Story Leases'
type: 'feature'
created: '2026-09-12'
baseline_commit: 'bd8752aa4f2da96bd6b9184e741ca5d85ae6dd3a'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Multiple autonomous agents and human developers operating across Git worktrees risk modifying the same story concurrently, leading to conflicting changes, invalidated evidence, and unclear ownership without explicit scope boundaries.

**Approach:** Introduce worktree-visible story leases stored in `.qdev/leases/` and mirrored in the shared Git directory, emitting an exported `QDEV_SESSION` token upon claim, refusing duplicate claims with exit code 5 naming the current holder and worktree, supporting clean release as well as `--force --justification` stale lease breaks that log a `lease_override` decision, automatically releasing on transition to `done`, and exposing stale lease diagnostics in `qdev doctor`.

## Boundaries & Constraints

**Always:**
- Write lease records as JSON to `.qdev/leases/<story-id>.json` containing `story_id`, `holder`, `author_type`, `worktree_path`, `branch`, `started_at`, and `session_token`.
- Mirror lease records into the repository's shared Git directory under `<git-common-dir>/qdev/leases/<story-id>.json` so all linked worktrees discover active leases immediately.
- Refuse duplicate claims with exit code 5 (`conflict`, code `already_leased`) naming the existing holder and worktree path.
- Require non-empty `--justification` when breaking a lease with `--force`; refuse missing or blank justification with exit code 3 (`needs_justification`).
- Log a committed `DEC-` record with `decision_type: lease_override` and ruling when a lease is broken with `--force --justification`.
- Release the story lease automatically when transitioning a story to `done`.
- Preserve active leases across backward transitions (e.g. `review -> in-progress`).
- In `qdev doctor`, report leases older than the configurable `stale_age_days` (default 3 days) under a `"leases"` section.

**Never:**
- Never allow two different holders or worktrees to claim the same story simultaneously.
- Never allow a non-holder to release another holder's lease without `--force --justification`.
- Never fail or mutate entity specifications during claim or release operations.
- Never modify committed Git files during lease operations other than the decision record created upon forced override.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Claim unleased story | `qdev claim story E12S4` on unleased story | Writes `.qdev/leases/E12S4.json`, mirrors to `.git/qdev/leases/E12S4.json`, prints confirmation and `export QDEV_SESSION=qs_E12S4_<hex4>`, exit 0 | Exit 1 if story entity does not exist (`entity_not_found`) |
| Claim already leased story | `qdev claim story E12S4` when E12S4 leased by `simon` in `/work/wt1` | Refuses claim, exit 5, outputs error message naming holder `simon` and worktree `/work/wt1` | Exit 5 `already_leased` |
| Release own lease with story id | `qdev release story E12S4` by holder | Deletes `.qdev/leases/E12S4.json` and mirrored file, prints release confirmation, exit 0 | Exit 1 if story not leased (`lease_not_found`) |
| Release own lease without story id | `qdev release` with 1 active lease in workspace | Detects active lease, removes it, exit 0 | Exit 1 if no lease held (`no_active_lease`); exit 2 if multiple leases held |
| Release other holder's lease without force | `qdev release E12S4` by non-holder | Refuses release without force, exit 3 | Exit 3 `policy_refusal` naming `--force --justification` |
| Break lease with force and justification | `qdev release E12S4 --force --justification "Agent crashed"` | Breaks lease, creates `DEC-{hex4+}` record of type `lease_override`, syncs cache, exit 0 | Exit 3 if justification missing or empty |
| Transition to done auto-releases | `qdev transition story E12S4 done` | Updates story status, closes DW, and removes active lease if present, exit 0 | Standard transition errors |
| Doctor lists stale leases | `qdev doctor` with lease older than `stale_age_days` | Emits `leases` diagnostic section reporting count of active leases and detail of stale leases | Exit 0 (doctor findings never alter exit code) |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/lease.rs` -- Implement `StoryLease`, `claim_story`, `release_story`, `find_active_lease`, `list_leases`, git common dir discovery, and atomic lease file management.
- `crates/qdev-core/src/config/types.rs` -- Add `LeasesConfig` with `stale_age_days: u32` (default 3) to `Config`.
- `crates/qdev-core/src/config/mod.rs` -- Add `"leases"` to `ALLOWED_TOP_LEVEL_SECTIONS` and implement `validate_leases_section`.
- `crates/qdev-core/src/transition.rs` -- Integrate automatic lease release on successful transition to `done`.
- `crates/qdev-core/src/doctor.rs` -- Implement `LeasesDoctorSection` checking lease ages against configured `stale_age_days` and wire into `default_doctor_sections`.
- `crates/qdev-core/schemas/payload-doctor.json` -- Update doctor payload JSON schema with the `leases` section definition.
- `crates/qdev-cli/src/cli.rs` -- Add `Claim(ClaimArgs)` and `Release(ReleaseArgs)` subcommands to CLI `Commands` enum.
- `crates/qdev-cli/src/main.rs` -- Implement CLI handlers `handle_claim` and `handle_release` supporting human text and `--json` envelope modes.
- `crates/qdev-core/tests/doctor_tests.rs` -- Update doctor section order test to include `leases` section.
- `crates/qdev-cli/tests/doctor_cli_tests.rs` -- Update doctor CLI section order test to assert `["cache", "validation", "leases"]`.
- `crates/qdev-core/tests/lease_tests.rs` -- Core unit tests for lease claims, collisions, release, force breaks with decisions, and git worktree mirroring.
- `crates/qdev-cli/tests/lease_cli_tests.rs` -- CLI integration tests for `qdev claim` and `qdev release` across single and multi-worktree environments.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/config/types.rs` & `crates/qdev-core/src/config/mod.rs` -- Add `LeasesConfig` (`stale_age_days`) to `Config`, update `ALLOWED_TOP_LEVEL_SECTIONS`, and add validation.
- [x] `crates/qdev-core/src/lease.rs` -- Implement `StoryLease`, claim/release logic, shared Git worktree mirror discovery, and forced override decision logging.
- [x] `crates/qdev-core/src/transition.rs` -- Add automatic lease release on transition to `done` while ensuring backward moves retain leases.
- [x] `crates/qdev-core/src/doctor.rs` & `crates/qdev-core/schemas/payload-doctor.json` -- Implement `LeasesDoctorSection`, update schema, and wire into `default_doctor_sections`.
- [x] `crates/qdev-cli/src/cli.rs` & `crates/qdev-cli/src/main.rs` -- Expose `qdev claim` and `qdev release` commands in text and `--json` modes.
- [x] `crates/qdev-core/tests/lease_tests.rs` & `crates/qdev-cli/tests/lease_cli_tests.rs` -- Comprehensive unit and integration test coverage.
- [x] `crates/qdev-core/tests/doctor_tests.rs` & `crates/qdev-cli/tests/doctor_cli_tests.rs` -- Update doctor order assertions to include `leases`.

**Acceptance Criteria:**
- Given no lease on story E12S4, when running `qdev claim story E12S4`, then `.qdev/leases/E12S4.json` is written and mirrored to `<git-common-dir>/qdev/leases/E12S4.json` containing holder, author_type, worktree_path, branch, started_at, and session_token, and `export QDEV_SESSION=...` is printed.
- Given an existing lease on story E12S4 held by another process or worktree, when running `qdev claim story E12S4`, then the command exits with exit code 5 (`conflict`) naming the holder and worktree.
- Given an active lease, when running `qdev release`, then the lease is ended in both local and shared git locations.
- Given an active lease on story E12S4, when running `qdev transition story E12S4 done`, then the story transitions to `done` and the lease is automatically released.
- Given an active lease on story E12S4 held by another worker, when running `qdev release E12S4 --force --justification "Orphaned lease"`, then the lease is broken and a committed `DEC-` record with `decision_type: lease_override` is created.
- Given active leases, when running `qdev doctor`, then leases older than `stale_age_days` are listed in the `leases` section.

## Implementation Notes

- Implemented `StoryLease` in `crates/qdev-core/src/lease.rs` with `story_id`, `holder`, `author_type`, `worktree_path`, `branch`, `started_at`, and `session_token` (`qs_<story_id>_<hex4>`).
- Implemented Git common directory and branch discovery supporting Git worktrees (`git rev-parse --git-common-dir` and `.git` `commondir`), mirroring leases into `<git-common-dir>/qdev/leases/<story-id>.json`.
- Implemented `claim_story` with existence checks and collision refusal returning exit code 5 (`already_leased`) naming the existing holder and worktree.
- Implemented `release_story` allowing clean release of own leases or bare `qdev release`, while requiring `--force --justification` for foreign leases, creating a committed `DEC-` record with `decision_type: lease_override` and syncing with the SQLite cache.
- Integrated automatic lease release into `transition` when `target_status` is `done`, preserving leases across backward transitions.
- Added `[leases]` section with `stale_age_days` (default 3) to configuration loader and validation.
- Added `LeasesDoctorSection` to `crates/qdev-core/src/doctor.rs` reporting total active leases and detailing stale leases exceeding `stale_age_days`, registering it in `default_doctor_sections` as the third section (`["cache", "validation", "leases"]`).
- Updated `payload-doctor.json` schema and exposed `qdev claim` and `qdev release` subcommands in `crates/qdev-cli`.
- Verified with 12 core unit tests and 10 CLI integration tests alongside existing test suites.

## Spec Change Log

## Review Triage Log

- `crates/qdev-core/src/lease.rs:1074`: `medium` -- `claim_story` and `handle_claim` accepted cached non-story entities (e.g. Epics) when claimed by bare ID; patched to verify `record.kind == EntityKind::Story`.
- `crates/qdev-core/src/lease.rs:1087`: `medium` -- Concurrent claims on same unleased story had a TOCTOU window; patched to acquire workspace advisory `write.lock`.
- `crates/qdev-core/src/lease.rs:1252`: `medium` -- `create_lease_override_decision` allocated IDs and updated SQLite without `write.lock`; patched with `write.lock` acquisition.
- `crates/qdev-core/src/lease.rs:911`: `low` -- Malformed date strings with day 0 caused unsigned underflow in `days_from_civil`; patched with day/month bounds validation.
- `crates/qdev-core/src/lease.rs:1136`: `low` -- Mirrored write to shared git dir swallowed errors; patched to check errors when git dir exists.
- `crates/qdev-core/src/schema.rs`: `low` -- `PayloadKind` and `schemas/` lacked `payload-claim.json` and `payload-release.json`; patched with registered schemas.
- `crates/qdev-core/src/lease.rs:896`: `low` -- `discover_git_branch` defaulted to "main" on detached HEAD; patched to return "HEAD".
- `crates/qdev-core/src/lease.rs:1200`: `low` -- Local lease file deletion errors during release were swallowed; patched to verify deletion.
- `crates/qdev-core/src/transition.rs:664`: `low` -- Transition to terminal states `superseded`/`abandoned` stranded active leases; patched to release on `target_state.is_terminal()`.
- `crates/qdev-core/src/doctor.rs:545`: `low` -- Unparseable `started_at` in doctor defaulted to `now` (age 0); patched to default to 0 so corrupt leases are flagged as stale.
- `crates/qdev-core/src/lease.rs:1067`: `low` -- Story IDs lacked path traversal guard (`..` or `/`); patched with traversal rejection.
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs`: `low` -- Pre-verified gap: `claim` and `release` omitted from `guarded_invocations`; patched test.
- `crates/qdev-cli/tests/lease_cli_tests.rs`: `low` -- Pre-verified gap: missing verification of custom `[leases] stale_age_days` in `qdev.toml`; patched test.
- `crates/qdev-cli/tests/lease_cli_tests.rs`: `low` -- Pre-verified gap: missing verification that `qdev get <dec_id> --json` resolves forced override decision; patched test.
- `crates/qdev-cli/tests/lease_cli_tests.rs`: `low` -- Pre-verified gap: missing test asserting rejection of non-story entity kinds; patched test.
- `crates/qdev-core/src/lease.rs:1252`: `false` -- Decision record creation does not invoke `git commit`; refutation: architecture rules strictly ban automatic git commits by CLI commands.
- `crates/qdev-core/src/lease.rs:1167`: `false` -- Release permission checks holder identity across worktrees; refutation: holder owns lease regardless of worktree per spec.
- `crates/qdev-core/src/lease.rs:1016`: `false` -- `find_workspace_leases` inspects current workspace; refutation: designed specifically for workspace-scoped release.


## Design Notes

Lease storage layout:
- Local worktree: `.qdev/leases/<story-id>.json`.
- Shared Git directory: `<shared-git-dir>/qdev/leases/<story-id>.json`.
- Detection of shared Git dir: resolves via `git rev-parse --git-common-dir` or parses `.git` file (`gitdir: ...` / `commondir`), falling back to workspace `.git` directory.
- Forced release decision creation: uses `allocate_decision_id_in_with_rng`, `canonical_file_name`, and `write_file_atomic` with `decision_type: lease_override`.

## Verification

**Commands:**
- `cargo test --test lease_tests` -- expected: All core unit tests for claim, release, collisions, and worktree mirroring pass.
- `cargo test --test lease_cli_tests` -- expected: CLI integration tests verify text output, `--json` mode, exit codes, and doctor reports.
- `cargo test --test doctor_tests --test doctor_cli_tests` -- expected: Doctor tests pass with updated section order.
- `cargo test` -- expected: Full test suite passes with zero regressions.
