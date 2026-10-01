# Epic 1 Cross-Story Review — Pass 2, 2026-09-10

Re-run of the acceptance gate after the three root-cause fixes the [first pass](epic-1-cross-story-review-2026-09-10.md)
required: `61cebe7` (identity resolution), `1089d41` (sweep/rebuild convergence), `60f509d` (init
configuration). The first pass set re-running itself as the condition for closing epic 1.

**Method.** The same four seam reviewers as pass 1 — boot path, write paths side by side,
finding/cache-state lifecycle, shared contracts — each told which seams the three commits had
disturbed and instructed not to assume the recent work correct. Plus a fifth agent whose only job
was to re-run each of pass 1's sixteen findings' own reproductions against the built binary.

**Verdict: epic 1 does not close on this pass either.** Six of the eight high findings are
verified fixed by reproduction. But one was never scheduled, one is partially fixed, and the
re-run found **eight further high or medium-high defects, four of them holes in the three fixes
just shipped**.

## Part 1 — Did pass 1's findings actually get fixed?

| ID | Verdict | Evidence |
|---|---|---|
| H1 `init` ignores local `[storage]` | **PARTIAL** | Config-before-`init`: one database, gitignored. The review's own order (`init`, then relocate) still leaves two databases with the live one committable, repaired only by re-running `init`. The newer-cache refusal also still does not name *which* cache file, which its AC required. |
| H2 `--fix-ids` produces unwritable entities | **FIXED** | Renumbered entity accepts `get`, `update`, `relate` with no manual step. |
| H3 `--fix-ids` abort loses the keeper | **FIXED** | Forced abort reports the write that landed; keeper present in the cache, inbound edge intact, converges with a rebuild. |
| H4 purge destroys other files' edges | **FIXED** | Sweep reports `dangling_relation` and matches a rebuild. |
| H5 deleting a duplicate erases the survivor | **FIXED** | `get` resolves the survivor; both deletion orders converge. |
| H6 rebuild swallows an unreadable file | **FIXED** *(for the reported reproduction)* | `validate` reports `read_error` and exits 1; `doctor` agrees. See NEW-6 for the neighbour that does not. |
| H7 computed findings from stale rows | **FIXED** *(for the reported reproduction)* | Sweep reports only `merge_conflict`, matching a rebuild. See NEW-5 for the probe that was missed. |
| H8 `--if-version` is not a fence on `relate` | **NOT FIXED — and never scheduled** | Reproduces verbatim: exit 0, `changed:false`, no version check. `unrelate` rejects the flag outright. Absent from `deferred-work.md` and from all three fix specs. |
| M1 `unrelate` accepts an unknown relation name | **NOT FIXED — review-by elapsed** | Reproduces. Its ledger entry said "review by: with the identity-resolution story"; that story landed and never touched it. |
| M2 duplicate ids in `state_dir` | **FIXED** | Reported by both paths. |
| M3 writers vs hydration kind rule | **FIXED** | `update` exits 1; next boot records no finding. |
| M4 not-found taxonomy | NOT FIXED (properly deferred) | Filed, review by epic 3 story 3.2. |
| M5 malformed file, five answers | NOT FIXED (properly deferred) | Same ledger entry. One new wrinkle: `relate`'s answer now depends on cache state. |
| M6 `.git` shadow workspace | Deliberately out of scope | Named in the init story's "Never"; filed. |
| M7 `init` swallows an unparseable config | **FIXED** | Exit 2, nothing scaffolded, same refusal as every command. |
| M8 `doctor` exits 5 on lock contention | NOT FIXED (properly deferred) | Filed, review by first release. |

**Two of these are my own process failures, not code defects.** Pass 1's disposition said all eight
highs were blocking and named H8 and M1 as "two isolated contract defects" beside the three root
causes — then three stories were written for the three root causes and H8 and M1 were never
scheduled or filed. M1's ledger entry pointed at a story that shipped without touching it. A
finding that is triaged, named, and then dropped is worse than one never found: the record says it
was handled.

