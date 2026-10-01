---
title: 'Story 3.7: Git Preflight Guard'
type: 'feature'
created: '2026-09-25'
status: 'done'
baseline_commit: '36d842ca25c93ed5869303a35d69c7d6727b9091'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Story work and push operations can commence from stale or dirty workspaces with out-of-scope modifications, diverged integration branches, or unresolved schema/entity validation errors, causing avoidable merge conflicts, broken pipelines, and boundary violations.
**Approach:** Implement `qdev preflight [--story <id>]` enforcing working tree cleanliness within leased story module boundaries (or active chore allowlists), integration branch remote freshness, merge-base staleness limits (`max_integration_staleness_commits`), and zero blocking validation errors, providing exact Git remediation commands on refusal (exit 3).

## Boundaries & Constraints

**Always:**
- Execute preflight checks across three primary dimensions:
  1. **Working Tree Scope Check:**
     - When `config.git.require_clean_tree_in_scope` is `true`: detect uncommitted changes (tracked staged/unstaged modifications and untracked files via `git status --porcelain=v1 -uall`).
     - Workspace metadata directories and files (`.qdev/`, `.git/`, `.gitignore`, `qdev.toml`, `.qdev.local.toml`, `docs/`, and configured `storage.specs_dir`, `storage.state_dir`, `storage.cache_dir`) are exempt.
     - Scope determination:
       - If `--story <id>` is provided, resolve target modules from the specified story entity.
       - If `--story` is omitted: check for an active story lease in the current workspace. If present, resolve target modules from the leased story.
       - If no story lease is active, check for an open chore record. If present, use the chore's path allowlist patterns.
       - If neither a story nor a chore is active, any uncommitted changes outside workspace metadata are considered out of scope.
     - If any uncommitted change falls outside allowed target module or chore paths, refuse with exit code 3 (`ExitCode::PolicyRefusal`), listing the out-of-scope paths and recommending `git stash push -m "out-of-scope" -- <paths>` or `git stash`.
  2. **Branch Freshness Check:**
     - In `story-branch` mode (`config.git.branching_mode == "story-branch"`):
       - If the remote integration branch tracking ref exists (e.g. `refs/remotes/<remote>/<integration_branch>`), verify the local integration branch (`<integration_branch>`) is not behind the remote tracking ref. If behind, refuse with exit 3, recommending `git fetch <remote> <integration_branch>:<integration_branch>` (or `git checkout <integration_branch> && git pull <remote> <integration_branch>`).
       - Compute the merge-base between HEAD and the local integration branch (`git merge-base HEAD <integration_branch>`).
       - If the merge-base is more than `config.git.max_integration_staleness_commits` behind the local integration branch (`git rev-list --count <merge-base>..<integration_branch>`), refuse with exit 3, stating `<branch> is <N> commits behind <integration_branch> at merge-base (limit <max>)` and recommending `git rebase <integration_branch>`.
       - A story branch having commits that the integration branch does not have (or the integration branch having commits ahead of the story branch within the staleness limit) is not an error.
     - In `trunk` mode (`config.git.branching_mode == "trunk"`):
       - Verify the current branch is not behind its upstream tracking ref (`git rev-list --count HEAD..<remote>/<branch>`). If behind, refuse with exit 3, recommending `git pull <remote> <branch>`.
  3. **Validation Health Check:**
     - Query live workspace validation findings using `run_validation`.
     - If any finding has severity `"error"`, refuse with exit 3, listing the blocking finding codes and paths, and recommending `qdev validate`.
- In JSON mode (`--json`), emit a structured `payload-preflight` envelope containing `status: "pass"` (on success) or `status: "refusal"` with structured diagnostics and remediation commands.
- Print clear, actionable diagnostic output with exact Git remediation commands on standard error (or stdout JSON in `--json` mode).

