---
title: 'One identity rule for reads and writes'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '8752763e18790f65a945011262412623a6cea168'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-8-write-path-atomic-files-locking-frontmatter-patching.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** qdev answers "which file is entity X?" two different ways. Every read path answers from frontmatter: hydration keys the entity on the `id` field and records its `source_path` (`sqlite.rs:4387`). Every write path answers from the filename: `resolve_entity_file` scans `<specs_dir>/<kind>/` for `<id>.md`, `<id>-*` or `<id>_*` (`write.rs:1107`, `find_file_in_dir` at `:1054`) and never consults the `source_path` sitting in the cache.

The epic 1 cross-story review found four defects at that seam, three reproduced against the binary:

- **`qdev validate --fix-ids` produces entities its own write path cannot touch.** It rewrites the frontmatter `id` and never renames the file, so the divergence is manufactured deliberately. After renumbering `E1S1.md` to `E1S2`: `qdev get E1S2` works, `qdev update E1S2` and `qdev relate E1S2 …` both fail `usage_error: Entity file not found for 'E1S2'`. Only a manual `mv` restores it. This is the tool's own remedy for duplicate ids leaving the workspace half-broken.
- **A `--fix-ids` abort loses the keeper entity from the cache.** `renumbered.push` happens after the relation and citation steps, and the reconcile is gated on `!renumbered.is_empty()` (`main.rs:2316`, `:2333`) — while the comment directly above it promises the reconcile "runs even when the loop aborted". Since the relation step fails exactly when a referencing file's name ≠ its id, the abort is reachable from the defect above. Reproduced: `E1S1` vanished from the cache permanently, `validate` reported `findings: []`, and only `sync --rebuild` repaired it.
- **A file outside the conventional directory is readable but not writable.** Hydration walks `specs_dir`/`state_dir` recursively; the write path looks only in `<specs_dir>/<kind>/`. A story at `specs/misc/E1S9.md` is returned by `get` and accepted as a `relate` target, yet `update E1S9` fails "not found" — and in the `relate` case the command's own cache precheck had already confirmed the entity exists.
- **Writers and hydration disagree about an entity's kind.** Writers take the kind from the directory, then the identifier grammar; hydration prefers frontmatter `kind:` (`determine_entity_kind`, `sqlite.rs:4402`). `--fix-ids` is the only writer that resolves it hydration's way, and its comment names the invariant the others break. Reproduced: `update` exits 0 having validated against the wrong schema, and the next boot records an error-severity `schema_violation` on the file it just wrote.

**Approach:** make one rule answer "which file is entity X?" for readers and writers alike, and make `--fix-ids` leave the workspace in a state its own write path accepts. Kind resolution collapses onto `determine_entity_kind`, the function hydration already uses, so a write validates against the schema the next sweep will validate against.

## Boundaries & Constraints

**Always:**
- One resolution rule, used by every writer. `resolve_entity_file` stays the single entry point; no caller grows its own lookup.
- Kind is resolved by `determine_entity_kind` — frontmatter `kind:` first — wherever a writer needs it, so a write and the following sweep validate against the same schema.
- `--fix-ids` leaves every entity it touched resolvable by the write path. A renumbered entity must accept `qdev update` immediately afterwards, with no manual step.
- `--fix-ids` reconciles the cache whenever it wrote anything at all, aborted or not, and reports what it wrote. The keeper entity is never left absent from the cache with its `sync_state` row intact.
- **Decision (2026-09-10, Simon):** option B — a file is named for the entity it holds. `<id>.md`, `<id>-<slug>.md` or `<id>_<slug>.md`, which is exactly what `find_file_in_dir` already assumes, so the write path keeps resolving by filename and gains no cache dependency (`qdev create story` outside a workspace depends on that). `--fix-ids` renames as it renumbers, and a filename that does not carry its entity's id becomes a validation finding.
- **The off-convention finding is `warning` severity**, not `error`, so it does not make `qdev validate` exit 1 on a workspace that was legal until this story shipped. `has_error_finding` already gates the exit code on `error` alone, and `warning` is an existing severity the taxonomy supports.
- The rename `--fix-ids` performs is reported in its payload, so the git-visible move is never silent: an entry names the old and new path as well as the old and new id.
- Ambiguity stays an error, never a guess: if a rule matches two files for one id, that is a usage error naming both, as `find_file_in_dir` does today.
- `qdev create story` keeps working outside an initialized workspace (no cache, no lock) — established by `3827acf`.

