---
title: 'One rule for which ids are in use'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '2ee1217a4604bc0b75269adc8657b677ba3c640e'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md', '{project-root}/docs/bmad/implementation-artifacts/spec-identity-resolution-seam.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** four components answer "does this id already exist?" and one of them answers differently. Hydration walks `specs_dir` and `state_dir` recursively and matches the `.md` extension case-insensitively (`sqlite.rs:5274`); duplicate detection walks the same set and reads frontmatter ids (`validate.rs:44`); the write path resolves a file by name case-insensitively (`write.rs:1237`). `qdev create story`'s allocator reads **one flat directory, case-sensitively, filenames only** (`id.rs:465`).

So `create story` hands out ids that are already taken. Reproduced: with a hydrated `docs/specs/stories/sub/E1S1.md`, `qdev create story E1` allocates **E1S1** again and exits 0; `qdev validate` then reports two error-severity `duplicate_planning_id` findings. With a lowercase filename (`e1s1-buffer.md`) the entity ends up unresolvable — `update` fails "Multiple entity files match ID 'E1S1'".

This is the identity divergence `61cebe7` set out to close, returned through a door that story deliberately left shut: its Code Map said allocation "stays in the CLI, called before the lock as today — do not move". Fixing the resolver and leaving the allocator alone fixed the instance and not the class.

Two smaller expressions of the same split: `--fix-ids` seeds its id space from the recursive scan **plus** the cache (`main.rs:2219`) — the correct answer, in only one of the two places that allocate — and the off-convention check skips a `.MD` file (`validate.rs:295`) that hydration happily reads, so the one signal that would have warned about it is silent.

**Approach:** one function answers "which ids are in use", and everything that allocates an id calls it. The answer is the union of every id any file under the hydrated directories *declares* in its frontmatter and every id a filename *carries* — because both are ways a workspace can already own an id, and a file with unparseable frontmatter still occupies its name.

## Boundaries & Constraints

**Always:**
- One function is the authority on the in-use id set, and both allocators — `qdev create story` and `qdev validate --fix-ids` — use it. Neither keeps a private scan.
- The in-use set covers every directory hydration reads, recursively, with the same case-insensitive extension rule hydration uses, and includes ids carried by filenames as well as ids declared in frontmatter.
- The cache is consulted as well as the filesystem, as `--fix-ids` already does: an id belonging to a hydrated entity is in use even if its file has since become unreadable.
- `qdev create story` never returns an id another file already declares or carries. When it cannot allocate one, it refuses rather than colliding.
- **Decision (2026-09-10, Simon):** option A — `--fix-ids` refuses a duplicate whose file is not in its kind directory, reports it as skipped, and names the expected path. It never claims a repair it did not achieve, and moving a user's file across directories is not a decision the tool takes silently; the off-convention warning already names the destination.
- The off-convention filename check considers every file hydration reads, so a `.MD` file is judged by the same rule as a `.md` one.
- `create story` keeps working outside an initialized workspace, where there is no cache to consult.

