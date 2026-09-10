---
title: 'A sweep and a rebuild converge'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '61cebe7901c15d655a1ea701a87a54ce25c0e900'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-7-incremental-hydration-sweep.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `rebuild_from_workspace` and `sweep_workspace` both claim to converge on identical state — "a full rebuild and a sweep of the same tree converge on identical findings/stale state" (`sqlite.rs:406`, `:3675`). The epic 1 cross-story review reproduced four ways that is false, each of which loses cache state a rebuild would have had:

- **A purge deletes rows owned by other, unchanged files.** Purge is id-keyed and cascading (`purge_entity_with_children` deletes `relations WHERE source_id = ?1 OR target_id = ?1`); re-hydration is path-keyed and change-gated. Delete `E1S1.md` and the `depends_on` edge declared by `E1S2.md` disappears — `E1S2.md` is unchanged, so nothing re-parses it, and `validate` cannot report a `dangling_relation` because the edge no longer exists to be dangling. `sync --rebuild` on the same tree reports it.
- **Deleting one of two files that declare the same id erases the survivor.** The cache holds one row for the id, whose `source_path` is whichever file sorted last. Delete *that* file and the entity is purged wholesale; the other file is still on disk, unchanged, therefore skipped. `qdev get` says `entity_not_found` and `validate` reports nothing, with the file sitting there. This is what the natural response to `duplicate_planning_id` — delete the copy — does today.
- **A rebuild silently swallows an unreadable file** (`Err(_) => continue`) where the sweep records a `read_error` and retains the previous rows stale. Since a rebuild also truncates `findings` first and `ensure_cache` takes one branch or the other, the command that triggered a rebuild sees an emptied, unrepopulated findings table: `qdev validate` exits 0 on a workspace with an unreadable spec file, and `qdev doctor` run immediately after reports it.
- **Computed checks read stale rows,** so a computed finding outlives the defect it describes: a story edited to remove `target_modules` still reports `target_module_not_registered` from its retained pre-edit row, and a rebuild of the identical tree reports only the parse failure.

Underneath all four: the sweep's change gate is an optimisation that assumes cache state can only be invalidated by a change to the file that owns it. A cascading purge breaks that assumption, and nothing repairs the damage.

**Approach:** restore the convergence invariant and make it the thing the tests assert. A purge stops destroying rows that belong to other files; the sweep learns to re-parse a file whose rows have gone missing, rather than trusting that unchanged means accounted-for; and the rebuild handles an unreadable file exactly as the sweep does.

## Boundaries & Constraints

**Always:**
- After either hydration path runs against the same tree, the cache holds the same rows and the same findings. That is the invariant, and a test asserts it directly rather than asserting the four symptoms.
- A purge deletes only what the removed file owned: its entity row, its detail and constraint rows, and the edges it *declared* (`source_id`). Edges **into** it are declared by other files and are left alone — they become dangling, which is exactly what `validate_relations_graph` then reports.
- Every known, readable entity file either has an `entities` row claiming it or a finding explaining why it does not. The sweep enforces this, so a file skipped as unchanged whose row has gone missing is re-parsed instead of being trusted.
- An unreadable file produces a `read_error` finding from both hydration paths.
- Duplicate-id detection covers every directory hydration reads, so a collision in `state_dir` is reported like one in `specs_dir`.
- **Decision (2026-09-10, Simon):** option A — the four computed checks exclude stale rows. qdev does not assert things about a file it could not read; the file already carries the `schema_violation`, `merge_conflict` or `read_error` finding that names the actionable problem, and a second finding derived from content the file no longer has sends the user looking for something they have already deleted. A rebuild has no stale rows at all, so this is also what makes the two paths agree.
- The exclusion is scoped to the four *computed* checks. Reads are unchanged: `qdev get` and `qdev list` keep returning a stale entity with its `stale` flag set, which is what the retention exists for.
- `qdev sync --rebuild` remains the guaranteed repair for any state this story does not anticipate.

