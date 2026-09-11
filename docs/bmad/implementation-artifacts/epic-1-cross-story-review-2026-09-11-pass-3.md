# Epic 1 Cross-Story Review — Pass 3, 2026-09-11

Re-run of the acceptance gate after the nine stories that followed [pass 2](epic-1-cross-story-review-2026-09-10-pass-2.md):
`61cebe7` identity resolution, `64d600e` one id-in-use rule, `015ed2e` change gate, `e6b1772`
stale means absent, `c00eb9a` relation contracts, `c9fefb9` init cache migration, `024a8cb` hex
allocators and invariant coverage, plus `60f509d` and `1089d41` from the pass-1 set.

**Method.** The same shape as pass 2 — four seam reviewers (boot/config/cache, write paths side by
side, finding and cache-state lifecycle, shared contracts), plus the agent whose only job is to
**re-run every previous finding's own reproduction** against the built binary, plus a record
auditor over the ledger, the retrospective items, the story records and the documented claims.
Every reviewer was told not to assume the recent work correct. I re-ran the most severe claims
myself before recording them; each finding below says who observed it.

A note on the run: the record auditor and three seam reviewers were killed mid-flight by an API
session limit and were relaunched in smaller waves. Two of the auditor's sub-audits had completed
and their results are folded in; the rest of the audit I did myself with targeted greps.

**Verdict: epic 1 does not close on this pass either.**

Fourteen of pass 2's reproductions now produce the right answer, including all four of the
highs the three invariant stories were written for. But:

1. **Two pass-2 blocking findings — NEW-4 and NEW-8 — were never scheduled and never filed.**
   They appear in no spec and in no `deferred-work.md` entry. Both still reproduce unchanged.
   This is the *same* tracking failure pass 2 named for H8 and M1, one pass later, by me.
2. **The identity fix introduced a new defect on the platforms this project is developed on**: an
   entity `qdev get` returns is refused by `qdev update` with a message naming a file that does
   not exist.
3. **The change-gate fix has a hole that turns a broken workspace into a clean bill of health** —
   `touch` a file carrying merge-conflict markers and `qdev validate` exits 0 with no findings,
   permanently, while a rebuild of the same tree exits 1. That is the epic's load-bearing
   invariant failing, in the story written to establish it.
4. **A fifth derivation site reads a stale row**, so `qdev get` reports a story unblocked by a
   dependency `qdev validate` calls dangling — a hole in the third invariant story too.
5. Twenty-odd further defects across the four seams, a dozen of them user-reachable without
   contrivance, including read-only commands failing with a write-lock conflict.

---

## Part 1 — Did pass 2's findings get fixed?

Every row re-run against the binary at `024a8cb`, each in its own workspace.