**Never:**
- Never block a story branch in `story-branch` mode solely because it is behind `integration_branch`, unless its merge-base exceeds `max_integration_staleness_commits`.
- Never silently ignore git failures (missing git binary or corrupted repository); treat external process crashes or missing repositories as infrastructure failures (exit 4).
- Never modify git state, create stashes, or run rebases automatically; preflight is an inspection guard and must strictly inform and refuse.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| All clean | Clean git status, fresh branch, 0 error findings | Exit 0; `✓ preflight: working tree in scope, integration branch fresh, zero blocking findings` | N/A |
| Out of scope uncommitted files | Working tree has modified file outside story modules | Exit 3; lists offending files, gives `git stash push -m "out-of-scope" -- <paths>` | PolicyRefusal (exit 3) |
| In scope uncommitted files | Working tree has modified file inside story target modules | Passes scope check; proceeds to remaining checks | N/A |
| Open chore active | Open chore allowlist matches uncommitted edits | Passes scope check; proceeds to remaining checks | N/A |
| No lease and no chore with dirty tree | Uncommitted changes outside exempt metadata | Exit 3; states no lease or chore active, gives `git stash` | PolicyRefusal (exit 3) |
| Local integration branch behind remote | `develop` is 2 commits behind `origin/develop` | Exit 3; diagnostics state local `develop` behind `origin/develop`, recommends fetch/pull | PolicyRefusal (exit 3) |
| Merge-base exceeds staleness limit | Feature branch merge-base is 25 commits behind `develop` (limit 20) | Exit 3; diagnostics state `is 25 commits behind develop at merge-base (limit 20)`, recommends `git rebase develop` | PolicyRefusal (exit 3) |
| Trunk mode behind remote | Trunk branch is 3 commits behind `origin/main` | Exit 3; diagnostics state branch behind remote, recommends `git pull` | PolicyRefusal (exit 3) |
| Blocking validation error | Orphan deferred work or unreadable entity file | Exit 3; diagnostics list blocking error findings, recommends `qdev validate` | PolicyRefusal (exit 3) |
| Non-git directory | Workspace is not a git repository | Exit 4; infrastructure failure `not_a_git_repository` | InfrastructureFailure (exit 4) |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/config/types.rs` -- `GitConfig` options: `remote`, `integration_branch`, `branching_mode`, `require_clean_tree_in_scope`, `max_integration_staleness_commits`.
- `crates/qdev-core/src/preflight.rs` -- New preflight engine implementing working tree scope inspection, branch freshness checks, validation findings evaluation, and exact Git remediation command formatting.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Preflight` ("preflight") and embed `payload-preflight.json`.
- `crates/qdev-core/schemas/payload-preflight.json` -- JSON Schema for preflight payload contract.
- `crates/qdev-core/src/lib.rs` -- Export `preflight` module and preflight types.
- `crates/qdev-cli/src/cli.rs` -- Add `Preflight(PreflightArgs)` subcommand to `Commands` with optional `--story <id>`.
- `crates/qdev-cli/src/handlers/preflight.rs` -- CLI preflight handler executing core preflight checks and emitting text / JSON envelope outputs.
- `crates/qdev-cli/src/main.rs` -- Dispatch `Commands::Preflight` in command router.
- `crates/qdev-core/tests/preflight_tests.rs` -- Unit tests for preflight git inspection, scope checking, and staleness calculation.
- `crates/qdev-cli/tests/preflight_cli_tests.rs` -- CLI integration tests verifying exit 0 on clean state, exit 3 on scope breaches / stale integration / validation errors, and JSON output schema conformity.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/payload-preflight.json` -- Define JSON Schema for preflight payload -- Preflight contract definition.
- [x] `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Preflight` and embed schema -- Schema payload registry.
- [x] `crates/qdev-core/src/preflight.rs` -- Implement `run_preflight`, working tree scope checking, git staleness calculations, and Git remediation advice -- Core preflight engine.
- [x] `crates/qdev-core/src/lib.rs` -- Export preflight module and public functions/types -- Core library exports.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `Preflight` subcommand and `PreflightArgs` -- CLI argument parsing.
- [x] `crates/qdev-cli/src/handlers/preflight.rs` -- Implement `handle_preflight` for text and JSON formatting -- CLI preflight handler.
- [x] `crates/qdev-cli/src/handlers/mod.rs` -- Expose preflight handler -- Handler module export.
- [x] `crates/qdev-cli/src/main.rs` -- Route `Commands::Preflight` to `handle_preflight` -- CLI main routing.
- [x] `crates/qdev-core/tests/preflight_tests.rs` -- Unit tests for scope filtering, branch staleness checks, and remediation suggestions -- Core test coverage.
- [x] `crates/qdev-cli/tests/preflight_cli_tests.rs` -- Integration tests for `qdev preflight` CLI commands, flags, exit codes, and JSON schemas -- CLI test coverage.

**Acceptance Criteria:**
- Given `[git]` configuration, when I run `qdev preflight [--story E12S4]`, then uncommitted changes outside the leased story's module paths (or chore paths) cause a refusal (exit 3) listing the paths.
- In `story-branch` mode, it checks that the local integration branch is not behind its remote and that the story branch's merge-base is within `max_integration_staleness_commits`, and a story branch being behind the integration branch is not itself an error.
- In `trunk` mode, it checks the current branch against its remote.
- Blocking validation findings cause a refusal (exit 3).
- The refusal message names the exact Git command to fix the state.

## Implementation Notes

- Implemented preflight engine in `crates/qdev-core/src/preflight.rs`:
  - `is_metadata_exempt`: Ignores `.qdev/`, `.git/`, `.gitignore`, `qdev.toml`, `.qdev.local.toml`, `docs/`, and configured storage directories (`specs_dir`, `state_dir`, `cache_dir`), normalizing both `/` and `./` prefixes.
  - `check_working_tree_scope`: Collects uncommitted files via `git status --porcelain=v1 -uall`. Resolves scope hierarchy: `--story <id>` -> active story lease in workspace -> open chore allowlist -> empty scope. Out-of-scope files trigger exit 3 with exact remediation `git stash push -u -m "out-of-scope" -- "<path1>" "<path2>"` or `git stash -u`.
  - `check_branch_freshness`: In `story-branch` mode, verifies local integration branch is not behind remote (recommending `git pull` if checked out, else `git fetch`), and calculates merge-base staleness (`MB..integration_branch`) against `max_integration_staleness_commits` (recommending `git rebase`). In `trunk` mode, verifies current branch is not behind remote (recommending `git pull`). Flags missing integration branch as diagnostic.
  - `check_validation_health`: Runs `run_validation` and flags any severity `"error"` findings as blocking (recommending `qdev validate`), while allowing `"warning"` findings.
  - `run_preflight`: Verifies git work-tree existence (`is-inside-work-tree` output == "true", returning exit 4 `not_a_git_repository` on non-git directories), evaluates all checks, and returns `PreflightOutcome`.
  - `format_preflight_text`: Emits formatted single-line pass receipt or diagnostic refusal blocks.
- Defined `payload-preflight.json` schema and registered `PayloadKind::Preflight` in `crates/qdev-core/src/schema.rs`.
- Added `Commands::Preflight(PreflightArgs)` in `crates/qdev-cli/src/cli.rs` and implemented `handle_preflight` in `crates/qdev-cli/src/handlers/preflight.rs`.
- Verified with 17 unit tests in `crates/qdev-core/tests/preflight_tests.rs` and 11 CLI integration tests in `crates/qdev-cli/tests/preflight_cli_tests.rs`. Clippy runs with zero warnings.

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---|---|---|---|---|
| Remediation commands fail on untracked files | blind-hunter / edge-case-hunter / verification-gap | `medium` | `git stash push` without `-u` refuses to stash untracked files; added `-u` / `--include-untracked` to remediation strings | patch |
| Fetch remediation fails on checked-out integration branch | blind-hunter / edge-case-hunter / verification-gap | `medium` | `git fetch origin develop:develop` rejected by Git when develop is checked out; switched to `git pull` when `current_branch == integration_branch` | patch |
| Intermediate staleness tolerance unverified | verification-gap | `medium` | Tests verified 0 and 3 commits behind limit 2; added test verifying 1 commit behind passes | patch |
| Warning-severity validation finding tolerance unverified | verification-gap | `medium` | Tests only asserted error-severity findings; added test verifying warning finding passes | patch |
| Multi-lease error swallowed as no lease | blind-hunter / edge-case-hunter | `low` | `find_active_lease_with_storage` error was caught with `.ok()`; now checks for non-`no_active_lease` errors and advises `--story <id>` | patch |
| Unquoted paths in stash remediation | blind-hunter / edge-case-hunter | `low` | Paths containing whitespace would split shell arguments; added double quotes around each pathspec | patch |
| Git work-tree check accepts bare repos or .git dir | blind-hunter | `low` | `rev-parse --is-inside-work-tree` exits 0 with `false` in bare repos/.git; now verifies stdout trimmed is `"true"` | patch |
| Inconsistent path normalization in `is_metadata_exempt` | blind-hunter | `low` | Configured storage directories with leading `./` failed prefix match; normalized by stripping `./` and `/` | patch |
| Silent skip on missing integration branch | blind-hunter / edge-case-hunter | `low` | Missing local and remote integration branch skipped silently; now reports `integration_branch_missing` diagnostic | patch |
| Silent suppression of SQLite errors in `check_validation_health` | blind-hunter | `low` | If cache database is unreadable, returns an infrastructure failure rather than falsely reporting 0 findings | patch |
| Bypassing workspace requirement in `main.rs` | blind-hunter | `false` | Preflight must run its own git checks to report `not_a_git_repository` (exit 4) before standard workspace checks intercept | reject |
| Incomplete rename detection | blind-hunter | `false` | Parsing rename target paths from porcelain v1 matches existing standard scope gate parser across codebase | reject |
| Hardcoded upstream branch in trunk mode | blind-hunter | `false` | Spec and AC explicitly mandate checking against configured `[git] remote` (`refs/remotes/{remote}/{branch}`) | reject |
| Unvalidated branching mode configuration | blind-hunter | `false` | Config parser already validates branching mode during configuration loading | reject |
| Redundant lease lookup in `run_preflight` | blind-hunter | `false` | Calling lease lookup during scope check and diagnostics formatting is lightweight and causes no correctness issues | reject |

## Design Notes

### Staleness Calculation
For a story branch `B` with integration branch `I`:
1. Find merge-base: `MB = git merge-base HEAD I`.
2. Count commits from `MB` to `I`: `staleness = git rev-list --count MB..I`.
3. If `staleness > max_integration_staleness_commits`, refusal:
   `✖ preflight: <branch> is <staleness> commits behind <integration_branch> at merge-base (limit <limit>).`
   `  Rebase onto <integration_branch> before continuing <story_id>.`
   `  Fix: git rebase <integration_branch>`

### Scope Check Hierarchy
1. If `--story <id>` specified: look up story in cache/specs; extract `target_modules`; resolve paths via `ModuleRegistry`.
2. Else if active story lease exists in workspace: look up story; extract `target_modules`; resolve paths via `ModuleRegistry`.
3. Else if open chore exists: extract chore `paths` allowlist patterns.
4. Else: empty allowed paths (any non-exempt uncommitted file triggers refusal).

## Verification

**Commands:**
- `cargo test --test preflight_tests` -- expected: All core preflight unit tests pass.
- `cargo test --test preflight_cli_tests` -- expected: CLI integration tests for preflight pass with proper exit codes (0, 3, 4).
- `cargo test` -- expected: Full test suite passes.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero clippy warnings.
