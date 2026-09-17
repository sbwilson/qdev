---
title: 'Story 2.10: Chores With Path Allowlists'
type: 'feature'
created: '2026-09-17'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
story_key: '2-10-chores-with-path-allowlists'
baseline_commit: 'ccd00a2'
spec_file: 'docs/bmad/implementation-artifacts/spec-2-10-chores-with-path-allowlists.md'
context:
  - 'docs/bmad/implementation-artifacts/epic-2-context.md'
  - 'docs/bmad/implementation-artifacts/spec-2-8-deferred-work.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** A typo fix, a doc tweak, a dependency bump — changes that belong to no story. Today there is no way to land one without attaching it to a story, and nothing stops unrelated edits from riding along in whatever commit is being made.

**Approach:** Add a `qdev chore` command pair. `qdev chore start "<title>" --paths <glob>…` records a chore holding a title and a list of path globs in a gitignored record. `qdev chore commit` stages only the changes matching those globs, lists every path it excluded, and commits — then records the chore as a `DEC-` decision of type `human_ruling`, topic `chore`, carrying the paths. The hygiene gate is deferred to Story 3.9 (D-2). No story state is touched at any point.

## Boundaries & Constraints

**Always:**
- `qdev chore start` and `qdev chore commit` require an initialized workspace (`requires_workspace`; outside one, exit 2).
- `start` stores the title, the globs, and a start timestamp somewhere a later `commit` invocation can read back; the two commands are separate processes.
- `--paths` is repeatable and takes workspace-relative globs (`README.md`, `docs/**`, `crates/*/src/**`). Matching reuses `qdev_core::validate::glob_match` — no new glob dependency.
- `commit` stages and commits only changes under the allowlist, prints every changed path it did **not** include, and exits 0.
- With `--strict`, any changed path outside the allowlist is a refusal, exit 3, and nothing is staged or committed.
- The hygiene gate is **not** run (D-2, deferred to Story 3.9); output and records must not claim a lint ran.
- A chore cannot start while the current worktree holds a story lease unless `--alongside` is passed.
- The commit is recorded as `DEC-…` with `decision_type: 'human_ruling'` and `topic: 'chore'`; use `log_decision` so cache synchronization and ID allocation stay in one place.
- Git failures follow the existing pattern: propagate as `QdevError::infrastructure_failure` with a stable code (`git_unavailable`, `git_commit_failed`, …); never swallow a git error and report success.
- All chore subcommands support `--json` and emit the standard envelope.

**Never:**
- Never create, read as owned, or transition any story; never create a lease; never touch `docs/state/sprints/`.
- Never commit a path outside the allowlist while the allowlist is in force — the `DEC-` record is the one exception (D-5).
- Never pull in a glob/globset crate, and never reimplement `glob_match`.
- Never build Story 3.9's hygiene linter here.
- Never `git add -A`, `git commit -a`, or commit the whole index — the allowlist is the whole point.
- Never `git reset`, unstage, or otherwise tidy the index for paths the chore did not declare; report what stays staged, do not clean it up behind the user.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Happy path | `chore start "fix readme typo" --paths README.md "docs/**"`; `README.md` and `src/main.rs` both modified; `chore commit` | Exit 0; commit contains the `README.md` change only; `src/main.rs` listed as excluded | N/A |
| Strict refusal | Same tree; `chore commit --strict` | Exit 3 `out_of_allowlist`; nothing staged, no commit | Refusal names `src/main.rs` |
| Nothing to commit | Allowlist globs match no changed file, or the only changes are out of the allowlist | No commit, no `DEC-` record | Exit 1 `nothing_to_commit` (D-3) |
| Active story lease | `E12S4` leased by this worktree; `chore start …` | Exit 3 `lease_held`, names `E12S4` and its holder; nothing recorded | Refusal says pass `--alongside` to proceed |
| Alongside allowed | Same, with `--alongside` | Chore starts, exit 0; the story lease is untouched | N/A |
| Already-staged out-of-allowlist change | `src/main.rs` staged before `chore commit` | Commit contains only the allowlisted paths; `src/main.rs` stays staged and is reported under `NOT INCLUDED — still staged:` | Exit 0; `--strict` → exit 3 (D-4) |
| Repeat path in list | `--paths README.md --paths README.md` | Deduplicated; the path appears once in the record | N/A |
| No active chore, or the chore was already committed | `chore commit` with no open chore | Nothing committed | Exit 2 `no_active_chore` (D-4) |
| DEC record outside the allowlist | Allowlist is `README.md` only | `docs/state/decisions/DEC-xxxx.md` is still staged and committed with the change | N/A |
| Malformed/empty glob list | `chore start "t"` with no `--paths` | Exit 2 `paths_required` | Nothing recorded |
| Untracked file under the allowlist | New `docs/new.md` matching `docs/**` | Staged and committed | N/A |
| Settle a chore that never gets committed | `chore start "docs left untouched" --paths 'docs/ghost.*'`; `chore commit` → `nothing_to_commit`; `chore close --reason "superseded by a real story"`; `chore start "second attempt" --paths README.md` | Record `status: "closed"` with the reason; the second chore starts, exit 0 | N/A (D-6) |
| Abandon a chore | `chore abort --reason "not worth the time" --author-type agent --author-id bot-9` | Record `status: "abandoned"`, `closed_by: {type: agent, id: bot-9}`; no commit, no `DEC-`; slot freed | N/A (D-6) |
| Settle with nothing open | `chore close` / `chore abort` with no open record | Nothing recorded | Exit 2 `no_active_chore` (D-6) |
| List records | `chore list` with two settled records | Both printed with status and reason; `--json` returns `chores: [ … ]` plus `record_dir` | An empty listing is success, not an error (D-6) |
| Relocated cache dir | `qdev.toml` with `cache_dir = "var/qdev-cache"`; `qdev init`; `chore start …` | Record at `var/chores/<id>.json`, gitignored there exactly as `init` set it up | `git status` never lists a record (D-7) |