**Never:**
- No change to how hydration resolves identity. Frontmatter `id` stays the entity's identity; this story changes the *write* side's answer, or the filename, but not that.
- No new entity kinds, no cache schema change, no change to the `get`/`list` payloads.
- No silent rename or move of a file the user did not ask about, outside `--fix-ids`.
- The duplicate-id detection scope (`specs_dir` only) and the purge/re-hydration asymmetry are **not** in this story — they are the other two root causes the cross-story review named.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Renumber then write | `--fix-ids` renumbers `E1S1.md` to `E1S2`, then `qdev update E1S2` | update succeeds; no manual `mv` | N/A |
| Renumber then relate | as above, then `qdev relate E1S2 depends_on E1S3` | succeeds | N/A |
| Fix-ids abort after a write | relation step fails on the second file | the first renumber is reported, the cache is reconciled, the keeper id is present in the cache afterwards | Exit per the abort's own error |
| Off-convention filename | story `E1S9` in a file not named for it | a `warning` finding naming the file and the expected name; `qdev validate` still exits 0 if nothing else is wrong | N/A |
| Renumber reports the move | `--fix-ids` renames a file | the payload entry names old and new *path* as well as old and new id | N/A |
| Rename target occupied | the file `--fix-ids` would rename to already exists | refuse that entry and record it as skipped, rather than clobbering | Exit per the abort's error |
| Kind disagreement | ADR file declaring `kind: story` | the write validates against the story schema, and the next sweep records no new finding for it | Exit 1 if that schema rejects it |
| Two files, one id | both name id `E1S1` | a usage error naming both files, unchanged from today | Exit 2 |
| Bare directory create | `qdev create story E12` with no workspace | unchanged: writes the story file only | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/write.rs:1107-1197` (`resolve_entity_file`) -- the write side's answer: kind from directory or grammar, then `find_file_in_dir`; four exit paths, all filename-based. The single place a new rule lands -- the seam
- `crates/qdev-core/src/write.rs:1054-1105` (`find_file_in_dir`) -- matches `<id>.md`, `<id>-*`, `<id>_*` case-insensitively in one directory; already returns a usage error naming both files on ambiguity -- keep that behaviour
- `crates/qdev-core/src/write.rs:1266` (`apply_entity_update`) and `:1878` (`apply_relation_change`) -- the only two callers; both take `(kind, id, file_path)` and validate against that `kind` -- what changes with the rule
- `crates/qdev-core/src/store/sqlite.rs:4402` (`determine_entity_kind`) -- hydration's rule: frontmatter `kind:`, then directory, then grammar, then `Story`. Already public and already used by `--fix-ids` -- adopt for writers
- `crates/qdev-core/src/store/mod.rs:26` (`EntityRecord::source_path`) and `:256` (`Store::get_entity`) -- the cache already holds the exact path for every hydrated entity; nothing on the write path reads it -- what option A would use
- `crates/qdev-cli/src/main.rs:2229-2345` (`handle_fix_ids`) -- plan phase, then the renumber loop; `renumbered.push` at `:2316` after the relation and citation steps, reconcile gated at `:2333` on a list the abort leaves empty. The comment at `:2329` states the intended behaviour -- the abort defect
- `crates/qdev-cli/src/main.rs:2470-2600` (`renumber_duplicate_file`) -- rewrites the frontmatter id, patches, validates with `determine_entity_kind`, writes atomically, upserts the cache; does not rename the file and does not purge the old id's row -- where a rename would go
- `crates/qdev-cli/src/main.rs:2335-2341` (`keepers_to_rehydrate`) -- drops each keeper's `sync_state` row so the sweep re-parses an unchanged file; the existing workaround for the same class of problem -- reuse, do not reinvent
- `crates/qdev-core/src/write.rs:887` (`upsert_cache_and_mark_dirty`) -- plain `ON CONFLICT(id)`; unlike hydration (`sqlite.rs:4416`) it does not purge a row claiming the same `source_path` under a different id, which is why a renumber leaves two rows pointing at one file -- contributing site
- `crates/qdev-cli/tests/validate_cli_tests.rs` -- the `--fix-ids` suite, including the interactive and `--yes` paths -- the contract to keep
- `crates/qdev-core/tests/write_tests.rs` -- `resolve_entity_file` unit coverage -- where a new rule's cases belong

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/write.rs` -- keep `resolve_entity_file` filename-based and make the convention it assumes explicit in its documentation; ambiguity stays a usage error naming both files -- one rule, stated
- [x] `crates/qdev-core/src/validate.rs` -- add the off-convention filename check at `warning` severity, reported by `run_validation` like the other computed checks -- makes the convention enforceable rather than assumed
- [x] `crates/qdev-core/src/write.rs` -- resolve kind through `determine_entity_kind` so a write validates against the schema the next sweep will use -- kind agreement
- [x] `crates/qdev-cli/src/main.rs` (`handle_fix_ids`) -- reconcile whenever anything was written, not only when the loop completed an entry; report partial work -- the abort defect
- [x] `crates/qdev-cli/src/main.rs` (`renumber_duplicate_file`) -- rename the file to carry the new id (preserving any `-slug`/`_slug` suffix), refusing rather than clobbering an occupied target, and purge the old id's row instead of leaving two rows on one file -- the manufactured divergence
- [x] `crates/qdev-cli/src/main.rs` + `crates/qdev-core/schemas/payload-fix-ids.json` -- report `old_path`/`new_path` in each renumber entry and describe them in the payload schema -- the move is never silent
- [x] `crates/qdev-core/tests/write_tests.rs`, `crates/qdev-cli/tests/validate_cli_tests.rs` -- cover every matrix row, including renumber-then-update and the abort-with-partial-write case -- verification
- [x] `docs/architecture.md` and `docs/cli-reference.md` -- state the identity rule once, where a reader looking for "how does qdev find my entity" will find it -- keep docs authoritative