| ID | Verdict | Evidence |
|---|---|---|
| H8 `--if-version` not a fence on `relate` | **FIXED** | `relate … --if-version 99` on an existing edge → exit 5 `version_mismatch`; `unrelate` accepts the flag and fences too. |
| M1 `unrelate` accepts an unknown relation name | **FIXED** | exit 2 `usage_error` listing all eight names, from both `relate` and `unrelate`, before any other verdict. |
| NEW-1 `init` cannot migrate a populated cache | **FIXED** | v1 cache holding a hydrated story → `init` exit 0, `cache_migrated: true`, `cache_files_rehydrated: 2`, `user_version` 3, rows intact. No FK error. |
| NEW-2 migration gate is decorative | **FIXED** | The gate is gone; `init` without `--yes` migrates at exit 0, as boot does. |
| NEW-3 `create story` mints ids another file carries | **PARTIAL — and changed shape** | The allocator half is fixed (nested `E1S1.md` → allocator issues `E1S2`, no duplicate). Its lowercase-filename variant is *worse*: see P3-2. |
| NEW-4 `kind` edit permanently breaks convergence | **NOT FIXED — never scheduled, never filed** | `update E1S1 --field kind=epic` → `entities.kind = epic` with the `stories` detail row surviving every sweep; `sync --rebuild` drops it. User-visible: `qdev list epics` returns an `epic_id` before a rebuild and none after. |
| NEW-5 `orphan_deferred_work` treats a stale row as present | **FIXED** | Merge-conflicted origin story → both paths report `merge_conflict` + `orphan_deferred_work`. |
| NEW-6 sweep misses a file unreadable without an mtime change | **FIXED** | `chmod 000` with mtime and size untouched → `read_error`, exit 1, from both paths. |
| NEW-7 same-size edit inside one wall-clock second | **FIXED** | Both halves — the `status:` edit lands, the id edit re-keys the row with no ghost, and findings converge. |
| NEW-8 unreadable directory silently purges what it owned | **NOT FIXED — never scheduled, never filed** | `chmod 000` on a story directory → rows purged, `sync` reports `purged: 0`, `validate` exits 0 on the emptied cache. And worse than pass 2 knew: see P3-3. |
| Med `--fix-ids` exits 0 where `validate` exits 1 | NOT FIXED (filed, review-by is this gate) | Workspace with a `merge_conflict`: `validate` exit 1, `validate --fix-ids --yes` exit 0. |
| Med `update --field schema_version=99` duplicates an envelope key | NOT FIXED (filed) | Payload carries `schema_version` twice. |
| Med `init` outside the advisory-lock protocol | **FIXED** | With the lock held, `init` waits and exits 5 `lock_timeout` on the same file, like every other mutator. |
| Med `./`-prefixed layout values | NOT FIXED (filed) | `.gitignore` gets `./cache/`; `git status` shows the live cache untracked-but-committable; `gates/` lands at the repo root. |
| Med `--fix-ids` renames within the current directory | **FIXED** | Off-convention *directory* target is refused, named, and reported in `skipped`. |
| Med `payload-doctor.json` says "four computed checks" | NOT FIXED | There are five; the binary serves the stale text. |
| Unsettled 1 `.MD` extension | **PARTIALLY SETTLED — and it is a defect** | The off-convention warning now fires. But the write path resolves `.MD` on a case-insensitive filesystem and writes a cache row naming a path no file has: see P3-6. |
| Unsettled 2 stale entity's child rows | Unchanged, benign for findings | `constraints` rows are retained with no staleness signal; findings still converge. |
| Unsettled 3 Windows path separators | Not testable here | Needs a Windows host. |
| Pass-1 spot checks H2, H4, H5 | **Still FIXED** | Renumbered entity writable; `dangling_relation` from both paths; duplicate deletion resolves to the survivor. |

**Tally: 14 fixed, 1 partial, 6 unchanged, 1 untestable.**

## Part 2 — The tracking failure, again

Pass 2's disposition said its eight high and medium-high findings were **blocking for epic 1
acceptance and not deferred**. Six of them were turned into stories. NEW-4 and NEW-8 were not, and
neither was filed in `deferred-work.md`:

```
$ grep -rn "NEW-4\|NEW-8" docs/bmad/implementation-artifacts/*.md
docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md   # only the pass-2 doc itself
```

Pass 2 wrote, about H8 and M1: *"A finding that is triaged, named, and then dropped is worse than
one never found: the record says it was handled."* The same sentence applies here, to me, one pass
later. The remedy pass 2 proposed — a `Review by:` on every ledger entry — does not help a finding
that never reaches the ledger. **What was missing is a closing step that reconciles a review's
disposition against what was scheduled**, before the review is called done. Both findings are
filed now, with the gate as their review-by point.

---

## Part 3 — New findings

### P3-1 A content-preserving touch erases a `merge_conflict` and un-stales the row, permanently — HIGH
*Reported by the finding-lifecycle reviewer; re-run and confirmed by me.*

The sweep's hash-unchanged branch treats *content unchanged since the last hydration* as *the file
is healthy*: it clears every finding for the path and clears `stale`
(`crates/qdev-core/src/store/sqlite.rs:3356-3372`). But the last hydration may have **failed**, in
which case the stored hash is the hash of the broken content. Once cleared, the row is
"accounted for", so no later sweep re-parses the file.

```
qdev sync            # after planting <<<<<<< HEAD in E1S1.md
qdev validate        # [error] merge_conflict, exit 1, entities.stale = 1
touch docs/specs/stories/E1S1.md
qdev sync
qdev validate        # findings: [], exit 0, entities.stale = 0, findings table empty
head -1 …/E1S1.md    # <<<<<<< HEAD        (still there)
qdev sync --rebuild
qdev validate        # [error] merge_conflict, exit 1
```