## Decisions

_Resolved during planning; recorded here so implementation does not re-open them._

- **D-1 (was OQ-1) — the in-flight chore lives in `.qdev/chores/<chore-id>.json`, gitignored like `.qdev/leases/`.** No new `EntityKind`, no new entity schema, and nothing outside the declared allowlist is ever committed. `qdev init` must create and gitignore `/.qdev/chores/` the same way it does `/.qdev/leases/`. Consequence accepted with this choice: the chore record itself never lands in Git; the only durable record is the `DEC-` entry, which is committed under the D-5 exception.
- **D-2 (was OQ-2) — the hygiene gate is deferred to Story 3.9.** `HygieneConfig` stays parsed-but-unenforced; `qdev chore commit` runs no lint and makes no claim that one ran. Recorded in `docs/bmad/implementation-artifacts/deferred-work.md`. Implementation must not invent a partial lint: no `forbid_patterns` matching, no `max_inline_comment_lines` check, no call to a `qdev hygiene check` that does not exist.
- **D-3 (was OQ-3) — nothing under the allowlist changed is a failure.** Exit 1, code `nothing_to_commit` (`LogicalFailure`), and no `DEC-` record is written. Covers all three shapes: the globs match no changed file, the only changes sit outside the allowlist, or an allowlisted file was deleted.
- **D-4 (was OQ-4) — pathspec commit, loud exclusion; committing closes the chore.** Stage and commit only the allowlisted set with `git commit -- <paths…>`; never `git reset` or otherwise repair the index for anything else. A change already staged before the command ran therefore stays staged, so it must print under `NOT INCLUDED — still staged:` and appear in the JSON payload as `{path, staged: true}` — impossible to skim past, because the next `git commit` by anyone picks it up. "Change" means anything `git status --porcelain` reports, so `--strict` also refuses a staged-only change (exit 3). After a successful commit the chore is closed, so re-running `qdev chore commit` gives `no_active_chore` (exit 2) instead of re-committing or a confusing exit 1.
- **D-5 (was OQ-5) — the `DEC-` record is committed as the single exception to the allowlist.** `docs/state/decisions/DEC-xxxx.md` is staged and included in the commit even when no declared glob covers it, so the audit trail lands in Git like every other `DEC-`/`DW-` record. This is the **only** permitted exception, and under `--strict` the record is not counted as an out-of-allowlist change.