**Acceptance Criteria:**
- Given a workspace where `--fix-ids` has renumbered a duplicate, when `qdev update` and `qdev relate` are run against the new id, then both succeed with no manual filesystem step.
- Given a `--fix-ids` run that aborts after writing at least one file, when it returns, then the payload reports what was written and the entity ids present in the cache match the ids on disk — verified by `qdev get` on the keeper, not by inspecting the cache directly.
- Given a story whose filename does not carry its id, when `qdev validate` runs, then it reports one `warning`-severity finding naming the file and the expected name, and exits 0 if nothing else is wrong — so a workspace that was legal before this story does not start failing.
- Given any entity the read path resolves *and whose file follows the convention*, when a write is attempted on it, then the write resolves the same file the read did, never "Entity file not found" for an entity `get` just returned.
- Given a file whose frontmatter `kind` disagrees with its directory, when `qdev update` succeeds on it, then the next boot sweep records no new `schema_violation` for that file.
- Given the existing `--fix-ids` and write-path test suites, when this story ships, then they pass unchanged except where a test asserted the defective behaviour, and any such change is called out in the Spec Change Log.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Implementation Notes

Implemented 2026-09-10 against baseline `8752763`.

**The rule, stated once.** `resolve_entity_file` stays filename-based and now carries the rule in
its doc comment; the two halves of it are single functions the write path and `validate` share —
`filename_carries_id` (`<id>.md`, `<id>-<slug>.md`, `<id>_<slug>.md`, case-insensitive) and
`directory_for_kind` (now `pub(crate)`). `find_file_in_dir` was refactored onto the first, so
there is one implementation of "does this file carry this id", not two. Ambiguity behaviour is
unchanged.

**Kind agreement.** New `write::kind_for_write(file_path, content, resolved_kind)` delegates to
hydration's `determine_entity_kind` on the content the write is about to leave on disk.
`apply_entity_update` resolves it from the *patched* content (so a `--field kind=…` edit is
honoured) and `apply_relation_change` from the content it read (a relate never edits `kind:`, and
resolving once there also covers the idempotent no-op return). For a conventional file with no
frontmatter `kind:` this is the directory answer — i.e. exactly what writers did before — so no
existing behaviour moved.