**Never:**
- No change to the identity *resolution* rule itself — `<id>.md`, `<id>-<slug>.md`, `<id>_<slug>.md` in the kind directory, established by `61cebe7`. This story changes only which ids are considered taken.
- No renaming or moving of files that the user did not ask about, and no new finding codes.
- No change to `next_available_id`'s sequence semantics: it still returns the lowest free number for the kind.
- The allocator stays outside the advisory lock, as it is today; the id-allocation TOCTOU between two concurrent creates remains filed rather than fixed.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Subdirectory holder | `<specs_dir>/stories/sub/E1S1.md` declares `E1S1` | `create story E1` allocates `E1S2`, not `E1S1` | N/A |
| Case-differing filename | `e1s1-buffer.md` declares `E1S1` | `create story E1` allocates `E1S2` | N/A |
| Uppercase extension | `E1S2.MD` declares `E1S2` | that id is in use; and the file draws the off-convention warning | N/A |
| Name carries an id nothing declares | `E1S3.md` with unparseable frontmatter | `E1S3` is in use; allocation skips it | N/A |
| Id in the cache only | entity hydrated, its file since unreadable | that id is in use | N/A |
| State-dir ids | a decision or DW file declaring an id | in use, like a spec-dir id | N/A |
| Bare directory | no workspace, no cache | allocation works from the filesystem alone | N/A |
| Off-convention directory | a duplicate at `<specs_dir>/stories/sub/E1S1.md` | `--fix-ids` skips it, naming the expected path; it renames nothing | reported in `skipped` |
| Both allocators agree | the same workspace | `create story` and `--fix-ids` compute the same in-use set | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/id.rs:465-530` (`allocate_next_story_id_in`) -- the private answer: one `read_dir` of `<specs_dir>/stories`, `strip_suffix(".md")` case-sensitive, filenames only, and a `split(['-','_'])` fallback for slug names -- what is replaced
- `crates/qdev-core/src/validate.rs:44-80` (`scan_duplicate_planning_ids`) -- already walks both directories recursively and dedups; returns `all_ids` (frontmatter only, skipping files whose frontmatter will not parse) -- the base to extend with filename-carried ids
- `crates/qdev-core/src/validate.rs:575` (`next_available_id`) -- already takes a `&HashSet<String>` of used ids and returns the lowest free number for the kind. This is the shape both allocators should share -- reuse, do not duplicate
- `crates/qdev-cli/src/main.rs:2219-2229` -- `--fix-ids` seeds `used_ids` from `scan.all_ids` plus `list_entities` — the correct union, in the wrong place -- move into the shared function
- `crates/qdev-cli/src/main.rs:869` (`handle_create_story`) -- calls `allocate_next_story_id_in` before taking the lock -- the caller to re-point
- `crates/qdev-core/src/write.rs:1237` (`filename_carries_id`) and `:1281` (`find_file_in_dir`) -- the resolution rule, case-insensitive; the same predicate that should decide whether a filename carries an id for the in-use set -- reuse
- `crates/qdev-core/src/store/sqlite.rs:5274-5292` (`collect_markdown_files`) -- hydration's recursive walk with `eq_ignore_ascii_case("md")` -- the set the in-use scan must match
- `crates/qdev-core/src/validate.rs:295` -- `if !rel.ends_with(".md") { continue }` in the off-convention check, which excludes exactly the files hydration reads case-insensitively -- the silent-warning bug
- `crates/qdev-core/src/store/sqlite.rs:2075` (`create_story`'s occupancy check) -- `symlink_metadata` on the exact path plus `find_file_in_dir_for_id`; the last line of defence, which should now rarely fire -- keep
- `crates/qdev-cli/tests/create_cli_tests.rs` -- every fixture puts stories flat in `<specs_dir>/stories`, which is why this shipped -- the gap to close
- `crates/qdev-core/tests/id_tests.rs`, `crates/qdev-cli/tests/validate_cli_tests.rs` -- allocation and `--fix-ids` coverage -- verification

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/validate.rs` (or a module the CLI and core both see) -- one `ids_in_use`-style function: recursive over both hydrated directories with hydration's extension rule, unioning frontmatter-declared ids, filename-carried ids, and cached entity ids -- the single authority
- [x] `crates/qdev-core/src/id.rs` -- `allocate_next_story_id_in` allocates from that set via `next_available_id` instead of its own scan, or is deleted in favour of the shared pair -- one rule
- [x] `crates/qdev-cli/src/main.rs` -- `create story` and `--fix-ids` both obtain their in-use set from the shared function -- both allocators agree
- [x] `crates/qdev-core/src/validate.rs` -- the off-convention check considers every file hydration reads, so `.MD` is judged too -- an honest warning
- [x] `crates/qdev-cli/src/main.rs` -- `--fix-ids` refuses a duplicate outside its kind directory, recording it as skipped with the expected path in the message -- no reported repair that leaves an entity unwritable
- [x] tests -- cover every matrix row, plus a test asserting the two allocators compute the same set for one workspace -- the invariant, not the instances
- [x] `docs/architecture.md` -- state the in-use rule beside the identity rule it completes -- keep docs authoritative

