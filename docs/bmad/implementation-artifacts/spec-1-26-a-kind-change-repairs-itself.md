---
title: 'A kind change repairs itself'
type: 'bugfix'
created: '2026-09-11'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '6c2f37b57f3934229e1c1609364c250da67286b4'
context: [ '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-11-pass-3.md', '{project-root}/docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** the write path overwrites `entities.kind` and leaves the previous kind's detail row
behind, where no later pass can find it. Hydration's repair compares the *cached* kind against the
new one — but the write already replaced the cached kind, so the two agree and the orphan survives
every sweep. `qdev update E1S1 --field kind=epic` leaves the `stories` row in place: `qdev get E1S1`
reports an `epic_id` for an epic, `qdev list --epic E1` lists it, and `qdev sync --rebuild` — and
only a rebuild — makes both answers change. The write path is a third implementation of hydration
alongside the sweep and the rebuild, and the convergence invariant never covered it.

**Approach:** the write path drops the previous kind's detail rows itself, in the same transaction
that changes the kind, using hydration's existing kind→table mapping rather than a second copy of
it. That mapping becomes the single site the question is answered at, with no wildcard arm, so a
future kind with a detail table cannot be added without deciding what a change away from it drops.

## Boundaries & Constraints

**Always:**
- The cache after a write is a state a sweep can repair. No row may be left that only
  `sync --rebuild` can remove.
- One kind→detail-table mapping, reused. Hardcoding a table in the write path is the defect.
- Deletion is scoped to the entity whose kind changed. `relations` (in or out), `constraints`, and
  every other entity's rows are untouched — the reasons are on `delete_entity_row_shallow`.
- The equal-kind case stays free: an ordinary `update` that does not change the kind does no extra
  delete.
- **Convergence is reached by the next pass, not within the write** (human decision, 2026-09-11).
  The write drops the orphan; the *new* kind's detail row is materialized by the sweep the dirty
  marker already forces — the same one-pass lag every non-story field of a decision, sprint or SOUP
  entry already has, and no command can observe the window. What may not survive that pass is the
  orphan.

**Never:**
- Do not make the write path materialize the *new* kind's detail row for kinds it has never
  written. Hydration owns that on the next sweep; only the orphan is this story's subject.
- Do not widen the existing `source_path`/id-change cleanup, and do not turn the kind change into a
  purge cascade.
- Not in scope: the two ledger entries whose review-by point names this work (`init` exiting 4 on a
  corrupt cache; read-only commands taking the write lock). Neither is about the kind mapping.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Detail kind → other kind | `E1S1` (story, `stories` row), `update --field kind=epic` | `stories` row gone at once; cache equals a rebuild of the same tree | N/A |
| Sprint → other kind | `sprint-3` with `sprints` + `sprint_assignments` rows, flipped to another kind | both the `sprints` row and its assignments gone | N/A |
| Other kind → detail kind | `AD-8` (adr, no detail row) flipped to a detail kind | nothing to drop; the new kind's row appears on the next sweep, as it does for any field change | N/A |
| Kind unchanged | `update E1S1 --status ready` | no detail-row delete; `stories` row updated as today | N/A |
| Flip and flip back | `story → decision → story` across three writes | no row from an intermediate kind survives; equals a rebuild | N/A |
| Refused write | a flip the new kind's schema rejects | file and cache both unchanged | exit 1 `schema_validation_failed`, as today |
| Id change | `validate --fix-ids` renumber | unchanged behaviour: the whole old row goes, inbound edges are redirected | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/write.rs:958` (`upsert_cache_with_relation`) — the defect. Step 1's
  `ON CONFLICT(id) DO UPDATE SET kind = excluded.kind` (`:1044`) replaces the cached kind with no
  cleanup; step 2 (`:1084`) writes the `stories` row and knows no other kind. Every non-hydration
  writer funnels here — the fix site
- `crates/qdev-core/src/write.rs:1033` → `delete_entity_row_shallow` (`:816`, mapping call at
  `:842`) — the *id*-change cleanup, which already reuses the mapping correctly. The shape to
  follow; do not extend it, it deletes the whole row
- `crates/qdev-core/src/store/sqlite.rs:4464` (`delete_kind_detail_row`) — the mapping to reuse.
  Two things to repair: the `_ => return Ok(())` arm at `:4494` (a new kind with a detail table is
  silently missed), and its doc comment, which claims it deletes `sprint_assignments` when the
  `Sprint` arm only touches `sprints`
- `crates/qdev-core/src/store/sqlite.rs:4509` (`clear_owned_child_rows`) — hydration's repair:
  prev-kind compare at `:4536-4555`, the separate `sprint_assignments` clear at `:4558`. Read it to
  see why the write path cannot rely on it, and why the reuse is not a one-liner
- `crates/qdev-core/src/store/sqlite.rs:737` / `:852` — `get_entity` / `list_entities`, both
  `LEFT JOIN stories` with `WHERE s.epic_id = ?3`; why an orphan `stories` row is user-visible
- `crates/qdev-core/src/write.rs:1541` (`kind_for_write`) → `sqlite.rs:5664`
  (`determine_entity_kind`) — how the new kind is resolved (frontmatter wins over directory); the
  new kind is already in hand at the upsert
- `crates/qdev-core/src/write.rs:2370` (`apply_relation_change`), `:2164` (`create_story`),
  `crates/qdev-cli/src/main.rs:2906` (`--fix-ids`) — the other callers of the same upsert; they
  inherit the fix, and `apply_relation_change` re-derives the kind too, so it can move it
- `crates/qdev-cli/tests/update_cli_tests.rs:1244` — the only existing kind-flip test, and the flip
  is *rejected*; no test exercises a successful one. `setup_workspace` (`:14`) and
  `create_sample_story` (`:31`) are the fixtures to reuse
- `crates/qdev-core/tests/sweep_tests.rs:3073` (`assert_sweep_equals_rebuild`) and `:114`
  (`dump_tables`) — the convergence assertion to mirror on the write side; today no write-path test
  calls `reset_and_rebuild` at all. `SqliteStore::reset_and_rebuild` is `sqlite.rs:389`
- `crates/qdev-core/tests/write_tests.rs` — the library-layer home for the matrix test;
  `crates/qdev-core/src/schema.rs:11` — the 13 `EntityKind` variants, six of which have a detail
  table
- `docs/architecture.md:543` (the convergence invariant) — states the rule for the sweep and the
  rebuild only; the write path is the third path and belongs in it

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/write.rs` -- read the cached kind before the `entities` upsert and, when
  it differs from the kind being written, drop the previous kind's detail rows in the same
  transaction through the shared mapping -- the fix
- [x] `crates/qdev-core/src/store/sqlite.rs` -- `delete_kind_detail_row`: replace the wildcard arm
  with the kinds that own no detail table, named, so a new variant does not compile until someone
  decides; make the `Sprint` arm delete `sprint_assignments` as its doc already claims, and leave
  the callers that clear them for their own reasons correct -- one mapping, and the enumeration
  this story owes
- [x] `crates/qdev-core/tests/write_tests.rs` -- every kind with a detail table flips to every
  other, each flip compared table-by-table against a rebuild of the same tree; include the
  flip-and-flip-back pair and the kind-unchanged case -- the matrix, mutation-verified
- [x] `crates/qdev-cli/tests/update_cli_tests.rs` -- one end-to-end case: `update --field kind=` on
  a story, then `qdev get` and `qdev list --epic` answer the same before and after
  `qdev sync --rebuild` -- the user-visible symptom from the review
- [x] `docs/architecture.md` -- extend the convergence invariant to the write path: a write leaves a
  state a sweep can repair -- the rule belongs where the invariant is stated

**Acceptance Criteria:**
- Given an entity of a kind with a detail table, when its `kind` is changed through the write path,
  then no row of the previous kind remains and the cache agrees with `sync --rebuild` table by
  table.
- Given a sprint with assignments, when its kind changes, then its `sprints` and
  `sprint_assignments` rows are both gone.
- Given a write that does not change the kind, when it commits, then the detail rows it does not
  own are untouched and no extra delete is issued.
- Given `qdev validate --fix-ids` renumbering a duplicate, when it commits, then its existing
  behaviour — including the inbound-edge redirect — is unchanged.
- Given the workspace, `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean.

## Implementation Notes

- **The fix (`write.rs`, step 0b of `upsert_cache_with_relation`).** The cached kind is read
  immediately before the `entities` upsert, and when it differs from the kind being written the
  previous kind's detail rows are dropped in the same transaction through
  `store::sqlite::delete_kind_detail_row`. Placed in the shared upsert, so `update`, `relate`,
  `unrelate`, `create story` and `--fix-ids` all inherit it. The equal-kind case issues no
  statement at all.
- **The mapping (`sqlite.rs`, `delete_kind_detail_row`).** The wildcard arm is gone: the seven
  kinds that own no detail table (`Prd`, `Requirement`, `Epic`, `Adr`, `Hazard`, `Release`,
  `Scratchpad`) are named, so a fourteenth variant — or an existing one acquiring a table — fails
  to compile. The `Sprint` arm now deletes `sprint_assignments` before `sprints`, as its doc
  already claimed; the delete order matters because `sprint_assignments.sprint_id` is a foreign
  key onto `sprints(id)` and foreign keys are enforced. `purge_entity_with_children`'s own
  `sprint_assignments` pre-clear became redundant and was removed — the mapping covers it, and
  the ordering is now the mapping's responsibility. `clear_owned_child_rows` keeps its
  assignments clear: that one fires when the *new* kind is `Sprint`, so a re-parse replaces
  assignments the file dropped, which is a different question.
- **Why the fixture frontmatter is a union.** The matrix test flips one file's `kind:` and nothing
  else, so the frontmatter carries every detail kind's fields at once. That is only possible
  because no entity schema sets `additionalProperties: false` and the three enum-constrained
  shared names (`appetite`, `safety_class`, `safety_risk`) carry identical enums wherever they
  appear.
- **Status has to move with the kind.** Three of the six detail tables carry a `CHECK` on
  `status` and no two agree (`sprints`: `planning|active|completed|paused|abandoned`;
  `deferred_work`: `open|done|wont_fix`; `gate_runs`: `pass|fail|infra`). So a flip whose
  destination is one of those three must carry a status the destination accepts, and the tests
  set both fields in one `update`. **Latent, pre-existing, out of scope:** a hand-edited file that
  changes `kind:` to one of those three without a compatible `status` makes the next *hydration*
  fail with a raw `sqlite_error` (`CHECK constraint failed`) rather than a `schema_violation`
  finding — the schemas do not constrain `status` at all, so nothing catches it earlier. Not this
  story's subject, and not reachable through `qdev update`, which validates the patched
  frontmatter and would still be refused only by the cache write.
- **Mutation-verified.** Disabling the kind comparison in `write.rs` fails all three write-path
  matrix tests and the CLI case (the CLI diff is exactly the review's symptom: `epic_id: "E1"`
  before the rebuild, absent after). Removing the `sprint_assignments` statement from the
  `Sprint` arm fails the sprint test. The flip-and-flip-back test sweeps between the two flips,
  so the intermediate kind genuinely owns a row before the flip back — without that sweep it
  passed under the mutation.

**Not done / risk:**
- The write path still does not materialize the *new* kind's detail row; that is the frozen
  decision, and the tests assert the one-pass lag explicitly rather than leaving it implicit.
- The `status`/`CHECK` mismatch above is a real hard-failure path for hand-edited files. Not
  filed here because it is not about the kind mapping; worth a ledger entry.

## Spec Change Log

- 2026-09-11 — Created from the pass-3 acceptance gate (NEW-4), third of the seven blocking
  stories. NEW-4 was raised as blocking in pass 2 and reached neither a spec nor the ledger until
  pass 3; this spec is the filing.

## Review Triage Log

Pass 1 (2026-09-11) — three layers (blind, edge-case, verification-gap); every finding gets a row.

- **medium / patch** — a *refused* kind flip had no test that the previous kind's detail row survives. Filed pre-verified by the verification-gap layer, which showed the damage would be durable: an unchanged story whose `stories` row is deleted answers `qdev get` with no `epic_id` until a rebuild, because the sweep skips the unchanged file. The refusal assertion in the CLI test uses an ADR, which owns no detail table and so cannot observe a wrongly-issued delete. Patched: `test_a_refused_kind_flip_leaves_the_previous_kinds_detail_row_alone` flips a story to `decision` with an invalid `decision_type` and asserts the `stories` row and the cached kind both survive.
- **low / patch** — step 0b spelled the no-rows case as a six-line `.map(Some).or_else(..)` where `clear_owned_child_rows`, the site it was modelled on, uses `.optional()`. Direct simplification; patched.
- **low / patch** — `upsert_cache_and_mark_dirty` and `upsert_cache_with_relation` are `pub` and re-exported, and their rustdoc described only the upsert, the dirty marker and the relation row; a caller could not learn from the docs that a changed `kind` now deletes rows. Patched on both.
- **low / patch** — `delete_kind_detail_row` labelled every statement's failure with the kind, so a `sprint_assignments` failure reported as the `sprints` row. Each statement now carries its table name; patched.
- **low / patch** — `purge_entity_with_children` keeps an unconditional `DELETE FROM stories` right after the shared mapping call, which reads as the hardcoding the new doc comment warns against. Behaviour-preserving and still load-bearing for caches written before this fix, so documented at the site as a legacy repair — `stories` is the one detail table `get_entity`/`list_entities` join, hence the only orphan a user can observe — and marked removable once no such cache is in use.
- **low / patch** — `assert_write_converges_with_rebuild`'s `sync_meta` skip was unexplained, unlike every other concession in the file (`found_at`, the pinned `updated_at`). Patched.
- **low / patch** — `detail_row_count`'s `unwrap_or(&[])` would let an "the orphan is gone" assertion pass vacuously for a kind missing from `DETAIL_TABLES` — the same fail-open the production mapping was just closed against. Patched to panic.
- **high / defer** — `update <id> --field kind=sprint|dw|evidence` succeeds at exit 0 with a status those tables' `CHECK`s reject, and every later command then fails with a raw `sqlite_error` naming the constraint; `sync --rebuild` fails the same way, so the documented repair does not repair it. Reproduced end to end. Pre-existing: no entity schema constrains `status`, and a hand edit of `kind:` does the same, so this change neither causes nor widens it. Ledger entry filed — the sharpest thing this review found.
- **medium / defer** — `sprint_number` folds every non-numeric sprint id to key `0`, and `schemas/sprint.json` constrains `id` only by `minLength`, so two such sprints collapse onto one `sprints` row and one set of assignments. Verified independently by two layers on a real workspace. Pre-existing in hydration — the insert uses the same collapsing key, so this change's delete is symmetric with it and introduces no divergence from a rebuild. Ledger entry filed.
- **medium / defer** — a cache written before this fix keeps its orphan detail rows: the write that made it already stored the new kind, so neither a sweep nor a later write can find them. Not caused by this change; the fix is a cache-version bump, which forces the rebuild that clears them. Ledger entry filed.
- **low / rejected** — the lookup-and-compare around the mapping is now a third near-verbatim copy (step 0b, `delete_entity_row_shallow`, `clear_owned_child_rows`). The thing that must not be duplicated — the kind→table mapping — is shared, and with `.optional()` applied the remaining copy is a few lines whose surrounding logic differs at each site (one deletes unconditionally, two compare first).
- **low / rejected** — a cached kind that fails `from_str_loose` silently skips the cleanup at all three sites. `entities.kind` is only ever written from `EntityKind::as_str`, so no reachable input produces it; the fix would add an error branch guarding a state never demonstrated.
- **low / rejected** — the `Sprint` arm's new `sprint_assignments` delete also reaches `delete_entity_row_shallow`, which the spec's Boundaries said not to widen. Verified benign and converging: the assignments belong to the sprint number whose row is going away, and hydration re-derives them from the file. The verification-gap layer separately established that `--fix-ids` cannot reach the shallow path for a sprint — a duplicate `sprint-3` pair reports `skipped`, not `renumbered`.
- **low / rejected** — no test drives the fix through `apply_relation_change`. Every command boots through the sweep before it writes, so the cached kind is current by the time `relate` upserts and the prev-kind branch does not fire; the inheritance is structural (one shared function), not a behaviour needing its own fixture.
- **low / rejected** — `dump_cache_tables` and `sprint_key` duplicate `sweep_tests`'s helpers. Acknowledged at both sites; a shared test-support module is a suite-wide refactor, not this story's.
- **low / rejected** — the equal-kind test asserts a row count rather than row contents, so a delete-then-reinsert would pass. The case is run on a *decision*, which the write path never re-inserts: a spurious delete drops the count to zero and fails. That is what the decision case is there for.
- **low / rejected** — `epic-1-context.md` churn is unrelated to the kind mapping. It is regenerated by the build workflow's own first step, because the epics file is newer than the cached context; it is not part of the change under review.
- **false** — "`sprint-status.yaml` says `in-progress` while the spec says `in-review`". That is the workflow's sequencing: the sprint key advances at hand-off, not at each internal step.

## Design Notes

- **Why hydration's repair cannot reach this.** `clear_owned_child_rows` asks the `entities` table
  what the previous kind was. That works for a hand-edited file, whose kind reaches the cache only
  through hydration. It cannot work after a qdev write, which has already stored the new kind — the
  comparison finds equality and concludes there is nothing to drop. The write path is the only
  place that still knows both halves, so it is the only place that can answer.
- **Why the fix belongs to the mapping, not to `update`.** The symptom is reachable from `update`
  today, and from `relate`/`unrelate` whenever the file's `kind:` moved underneath them, since both
  re-derive the kind and write through the same upsert. Fixing the shared upsert covers every
  writer at once, now and later.
- **The enumeration.** `delete_kind_detail_row`'s wildcard is the reason a kind can acquire a detail
  table and not a deletion. Naming the seven kinds that own no table turns the next such variant
  into a compile error — the same mechanism story 1-25 used for finding codes, and the reason the
  matrix test is over *every* pair rather than over the one pair the review reproduced.

## Verification

**Result (2026-09-11):** `cargo test --workspace` green (33 suites, 0 failures),
`cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo fmt --check` clean. Manual
reproduction confirmed against the debug binary: after `qdev update E1S1 --field kind=epic`,
`qdev get E1S1 --json` and `qdev list epic --epic E1 --json` return byte-identical payloads either
side of `qdev sync --rebuild`, with no `epic_id` on the epic and an empty `--epic E1` listing.

**Commands:**
- `cargo test --workspace` -- expected: clean, with the new matrix and CLI cases
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: no output
- `cargo fmt --check` -- expected: no output

**Manual checks:**
- In a scratch workspace: `qdev update E1S1 --field kind=epic`, then `qdev get E1S1 --json` and
  `qdev list --epic E1 --json`; run `qdev sync --rebuild` and repeat both. Expected: identical
  answers either side of the rebuild, and no `epic_id` on the epic.