`qdev doctor` agrees with the false answer. The `schema_violation` variant is worse: the stale row
is un-staled and served as live, so `qdev get` returns pre-edit content with `"stale": false` for a
file that no longer has a title. Reached by any content-preserving metadata change: `touch`,
`chmod`, `git checkout -- .`, `git stash pop`, an editor save-with-no-change, `cp -p`. A CI job
that checks out and runs `qdev validate` is exactly the shape that gets the false pass.

The clear-and-unstale belongs only to the case the comment was written for — a previous
`read_error`, where the file could not be read at all. It is wrong for a path whose stored hash is
the hash of content that failed to parse. This is a hole in `015ed2e`, my own story.

### P3-2 An entity `get` returns is unwritable, refused with a phantom duplicate — HIGH
*Reported by the reproduction re-runner as NEW-3's changed shape; re-run and confirmed by me.*

`find_file_in_dir` (`crates/qdev-core/src/write.rs`) probes `dir.join("<id>.md").is_file()` and
then also scans the directory. On a case-insensitive filesystem — macOS and Windows, i.e. the
platforms this is developed on — a single `e1s1.md` satisfies both: the probe resolves it under
the canonical spelling, the scan finds it under its real one, and the two `PathBuf`s differ as
strings. The result is a refusal naming a file that does not exist:

```
$ ls docs/specs/stories            →  e1s1.md          (one file)
$ qdev get E1S1                    →  exit 0
$ qdev update E1S1 --field status=ready
usage_error: Multiple entity files match ID 'E1S1' in '…/docs/specs/stories': ["E1S1.md", "e1s1.md"]
exit 2
$ qdev validate                    →  no finding for it
```

Introduced by `61cebe7`, the story written to make reads and writes agree about identity. It is
pass-1 H2's class — an entity readable but unwritable — reached through a new door.

### P3-3 An unreadable directory purges its entities, reports a clean workspace, and then the allocator hands out the id it just erased — MEDIUM-HIGH
*Reported by the finding-lifecycle reviewer; re-run and confirmed by me.*

Extends NEW-8. `collect_markdown_files` swallows a failed `read_dir`, so the sweep purges the rows
and `validate`/`doctor` report nothing. Because the row is gone, the *cached* half of `ids_in_use`
no longer covers those ids either:

```
chmod 000 docs/specs/stories/sub          # sub/E1S3.md lives here
qdev validate                              # findings: [], exit 0   (E1S3's row purged)
qdev create story E1 --title collide       # allocates E1S3 → docs/specs/stories/E1S3.md
chmod 755 docs/specs/stories/sub
qdev validate                              # duplicate_planning_id ×2, exit 1
```

This also falsifies a sentence I wrote in the ledger yesterday — that with a store present the
cache half covers an unreadable path. It does for an unreadable *file* (the row is retained
stale); it does not for an unreadable *directory* (the row is purged).

### P3-4 `--fix-ids` keeps a group member no writer can resolve, and reports success — HIGH
*Reported by the write-path reviewer; re-run and confirmed by me.*

The keeper guard enforces only the *directory* half of the identity rule
(`off_convention_directory_target`, `crates/qdev-cli/src/main.rs:2657`), never the filename half.
So when the lexicographically-first file in a duplicate group carries no id in its name, `--fix-ids`
renumbers the convention-compliant file and leaves the id owned by the unresolvable one:

```
cp docs/specs/stories/E1S1.md docs/specs/stories/A-copy.md    # both declare E1S1
qdev validate                     → exit 1 (duplicate_planning_id ×2)
qdev validate --fix-ids --yes     → "Renumbered E1S1.md (E1S1 -> E1S3)", exit 0
qdev get E1S1                     → exit 0
qdev update E1S1 --status ready   → usage_error: Entity file not found for 'E1S1', exit 2
qdev validate                     → exit 0     (warning only)
```

The repair reports success while creating the state its own story exists to remove, and the
workspace then validates clean. Any copy named to sort before `<id>.md` reaches it.

### P3-5 `update --field relations=…` writes exactly the edges `relate` refuses — HIGH
*Reported by the write-path reviewer; re-run and confirmed by me.*

`relate` refuses `dependency_cycle`, `dangling_relation` and `invalid_relation_kind` before
writing. `--field` has no such gate — its deny list is `id`/`version`/`updated_by`/`created_by`
(`crates/qdev-cli/src/main.rs:890`) — so the same three edges go in at exit 0:

```
qdev relate E1S2 depends_on E1S1  → dependency_cycle …
qdev relate E1S1 depends_on E9S9  → dangling_relation …
qdev update E1S2 --field 'relations={"depends_on": ["E1S1", "E9S9"]}'   → exit 0
qdev validate                     → dependency_cycle ×2, dangling_relation, exit 1
```

Unknown relation *names* are caught, because the schema is `additionalProperties: false`. The leak
is exactly the three guards that need the graph, which `update` never consults.

### P3-6 `.MD` resolves for writers on macOS and Windows but not on Linux, and the warning that names it is factually wrong — MEDIUM-HIGH
*Reported by the write-path reviewer.*

`find_file_in_dir`'s `.is_file()` probe hits `E1S7.MD` on a case-insensitive filesystem, so
`update` and `relate` succeed and report the path `docs/specs/stories/E1S7.md`, a spelling no file
has; the cache row is written with that path and corrected by the next sweep. `filename_carries_id`
requires a literal `.md`, so `validate`'s warning says the name "does not carry that id" — on the
platform where the write path just resolved it. Same root as P3-2. The Linux half (not-found for an
entity `get` returns) follows from the code but was not observed here.

### P3-7 A corrupt cache heals on every command's boot, but makes `init` — the repair path — exit 4 — HIGH/MEDIUM
*Reported by the boot-seam reviewer.*

`ensure_cache` treats an inspection error as "rebuild" and deletes the file
(`crates/qdev-core/src/store/sqlite.rs:3817`, `:3844`); `init` propagates the same error with `?`
(`crates/qdev-core/src/init.rs:209`, `:506`). So `c9fefb9`'s reconciliation holds for `Valid`,
`Mismatch` and `NewerThanSupported`, and not for `Err`:

```
printf 'junk' > .qdev/cache/cache.sqlite
qdev list stories   → exit 0 (rebuilt)
qdev init …         → exit 4  sqlite_error: Failed to read user_version: file is not a database
```

Reached by a truncated cache from a crash, a full disk or an interrupted copy — then running the
idempotent repair command.

### P3-8 `init` discards `--developer`/`--team` whenever `.qdev.local.toml` already exists — MEDIUM
*Reported by the boot-seam reviewer.*

`init` refuses to run without those flags, writes them into `qdev.toml`'s `[teams]`, but writes
`[identity]` only when the local file is absent (`crates/qdev-core/src/init.rs:317`). Hand-write a
`.qdev.local.toml` first — which the CLI reference documents as an editable file, and which the
"relocate your cache, then re-run `init`" workflow produces — and the effective identity silently
falls back to `git config user.email`, which is then what attribution records.

### P3-9 `init` from a subdirectory scaffolds the enclosing repo, and the human output never says where — MEDIUM
*Reported by the boot-seam reviewer.*

`root` is `find_workspace_root(cwd)`, so `qdev init` in `repo/sub/deep` initializes `repo/`, and
the text renderer prints only relative paths. In a git-tracked `$HOME`, `qdev init` in any new
directory scaffolds `$HOME`. Neither `architecture.md` §9 nor `cli-reference.md` §7 states which
root `init` picks. (The JSON payload does carry `root`.)

### P3-10 The fresh-cache branch of `init` leaves an empty cache and reports no rehydration at all — LOW/MEDIUM
*Reported by the boot-seam reviewer.*

`cli-reference.md` promises `cache_files_rehydrated` as the signal that a rebuild "read nothing".
That number exists only on the migration branch; delete the cache file — which the
`schema_version_mismatch` message itself advises — and `init` prints `✔ cache schema v3` over a
cache with zero rows and `cache_files_rehydrated: null`. The next command's boot sweep repopulates
it, so this is a false all-clear rather than lost data. Noted against my own reporting work in
`c9fefb9`.

### Smaller, all reproduced
- **`--fix-ids` aborts mid-repair on an off-convention relation source**, dropping the reference
  redirect it promised: the renumber lands, the edge still points at the old id — which is now a
  *different* entity — and the next `validate` is clean.
- **`--field Status=ready` appends a dead second key** (`status:` and `Status:` both present, no
  reader honours the second) while the CLI's own flag-conflict guard is case-insensitive.
- **`update` bumps the version for a no-op write; `relate`/`unrelate` do not.** An agent treating
  "exit 0, version unchanged" as "already in the desired state" gets opposite answers.
- **Corrupt or truncated evidence `.json` and scratch `.jsonl` files produce no finding on either
  path** and are counted as `parsed`.
