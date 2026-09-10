# Epic 1 Cross-Story Review — 2026-09-10

Run before closing epic 1, per retrospective action item 12 ("run a cross-story review before
closing an epic; per-story review is structurally blind to seam defects").

**Scope.** Epic 1's shipped code at `761bbd8`, including the eleven commits landed after the
2026-09-09 retrospective, which no cross-story pass had seen.

**Method.** Four independent reviewers, each given one *seam* rather than one story, and each
instructed to look only for defects that appear where two stories' work meets — inconsistent
rules, drifted duplicate logic, an invariant one side maintains and the other violates. Seams:
the boot path; every write path side by side; the finding/cache-state lifecycle; and the shared
contracts (exit codes, JSON envelope, `[storage]` threading) checked across commands rather than
within one. Reviewers ran the built binary rather than reasoning from the source alone.

**Result: 20 findings, 8 of them high severity, every one of the 8 reproduced against the
binary.** Four were re-verified independently by the reviewing session (marked ✅ below). Nothing
here was visible to per-story review, and the full test suite is green on all of it.

## High — reproduced, and each one loses or corrupts user-visible state

### H1 ✅ `init` ignores `[storage]` in `.qdev.local.toml`, so the effective cache is committable

`init::resolve_storage` (`crates/qdev-core/src/init.rs:42-65`) reads only `qdev.toml`, while the
config loader merges `[storage]` from both files (`config/mod.rs:893-911`) and `ensure_cache` is
handed the merged value (`main.rs:166`). `[storage]` is legal in the local file — the same
allowed-section list applies to both.

Re-verified: after `qdev init`, adding `[storage] cache_dir = "local/cache"` to
`.qdev.local.toml` and running any command yields **two** databases; `.gitignore` lists only
`.qdev/cache/`, so the cache every command actually uses is untracked-and-committable. That is
the exact failure `resolve_storage`'s own doc comment says it exists to prevent. Worse,
`check_cache_status` also resolves through it, so with the effective cache stamped newer, every
command exits 5 while `qdev init` reports success — and the refusal's advice ("delete the cache
file") names the wrong file.

Seam: story 1.2 (dual configuration) meets stories 1.3/1.6 (init, cache). Open product question:
whether `[storage]` in the local file should be legal at all. Either answer closes it.

### H2 ✅ `--fix-ids` produces entities its own write path cannot touch

Write paths resolve an entity by *filename* (`write.rs:1054`, `:1107` — `<id>.md`, `<id>-*`,
`<id>_*`); every read path resolves by frontmatter `id` (`sqlite.rs:4387`), with the exact
`source_path` sitting unused in the cache. `--fix-ids` rewrites the frontmatter `id` and never
renames the file, so it *manufactures* the divergence.

Re-verified: two files declaring `E1S1`; `qdev validate --fix-ids --yes` renumbers
`E1S1.md` → `E1S2`. Afterwards `qdev get E1S2` works, while `qdev update E1S2` and
`qdev relate E1S2 …` both fail `usage_error: Entity file not found for 'E1S2'`. The entity is
permanently unwritable through the CLI; only a manual `mv` fixes it. The tool's own remedy for
the duplicate-id condition leaves the workspace half-broken.

### H3 `--fix-ids` aborts lose the keeper entity from the cache entirely

`renumbered.push` happens only after the relation *and* citation steps (`main.rs:2316`), and the
reconcile that repairs the cache is gated on `!renumbered.is_empty()` (`:2333`) — despite the
comment at `:2329` promising it runs even when the loop aborted. The relation-rewrite step fails
whenever a *referencing* entity's filename ≠ its id, which is precisely what H2 creates.

Reproduced: after an abort, `"renumbered": []` was reported while the first file had already been
renumbered on disk; because the list was empty the reconcile never ran, and `E1S1` vanished from
the cache permanently — `get` says `entity_not_found`, `validate` reports `findings: []`, the
inbound `depends_on` edge is silently gone. Repeated `qdev sync` never repairs it (the file is
unchanged and not dirty); only `sync --rebuild` does.

### H4 The purge cascade deletes rows owned by other, unchanged files — and they are never restored

Purge is id-keyed and cascading (`sqlite.rs:3112`, `purge_entity_with_children` at `:4141` deletes
`relations WHERE source_id = ?1 OR target_id = ?1`); re-hydration is path-keyed and change-gated
(`:3131`). Purging path P therefore deletes rows belonging to files *other* than P, and nothing
re-parses those files.

Reproduced: `E1S2.md` declares `depends_on: [E1S1]`; delete `E1S1.md`. `qdev validate` reports
`findings: []` and `qdev graph --dot` shows no edge — the edge no longer exists, so no
`dangling_relation` finding *can* be produced. `qdev sync --rebuild` on the identical tree
reports the finding. That contradicts the convergence contract asserted at `sqlite.rs:406` and
`:3675` ("both converge on identical findings/stale state").

### H5 Deleting one of two duplicate-id files erases the survivor's entity

Same root cause as H4, worse outcome. Two files declaring id X collapse to one row whose
`source_path` is the alphabetically last; deleting *that* file purges X wholesale, while the other
file — still on disk, unchanged — is skipped, and the per-path finding clear removes the evidence.

Reproduced: `qdev get E1S1` → `entity_not_found`, `qdev validate` → `findings: []`, with
`E1S1.md` untouched on disk. The natural user response to `duplicate_planning_id` (delete the
copy) silently deletes the real entity and reports a clean workspace.

### H6 `validate` exits 0 on an unreadable spec file, on the first command after any rebuild

Rebuild swallows an unreadable file (`sqlite.rs:492`, `Err(_) => continue`); the sweep records a
`read_error` finding for the same failure (`:3142`). `ensure_cache` takes one branch or the other,
never both, and rebuild truncates `findings` first — so the command that triggered the rebuild
sees an emptied, unrepopulated findings table.

Reproduced: `chmod 000` a story file, delete the cache, then `qdev validate --json` →
`{"findings": []}` exit 0, and `qdev doctor --json` immediately after → `read_error: 1`. Two
consecutive commands, same workspace, opposite answers. Reachable on a fresh clone, a corrupt
cache, a binary upgrade, and via `sync --rebuild`.

### H7 Computed checks read `stale` cached rows, so a computed finding outlives the defect

`run_validation`'s four checks read `list_entities` with no `stale` predicate (`validate.rs:195`,
`:123`, `:152`), while hydration's contract on a parse failure is "retain the previous row, flag
it stale" — pre-edit content.