## Part 2 — New findings

Four of these are holes in the fixes just shipped, which is the more important fact about them.

### NEW-1 `qdev init` cannot migrate any populated cache — HIGH
`drop_all_user_tables` protects itself with `PRAGMA foreign_keys = OFF` (`sqlite.rs:3535`), which is
a **no-op inside a transaction**. `ensure_cache` calls it outside one, so it works; `init` wraps it
in `BEGIN IMMEDIATE` (`init.rs:563`), so it does not. Re-verified by me: a cache holding one story,
stamped to v1, then `qdev init --yes` → `sqlite_error: Failed to drop table entities during
migration: FOREIGN KEY constraint failed`, cache left at v1. The very next `qdev list` migrates it
successfully. Every migration fixture in the suite uses an *empty* cache, which is why it passes.
Pre-existing, and it affects every real workspace upgrading a binary.

### NEW-2 The migration confirmation gate is decorative — HIGH
`init` demands `--yes` before a cache migration (exit 3), and every other command performs the same
drop-and-rebuild silently at boot. Two specs contradict each other directly: story 1.3 requires the
confirmation, story 1.6 requires the unconditional boot rebuild. So the gate protects nothing, and
per NEW-1 it is broken in the one place it fires.

### NEW-3 `create story` mints ids another file already carries — HIGH, and it reopens H2's class
The allocator (`id.rs:465`) reads one flat directory, case-sensitively; every other component
resolves identity recursively and case-insensitively (`sqlite.rs:5274`, `write.rs:1237`,
`validate.rs:66`). Re-verified by me: with a hydrated `docs/specs/stories/sub/E1S1.md`,
`qdev create story E1` allocated **E1S1** again at exit 0, and `qdev validate` then reported two
error-severity `duplicate_planning_id` findings. A reviewer's lowercase-filename variant leaves the
entity unwritable (`Multiple entity files match ID 'E1S1'`). The identity story fixed the *resolver*
and left the *allocator* alone — its Code Map said so explicitly — so the same divergence returned
through the other door.

### NEW-4 A `kind` edit through the write path permanently breaks convergence — HIGH
Hydration repairs a kind change by comparing the freshly parsed kind against the **cached** kind
(`sqlite.rs:4577`); the write path's upsert has already overwritten the cached kind and does not
clear the old kind's detail row (`write.rs:1071`). So after `qdev update <id> --field kind=epic`, the
`stories` row survives forever, no sweep repairs it, no finding is recorded, and only a full rebuild
differs. `delete_entity_row_shallow` in the same file already knows this rule.

### NEW-5 `orphan_deferred_work` treats a stale row as present — HIGH, hole in H7's fix
The stale exclusion was applied to the checks' own rows but not to the existence probe
`store.get_entity(origin)?.is_some()` (`validate.rs:160`), which returns stale rows. So a
merge-conflicted origin story suppresses the finding on the sweep path and a rebuild reports it.
This contradicts the sentence added to `architecture.md:430` by that same story: "a stale row is
treated as *absent* by everything that derives a finding".

### NEW-6 The sweep never notices a file that became unreadable without an mtime change — HIGH
`chmod 000` on a hydrated story: `qdev validate` → `{"findings": []}`, exit 0; `sync --rebuild` →
`read_error`, exit 1. The new `unaccounted` gate does not help — the file still has a row, so it is
"accounted for". This falsifies `architecture.md:426` verbatim: "An unreadable file produces a
`read_error` finding from both paths. Neither swallows it." Both tests for it move mtime first.

