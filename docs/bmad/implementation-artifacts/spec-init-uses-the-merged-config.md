---
title: '`init` resolves configuration the way every other command does'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '1089d41c6ec7b5379613a749e811fbbdc08ddf83'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-2-dual-configuration-loader.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev init` has a private, partial configuration reader. `init::resolve_storage` (`init.rs:42-65`) parses `qdev.toml` alone and swallows a parse failure by returning defaults, while every other command goes through the loader, which merges `.qdev.local.toml` key by key (`config/mod.rs:893-911`) and treats an unparseable file as a usage error before dispatch. The epic 1 cross-story review reproduced three consequences:

- **The cache every command uses is the one `.gitignore` does not cover.** `[storage]` is legal in `.qdev.local.toml` — one allowed-section list serves both files — so `cache_dir = "local/cache"` there yields *two* databases: `init` creates and stamps `.qdev/cache/cache.sqlite` and gitignores that path, while every later command creates and uses `local/cache/cache.sqlite`, untracked only by luck and committable. This is the exact failure `resolve_storage`'s own doc comment says it exists to prevent.
- **`init` reports success on a workspace no command can use.** `check_cache_status` resolves through the same private reader, so with the effective cache stamped by a newer binary, every command exits 5 `schema_version_mismatch` while `qdev init` inspects the abandoned database and exits 0 reporting `already_initialized`. The refusal's own advice — delete the cache file — names the wrong file. Separately, an unparseable `qdev.toml` makes every command exit 2 while `init`, the tool's own remedy, exits 0 and repairs nothing.
- **`init` reports paths it did not create.** `created_files` pushes the literal `.qdev/cache/cache.sqlite` (`init.rs:346`) and the text output hardcodes `.qdev/cache/`, `docs/specs/{…}`, `docs/state/{…}` (`main.rs:735-738`), whatever the configured layout. A wrapper consuming that list, or a human following it, is sent to a path that does not exist.

**Approach:** `init` stops having its own answer. It resolves configuration through the same loader every other command uses, so what it scaffolds, stamps, gitignores and reports is the layout the next command will actually use — and an unparseable config is the same refusal everywhere.

## Boundaries & Constraints

**Always:**
- One resolver. `init` uses the loader's merged configuration; `init::resolve_storage` stops being a second implementation. Whatever `init` creates, stamps, checks and reports is derived from that one answer.
- An unparseable `qdev.toml` or `.qdev.local.toml` is the same usage error (exit 2, naming key and file) from `init` as from every other command. `init` never silently substitutes defaults for a file it could not read.
- **Decision (2026-09-10, Simon):** option C — `.qdev.local.toml` may set `cache_dir` only. `specs_dir` and `state_dir` hold committed content, so they are a project decision and belong in `qdev.toml`; the cache is a machine-local rebuildable artifact, so relocating it is a developer's business. A `specs_dir` or `state_dir` key in the local file is a schema error (exit 2, naming key and file) like any other.
- Because `.gitignore` is committed, the ignore entries must cover the project `cache_dir` **and** any locally configured one, so a developer relocating their cache never leaves it untracked-by-luck — and the committed file stays meaningful to everyone else. This asymmetry is documented, not implied.
- Everything `init` reports it created is a path it actually created: `created_files`, `created_directories` and the human output all come from the resolved layout.
- `.gitignore` covers the cache directory that will actually be used.
- `qdev init` on an already-initialised workspace stays idempotent and keeps reporting `already_initialized`.
- Bootstrapping still works from nothing: with no config files at all, `init` scaffolds the default layout exactly as today.

**Never:**
- No new configuration keys, no change to the merge semantics the loader implements (scalar keys override individually; `[[modules]]` and `[[gates]]` replace wholesale), and no change to `qdev config show`.
- No change to the cache schema, the stamping rule, or the newer-than-supported refusal.
- Workspace-root discovery is out of scope — the review's finding that `.git` is accepted as a root marker while the guards require `qdev.toml` is a separate resolver and stays filed.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Bootstrap | no config files | default layout scaffolded, as today | N/A |
| Local `cache_dir` | `.qdev.local.toml` sets `cache_dir` | exactly one cache exists, at the configured path; `.gitignore` covers both it and the project `cache_dir` | N/A |
| Local `specs_dir` or `state_dir` | either key in `.qdev.local.toml` | schema error naming the key and the file, from every command including `init` | Exit 2 |
| Unparseable `qdev.toml` | garbage appended | `init` exits 2 naming the file, same as every other command; nothing is scaffolded | Exit 2 |
| Unparseable local file | garbage in `.qdev.local.toml` | same refusal | Exit 2 |
| Configured layout, reporting | `[storage]` sets all three directories | `created_files`/`created_directories` and the text output name the configured paths only | N/A |
| Newer cache, configured layout | effective cache stamped newer | `init` refuses with `schema_version_mismatch` naming the cache it actually inspected | Exit 5 |
| Re-init | already initialised | `already_initialized`, nothing rewritten | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/init.rs:42-65` (`resolve_storage`) -- the private reader: `qdev.toml` only, `let Ok(table) = raw.parse() else { return default }`. Callers are `init` itself (`:273` scaffolding and `.gitignore`) and `check_cache_status` (`:153-156`) -- what collapses into the loader
- `crates/qdev-core/src/config/mod.rs:893-911` (storage merge) and `:1216-1228` (parse failure → usage error) -- the loader's answer, and the refusal `init` must share -- the one resolver
- `crates/qdev-core/src/config/mod.rs:17-32` (`ALLOWED_TOP_LEVEL_SECTIONS`) and `:212-226` (`validate_storage_section`) -- one list for both files, section-level validation; where option B or C would land -- the Open Question's site
- `crates/qdev-core/src/init.rs:69-99` (`standard_directories`) and `:104-115` (`gitignore_entries`) -- both already take `&StorageConfig`, so they need the right value passed, not new logic -- reuse
- `crates/qdev-core/src/init.rs:346` -- `created_files.push(".qdev/cache/cache.sqlite")`, the hardcoded report, beside `:277` which computes the real path -- the reporting defect
- `crates/qdev-cli/src/main.rs:733-738` (`handle_init` text output) -- four hardcoded `println!` lines; `InitResult` already carries the created directories -- render from the result
- `crates/qdev-core/src/init.rs:153-208` (`check_cache_status`) -- resolves through the private reader, then compares versions; the newer-cache refusal it raises is shared with `ensure_cache` -- fix the resolution, not the refusal
- `crates/qdev-cli/src/main.rs:143-149` -- the CLI loads config before dispatch and exits 2 on failure, which is why every command *except* `init` already refuses an unparseable file. `init` is dispatched after this point, so the refusal may already be available to it — confirm before adding a second check -- likely simpler than it looks
- `crates/qdev-cli/tests/init_cli_tests.rs` -- the init suite, including the configured-layout scaffolding test and the newer-cache refusal -- the contract to keep
- `crates/qdev-core/tests/config_tests.rs` -- merge-semantics coverage; where a local `[storage]` case belongs -- verification

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/init.rs` -- `init` and `check_cache_status` take the resolved configuration instead of reading it themselves; `resolve_storage` is deleted or reduced to a thin adapter over the loader -- one resolver
- [x] `crates/qdev-core/src/config/mod.rs` -- accept `cache_dir` from `.qdev.local.toml` and reject `specs_dir`/`state_dir` there with the loader's usual schema error -- per-key locality
- [x] `crates/qdev-core/src/init.rs` -- `.gitignore` covers the project `cache_dir` and any locally configured one -- a relocated cache is never untracked by luck
- [x] `crates/qdev-core/src/init.rs` -- `created_files` and `created_directories` report the paths actually created -- honest reporting
- [x] `crates/qdev-cli/src/main.rs` -- render `init`'s human output from `InitResult` rather than hardcoded strings, and make an unparseable config the same refusal it is for every other command -- honest reporting, one refusal
- [x] `crates/qdev-cli/tests/init_cli_tests.rs`, `crates/qdev-core/tests/config_tests.rs` -- cover every matrix row, including the two-caches reproduction and the unparseable-config refusal -- verification
- [x] `docs/architecture.md`, `docs/cli-reference.md` -- state where the layout may be configured and that `init` resolves it like everything else -- keep docs authoritative

**Acceptance Criteria:**
- Given the review's reproduction — `.qdev.local.toml` setting `cache_dir` — when `qdev init` runs and then any other command runs, then exactly one cache database exists on disk, at the path the commands use, and `.gitignore` covers it.
- Given `.qdev.local.toml` setting `specs_dir` or `state_dir`, when any command runs — `init` included — then it exits 2 naming the key and the file.
- Given a workspace whose effective cache is stamped newer than the binary supports, when `qdev init` runs, then it refuses with `schema_version_mismatch` naming that cache — not exit 0 having inspected a different file.
- Given an unparseable `qdev.toml`, when `qdev init` runs, then it exits 2 naming the file, and no directories, config files or cache are created.
- Given a workspace configured for a non-default layout, when `qdev init --json` runs, then every path in `created_files` and `created_directories` exists on disk, and the text output names no path outside the configured layout.
- Given no configuration at all, when `qdev init` runs, then the default layout is scaffolded exactly as before this story.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Implementation Notes

**The Design Note had the dispatch order backwards, and that turned out to be the whole fix.**
`init` was dispatched *before* `load_config` (`main.rs:132-135`, ahead of `:143`), with the
comment "so bootstrapping works in uninitialized directories" — which is why it needed a private
reader and why an unparseable file reached it as "absent". But bootstrapping needs no exemption:
both files absent is not an error to the loader, it is the default configuration. So `init` is now
dispatched *after* `load_config`, and every consequence follows from that one move: the merged
`[storage]` is available to it, and an unparseable or invalid file is the same exit-2 refusal
before `init` scaffolds anything. `resolve_storage` is deleted rather than adapted.

**Two `StorageConfig` values, not one.** `.gitignore` and the gate scripts under
`<qdev_dir>/gates/` are committed, so they cannot be derived from a developer's local
`cache_dir`. `InitLayout` carries both the effective (merged) layout and the project-only one, and
its doc comment states which one answers which question. The project layout comes from
`config::load_project_storage`, which is the same parse, the same validation and the same
`merge_configs` call as `load_config` with the local table withheld — not a second resolver. Only
`cache_dir` can differ between the two, since `specs_dir`/`state_dir` are now rejected in the
local file.

**Reporting.** `InitResult` gained `storage` and `qdev_dir`, so the payload says which layout the
run resolved, and the CLI's human output is rendered from the result instead of five hardcoded
`println!`s. The `{prd,requirements,…}` subdirectory lists are now the shared
`SPEC_SUBDIRECTORIES` / `STATE_SUBDIRECTORIES` constants that `standard_directories` builds from,
so the output cannot drift from what `init` creates.

**Deliberately unchanged:** `merge_configs` — the per-key locality rule lives in
`validate_storage_section` alone, so there is one place that encodes it. The `qdev_dir` rule
(parent of the *project* `cache_dir`) is the rule story 1.2's `[storage]` amendment shipped, now
pinned to the project value so a local cache override does not move committed gate scripts.

## Spec Change Log

- 2026-09-10 — Created from the epic 1 cross-story review (findings H1, L2, M7), the last of the three root causes that review named as blocking epic 1 acceptance. The first shipped as `61cebe7`, the second as `1089d41`.
- 2026-09-10 — Open Question answered by Simon: option C, `cache_dir` is locally overridable and the other two are project-only. Recorded in the frozen block with the `.gitignore` consequence (entries cover both the project and any local cache directory). Matrix, tasks and AC extended; this **does** narrow story 1.2's boundary, which says local keys override project keys *individually* — two storage keys stop being overridable. Less than option B would (it rejected the section outright), but it is a restriction of an approved contract, not a free reading of it. Flagged to Simon rather than glossed; `spec-1-2-dual-configuration-loader.md` should carry an amendment note if this ships, in the manner of its 2026-09-10 `[storage]` amendment.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | Deleting `resolve_storage` removed its `if !v.trim().is_empty()` guard with nothing in its place: an empty `[storage]` value made `root.join()` yield an absolute path, so `init` wrote a bare `/` into the committed `.gitignore`, tried to open `/cache.sqlite`, and with `specs_dir = ""` attempted to create `/prd` — blocked here only by a read-only root (all three reviewers; demonstrated against the binary). | high | patch | Reproduced by me too. Fixed in the loader, not in `init`: `validate_storage_section` now refuses an empty, absolute or `..`-containing value with the section's usual exit-2 error naming key and file. Verified: all three shapes refuse with nothing scaffolded. |
| 2 | `init` trimmed `[storage]` values via a new private `trim_dir` while the loader stored them raw, so `cache_dir = " var/cache "` gave *two* databases with the live one outside `.gitignore` — the H1 defect this story exists to remove, in a new disguise (verification-gap, pre-verified; blind independently). | high | patch | Reproduced against the binary by the reviewer and again by me. The loader now normalizes once (whitespace and trailing `/`), so every consumer joins the same string; `trim_dir` is reduced to a guard for hand-built configs and documented as never being the only normalization. Verified: one cache at `var/cache/`, gitignored. |
| 3 | The `.gitignore` fix only holds when the local `cache_dir` exists *before* `init` runs. The spec's own Verification note prescribes the other ordering, which leaves the live cache untracked (verification-gap, pre-verified). | medium | patch | Confirmed. `.gitignore` is committed, so no ordinary command may edit it — re-running `init` is the remedy. Now documented in `cli-reference.md` with the accumulation consequence, and pinned by a test asserting a plain command does *not* touch `.gitignore`, that a re-run adds the entry, and that entries are not duplicated. |
| 4 | The project-only rule was dispatched on a filename string, so a caller passing a path or `./`-prefixed form silently fell through to the permissive branch (blind). | medium | patch | Confirmed — `validate_config_table` is public. Now matched on the file-name component. |
| 5 | `init`'s human output still printed `✔ qdev.toml` and `✔ .qdev.local.toml` unconditionally, including on a re-init that wrote neither — the same rule the hardcoded paths broke (blind, edge as a claim). | medium | patch | Confirmed. Both lines now render from `created_files`, and the new re-run test asserts neither file is reported. |
| 6 | `GITIGNORE_ENTRIES` became dead (no callers, still exported) and `STANDARD_DIRECTORIES` is a hand-maintained duplicate of the default layout that tests pin, so it can drift from the shared subdirectory constants (blind, verification-gap). | low | patch | Confirmed. Dead constant deleted; a test now asserts `STANDARD_DIRECTORIES` equals what `standard_directories` builds for the default layout. |
| 7 | `load_project_storage` re-reads and re-validates both files immediately after `load_config` did, so the two layouts can come from different content if a file is edited between them, and `.gitignore` is written from the mismatch (blind). | medium | defer | Real. The fix is for the loader to return both layouts from one read (`AnnotatedConfig` carrying the project-only value), which changes a public struct. The window is one process start and both files are small. Filed. |
| 8 | Every write path still resolves its lock directory as `storage.as_ref().map(...).unwrap_or(".qdev/cache")`, so a caller omitting `storage` locks a different file than one supplying it — two locks means no mutual exclusion (blind). | medium | defer | Confirmed at four call sites. Making `storage` non-optional is a public API change across the write path; the CLI always supplies it today, so no caller currently diverges. Filed as the last hardcoded-default-layout site. |
| 9 | `.gitignore` entries accumulate and are never pruned; a directory that stops being anyone's cache keeps its line (blind, edge). | low | patch | Confirmed. Documented rather than pruned — pruning a committed file on another developer's behalf is worse than a harmless stale line. |
| 10 | Configured-layout idempotency was untested: every re-init test used the default layout (blind). | low | patch | Confirmed. The new re-run test covers `already_initialized`, no duplicated ignore entries, and no re-reported config files under a configured layout. |
| 11 | An existing workspace with a local `specs_dir` now hard-fails every command, including `init`, with no in-tool migration (edge). | low | rejected | True, and the message *is* the remedy: it names the key, the file, and that the key belongs in `qdev.toml`. A one-line manual move is the whole fix, and exempting `init` would reintroduce the "init has its own answer" defect this story removes. |
| 12 | `init` is not atomic on the failing path — with an invalid layout it had already written the config files and directories before the cache open failed (verification-gap, other). | low | rejected | No longer reachable for the shapes reported: an invalid layout is now refused by the loader before `init` is dispatched, and the test asserts nothing is scaffolded. A mid-scaffold I/O failure remains non-atomic, which is pre-existing and out of this story's intent. |
| 13 | Comment and wording defects: `InitLayout::qdev_dir`'s doc claimed leases are committed; the spec-1-2 amendment said "two of the fourteen sections' keys" where it meant two of one section's three; a single-segment `cache_dir` puts `gates/` under an unrelated `.qdev/` (blind). | low | patch | First two corrected. The third is real but is `parent_dir_of`'s documented fallback; noted rather than changed, since choosing a different parent for a one-segment cache path is a layout decision, not a fix. |
| 14 | Test hygiene: the unparseable-config test writes a valid `qdev.toml` then overwrites it on the first iteration, and neither new test asserts the error `code` or `details.file` (blind). | low | patch | Confirmed; the config-level tests added for finding 1 assert `details.key` and the exit code, which closes the substantive half. |

### Review pass adjustments (2026-09-10)

- **The spec's own Design Note was wrong**, and the implementer was right to say so: `init` was
  dispatched *before* `load_config`, which is precisely why it had a private reader. It is now
  dispatched after, so an unparseable configuration is the same exit-2 refusal from the tool's own
  remedy as from everything else. Bootstrapping still works — both files absent is the default
  configuration, not an error — and a test pins it.
- Findings 1 and 2 are the same lesson twice: this story's whole point is that a layout must be
  resolved in one place, and both regressions came from *validating* or *normalizing* it somewhere
  else. Both fixes went into the loader for that reason.

## Design Notes

- The bootstrapping objection to "just use the loader" is weaker than it looks: `init` runs *after* the CLI has already loaded configuration and exited 2 on failure (`main.rs:143-149`), so the merged answer is available at the point `init` is dispatched. Confirm that before building anything more elaborate — the fix may be threading a value rather than writing a resolver.
- `standard_directories` and `gitignore_entries` already take a `&StorageConfig`; the bug is exclusively in *which* `StorageConfig` they are handed. That is why this story is mostly deletion.
- The `.gitignore` wart under option A is worth stating in the docs whichever way the Open Question goes: an ignore file is committed, so any layout key that can be set locally produces a committed entry derived from one developer's machine.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- Manual, the review's reproduction: `qdev init`, add `[storage] cache_dir = "local/cache"` to `.qdev.local.toml`, run any command, and confirm exactly one cache exists and `.gitignore` covers it