Reproduced: a story with `target_modules: [bogus]` reports `target_module_not_registered`
correctly; edit the file to remove `target_modules` and introduce a merge-conflict marker, and the
finding persists against a file that no longer says it. `sync --rebuild` on the identical tree
reports only `merge_conflict`. This is the flip side of the deliberate read-only design: not
persisting computed findings stops them outliving a *sweep*, but nothing stops them being computed
from stale inputs.

### H8 ✅ `--if-version` is not a fence on `relate`/`unrelate`

`apply_relation_change` returns `Ok(changed: false)` before it ever calls `patch_frontmatter`
(`write.rs:2045`), so the version is never compared; `apply_entity_update` has no such
short-circuit and always checks (`write.rs:1320`, `:435`).

Re-verified: on a story at version 2 with the edge already present,
`qdev relate E1S1 depends_on E1S2 --if-version 99 --json` exits **0** reporting
`changed: false, version: 2`, while `qdev update … --if-version 99` exits 5 `version_mismatch`.
An agent using `--if-version` as a compare-and-swap fence reads that exit 0 as "my expected
version was current" when it was not.

## Medium

- **M1 ✅ `unrelate` with a misspelled relation name exits 0 and leaves the edge.** Re-verified:
  `qdev unrelate E1S1 dependson E1S2` → exit 0, `changed: false`, real `depends_on` edge intact.
  `relate` reports the same input as `invalid_relation_kind` (exit 1) with a factually wrong
  message ("not an allowed kind pair" — it is not a relation). Every other enum-valued argument in
  the binary is an exit-2 usage error. `dag.rs:14-27`, `main.rs:1485`, `main.rs:1605-1630`.
- **M2 `duplicate_planning_id` scans `specs_dir` only; hydration scans `specs_dir` *and*
  `state_dir`** (`validate.rs:38` vs `sqlite.rs:2922`). Every id collision among sprints, DW,
  decisions, releases and SOUP is invisible — and then triggers H5 when one file is deleted.
- **M3 Schema validation before a write uses a different kind rule than hydration uses after it.**
  Writers validate against the kind from `resolve_entity_file` (directory, then grammar); hydration
  prefers frontmatter `kind:` (`sqlite.rs:4402`). `--fix-ids` is the only writer that gets this
  right, and its comment names the invariant the others break. Reproduced: `update` exits 0, and the
  next boot records an error-severity `schema_violation` on the file it just wrote.