**`--fix-ids` renames as it renumbers.** `renumber_duplicate_file` computes the target name with
the new core helper `renamed_file_name` (slug preserved, canonical casing), refuses an occupied
target with `rename_target_exists` before writing anything, writes the new file and then removes
the old one (an interruption leaves two copies — the duplicate condition already being repaired —
rather than none), and returns the new relative path. The stale `(old id, old path)` row is
dropped by the new `write::purge_entity_row_for_moved_file`, guarded on `source_path` so the
keeper's row (same id, different file, untouched on disk) survives. Separately,
`upsert_cache_and_mark_dirty` now purges any row claiming the same `source_path` under a
different id, which is hydration's own rule for an in-place id edit; both purges are deliberately
*shallow* (entities + stories + dirty marker, never `relations`), because `--fix-ids` redirects
inbound edges through the relate write path immediately afterwards and a cascade would delete the
edges it is about to move.

**The abort defect.** The `FixIdsEntry` is pushed the moment the file is written, before the
relation and citation steps, and its `relations_rewritten`/`citations_rewritten` are filled in
afterwards; the reconcile is gated on a new `wrote_anything` flag set when the renumber is
attempted under the lock, not on `!renumbered.is_empty()`. A renumber that fails also records its
path in `skipped`, so a refused rename target is reported rather than silently absent.

**Off-convention finding.** `find_off_convention_entity_files` (in `run_validation`, alongside the
other computed checks) reports `entity_file_off_convention` at `warning` severity for any hydrated
entity whose `.md` file either does not carry its id or sits outside every standard entity
directory. The *directory* half is checked against the whole standard set rather than the one
directory for the entity's kind, because that is what the write path accepts (its cross-kind
fallback resolves an id in any standard directory) — which is also what keeps the kind-disagreement
matrix row from producing a new finding on every sweep.

**Payload.** Each renumber entry gains `old_path` and `new_path`; `path` is retained as a
duplicate of `old_path` so existing consumers keep working, and the schema says so. Text output
prints a `Renamed <old> -> <new>` line. The undocumented `error` field is L3 in the cross-story
review and was left for that item.

### Deliberately not done

- L7 is now *reported*, not fixed: a file outside every entity directory stays unwritable, and
  `validate` names it and the path it should have. Making the write path resolve arbitrary
  locations needs the cache (option A), which the frozen decision rules out.
- The duplicate-id detection scope (`specs_dir` only) and the purge/re-hydration asymmetry are
  the other two root causes and remain out of scope, per the frozen Never list.

## Spec Change Log