**Never:**
- No change to the 30 ms boot budget's shape: the sweep still hashes only files whose `mtime`/size moved. The new check is a cache query, not a re-read of the tree.
- No cache schema change, no new finding codes beyond what the duplicate-id scope widening produces, and no change to the `get`/`list` payload shapes.
- The identity rule, `--fix-ids`, and the `[storage]` merge are out of scope — the first shipped in `61cebe7`, the third is the review's remaining root cause.
- A dangling edge is reported, not repaired: qdev never invents or deletes a relation a file declares.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Delete a depended-on story | `E1S2.md` declares `depends_on: [E1S1]`; `E1S1.md` removed | sweep reports `dangling_relation` on `E1S2.md`, same as a rebuild | N/A |
| Delete the duplicate that owned the row | two files declare `E1S1`; the one the cache points at is removed | the survivor is re-parsed; `qdev get E1S1` still resolves | N/A |
| Delete both duplicates | both files removed | the entity is gone from the cache, no findings for either path | N/A |
| Unreadable file, rebuild path | file unreadable, cache absent or mismatched | `read_error` finding; `validate` exits 1 | Exit 1 |
| Unreadable file, sweep path | file unreadable, cache healthy | `read_error` finding, previous rows retained stale — unchanged from today | Exit 1 |
| Stale row, computed check | story's `target_modules` edited away, file now unparseable | the parse failure is reported; no computed finding is asserted from the retained pre-edit row | N/A |
| Duplicate ids in `state_dir` | two files in `state_dir` declare `sprint-1` | `duplicate_planning_id` reported, as in `specs_dir` | N/A |
| Convergence property | any workspace, sweep vs rebuild | identical rows in every table and identical findings | N/A |
| Boot budget | 1,000 entities, one modified file | still within 30 ms | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/store/sqlite.rs:4141` (`purge_entity_with_children`) -- `DELETE FROM relations WHERE source_id = ?1 OR target_id = ?1`; the `target_id` half is what destroys other files' edges -- the core fix
- `crates/qdev-core/src/store/sqlite.rs:4209` (`purge_removed_path`) and `:3112` (the sweep's purge step) -- purge is driven from paths absent from disk, via `entity_rows_for_source_path` -- the caller
- `crates/qdev-core/src/store/sqlite.rs:3131` (the sweep's change gate) -- `mtime`/size unchanged and not dirty means skip, with no check that the file still has a row -- where the "every known file is accounted for" enforcement lands
- `crates/qdev-core/src/store/sqlite.rs:492` (rebuild's `Err(_) => continue`) vs `:3142` (the sweep's `read_error` + `flag_stale_by_source_path`) -- the two answers to an unreadable file; the sweep's is the one to keep -- H6
- `crates/qdev-core/src/store/sqlite.rs:463` (rebuild truncating `findings`) and `:3624`/`:3661` (`ensure_cache` taking one branch or the other) -- why H6 surfaces on the first command after a rebuild -- context
- `crates/qdev-core/src/store/sqlite.rs:3204` and `:504` (`validate_relations_graph`) -- derives the three relation codes from surviving `relations` rows, clearing them first; keeping inbound edges is what lets it report a dangling target -- why the fix works
- `crates/qdev-core/src/validate.rs:195`, `:250`, `:123`, `:152` -- the four computed checks' `list_entities`/`list_deferred_work` reads, none filtering `stale` -- H7. `EntityRecord.stale` is already returned, so this is a Rust-side filter, not a query or `EntityFilter` change
- `crates/qdev-core/src/validate.rs:38` (`scan_duplicate_planning_ids`) -- joins `specs_dir` only; hydration collects from `specs_dir` **and** `state_dir` (`sqlite.rs:2922`, `:419`) -- M2
- `crates/qdev-core/tests/sweep_tests.rs:715` -- the existing convergence test, which diffs every table after sweep vs rebuild but deliberately uses *new* conflicted and schema-invalid files ("both new, so neither path has a previous row"), a removed file with no inbound relations, and no unreadable file — the three shapes that diverge -- the test to extend, and the reason all four defects passed it

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- purge only the edges the removed file declared; leave inbound edges to be reported as dangling -- stop destroying other files' state
- [x] `crates/qdev-core/src/store/sqlite.rs` -- the sweep re-parses a known, readable entity file that no `entities` row claims, instead of skipping it as unchanged -- the change gate stops assuming
- [x] `crates/qdev-core/src/store/sqlite.rs` -- the rebuild records `read_error` and retains previous rows stale, exactly as the sweep does -- one answer to an unreadable file
- [x] `crates/qdev-core/src/validate.rs` -- the four computed checks skip entities whose row is stale, leaving reads untouched -- no finding about content a file no longer has
- [x] `crates/qdev-core/src/validate.rs` -- duplicate-id detection covers every directory hydration reads -- collisions in `state_dir` are reported
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- extend the convergence test to the shapes that diverge (inbound relations to a removed file, a duplicate-id pair with one removed, an unreadable file, a stale row), and cover every matrix row -- the invariant becomes the test
- [x] `docs/architecture.md` §11 -- state the convergence invariant and what a purge does and does not delete -- keep docs authoritative

**Acceptance Criteria:**
- Given any of the review's four reproductions, when the same tree is hydrated by a sweep and by a rebuild, then both produce identical rows in every table and identical findings — asserted by the convergence test, not by checking the four symptoms individually.
- Given `E1S2.md` declaring `depends_on: [E1S1]` and `E1S1.md` deleted, when any command boots, then `qdev validate` reports `dangling_relation` on `E1S2.md` and exits 1.
- Given two files declaring `E1S1` and the one the cache points at deleted, when any command boots, then `qdev get E1S1` still resolves to the surviving file.
- Given an unreadable spec file and no cache, when `qdev validate` runs, then it reports `read_error` and exits 1 — it does not exit 0 on the first command after a rebuild.
- Given a story whose row is stale because its file no longer parses, when `qdev validate` runs, then the parse-failure finding is reported and no computed finding is derived from the retained row — while `qdev get` still returns that entity with `stale: true`.
- Given the 1,000-entity benchmark with one modified file, when the sweep runs, then it still completes within the 30 ms budget.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Implementation Notes

- **Purge (`sqlite.rs`).** `purge_entity_with_children` drops the `OR target_id = ?1` half of its
  relations delete; everything else it deletes is either keyed to the entity's own id or a child
  row its file declared. The same function is also the in-place-id-edit purge inside
  `hydrate_markdown_file`, where keeping inbound edges is right for the same reason.
- **The accounted-for check (`sqlite.rs`, `sweep_workspace`).** Implemented as bookkeeping rather
  than a per-path query. The sweep now loads `SELECT id, source_path FROM entities`
  unconditionally into `path_of_id` / `ids_by_path` (replacing the conditional dirty-path scan
  *and* the `SELECT DISTINCT source_path` purge scan, so it is two queries where a warm boot previously ran one (the `SELECT id, source_path` scan used to be gated on a non-empty dirty set); measured at 6 ms median in release against the 30 ms budget, so the extra query is affordable rather than free) plus
  one `SELECT DISTINCT path FROM findings`, and keeps both in step with every purge and hydration
  the pass makes. `unaccounted` is then two in-memory lookups per unchanged markdown path.
  A first attempt used two `SELECT EXISTS` queries per path and cost 51 ms on the 1,000-entity
  warm benchmark (budget 30 ms); the map version measures 11 ms median, against ~10 ms before
  this story.
  Keeping the maps live rather than snapshotting them is load-bearing for the duplicate-id case:
  two files declaring one id each take the row from the other, so both re-parse and the file left
  owning it is the last in the sorted order the loop walks — the same one a full rebuild leaves
  owning it. Verified stable across repeated sweeps (`parsed: 2` every pass, same winner).
- **`read_error` on the rebuild path.** Both hydration paths now route an unreadable file through
  one `record_read_error` helper (clear the path's findings, record `read_error`, flag any rows it
  owns stale). Applied to all four of the rebuild's read sites — entity markdown, scratch,
  evidence and `qdev.toml` — since the sweep's read-failure branch is role-independent.
- **Stale exclusion (`validate.rs`).** `find_unregistered_target_modules` and
  `find_off_convention_entity_files` filter on `EntityRecord.stale`; the two deferred-work checks
  use a new `deferred_work_is_stale` helper (`get_entity(dw.id)`, already the lookup
  `deferred_work_path` does). A DW row the cache has *no* entity for is deliberately still
  reported — that is a broken cache, not a known-unparseable file, and dropping it would let
  `validate` exit 0 on it. `find_orphan_deferred_work`'s existence test for `origin_story_id`
  also still treats a stale row as existing: the story exists on disk, it just does not parse
  right now, and its own parse failure is what is reported.
- **Duplicate-id scope (`validate.rs`).** `scan_duplicate_planning_ids` /
  `find_duplicate_planning_ids` now take `&StorageConfig` instead of `&str` and walk `specs_dir`
  and `state_dir`, with the file list sorted and deduped so an overlapping configuration cannot
  report a file as a duplicate of itself. `--fix-ids` is unaffected in behaviour: a state-kind id
  fails `old_id.parse::<Identifier>()` (or `next_available_id`'s `unsupported_renumber`) and the
  path is recorded as skipped, which is the existing per-entry non-fatal route.
- **Tests.** The existing convergence test gained the three shapes it was written to avoid — a
  removed file with an inbound edge, a duplicate-id pair whose row-owning file is the one
  removed, and a *new* unreadable file — and still diffs every table. The retention exception
  (previously parsed file, now unreadable) is asserted in its own test: findings converge, and the
  retained stale row is named as the documented divergence. `make_unreadable` returns whether the
  `chmod 000` actually took, so a root or non-POSIX run skips those assertions instead of failing
  for the wrong reason.

**Not done / risk:**
- `Store::delete_entity` (review L5) is still a second, non-cascading deletion implementation with
  no caller outside tests. Out of scope here, still latent.
- While two files declare one id, every sweep re-parses both (two extra parses, no extra I/O
  beyond the two reads). That is the price of the "last sorted wins" convergence rule, and the
  condition is an `error`-severity `duplicate_planning_id` the user is being told to fix.

## Spec Change Log

- 2026-09-10 — Created from the epic 1 cross-story review (findings H4, H5, H6, H7, M2), the second of the three root causes that review named as blocking epic 1 acceptance. The first shipped as `61cebe7`.
- 2026-09-10 — Open Question answered by Simon: option A, computed checks exclude stale rows. Recorded in the frozen block, with the scope limited to the computed checks so reads keep returning stale entities.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | The invariant was still broken by stale rows: `validate_relations_graph` read every entity and every relation with no `stale` filter, so a retained pre-edit row suppressed the `dangling_relation` a rebuild reports on edges into it, and its own retained edges could close a `dependency_cycle` a rebuild never sees (blind, edge, twice as a claim). | high | patch | Confirmed at `sqlite.rs:3922`/`:3977`. Both queries now read live entities only, and only edges a live entity declares — so an edge *into* a stale entity dangles (as on a rebuild) and a stale entity's own edges are invisible (as on a rebuild). The architecture doc's retention exception was rewritten to say this rather than claim findings "still agree". |
| 2 | The "or a finding explaining why" half of the new invariant accepted *any* finding, including `dangling_relation` and `dependency_cycle`, which are recorded against files that parsed successfully — so a duplicate-loser file carrying a relation finding was treated as accounted for and never re-parsed (blind, edge, and as a claim). | high | patch | Confirmed. The query is now restricted to `schema_violation`, `merge_conflict`, `read_error`. Documented in §11 too, since "any finding" was the natural misreading. |
| 3 | The new duplicate-id scan dedups its file list; the two hydration collectors it claims to mirror do not, so an overlapping `specs_dir`/`state_dir` layout hydrates each overlapping file twice (blind). | medium | patch | Confirmed at `sqlite.rs:423`/`:2948`. Both collectors now dedup. Nothing rejects an overlapping layout, so this was reachable by configuration. |
| 4 | `Store::delete_entity` still cascaded `relations WHERE target_id = ?1`, so the second deletion path would regress the fix if it ever gained a caller (edge, filed as a deletion risk). | medium | patch | Confirmed. Aligned to `source_id` only, with the same comment. Still has no caller outside tests (review item L5). |
| 5 | Three of the four stale-row skips had no test — every validate fixture builds `stale: false`, so removing the guards left the suite green (verification-gap, pre-verified). | medium | patch | Accepted. New `test_computed_checks_skip_stale_rows` asserts all three, *and* that the same rows produce all three findings when live, so it cannot pass for want of a triggering fixture. Mutation verified. |
| 6 | The rebuild's new `read_error` arms for scratch and evidence files were unreachable by any test: `make_unreadable` only ever wrote a story (verification-gap, pre-verified). | medium | patch | Accepted. Helper generalised; new test covers both arms on both hydration paths. Mutation verified: reverting either arm now fails it. |
| 7 | Widening the scan to `state_dir` made the `--fix-ids` "not a renumberable identifier" branch reachable, and it skipped silently — unlike its sibling, which explains itself (verification-gap, blind). | medium | patch | Accepted. The branch now says why, and a new test pins the whole shape: one JSON document, nothing renumbered, the group reported as skipped, both files byte-identical, and the duplicate still reported by a following `validate`. |
| 8 | `test_fix_ids_does_not_allocate_an_id_already_used_outside_specs_dir` no longer tests what it says: the scan now walks `docs/state/scratch`, so `scan.all_ids` supplies the id and the cache-seeding it pins can be deleted with the test green (verification-gap, pre-verified). | medium | defer | Accepted as a real gap, but the suggested fix does not exist: I built the fixture (an entity in the cache under no scanned directory) and it is **unconstructible** — the next sweep sees the path as absent and purges the row, correctly. The allocation then reuses the id, which is right. So the cache-seeding's remaining purpose is unclear; recorded in the test's comment and filed, rather than papered over with a fixture that cannot exist. |
| 9 | The Implementation Notes claimed the map approach was "one query fewer than before"; a warm boot previously ran one query and now runs two (blind). | low | patch | Confirmed. Note corrected, with the measured figure: 6 ms median in release against the 30 ms budget. |
| 10 | `docs/architecture.md` §11 step 1 still stated the change-gate assumption this story breaks, 15 lines above the new subsection (blind). | low | patch | Confirmed. Step 1 now carries the qualifier and links the invariant. |
| 11 | The "last sorted path owns the row" tie-break is load-bearing for the convergence argument but appeared only in code comments and test assertions — and `--fix-ids` keeps the id on the *first* sorted path, the opposite way round (blind). | low | patch | Confirmed. Both stated in §11. |
| 12 | The frozen Never says the sweep "still hashes only files whose `mtime`/size moved"; an unaccounted file is read and hashed with its metadata unmoved, and a duplicate-id pair does so on every boot until the user fixes it (edge, as a claim). | medium | rejected | True as literally worded, and unavoidable: restoring a missing row *requires* re-reading the file. The frozen sentence's own qualifier — "the new check is a cache query, not a re-read of the tree" — holds: the check costs no I/O; only the files it finds unaccounted for are read. Bounded (two files per duplicate pair, which carries an error-severity finding telling the user to fix it) and measured inside budget. Flagged to the human rather than silently reinterpreted. |
| 13 | An unreadable `qdev.toml` is a fifth divergence: the rebuild truncates `gates` and never repopulates, where the sweep retains the previous rows, and gates carry no `stale` flag (blind). | low | defer | Real but unreachable through the CLI — config loading fails hard before dispatch (`config/mod.rs:1217`), as the verification-gap layer independently confirmed. Filed. |
| 14 | A read failure is counted as `unchanged` in `qdev sync`'s summary (blind). | low | defer | Pre-existing wording, unchanged by this story; the finding is what carries the signal. Filed with the `sync` summary's other known gap. |
| 15 | `deferred_work_is_stale` costs a `get_entity` per DW row, called from two checks, plus a third for `deferred_work_path` — the opposite approach to the sweep changes in the same diff (blind, verification-gap). | low | defer | Confirmed. `list_deferred_work` already joins `entities`, so adding a `stale` field to `DeferredWorkRecord` removes N per-row queries. Public struct change; filed. |
| 16 | Test-coverage gaps: `test_unchanged_file_whose_row_is_intact_is_still_skipped` asserts `parsed == 0` but not `unchanged == 2`; nothing asserts repeated sweeps settle on the same winner as a rebuild; nothing covers three files declaring one id (blind). | low | defer | Real. The winner-stability property is the most valuable of the three and is the one the manual check covered by hand. Filed together. |

### Review pass adjustments (2026-09-10)

- A flake surfaced during verification and was traced to **a test written earlier in this session**:
  `doctor_cli_tests::test_doctor_findings_by_code_is_code_sorted_and_stable` compared two whole
  `doctor --json` payloads for determinism, including the `cache` section's `last_synced_at`, which
  moves when the two runs straddle a second boundary. Narrowed to compare the breakdown's key order
  and count. It failed once in a full run and never in nine targeted runs, which is exactly how a
  timing-dependent assertion presents.
- Both queries in `validate_relations_graph` now filter on `stale = 0`; a stale entity is treated as
  absent by everything that derives a finding, which is what makes retention convergent rather than
  an exception.

## Design Notes

- **Why keeping inbound edges is right, not lazy.** An edge is declared by the file that names it. Deleting `E1S2.md`'s `depends_on` because `E1S1.md` vanished discards a fact `E1S2.md` still asserts, and it is the *reporting* of that fact — `dangling_relation` — that the user needs. The `relations` table has no foreign key, so a row pointing at an absent entity is representable by design. It does mean `get --expand relations` and `graph --dot` show an edge to an entity that is not there, which is the truth about the workspace.
- **The "every known file is accounted for" check is a cache query, not a scan.** The sweep already stats every file; the addition is asking whether each unchanged path still has a row, which is one indexed lookup per path and no extra I/O. That is what keeps the 30 ms budget intact.
- The existing convergence test is worth reading before writing the new one: its comments show it was written to *avoid* the divergent shapes, which is why four defects lived behind a passing assertion of the very invariant they break.

## Verification

**Result (2026-09-10):** `cargo test --workspace` green (33 suites, 0 failures),
`cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo fmt --check` clean, warm
sweep benchmark 11 ms median / 15 ms max against the 30 ms budget. Both manual reproductions
confirmed against the release binary, plus H6 end to end: `qdev validate` on an unreadable spec
file with no cache exits 1 reporting `read_error`, and `qdev doctor` immediately after reports the
same one finding.

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- Manual, the review's reproductions: (1) `E1S2.md` depends on `E1S1`, delete `E1S1.md`, confirm `validate` reports `dangling_relation` without a rebuild; (2) two files declaring `E1S1`, delete the one the cache points at, confirm `qdev get E1S1` still resolves