- **D-6 (added at the reviewer's direction, 2026-09-18) — `qdev chore list`, `qdev chore close`, and `qdev chore abort` exist so a chore that is never going to be committed can still be settled.** Without them a mistyped glob was a dead end: `commit` refused with `nothing_to_commit` (D-3), the record stayed `open`, and every later `chore start` was refused with `chore_in_progress` — the only escape was deleting the record file by hand, which no command mentioned. `close` records the work as finished some other way (`status: "closed"`), `abort` as not happening (`status: "abandoned"`). Both keep the record on disk as history, free the slot for the next chore, and write **no** `DEC-` — only `commit` records a ruling (D-5). `list` shows every record with its status, so skipped work stays visible instead of vanishing.
- **D-7 (added at the reviewer's direction, 2026-09-18) — the record directory follows the configured `cache_dir`, not a hardcoded `.qdev`.** `init` derives `<qdev>/chores` from `storage.cache_dir` and gitignores that path, so `chore_dir` must derive the same path from the `StorageConfig` it is handed. With `cache_dir = "var/qdev-cache"` the record belongs in `var/chores/`; writing it to `.qdev/chores` left it in git-visible space, in a directory nothing created — breaking the one guarantee D-1 exists to provide. `init`'s layout rule is now a shared `qdev_dir(storage)` rather than a copy in two places.

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/chore.rs` — **new**. `start_chore`, `commit_chore`, `ChoreRecord`, glob matching against `git status` output, staged-diff assembly, and the commit path. Nothing in the repo stages or commits today; this is the first git write path.
- `crates/qdev-core/src/lib.rs` — export the chore API alongside the existing `dw`, `lease`, `decision` exports.
- `crates/qdev-core/src/validate.rs:1138` — `glob_match(pattern, rel_path)`; **reuse as-is**, do not modify. Handles `*` (no separator) and `**`; a pattern with no metacharacters names a directory or file.
- `crates/qdev-core/src/lease.rs` — `find_active_lease(workspace_root)` returns the worktree's lease or `no_active_lease`; use it for the `--alongside` gate. Do not add lease-writing code here.
- `crates/qdev-core/src/decision.rs` — `DecisionInput` (`subject_id`, `decision_type`, `topic`, `context`, `ruling`, `author`, `validate_subject`) and `log_decision` / `log_decision_with_store`; cache sync and `DEC-` allocation live inside. `human_ruling` is already in `VALID_DECISION_TYPES`; `topic` is free-form, so `chore` needs no schema change.
- `crates/qdev-core/src/schema.rs:151` — `PayloadKind`; add `Chore` with `schemas/payload-chore.json`, then bump `all()`'s array length and the `schema_str`/`as_str`/`from_str_loose` arms.
- `crates/qdev-core/schemas/payload-chore.json` — **new**; `additionalProperties: false` schema for the start/commit payloads.
- `crates/qdev-core/src/config/types.rs:142` — `HygieneConfig` (parsed, never enforced); `:101` `state_dir`; `:127` `ModuleConfig.paths` shows the existing glob-list convention.
- `crates/qdev-core/src/init.rs:109,133` — `standard_directories` / `gitignore_entries`; per D-1 add `.qdev/chores` to both, mirroring the existing `leases` entries (`standard_directories` returns `"{qdev_dir}/leases"`, `gitignore_entries` pushes `"{qdev_dir}/leases/"`).
- `crates/qdev-cli/src/cli.rs:32` — `Commands` enum; add `Chore(ChoreArgs)`, following `DwArgs`/`DwCommands` (L570) and `SprintArgs`/`SprintCommands` (L648).
- `crates/qdev-cli/src/main.rs:463` — `requires_workspace`; add the `Chore` variant (guard `true`), and add handlers next to `handle_dw` (L4943) / `handle_sprint` (L5393). Author comes from `resolve_author` (L2156).
- `crates/qdev-cli/tests/workspace_guard_cli_tests.rs:15` — add `chore start …` and `chore commit` to `guarded_invocations()`.
- `crates/qdev-cli/tests/schema_payload_cli_tests.rs:639` — `test_every_advertised_payload_name_has_a_compilable_schema` iterates `PayloadKind::all()`; add a `payload-chore.json` round-trip test beside the others.
- `crates/qdev-core/tests/chore_tests.rs`, `crates/qdev-cli/tests/chore_cli_tests.rs` — **new**; fixtures create a real `git init` repo the way `crates/qdev-cli/tests/init_cli_tests.rs:586` and `crates/qdev-core/tests/validate_tests.rs` already do.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/chore.rs` — Create the module: `ChoreRecord`, `start_chore`, `commit_chore`, glob-matched change selection via `glob_match`, staged-diff assembly, git stage/commit through `Command`, and the `DEC-` record via `log_decision`. No hygiene lint (D-2).
- [x] `crates/qdev-core/src/lib.rs` — Export the chore API.
- [x] `crates/qdev-core/src/schema.rs` + `crates/qdev-core/schemas/payload-chore.json` — Add the `Chore` payload kind, schema, `all()` length, and match arms.
- [x] `crates/qdev-core/src/init.rs` — Per D-1, add `.qdev/chores` to `standard_directories` and `/.qdev/chores/` to `gitignore_entries`, beside the existing `leases` entries.
- [x] `crates/qdev-cli/src/cli.rs` — Add `Chore(ChoreArgs)` with `Start { title, paths, alongside }` and `Commit { strict }`.
- [x] `crates/qdev-cli/src/main.rs` — Wire `Commands::Chore`, implement both handlers, register in `requires_workspace`, resolve the author, emit text and `--json` output.
- [x] `crates/qdev-cli/tests/workspace_guard_cli_tests.rs` — Add both chore invocations to `guarded_invocations()`.
- [x] `crates/qdev-core/tests/chore_tests.rs` — Unit tests: glob matching (`**`, plain dir, repeated globs), allowlist selection over `git status`, decision-record contents, and the nothing-under-allowlist case (D-3). No hygiene test — D-2.
- [x] `crates/qdev-cli/tests/chore_cli_tests.rs` — Integration tests against a real `git init` fixture: happy path with one excluded path, `--strict` exit 3, lease refusal without `--alongside`, `--alongside` success, commit contents verified with `git show --stat`, the `DEC-` record committed outside the allowlist (D-5), a pre-staged out-of-allowlist change reported but left staged (D-4), and `no_active_chore` after a successful commit.
- [x] `crates/qdev-cli/tests/schema_payload_cli_tests.rs` — Add the `payload-chore.json` round-trip and confirm the invariant test covers `chore`.
- [x] `crates/qdev-core/src/chore.rs` — (post-review) Take the configured `StorageConfig` in `chore_dir`/`list_chore_records`/`find_open_chore`, and add `close_chore`/`abort_chore` with `FinishChoreInput` (D-6, D-7).
- [x] `crates/qdev-core/src/init.rs` — (post-review) Share one `qdev_dir(storage)` rule between `InitLayout` and the record directory (D-7).
- [x] `crates/qdev-cli/src/cli.rs` + `crates/qdev-cli/src/main.rs` — (post-review) Add `List`, `Close`, `Abort` subcommands and their handlers, including the record directory in `list` output (D-6).
- [x] `crates/qdev-core/schemas/payload-chore.json` — (post-review) Add the list shape and the `closed`/`abandoned` statuses; the record shape is defined once in `definitions` and reused (D-6).
- [x] `crates/qdev-core/tests/chore_tests.rs` + `crates/qdev-cli/tests/chore_cli_tests.rs` — (post-review) Cover the closed loop (wedge → `close` → start again), `abort` attribution, settling with nothing open, and a relocated cache dir.

**Acceptance Criteria:**
- Given `chore start "fix readme typo" --paths README.md "docs/**"` with `README.md` and `src/main.rs` modified, when running `qdev chore commit`, then only the `README.md` change is committed, `src/main.rs` is listed as excluded, and the command exits 0.
- Given the same tree, when running `qdev chore commit --strict`, then the commit is refused with exit 3 and nothing is staged.
- Given nothing under the allowlist changed, when running `qdev chore commit`, then the command exits 1 with `nothing_to_commit` and writes no `DEC-` record (D-3).
- Given a successful `chore commit`, when running `qdev chore commit` again, then it exits 2 with `no_active_chore` — committing closes the chore (D-4).
- Given a successful `chore commit`, then a `DEC-` record exists with `decision_type: 'human_ruling'`, `topic: 'chore'`, and the declared paths, that record is in the commit even when outside the allowlist (D-5), and no story file or story state changed.
- Given this worktree holds a lease on `E12S4`, when running `qdev chore start` without `--alongside`, then the start is refused naming `E12S4`; with `--alongside` it succeeds and the lease is unchanged.
- Given a chore whose allowlist matches nothing, when running `qdev chore close` (or `abort`), then the record is settled (`closed` / `abandoned`), a later `qdev chore start` succeeds, and `qdev chore list` still shows the settled record (D-6).
- Given no open chore, when running `qdev chore close` or `qdev chore abort`, then the command exits 2 with `no_active_chore` and records nothing (D-6).
- Given a workspace whose `cache_dir` is relocated, when starting a chore, then its record is written under the configured cache directory and never appears in `git status` (D-7).

## Implementation Notes

Done as specified; two places where the implementation had to decide something the spec left open, and three gotchas worth recording.

**Decisions made while building:**
- `--strict` is evaluated **before** the `nothing_to_commit` check (row 21). With only out-of-allowlist changes, `nothing_to_commit` would otherwise win and the strict refusal the user asked for would never appear. The matrix's "Nothing to commit" row does not name `--strict`, so it stays as written — the precedence is documented here rather than in the frozen block.
- Ids that collide on disk take a `-2`, `-3` suffix (the record for the earlier chore with the same title stays as history). The spec asked for this in Task 2; it is not the same as the one-open-chore rule, which only looks at `status: "open"`.

**Gotchas:**
- `git commit -- <paths>` needs `--` before the pathspec, and the commit **must** be made with the pathspec rather than by staging alone: `git add` + plain `git commit` would absorb anything already in the index, which is exactly what D-4 forbids.
- Text-mode errors go to **stderr** and JSON-mode error envelopes to **stdout** (`OutputEmitter::emit_error`, AD-13). A test that only reads stdout misses every error assertion — `run()` in `chore_cli_tests.rs` returns the two combined.
- Paths under `.qdev/` are filtered out of the change set. Without that, the chore's own record file (and cache/lease churn) counts as an out-of-allowlist change, and `--strict` would refuse any commit in a workspace that somehow lost its `.gitignore`.
- Record ids are not `Identifier` values, so `log_decision` is called with `validate_subject: false` — a chore record is not an entity, and `qdev get <chore-id>` cannot resolve one (D-1).

**Added at the reviewer's direction after the first review (2026-09-18):**
- `list`/`close`/`abort` exist because D-3 plus the one-open-chore rule made a wrong glob a dead end. `close` and `abort` keep the record as history and write no `DEC-` — a settled chore is local bookkeeping, not a ruling anyone else has to be told about (D-6).
- `chore_dir`, `list_chore_records`, `find_open_chore`, and both input structs now carry/derive from `StorageConfig`, so the record directory is wherever the configured cache says it is. `init` creates and gitignores that same path, so `git status` never sees a record — including with `cache_dir = "var/qdev-cache"`, which is now tested (D-7).
- `payload-chore.json` puts the record and commit shapes in `definitions` and reuses them, so the `list` shape (`{ chores: [ … ], record_dir }`) does not restate the record. `status` gained `closed` and `abandoned`; `closed_by` reuses the same author shape.

**Added in the review cycle (all `patch` rows in the triage log):**
- The `DEC-` a failed attempt wrote is now recorded in the chore record and reused, instead of a fresh ruling per attempt — and its path is excluded from the change set, so it can neither be counted out of the allowlist nor wedge `--strict`.
- `--author-type`/`--author-id` at commit time reach the decision record; previously the start-time author was always used.
- A rename commits as a rename: both endpoints go into the commit pathspec (only the destination into `git add`, since the source is already staged and gone), and the `peekable()` that was never `peek()`ed is gone.
- Titles longer than 60 characters are capped as the doc always claimed, and non-ASCII titles keep their letters instead of sharing `chore-untitled`.
- `--paths` takes one value per flag, so `chore start --paths README.md "title"` works; excluded paths are labelled `still staged` or `not staged` truthfully; every lease is named in a `lease_held` refusal; and `start_chore` holds `acquire_workspace_write_lock` across its check-then-write.

## Spec Change Log

| Date | Source | What was flagged | What changed |
|------|--------|------------------|--------------|
| 2026-09-18 | reviewer (walkthrough) | A wrong allowlist was a dead end: `commit` → `nothing_to_commit` (D-3), the record stayed `open`, every later `start` → `chore_in_progress`, and no command could list or settle it | Added D-6 — `chore list`, `chore close`, `chore abort`; implemented with tests, and the matrix gained rows for each |
| 2026-09-18 | reviewer (walkthrough) | `chore_dir` hardcoded `.qdev/chores` while `init` derives `<qdev>/chores` from `cache_dir`; with a relocated cache the record was git-visible, breaking the D-1 guarantee | Added D-7 — derive the record directory from the configured `StorageConfig`, sharing `init`'s `qdev_dir` rule; verified against `cache_dir = "var/qdev-cache"` |

## Review Triage Log

_Review 1 (three layers, `baseline_commit: ccd00a2`). One row per finding; verdicts rendered before grouping, then grouped by shared root cause and routed. `patch` entries are fixed in this cycle; `defer` entries went to `deferred-work.md`; `false`/rejected rows are recorded with what disproves them._

| # | Layer | Finding | Verdict | Evidence | Route |
|---|-------|---------|---------|----------|-------|
| 1 | blind-hunter, edge-case-hunter | `commit_chore` builds `DecisionInput` from `record.author`, so `qdev chore commit --author-type/--author-id` is silently ignored | medium | Ran `chore start` as `agent/bot-9` then `chore commit --author-id simon`: the record's `created_by` stayed `{id: bot-9, type: agent}`. A commit-time override that does nothing, and a ruling credited to the wrong party, are both user-visible | patch — fixed: `input.author` now feeds the record |
| 2 | blind-hunter, edge-case-hunter | The `DEC-` is written before the commit, so a failed commit leaves a ruling for a commit that never happened, every retry writes another, and the orphan wedges `--strict` forever | medium | Reproduced with a failing `pre-commit` hook: `git_commit_failed`, `DEC-90b3.md` left on disk, record still `open` with `decision_id: null`; retry wrote `DEC-5636.md`; `chore commit --strict` then refused with `out_of_allowlist: … DEC-90b3.md` on every later attempt | patch — fixed: the record stores the ruling it wrote and reuses it; its path is never counted as an out-of-allowlist change |
| 3 | blind-hunter | `git commit -- <path>` commits the worktree copy of an allowlisted path, so a partially staged file is committed whole | low | Confirmed: staged `v2-staged`, then edited to `v3-worktree`; the commit contains `v3-worktree`. But committing that path's current content is what "commit this path" means, no data is lost, and the alternative (stage and commit from the index) changes behaviour D-4 relies on | rejected — unlikely to be met in everyday use, and the fix is more than a direct correction |
| 4 | blind-hunter | `--paths` with `num_args = 1..` swallows the positional title, so only `start "title" --paths X` works | low | `chore start --paths README.md "t3"` fails with `the following required arguments were not provided: <TITLE>` | patch — fixed: `num_args = 1` with `action = Append`; test added |
| 5 | blind-hunter, edge-case-hunter, verification-gap | `derive_chore_id`'s doc promises a length cap that does not exist | low | Confirmed: a 360-char title fails with `io_error: … File name too long (os error 63)` — loud, but the doc claimed otherwise | patch — fixed: capped at 60 characters, and the doc now matches the code |
| 6 | blind-hunter | Every non-ASCII (or punctuation-only) title collapses to `chore-untitled` | low | Confirmed: `修復錯字` records as `chore-untitled`; the next such title becomes `chore-untitled-2` | patch — fixed: `is_alphanumeric` keeps the letters, so `chore-修復錯字`; test added |
| 7 | blind-hunter | `chore_dir` hardcodes `.qdev/chores` while `init` derives the directory from the configured `cache_dir` | low | Confirmed with `cache_dir = "var/qdev-cache"`: `init` creates and ignores `var/chores/`, records are written to `.qdev/chores` — visible to git. Same hardcoding already exists in `find_workspace_leases`, so this is an existing project-wide pattern, not a Story 2.10 invention | defer → `deferred-work.md` |
| 8 | blind-hunter, edge-case-hunter | The lease gate uses `find_workspace_leases(…).first()` rather than the `find_active_lease` the Code Map names | low | Confirmed: with several lease files it silently picks the first. In practice a lease in this worktree's directory does block this worktree, so the refusal is correct — only the naming is imprecise | patch — fixed: every lease is named, so the refusal lists what it counted |
| 9 | blind-hunter | No CLI way to list, close, or abandon a chore, so an abandoned one blocks `chore start` indefinitely | low | True: `start` refuses while any record is `open`, and only deleting `.qdev/chores/<id>.json` clears it. But the refusal says so — "commit or delete it" — and `chore commit` closes it; nothing in the story asks for a `list`/`close` command | rejected — unlikely in everyday use, and the fix adds public surface |
| 10 | blind-hunter, verification-gap | `list_chore_records` swallows parse errors and its doc claims an ordering it does not implement | low | Confirmed: records that fail to parse are skipped and the function always returns `Ok`; the doc said "oldest id first" while the code sorts by id | patch — fixed: the doc now states the real ordering and the deliberate tolerance, which mirrors `find_workspace_leases` |
| 11 | blind-hunter | Text output prints every excluded path under `NOT INCLUDED — still staged:`, including paths that were never staged | low | Confirmed with only unstaged edits: `NOT INCLUDED — still staged: src/main.rs`. D-4 and the matrix promise that heading for genuinely staged work only | patch — fixed: separate `still staged` / `not staged` lists |
| 12 | blind-hunter | `ChoreCommitResult.committed` is always `true`, and two `unwrap_or` fallbacks are unreachable | false | There is no path that returns `Ok` without committing, so `committed` reports the truth; the `unwrap_or` arms are defensive literals, not defects. No caller or user is misled | rejected |
| 13 | blind-hunter, verification-gap | The rename/copy branch in `changed_paths` is untested, and its `peekable()` is vestigial | medium | Verification-gap layer filed it as a gap; checked: a renamed allowlisted file commits as `A docs/old-guide.md` with `D docs/guide.md` stranded in the index — the commit is half-done, and `no_active_chore` on the next call means nothing picks the rest up | patch — fixed: both endpoints go in the commit pathspec (only the destination is staged), `peekable` removed, and a rename test pins `R100 docs/guide.md → docs/old-guide.md` |
| 14 | blind-hunter | No test for the JSON-mode error paths, for `chore_in_progress` at CLI level, or for the empty-title case | false | Every I/O-matrix row is covered by a running test; none of these three has a matrix row, and `chore start` cannot be invoked without a title (clap requires the positional) | rejected |
| 15 | blind-hunter | New tests were inserted between `test_every_advertised_payload_name_has_a_compilable_schema` and its doc comment, leaving the function undocumented | low | Confirmed at `crates/qdev-cli/tests/schema_payload_cli_tests.rs:636` — the invariant comment sat above `test_schema_payload_chore_text_and_json`, and `…compilable_schema` had none | patch — fixed: the comment is back on its own function |
| 16 | blind-hunter | Bookkeeping never advances (`status: in-progress`, `review_loop_iteration: 0`, empty logs, `git show --stat` claim, no trailing newline) | false | `status` was set to `in-review` at step-04 and `sprint-status.yaml` to `in-progress` at step-03 — the file moves to `review` at step-05, not before; the logs are filled by this very section, and the `--stat`/`--name-only` point is spec wording, not behaviour | rejected |
| 17 | blind-hunter | ~100 lines of the diff are rustfmt-only churn in `dw.rs`, `dw_tests.rs`, `decision_cli_tests.rs`, `dw_cli_tests.rs`, and the new deferred-work entry omits a `Review by:` trigger | low | The churn is required: those files were committed in `ccd00a2` *without* rustfmt formatting, so `cargo fmt --check` fails against them until they are formatted. The missing `Review by:` point is real — the file's own header says every entry carries one | partial: churn rejected as necessary; `Review by:` fixed (`story 3.9`) |
| 18 | edge-case-hunter | With a merge or rebase in progress, a conflicted allowlisted file gets staged with markers and `git commit -- <path>` then fails with "cannot do a partial commit during a merge" | low | Confirmed by the reviewer's own experiment. Refusing to commit mid-merge is correct behaviour, and it fails loudly at a state this story never invites | rejected — unlikely in everyday use, and the fix adds a guard branch |
| 19 | edge-case-hunter | `start_chore`/`commit_chore` do not take `acquire_workspace_write_lock`, so two concurrent runs can clobber one record | low | Confirmed by inspection: `lease.rs:367` and `write.rs` take the lock; `chore.rs` does not | patch — fixed: `start_chore` holds it across the check-then-write; `commit_chore` cannot hold it because `log_decision` acquires the same lock (holding it there times out with `lock_timeout`), which is documented at the call site |
| 20 | edge-case-hunter | D-3 says a deleted allowlisted file yields `nothing_to_commit`, but the code commits the deletion and exits 0 | false | A deletion *is* a change under the allowlist, and committing it is what a chore for that path should do — the behaviour is tested (`a_deleted_file_under_the_allowlist_is_committed_as_a_deletion`). The claim's fix would be a spec edit, which the triage rules reject | rejected |
| 21 | verification-gap | `--strict` is tested in one configuration only: no test for a pre-staged out-of-allowlist change, an only-out-of-allowlist change, or an all-inside change | medium | Filed pre-verified and accepted as-is; swapping the two checks keeps every existing test green while changing the exit code, which proves the coverage hole | patch — fixed: all three configurations are now tested (3, 3, and 0 with the `DEC-` committed and not counted against the allowlist) |
| 22 | verification-gap | Rename/copy handling has no test at all | medium | Filed pre-verified; same root cause as row 13 — see the `R100` evidence there | patch — fixed together with row 13 |

**No `intent_gap` or `bad_spec` entries: nothing here required re-opening the frozen intent, so there was no loopback.** All `patch` rows are implemented; row 7 is recorded in `docs/bmad/implementation-artifacts/deferred-work.md`; rows 3, 9, 12, 14, 16, 18, 20 are rejected with their refutations.

_Review 2 (walkthrough, after implementation and approval). The two entries the reviewer raised were agreed by the story owner and implemented the same day; both were reproducible against the built binary._

| ID | Source | Location | Description | Verdict | Evidence | Route |
|---|--------|----------|-----------|---------|----------|-------|
| W-1 | reviewer (walkthrough) | `crates/qdev-core/src/chore.rs:24`, `crates/qdev-cli/src/cli.rs:93` | Nothing could list or close a chore: a record with a mistyped glob stayed `open` forever, so `commit` gave `nothing_to_commit` and every later `start` gave `chore_in_progress` | agreed — added | Ran against the built binary: `start` with `--paths 'docs/ghost.*'` → `commit` exit 1 `nothing_to_commit` → `start` exit 5 `chore_in_progress`, with only `start`/`commit` in `chore --help` | patch — fixed: added `list`, `close`, `abort` (D-6), with core and CLI tests for the whole loop |
| W-2 | reviewer (walkthrough) | `crates/qdev-core/src/chore.rs:24` vs `crates/qdev-core/src/init.rs:118` | `CHORE_DIR` was hardcoded while `init` derived the directory from `cache_dir`; a relocated cache left the record in git-visible space (already deferred, re-confirmed as real) | agreed — deferred item still open, now fixed | After `qdev init` with `cache_dir = "var/qdev-cache"`, `.gitignore` listed `var/chores/` and the record was written to `.qdev/chores/chore-relocated.json`, shown as `?? .qdev/` by `git status` | patch — fixed: `chore_dir` derives from the configured `StorageConfig` (D-7); `deferred-work.md` entry closed |

## Design Notes

Selection algorithm, as built: get every changed path from `git status --porcelain=v1 -z --untracked-files=all` (modified, staged, **and** untracked — untracked files under the allowlist must still be committable; for a rename or copy both endpoints are offered to the allowlist, so a renamed file cannot escape it by changing names); partition it by `any(glob_match(g, path))` over the declared globs; commit the matched set with an explicit pathspec (`git commit -- <paths…>`), leaving everything else exactly as it was — which is what `--strict` exists to enforce and what D-4 settles for already-staged work.

Commit message: use the chore title, prefixed `chore: `, with the declared globs as a trailer, so `git log` shows what was allowed.

## Verification

**Commands:**
- `cargo test --test chore_tests` — expected: glob matching, allowlist partitioning, decision-record tests, `close`/`abort` freeing the slot, and the configured-cache-dir case (no hygiene lint — D-2).
- `cargo test --test chore_cli_tests` — expected: end-to-end start/commit against a real git repo, including `--strict`, lease gating, `git show --stat` proof of commit contents, and the whole wedge → `close` → start-again loop.
- `cargo test --test schema_payload_cli_tests` — expected: `payload-chore.json` round-trips and the `PayloadKind::all()` invariant test includes `chore`.
- `cargo test --test workspace_guard_cli_tests` — expected: both chore commands are refused outside an initialized workspace.
- `cargo test --workspace` — expected: full suite green.
- `cargo clippy --workspace --all-targets -- -D warnings` — expected: no warnings (CI enforces this).
- `cargo fmt --all -- --check` — expected: no diff in files this story touches.