- 2026-09-10 — Created from the epic 1 cross-story review (findings H2, H3, M3, L7), the first of the three root causes that review named as blocking epic 1 acceptance.
- 2026-09-10 — Open Question answered by Simon: option B, the filename carries the id. Recorded in the frozen block along with two follow-on decisions the answer implied: the new finding is `warning` severity (so a previously-legal workspace does not start failing `validate`), and `--fix-ids` reports the rename in its payload. Matrix, tasks and AC extended.
- 2026-09-10 — Implemented. Three existing `--fix-ids` CLI tests asserted the defective
  behaviour (reading the renumbered entity back under its *old* file name, which only worked
  because nothing renamed the file) and were updated to assert the rename instead:
  `test_fix_ids_with_yes_renumbers_duplicate_and_leaves_workspace_clean`,
  `test_fix_ids_accepting_the_prompt_renumbers` and
  `test_fix_ids_reports_what_it_wrote_when_a_later_file_fails` in
  `crates/qdev-cli/tests/validate_cli_tests.rs`. No other test changed. New coverage: five unit
  tests in `crates/qdev-core/tests/write_tests.rs` (rename naming, resolve round trip, kind rule,
  both cache purges), six in `crates/qdev-core/tests/validate_tests.rs` (the off-convention check,
  including the configured-layout and kind-disagreement cases), four in
  `crates/qdev-cli/tests/validate_cli_tests.rs` (renumber-then-update/relate, abort in the
  relation step, occupied rename target, the `warning` finding at exit 0) and two in
  `crates/qdev-cli/tests/update_cli_tests.rs` (kind disagreement, both directions).

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap. 20 findings.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | An occupied rename target aborted the whole run, where the frozen matrix says refuse *that entry* and record it as skipped — so one collision abandoned duplicate groups the user had already confirmed (blind, edge). | high | patch | Direct deviation from the frozen contract. Now `continue`s on `rename_target_exists` only. Verified manually: the refused entry is skipped and an independent group is still repaired. |
| 2 | The rename guard checked only the exact target path, so a prefix sibling (`E1S2-old.md`) still claimed the new id and every later write would fail "multiple entity files match" — defeating this story's own acceptance criterion (edge, filed as a claim). | high | patch | Confirmed by reading `find_file_in_dir`. The guard now asks `find_file_in_dir_for_id`, the same question the write path asks. Verified manually with a file *named* for a free id. |
| 3 | `delete_entity_row_shallow` deleted only the `stories` detail row, so a renumbered deferred-work item, decision, sprint, SOUP entry or evidence record left an orphan detail row; constraints owned by the entity were never purged either (blind, edge). | high | patch | Confirmed against hydration's `delete_kind_detail_row`, which is now reused (made `pub(crate)`) rather than re-implemented. Constraints purged; `relations` deliberately still not, since `--fix-ids` redirects those next. |
| 4 | Entries planned but never reached after an abort appeared in neither `renumbered` nor `skipped` (blind, edge). | medium | patch | Confirmed. The remaining plan entries are now folded into `skipped`. |
| 5 | The third writer, `renumber_duplicate_file`, still called `determine_entity_kind` directly instead of the new `kind_for_write` — so "one rule, one place" was not actually reached (blind). | medium | patch | Confirmed. Now routes through `kind_for_write` like the other two. |
| 6 | `relate`'s adoption of the frontmatter-kind rule was pinned by nothing: every fixture in the relate suite has agreeing kinds, and deleting the call left the suite green (verification-gap, pre-verified). | medium | patch | Accepted, and the reviewer's suggested fixture **did not work** — a story-invalid file never hydrates, so `relate` refuses at the cache precheck (exit 2) before reaching the write. Rebuilt via the stale-row path: hydrate valid, then edit invalid. Mutation now fails the test. |
| 7 | Resolving the update writer's kind from *patched* rather than existing content — the only reason to prefer patched — was untested (verification-gap, pre-verified). | medium | patch | Accepted. New `--field kind=story` test; mutation to `existing_content` now fails it. |
| 8 | `FixIdsEntry.path` was retained as a duplicate of `old_path` "so existing consumers keep working" — but after the rename it names a deleted file (edge, filed as a claim). | medium | patch | Confirmed. Removed rather than documented: a field that reliably names a nonexistent path is worse than an absent one, and there are no external consumers pre-1.0. |
| 9 | A dangling symlink at the rename target passed the `exists()` guard, so the write would follow the link instead of refusing (edge). | medium | patch | Confirmed by inspection; `symlink_metadata` now, matching `create_story`'s check. |
| 10 | `rel_path` carrying backslash separators would defeat `rsplit_once('/')` and write the renamed file into the workspace root (edge). | medium | patch | Confirmed by inspection. Normalised first. |
| 11 | The text-mode `Renamed` line was asserted by no test, though the repo asserts text-mode stdout elsewhere (verification-gap, pre-verified). | low | patch | Accepted. Assertion added to the `--yes` text-mode test. |
| 12 | `doctor.rs`'s doc comments still said four computed checks of eight after a fifth was added — the one place that exists so `doctor` cannot describe a different check set than `validate` (blind, edge). | low | patch | Confirmed. Corrected. |
| 13 | Every `--fix-ids` test renumbers a Story, so the non-Story path — where the kind-dependent purge differs — was uncovered (blind). | low | patch | Confirmed. New ADR renumber test covering rename, writability and a clean workspace afterwards. |
| 14 | `wrote_anything` is set before the write is attempted, so it means "attempted", and the reconcile runs after a pure refusal (blind, edge). | low | rejected | True as stated, and deliberate: a failure *inside* `renumber_duplicate_file` can have written the renamed file already, which is exactly the state the reconcile exists to make coherent. Running it after a no-op refusal costs one sweep. The comment says so; the flag name is imprecise, which is not worth a rename. |
| 15 | A partial relation or citation rewrite is reported as `[]`/`0` — the same reporting gap as the H3 abort this story fixes (blind, edge). | medium | defer | Real. The fix changes `rewrite_relations_to`'s signature to carry partial progress out of the error path, which is more than a patch, and no state is lost — only the report is incomplete. Filed. |
| 16 | The planner can allocate an id whose canonical filename is already occupied, so the user confirms a rename that must then be refused (blind, edge). | medium | defer | Real, and now the only way to reach the refusal at all (ids in use are already skipped). Making the planner check filename availability turns a refusal into a repair. Filed. |
| 17 | `skipped` now has three causes (declined, unparseable, attempted-and-failed) with one undocumented shape, and `relations_rewritten`/`citations_rewritten` can read as zero when they are merely unknown (blind). | low | defer | Confirmed; filed with 15, which is the same reporting fix. |
| 18 | L7: a file outside every entity directory is still readable but not writable — reported by the new check, not fixed (implementer's own note, blind). | medium | defer | Correct and expected: fixing it needs cache-driven resolution, which the frozen decision (option B) rules out. The new `warning` names the file and the path it should have. Filed. |
| 19 | `kind_for_write` can promote a non-story id to `Story`, yielding an entity with no `stories` detail row (edge). | false | rejected | Hydration's `determine_entity_kind` has the identical default, so a write and the sweep still agree — which is this story's invariant. Not introduced here, and changing the default would change hydration. |
| 20 | `renamed_file_name` slices `current_name[old_id.len()..]`, which could panic on a non-char boundary (edge). | low | patch | Not reachable — ids are ASCII by grammar, so a case-insensitive match has equal byte length — but hardened to `get(..).unwrap_or("")` since it costs nothing. |

### Review pass adjustments (2026-09-10)

- `crates/qdev-core/src/store/sqlite.rs`'s `delete_kind_detail_row` became `pub(crate)` so the
  write path's purge reuses hydration's kind mapping instead of carrying a second copy — the
  duplication that produced this whole seam.
- `find_file_in_dir_for_id` is a new public wrapper over the private `find_file_in_dir`, so the
  rename pre-flight asks exactly the question the write path will ask later.
- `FixIdsEntry.path` was removed (see triage row 8), and `payload-fix-ids.json` updated.

## Design Notes

- The cache already holds the answer. `EntityRecord::source_path` is written by hydration for every entity and read by nothing on the write path — option A is less "new machinery" than it first looks.
- `--fix-ids` is the sharp end of this seam twice over: it is the only writer that changes an id, and the only one that resolves kind correctly. Both defects it exhibits are the seam, not the command.
- The abort defect is two lines and independent of the identity question: `renumbered.push` happens after two fallible steps, so the reconcile's guard is testing the wrong condition. It should be fixed whichever option is chosen, and its test should assert through `qdev get` rather than by reading the cache, so it survives a change of resolution rule.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- Manual, the reproduction from the cross-story review: two files declaring `E1S1`, `qdev validate --fix-ids --yes`, then `qdev update <new-id> --status ready` and `qdev relate <new-id> depends_on E1S1` — both must succeed

**Results (2026-09-10):**
- `cargo test --workspace` — 33 suites, 473 tests, 0 failures.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo fmt --all --check` — clean.
- Manual reproduction, release binary in a fresh workspace: two files declaring `E1S1`;
  `qdev validate` reports both `duplicate_planning_id`s; `qdev validate --fix-ids --yes
  --non-interactive --json` renumbers `E1S1.md` to `E1S2` **and renames it to
  `docs/specs/stories/E1S2.md`**, reporting `old_path`/`new_path`; `qdev update E1S2 --status
  ready` and `qdev relate E1S2 depends_on E1S1` both exit 0; `qdev get E1S1` (the keeper) exits
  0; the cache holds exactly two rows, `E1S1 -> E1S1-copy.md` and `E1S2 -> E1S2.md`; a final
  `qdev validate` reports `findings: []` at exit 0. Adding `docs/specs/misc/E1S9.md` then yields
  exactly one `warning`-severity `entity_file_off_convention` finding naming the expected path,
  still at exit 0.