### NEW-7 A same-size edit inside one wall-clock second is permanently invisible — HIGH
`file_mtime_size` truncates to whole seconds (`sqlite.rs:3814`) and the `unaccounted` gate asks only
whether *a* row claims the path, never whether it agrees with the file — though `sync_state`
and `entities` both store a content hash. Reproduced by a reviewer by hand and hit twice by a
randomized mutation run: a same-length `status:` edit never lands, and a same-length **id** edit
leaves a ghost row for an id no file declares, answered by `qdev get`, with a spurious
`entity_file_off_convention` that never clears. Machine-speed agent edits land inside one second.

### NEW-8 An unreadable *directory* silently purges everything it owned — MEDIUM-HIGH
`collect_markdown_files` swallows a `read_dir` failure and yields zero files (`sqlite.rs:5278`), and
the purge set is `known_paths - disk_paths`. `chmod 000` on `docs/specs/stories` empties every row
and finding for those entities, `qdev sync` prints `purged: 0`, and `validate` exits 0 on the empty
cache. Transient rather than permanent — restoring the directory re-hydrates — but nothing reports
it, and both paths agree so the convergence test cannot see it.

### Medium
- **`validate --fix-ids` exits 0 on a workspace `validate` exits 1 on**, when there are no duplicates to fix (early return at `main.rs:2198`) — a CI self-healing step goes green on error findings.
- **`update --field schema_version=99` emits a duplicated envelope key.** `MANAGED_FIELDS` blocks `id`/`version`/`updated_by`/`created_by` but not `schema_version`, and the payload is flattened into the envelope. The doctor serializer guards against exactly this; the update payload does not.
- **`init` is the only cache mutator outside the advisory-lock protocol** — reported independently by two reviewers. No wrong end state constructed; the invariant is simply unenforced there.
- **`./`-prefixed layout values defeat the normalization just added.** `cache_dir = "./cache"` yields `.gitignore` entries git cannot honour (live cache committable, verified with `git status`), and `parent_dir_of("./cache")` puts `gates/` at the repo root where `"cache"` puts it under `.qdev` — the same value written two ways, two layouts.
- **`--fix-ids` renames within the file's current directory**, ignoring the directory half of the identity rule, so it reports a repair that leaves the entity unwritable — the state its own story says the rename removes.
- **`payload-doctor.json` still says "four computed checks"**; there are five. The doctor doc comments were corrected, the schema description was not.

### Unsettled
- A `.MD` extension: hydration accepts it case-insensitively, the off-convention check skips it, and `filename_carries_id` requires a literal `.md`. On a case-sensitive filesystem the write path would report not-found for an entity `get` returns; not reproducible on macOS.
- Retained child rows of a stale entity (`constraints`) carry no staleness signal, unlike `entities.stale`. No finding derives from them, so findings still converge.
- Windows path separators: `update`/`relate` store `source_path` unnormalized where hydration normalizes; the reviewer expects an extra re-parse rather than data loss and could not test it.

## What this says about the approach

Pass 1 found eight high defects; the three stories fixed six and the re-run found eight more. That
is not a converging loop, and the reason is visible in which findings recurred: **each fix addressed
the instance the review reproduced rather than the class it belonged to.**

- Identity resolution was made consistent in the *resolver*, and the *allocator* — explicitly left
  alone by that story's Code Map — reopened the same divergence (NEW-3).
- Convergence was established against four reproductions, and three further divergence sources
  remained: the kind-change repair (NEW-4), the stale existence probe (NEW-5), and the change gate's
  presence-only test (NEW-6, NEW-7).
- Layout normalization was centralized in the loader, and a value shape it does not normalize
  (`./`) still produces two layouts.

Three of those classes have a single-sentence statement that would settle them, and none of the
three fix stories stated it: *one resolution rule, used by everything that resolves or allocates an
id*; *the change gate compares content, not presence*; *stale means absent, enforced by one query
helper rather than by remembering at each site*.

The verification agent's method is also worth keeping: re-running each finding's own reproduction
found the unscheduled H8 and the elapsed M1 that no amount of code review would have surfaced,
because the defect was in the tracking, not the code.