- **M4 "Entity does not exist" has three codes and two exit classes** — `entity_not_found`/2,
  `usage_error`/2, and `dangling_relation`/**1** for a `relate` *target*. One mistyped id lands in
  "your invocation was wrong" or "the workspace has a defect" depending on which argument it was.
- **M5 A malformed entity file gets five different answers from five commands** (`update` → 1
  `missing_frontmatter`, `relate` → 2 "not found", `get` → 2 `entity_not_found`, `sync` → 0 with a
  finding, `validate` → 1 `schema_violation`). The `relate` answer is actively misleading.
- **M6 `find_workspace_root` accepts `.git` as a root marker while the guards require `qdev.toml`**
  (`config/mod.rs:1194` vs `main.rs:1035`), and `create story` is exempt from the guard. Inside a
  real workspace, a submodule or vendored clone becomes a shadow root: `list` exits 2 there while
  `create story` exits 0 and writes a duplicate id no sweep will ever see.
- **M7 `init` treats an unparseable `qdev.toml` as absent; every other command treats it as fatal**
  (`init.rs:44-49` vs `config/mod.rs:1216`). The tool's own remedy reports success on a workspace no
  command can use, and scaffolds the default layout over a configured one.
- **M8 `doctor` exits 5 on lock contention**, contradicting `cli-reference.md:407`'s "a report,
  never a gate" — a doctor run overlapping any concurrent write becomes a conflict exit. (Boot's
  sweep takes the lock; adjudicated as intended for *reads* by spec 1.7, but the doctor promise
  post-dates that and is now wrong.)

## Low

- **L1 `create_story`'s cache predicate is weaker than the boot cache guard** (`write.rs:1807` vs
  `main.rs:165`): with `qdev.toml` absent but a cache present, the story write reaches the cache
  with no version check.
- **L2 `init` reports hardcoded default paths** in `created_files` (`init.rs:346`) and its text
  output (`main.rs:736-738`) regardless of the configured layout — a wrong path in a
  machine-readable payload.
- **L3 `payload-fix-ids.json` does not describe the `error` field the payload can carry**; and
  `cli-reference.md` contradicts itself on which payload schemas exist (7 vs 3) and claims every
  command accepts `--json` when `graph` does not.
- **L4 `qdev sync`'s `findings=` count is cache-native only**, with none of the escape hatch the
  cache doctor section now documents.
- **L5 `Store::delete_entity` is not the purge cascade** (`sqlite.rs:878` vs `:4105`) — latent, no
  caller outside tests, but one of two deletion implementations enforces the seam's invariant.
- **L6 Minor writer drift**: `--fix-ids` drops `validation_errors` from its schema-failure envelope;
  `renumber_duplicate_file` derives `epic_id`/`seq` from the id grammar where hydration prefers
  frontmatter. Both self-heal on the next sweep.
- **L7 Off-convention entity files are readable but not writable** — hydration walks recursively,
  the write path resolves by fixed `<specs_dir>/<kind>` convention. Same root cause as H2.
- **L8 Story-id allocation happens outside the advisory lock** that exists to serialize it;
  concurrent creates produce a spurious `file_exists` instead of the next id. (Already in the
  ledger from the create-story review.)

## Unsettled

- Whether a file that becomes unreadable *without* an mtime/size change is meant to be invisible to
  the sweep (it is — the cache keeps serving pre-`chmod` content as non-stale).
- Whether SQLite's own `busy_timeout` expiry (exit 4) is reachable where the advisory `lock_timeout`
  (exit 5) is intended. No reviewer could construct it.
- Absolute or `..`-containing `[storage]` values are neither normalised nor rejected
  (`config/mod.rs:212-226`); nobody pursued an absolute path placing the cache outside the workspace.

## Disposition

H1–H8 are **blocking for epic 1 acceptance**: each is reproduced, each loses or misreports
user-visible state, and three of them (H2, H3, H5) are triggered by the workflow a user follows
after the tool tells them something is wrong. They cluster into three root causes rather than eight
independent bugs — identity resolution (H2, H3, L7, M3), purge/re-hydration asymmetry (H4, H5, H6,
H7, M2), and configuration merge (H1, L2, M7) — plus two isolated contract defects (H8, M1).

M1–M8 and L1–L8 are filed in `deferred-work.md` with review-by points.

**Recommendation: epic 1 does not close on this review.** The three root causes above are one
story each; the epic can close when they land and this review is re-run against the result.

## What this says about the process

The retrospective's premise is confirmed. Every one of these twenty findings sat in code that
passed its own story review, and the suite is green on all of them. What made them visible was
asking one question of four writers at once, or following one piece of state across four commands —
neither of which a per-story review is shaped to do. Two further observations worth carrying into
epic 2:

- **The most damaging findings are in the repair paths**, not the happy paths: `--fix-ids`, purge,
  rebuild. Those run precisely when a workspace is already damaged, and they are the least
  exercised by tests, which build clean fixtures.
- **Reviewers who ran the binary found things reviewers who read code could not.** H4, H5 and H6 are
  each invisible in the source — they need two commands in sequence against real state.