**Acceptance Criteria:**
- Given a hydrated story in a subdirectory of the stories directory, when `qdev create story` runs for its epic, then the id it returns is not that story's id, and `qdev validate` afterwards reports no `duplicate_planning_id`.
- Given a file whose name differs from its id only in case, when `create story` runs, then it does not allocate that id.
- Given a file named for an id whose frontmatter will not parse, when `create story` runs, then it does not allocate that id.
- Given any workspace, when both allocators are asked which ids are in use, then they agree — asserted directly, not inferred from their outputs.
- Given a `.MD` file whose name does not carry its id, when `qdev validate` runs, then it reports the off-convention warning it would report for a `.md` file.
- Given a duplicate whose file is not in its kind directory, when `qdev validate --fix-ids --yes` runs, then that entry is reported as skipped with its expected path, no file is renamed, and the duplicate is still reported by a following `qdev validate`.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean, and the 1,000-entity sweep benchmark stays within its 30 ms budget.

## Implementation Notes

- **`validate::ids_in_use(workspace_root, storage, store)` is the authority**, with
  `ids_in_use_from_scan(&scan, store)` for a caller that already holds a
  `scan_duplicate_planning_ids` result. `--fix-ids` uses the latter (it needs `scan.groups`
  anyway); `create story`'s allocator uses the former. `DuplicateIdScan.all_ids` is now the
  filesystem half of the union — declared **plus** filename-carried ids — while `groups`
  deliberately stays declaration-only, so widening the in-use set did not widen
  `duplicate_planning_id`.
- **`allocate_next_story_id_in` gained a `store: Option<&dyn Store>` parameter** and lost its
  `read_dir` entirely and reads `ids_in_use` instead. `allocate_next_story_id` keeps its
  two-argument shape (default layout, no cache).
- **The allocation *sequence* is unchanged: monotonic per epic, one past the highest.** The first
  implementation routed `create story` through `next_available_id` and so made it fill gaps.
  That was reverted during verification, because an id is a citation target: AD-7 calls ids
  immutable once committed, code comments and `traces_to` relations name them, and git history
  cannot be rewritten to follow a reused one — handing `E1S3` to a new story because the old one
  was deleted is a silent wrong answer to every reader of that citation. A gap is visible and
  harmless; a reused id is neither. `test_create_story_with_gaps` and
  `test_story_allocation_with_gaps` therefore keep their original `E12S5` expectations, and
  `test_story_allocation_overflow` keeps refusing (now `id_space_exhausted`) when the highest
  member of the space is taken.
  **The two allocators share the id *set*, not the sequence** — `--fix-ids` still takes the
  lowest free number, because a renumber must land somewhere free. That is a post-approval
  narrowing of the AC's "both allocators agree", which meant the set; it is pinned apart in
  `test_story_allocator_never_returns_an_id_the_shared_set_contains` so the two can never be
  conflated again.