- **Child rows keyed by anything but the entity id can be stolen or lost**: two evidence files
  declaring one `id` leave the sweep and a rebuild disagreeing about which gate run is current
  (`pass` vs `fail`), and deleting one loses the row entirely with no finding.
- **`_QDEV_MOCK_TTY=1` is a shipped test backdoor** in interactivity resolution: the release binary
  can be made to prompt on a pipe.
- **`specs_dir = "."` is accepted**, after which the sweep walks the whole repo and every stray
  `.md` (`README.md`, `node_modules/**`) becomes an error-severity `schema_violation`.
- **A nested `.git` still shadows the workspace** (pass 2's M6, filed as out of scope): reads
  refuse, `create story` writes into the shadow. New detail: when the nested repo sits inside
  `specs_dir`, the result is a duplicate id in the *outer* workspace, and both runs print the same
  relative path so the user cannot tell which tree was written.

### P3-11 `blocked` is derived from a stale row, so `qdev get` says go where `qdev validate` says the dependency does not exist — MEDIUM-HIGH
*Reported by the contracts reviewer; re-run and confirmed by me.*

`crates/qdev-core/src/query.rs:146` computes `blocked` with a bare `store.get_entity(...)` — the
exact call `entity_exists_for_derivation` exists to replace. It is a fifth derivation site, and
`spec-stale-means-absent.md` inventoried four:

```
# E1S1 done, E1S2 depends_on E1S1, then E1S1's frontmatter made unparseable
qdev validate     → schema_violation on E1S1, dangling_relation on E1S2 ("targets 'E1S1', which does not exist"), exit 1
qdev get E1S1     → stale: true, status: done
qdev get E1S2     → blocked: false
rm E1S1.md; qdev sync
qdev get E1S2     → blocked: true
```

So the next-work decision and the validator disagree about the same edge, and
`architecture.md`'s "a stale row is treated as absent by everything that derives" is false at this
site. Hole in `e6b1772`, my own story.

### P3-12 Read-only commands take the write lock, so `qdev doctor` — "a report, never a gate" — exits 5 — MEDIUM-HIGH
*Reported by the contracts reviewer; confirmed by me for `doctor` and `validate`.*

Every command's boot runs `ensure_cache`, whose healthy path sweeps
(`crates/qdev-core/src/store/sqlite.rs:3858`), and `sweep_workspace` takes the advisory write lock
unconditionally (`:2966`). With the lock held by another process, `qdev doctor --json` and
`qdev validate --json` each stall 5 s and exit 5 `lock_timeout`, naming a write lock the read never
asked for and offering no remedy. `cli-reference.md:70` attaches `lock_timeout` and exit 5 to the
write path only, and `:549` calls `doctor` a report that never gates. Reached by exactly the
concurrency the tool exists to support: one agent syncing while another reads.

### P3-13 `--fix-ids` can exit 1 while its payload names no finding — MEDIUM
*Reported by the contracts reviewer.*

`crates/qdev-cli/src/main.rs:2573` re-runs `run_validation` and returns exit 1, but
`payload-fix-ids.json` has neither a `findings` array nor an `error` on that path, and text mode
prints only `Renumbered …`. A CI consumer gets a logical failure with a success-shaped payload.
This is the opposite branch from the known exit-0 case at `:2197` — the command has two exit bugs,
in opposite directions.

### Contracts, smaller — all reproduced by the reviewer
- **`qdev graph` refuses `--json`** (exit 2), contradicting AD-13 and `cli-reference.md:214`
  ("every command accepts `--json`"); the refusal is undocumented.
- **`qdev schema payload` covers 7 of the 13 payload shapes the binary emits** — nothing for
  `init`, `config show`, `status`, `--version`, `create story`, `update`, `relate`, `unrelate` —
  and `qdev get <entity>/<constraint>` returns a second shape that fails the only `get` schema
  with seven errors. AD-13 says `qdev schema` prints the schema for any payload.
- **"That entity does not exist" comes back as three codes across two exit codes**
  (`entity_not_found` exit 2, `usage_error` exit 2, `dangling_relation` exit 1); only the last is
  documented. Partly covered by the filed not-found-taxonomy entry, but wider than that entry says.
- **`update`/`unrelate` report "Entity file not found" for a file that exists and is readable**
  (an off-convention name), while `validate` names both the file and the rename that would fix it.
- **The CLI reference's own `create story` example** (`--module bridge`) produces a workspace that
  immediately fails `qdev validate` with `target_module_not_registered`, because `init` writes no
  `[[modules]]` and `create` validates against the schema but not the registry.
- **`payload-fix-ids.json` documents two causes for `skipped`; the code has six**, and three of
  them print their reason only in text mode, so a JSON consumer gets a bare path.
- **`qdev status` is undocumented and its text and JSON modes share no content**;
  **`qdev --json --help` writes non-JSON to stdout**; **`graph --epic` silently drops cross-epic
  edges**; **`--file` without `--section` is ignored** while the converse is a clear error;
  **the constraint projection is `owner_id` in JSON and `owner` in text**.

---

## Part 4 — What this says, and what would close the epic

Pass 2 diagnosed the pattern as *fixing the instance rather than the class*, and the three
invariant stories that followed did fix classes: NEW-5, NEW-6, NEW-7 and the allocator half of
NEW-3 are all closed, and the convergence invariant now has real tests behind it. That worked.

What this pass exposes is a different failure mode, and it is not a coding one:

- **Two blocking findings evaporated between the review and the schedule.** The review's own
  disposition is not a work list until someone reconciles it against the ledger and the sprint.
- **Three of the new highs are holes in the fixes themselves** (P3-1 in the change gate, P3-2 and
  P3-6 in the identity rule, P3-4 in `--fix-ids`), and every one of them was reachable by the
  reviewers' first or second attempt. Per-story review passed all of them, because per-story review
  checks the reproduction the story names.
- **The two write paths still disagree** (P3-5): `relate` guards the graph and `update --field`
  does not, so the epic's relation contract is enforced at one door of two.
- **All three invariant stories have exactly one hole each** — the change gate (P3-1), the identity
  rule (P3-2/P3-6), stale-means-absent (P3-11) — and in each case the hole is a site the story's
  own inventory did not list. The lesson is not "write better inventories": it is that an
  invariant needs a mechanical way to enumerate its sites, or the next site added will not know
  the rule exists.

Before a pass 4 is worth running, I would want: NEW-4, NEW-8 and P3-1 through P3-5 scheduled as
work with owners; a single rule for "which file holds entity X" that is decided by the identity
rule rather than by the filesystem's case behaviour (P3-2, P3-6 are one defect); and a closing step
in the review process that reconciles disposition against ledger and sprint before a pass is
called done.

## Record corrections made during this gate

- The retrospective's action table had nine of twelve items substantively done but unstruck; each
  now carries its commit and evidence.
- `validate::ids_in_use`'s rustdoc and `architecture.md` claimed no reservation protocol was needed
  "while every allocator's caller writes under the advisory write lock". False, and mine: both
  callers allocate *outside* the lock (`main.rs:820` then `write.rs:2017`; `main.rs:2299` then
  `:2405`). Corrected in both places and in the ledger entry that repeated it.
- The unidentified test flake now has a verification command that preserves failure output; ten
  consecutive clean full-suite runs since, four of them under load. Still open, still unidentified.

---

## Part 5 — The schedule

Seven stories, keyed in `sprint-status.yaml` at `backlog`, each owning named findings. Every
blocking ledger entry names the key that owns it, so the disposition, the ledger and the sprint
say the same thing — the reconciliation whose absence dropped NEW-4 and NEW-8.

Each story is stated as **the rule it establishes**, not the bug it fixes. That is pass 2's lesson
applied: the three invariant stories that stated a rule closed their classes, and each still has
exactly one hole at a site its inventory did not list — so every story below must also say *how its
sites are enumerated*, not just fix the ones named here. Specs are written by `bmad-build` when the
story starts.

### 1-24 `identity-answers-from-the-rule` — owns P3-2, P3-6
**Rule: which file holds entity X is decided by the identity rule, never by the filesystem's case
behaviour.** `find_file_in_dir`'s `dir.join("<id>.md").is_file()` probe delegates the question to
the OS, which on macOS and Windows answers for spellings no rule admits — producing a phantom
"Multiple entity files match" for a lone `e1s1.md`, and resolving `E1S7.MD` for writers while
`validate` says that name carries nothing. Enumerate the directory once and judge every entry by
the rule; two matches must mean two files, which means comparing canonical paths (or identity, not
spelling). Sites are enumerable because there is one resolver — that is the point of the story.
Also settles what `validate`'s off-convention warning may claim, since today it is wrong about
`.MD` on the platform this is developed on, and gives `update`/`unrelate` a message that names the
file and the rename rather than "Entity file not found" for a file that exists.

### 1-25 `unchanged-is-not-healthy` — owns P3-1
**Rule: a file whose content has not changed is unchanged, not healthy — the sweep may clear only
findings whose cause it has re-checked.** Today the hash-unchanged branch clears every finding for
the path and un-stales the row, so `touch` on a file holding merge-conflict markers makes
`qdev validate` exit 0 forever while a rebuild exits 1. The clear-and-unstale was written for a
previous `read_error`, where the file could not be read at all; it is wrong for a stored hash that
is the hash of content that failed to parse. The distinction to encode: findings that mean *I could
not look* versus findings that mean *I looked and it was broken*. Enumeration: the finding codes
are a closed set in `SCHEMA_DDL`'s writers, so the story must classify all six.

### 1-26 `a-kind-change-repairs-itself` — owns NEW-4
**Rule: after any write, the cache holds what a rebuild of the same tree would hold.** A `kind` edit
through the write path overwrites the cached kind before hydration's repair can compare against it,
and the previous kind's detail row survives every sweep — `qdev list epics` answers differently
either side of a rebuild. `delete_entity_row_shallow` already knows the kind→detail-table mapping;
the write path must use it. Enumeration: the mapping is one table, so the test is that every kind
with a detail table round-trips a change to every other.

### 1-27 `fix-ids-leaves-every-entity-resolvable` — owns P3-4 *(after 1-24)*
**Rule: `--fix-ids` may not finish with an id owned by a file no writer can resolve, and what it
reports is what it did.** The keeper guard enforces only the directory half of the identity rule, so
a copy named to sort before `<id>.md` is kept, the compliant file is renumbered, and the run exits 0
reporting success. Depends on 1-24 for the resolution question. Folds in the two filed exit-code
defects — exit 0 where `validate` exits 1, and exit 1 with a payload naming no finding — and the
mid-repair abort that drops the reference redirect it promised, leaving an edge pointing at what is
now a different entity.

### 1-28 `an-unreadable-directory-is-a-finding` — owns NEW-8 / P3-3
**Rule: a directory qdev cannot read is a finding, not an empty directory.** `collect_markdown_files`
and `scan_duplicate_planning_ids` both swallow `read_dir` failures, so the sweep purges the rows of
everything under an unreadable directory, `validate` and `doctor` report a clean workspace, and the
allocator — whose cached half those rows were — then hands out an id that is taken. Purge must not
treat "not seen" as "deleted" when the walk itself failed. Closes the two filed scan-blindness
entries with it.

### 1-29 `one-gate-for-relation-writes` — owns P3-5
**Rule: an edge enters the workspace through one gate, whichever command writes it.**
`update --field relations=…` writes the cycle, dangling-target and invalid-kind edges `relate`
refuses, at exit 0. The three guards need the graph, which `--field` never consults. Enumeration:
route the generic field writer's `relations` value through the same validation `relate` uses, so a
future fourth guard cannot be added to one door only. Folds in the case-variant `--field Status=`
key, the same shape at a smaller scale.

### 1-30 `derivation-sites-are-enumerable` — owns P3-11
**Rule: "a stale row is absent for derivation" is enforceable, not remembered.** `query.rs`'s
`blocked` is a fifth site reading a bare `get_entity`, so `qdev get` says go where `qdev validate`
calls the same dependency dangling — after a story whose whole subject was this rule, with an
inventory of four. A fourth manually-found site would be the same bug again, so this story owes a
mechanism: renaming the read to `get_entity_including_stale` (already filed as an option), a clippy
`disallowed-methods` entry, or a type that cannot be read without answering the question. The choice
is a trade recorded in the ledger; the story's job is to make it.

**Sequencing.** 1-24 first (1-27 depends on it). 1-25, 1-26 and 1-30 are independent and touch the
cache lifecycle, so running them together risks conflicts in `sqlite.rs` — sequence them. 1-28 and
1-29 are independent of everything else.

**What closes the epic.** All seven `done`, then a pass 4 whose reproduction agent re-runs
pass 3's findings *and* pass 2's, since two of pass 2's survived a pass. The gate closes when a pass
finds no blocking defect and no dropped finding — not when the list happens to be short.