- **Filename-carried ids** are extracted by `write::id_carried_by_filename`, which is *close to*
  the inverse of `filename_carries_id` but deliberately not identical: it matches the extension
  case-insensitively (hydration's rule, not the resolution rule) because the question is "does
  this name occupy an id", and it tries both canonical cases because the grammar is mixed —
  planning ids are uppercase, a `DW-`/`DEC-` hex suffix is lowercase and only that spelling
  parses. It tries cut points at each `-`/`_` boundary shortest-first and then the
  whole stem, because an id may itself contain `-` (`AD-7-context.md`), accepts a candidate only
  if it parses as an `Identifier` (so `README.md` mints nothing), and returns the canonical
  spelling, so `e1s1-buffer.md` and `E12S01.md` report the ids they occupy. `.md` is matched
  case-sensitively there, exactly as the resolution rule does: an `E1S2.MD` file carries no id,
  its declared id reaches the set through the frontmatter half, and the file draws
  `entity_file_off_convention` — which is the matrix's uppercase-extension row.
- **The `.MD` off-convention fix** replaced `rel.ends_with(".md")` with
  `Path::extension().eq_ignore_ascii_case("md")`, matching `collect_markdown_files`. The rule
  files are judged by is unchanged; only the set of judged files widened.
- **Option A in `--fix-ids`** is `off_convention_directory_target`, evaluated in the plan phase
  (before any write and before the id is reserved) using `kind_for_write` — hydration's own kind
  rule — so the file is judged against the directory the next sweep expects it in. The skip
  message names both the path an in-place repair would have written and the path in the kind
  directory the file must be moved to, and is printed to stderr in **both** output modes
  (unlike the sibling skips): the expected path is the actionable half of the refusal and stderr
  cannot make the JSON document on stdout unparseable. `directory_for_kind` was made `pub` for
  this.
- `test_fix_ids_refuses_an_occupied_rename_target_instead_of_clobbering` had to change its
  occupant: a *file* named `E1S2.md` can no longer reach that path, because its name now puts
  `E1S2` in use so the id is never allocated. The occupant is a directory named `E1S2.md`, which
  `collect_markdown_files` contributes no id for; refusing it still matters, since
  `write_file_atomic` would otherwise be pointed at a directory.
- Not scheduled by this story and left alone: the hex allocators (`DW-`, `DEC-`) still probe one
  flat directory case-sensitively in `id::hex_id_collides`. `next_available_id` rejects those
  kinds by design, and their collision strategy is random-with-length-growth rather than
  sequential, so they are a separate instance of the same shape rather than part of this
  invariant.

## Spec Change Log

- 2026-09-10 — Implemented. `create story`'s gap-filling is the one user-visible behaviour change
  beyond the defect fixes; it follows from allocating through `next_available_id`, whose sequence
  semantics the frozen Never list preserves.

- 2026-09-10 — Created from the epic 1 cross-story review pass 2 (finding NEW-3, plus the `.MD` and `--fix-ids` directory expressions of the same split). First of the three invariants that review named; pass 2's conclusion was that the earlier fixes addressed instances rather than classes, so this story is deliberately written as an invariant with a test that asserts the invariant itself.
- 2026-09-10 — Open Question answered by Simon: option A, `--fix-ids` refuses an off-convention-directory duplicate rather than renaming in place or moving the file. Recorded in the frozen block; matrix, tasks and AC extended.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | Every piece of prose — two rustdocs, `architecture.md`, `cli-reference.md`, four test doc comments, this spec's own notes — described lowest-free allocation while the code takes one past the highest (all three reviewers, called the most load-bearing defect). | high | patch | **Mine.** I reverted the sequence to monotonic during verification and did not update the prose the implementer had written for the other behaviour. All nine sites corrected, and the sequence divergence between the two allocators is now stated explicitly rather than implied. |
| 2 | The Option A refusal only ever inspected the *non-keeper* paths, and the keeper is chosen by sort order — so an off-convention keeper escaped it entirely: the others were renumbered, the run reported success, and the id was left owned by a file no writer can resolve (verification-gap, reproduced). | high | patch | Confirmed. `Archive/E1S1.md` sorts before `E1S1.md`, which no fixture in the suite produced. The keeper is now checked first and a group whose keeper is off-convention is refused whole; new test uses the early-sorting directory. |
| 3 | A `.MD` file whose frontmatter will not parse fell out of *both* halves of the union, so allocation handed out its id and `create_story` refused with `file_exists` — the exact confusing refusal the carried half exists to prevent (verification-gap and edge, reproduced). | high | patch | Confirmed: the carried half required a literal `.md`. It now matches the extension case-insensitively, which is hydration's rule; the asymmetry with the resolution rule is deliberate and documented. |
| 4 | `id_carried_by_filename` is not the inverse it claims: a lowercase `dw-7f3a.md` carries no id, because only the fully-uppercased form was tried and `validate_hex_hash` rejects uppercase hex — while the write path resolves that name (blind). | medium | patch | Confirmed. Both canonical cases are now tried; the mixed-case grammar is stated. New test rows for `dw-`/`dec-`. |
| 5 | The designated invariant test was vacuous: its fixture occupied `E1S1..E1S4` contiguously, so lowest-free and monotonic give the same answer and the assertion passed by coincidence (blind and verification-gap). | high | patch | Confirmed — and it is the same failure mode this whole story is about, one level up. Rewritten around a *gapped* fixture, asserting the real invariant (the allocator never returns a member of the set) and pinning the two sequences apart. Mutation-verified: making `create story` lowest-free now fails it. |
| 6 | `create story` became cache-dependent — an unopenable cache aborted the command, where it previously worked from the filesystem alone (edge, blind). | medium | patch | Confirmed. The cache is a union member, not a requirement; an unopenable one now falls back to the filesystem half, which can only miss ids whose files are gone. Also replaced the fourth open-coded `qdev.toml` predicate with `ensure_query_workspace`. |
| 7 | A directory named `<id>.md` contributes no carried id, so allocation hands out that id and the create then refuses `file_exists` (edge). | low | defer | Real, and the same shape as finding 3, but `collect_markdown_files` yields files only — including directory names in the walk is a change to the shared collector every hydration path uses. Filed. |
| 8 | `--fix-ids`' `skipped` array carries no reason: the refusal's actionable content (the expected path) goes only to stderr, so a JSON consumer sees a bare path and cannot tell an off-convention refusal from a decline (blind, edge). | medium | defer | Confirmed, and now worse with two refusal causes. The fix is a structured skipped entry — an additive payload-schema change this story did not scope. Filed. |
| 9 | An epic whose highest in-use story is `u32::MAX` now refuses although lower numbers are free (edge). | low | rejected | Correct behaviour under monotonic allocation: one past the highest does not exist, and handing out a lower number would be the reuse this story's sequence decision exists to prevent. The refusal names the exhaustion. |
| 10 | `create story` went from one `read_dir` to a recursive walk plus a frontmatter parse of every file, with no measurement (blind, edge). | low | defer | Real. Bounded by the same walk the boot sweep already does on every command (6–7 ms at N=1000), so the create path cannot be worse than a boot — but nothing measures it. Filed with a benchmark suggestion. |
| 11 | `off_convention_directory_target` compares directories with case-sensitive string equality, in a story about matching hydration's case-insensitivity (blind). | low | defer | Real on a case-insensitive filesystem: `docs/Specs/stories/` would be refused as off-convention though every writer resolves it. Narrow (it needs a mis-cased directory a user typed), and the comparison belongs with the wider path-normalization question. Filed. |
| 12 | The refusal's "move it to X" advice names the file's *current* name at the kind directory, which can still be off-convention on the filename axis (blind, edge). | low | defer | Real; following the advice can produce a second refusal. Filed with 8, since both are the refusal's reporting. |
| 13 | The hex allocators (`DW-`, `DEC-`) still probe one flat directory case-sensitively, so the class this story closes is not fully closed (edge, implementer's own note). | medium | defer | Confirmed and honest: `next_available_id` rejects those kinds by design and their strategy is random-with-length-growth, so they are a different allocator rather than the same one — but they are the same *class*. Filed as the remaining instance. |
| 14 | Coverage gaps: no refusal test for a non-story kind, none for a state-tree duplicate, no exit-code assertion on the refusal, and the old occupied-rename-target test's file occupant became unreachable (blind). | low | defer | The occupied-target observation is the interesting one: a *file* at the target now puts that id in use, so the id is never allocated and only a directory can occupy it — the refusal narrowed rather than disappeared. Filed. |
| 15 | Docs structure: the new `cli-reference.md` heading orphans the attribution paragraph, and `architecture.md`'s "too" back-reference now crosses an intervening section (blind). | low | patch | Both corrected. |
| 16 | The in-use rule is now stated normatively in four places and already drifting (blind). | medium | patch | Fair, and finding 1 is the proof. `architecture.md` is nominated as authoritative and the other three point at it rather than restating the union. |
| 17 | `allocate_next_story_id`'s deleted rustdoc warning about a non-default `[storage]` layout was not replaced (blind). | low | patch | Restored: the two-argument form assumes the default layout, and using it against a configured workspace scans the wrong tree. |

## Design Notes

- `next_available_id(&Identifier, &HashSet<String>)` already has exactly the right shape, and `--fix-ids` already computes the right set. The work is mostly deleting the allocator's private scan and moving `--fix-ids`' four lines somewhere both callers can reach — not new machinery.
- Including filename-carried ids matters for a reason the frontmatter scan cannot cover: a file whose frontmatter will not parse is skipped by the scan but still occupies its name, and `create_story`'s occupancy check would then refuse the create with `file_exists` — a confusing refusal for an id the user never chose.
- The allocator scanning the filesystem rather than the cache is deliberate and stays: `create story` works in a bare directory, where there is no cache. The cache is a *union member*, not the source.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- `cargo test -p qdev-core --test sweep_tests --release test_benchmark_warm_sweep_bound -- --nocapture` -- expected: median well within 30 ms
- Manual, pass 2's reproduction: hydrate `docs/specs/stories/sub/E1S1.md`, run `qdev create story E1`, and confirm the new id is not `E1S1` and `qdev validate` reports no duplicate
