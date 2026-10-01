---
title: 'Stale means absent, in one place'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '015ed2efd182069d84e6c06834940c716c91172e'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md', '{project-root}/docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** "a stale row is treated as *absent* by everything that derives a finding" is asserted in `architecture.md`'s convergence-invariant section and implemented by remembering it at each site. One site forgot.

`find_orphan_deferred_work` skips a deferred-work row whose *own* entity is stale (`deferred_work_is_stale`) and then asks whether its origin story exists with `store.get_entity(origin)?.is_some()` in `find_orphan_deferred_work` — which returns stale rows. So a merge-conflicted origin story suppresses the `orphan_deferred_work` finding on the sweep path while a rebuild, having no stale rows at all, reports it. Reproduced by a reviewer: `qdev validate` reports one finding after a sweep and two after `sync --rebuild` on the identical tree, and which answer you get depends on hydration history.

The pattern is the point. Five sites now handle staleness — three checks filter their own rows, two SQL queries filter with `WHERE stale = 0`, and one existence probe does not — and each was written separately by a different story. The rule is remembered rather than enforced, so the next derivation site added will forget it too, exactly as this one did within hours of the rule being written down.

**Approach:** one helper answers "does this entity exist, for the purpose of deriving a finding?", treating a stale row as absent, and every derivation site asks it. Reads are untouched: `qdev get` and `qdev list` keep returning a stale entity with its flag set, which is what retention is for.

## Boundaries & Constraints

**Always:**
- One helper on `Store` answers entity existence for derivation, and a stale row is absent to it. Every site that derives a finding from an entity's existence uses it — none re-implements the filter.
- Reads keep returning stale rows: `get_entity`, `list_entities` and the `get`/`list` payloads are unchanged, including the `stale` flag they expose.
- The convergence invariant holds for **every** computed check: for each finding code, a workspace whose relevant row is stale produces the same finding set from a sweep as from a rebuild.
- The three checks that already filter their own rows keep working; they may be rewritten in terms of the helper but must not change what they report for non-stale rows.

**Never:**
- No new finding codes, no change to any check's meaning for a live row, and no change to the `findings` table.
- No change to what retention does — a file that fails to parse still keeps its previous rows, flagged stale.
- The relation-graph queries' `WHERE stale = 0` filters stay in SQL; pulling whole-graph validation through a per-row helper would trade a query for N.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Stale origin story | DW names `origin_story_id: E1S1`, `E1S1.md` merge-conflicted | `orphan_deferred_work` reported, matching a rebuild | N/A |
| Live origin story | the same DW, `E1S1.md` parsing | no `orphan_deferred_work` | N/A |
| Missing origin story | the same DW, `E1S1.md` deleted | `orphan_deferred_work`, as today | N/A |
| Stale DW row | the DW file itself merge-conflicted | its own parse failure is reported; no computed DW finding | N/A |
| Stale story, module check | story with an unregistered module, now unparseable | no `target_module_not_registered`, as today | N/A |
| Convergence, every code | one workspace per computed code, relevant row stale | sweep finding set equals rebuild finding set | N/A |
| Reads unaffected | any stale entity | `qdev get` returns it with `stale: true` | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/validate.rs:232` -- `store.get_entity(origin)?.is_some()`, the site that forgot; a stale origin reads as present -- the defect
- `crates/qdev-core/src/validate.rs:216` (`deferred_work_is_stale`) -- the local answer to the same question for the DW's own row, called from both DW checks -- fold into the helper
- `crates/qdev-core/src/validate.rs:305`, `:361` -- the two checks that filter `entity.stale` inline over `list_entities` -- rewrite in terms of the helper where it fits, or leave and document why not
- `crates/qdev-core/src/store/sqlite.rs` (`validate_relations_graph`'s two queries) -- the relation graph's two `WHERE stale = 0` queries; whole-graph, deliberately in SQL -- do not change
- `crates/qdev-core/src/store/mod.rs` (`Store::get_entity`) -- returns stale rows by design, because reads must keep working. The new helper sits beside it, not in place of it -- the distinction to preserve
- `crates/qdev-core/src/validate.rs:200` (`deferred_work_path`) -- also calls `get_entity`, but to find a *path to report against*, not to decide existence; a stale row's path is still the right path -- must not be changed
- `crates/qdev-core/src/validate.rs:187` (`ids_in_use`) -- also calls `list_entities` including stale rows, deliberately: a stale row's id is still taken -- must not be changed
- `crates/qdev-core/tests/sweep_tests.rs` (`test_rebuild_and_sweep_findings_equal`) -- compares tables after sweep vs rebuild; its fixtures never made a *referenced* row stale, which is why this shipped -- extend
- `crates/qdev-core/tests/validate_tests.rs` (`test_computed_checks_skip_stale_rows`) -- covers three of the checks with a hand-built stale row; the existence probe is the case it misses -- extend

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/mod.rs` + `sqlite.rs` -- one helper answering entity existence for derivation, with a stale row absent -- the single rule
- [x] `crates/qdev-core/src/validate.rs` -- every derivation site asks it, including the origin-story probe; `deferred_work_path` and `ids_in_use` keep using the unfiltered read and say why -- no site remembers the rule
- [x] `crates/qdev-core/tests/validate_tests.rs` -- extend the stale test to the existence probe, and assert the *live* case still reports nothing, so it cannot pass for want of a triggering fixture -- the instance
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- one convergence case per computed finding code, each with the relevant row stale, asserting sweep and rebuild agree -- the invariant, table-driven
- [x] `docs/architecture.md` -- state that the rule is enforced by one helper and name the two deliberate exceptions -- keep docs authoritative

**Acceptance Criteria:**
- Given a deferred-work row whose origin story's file is merge-conflicted, when `qdev validate` runs after a sweep, then it reports `orphan_deferred_work` — and reports exactly what `sync --rebuild` then reports on the same tree.
- Given one workspace per computed finding code with the relevant row stale, when each is hydrated by a sweep and by a rebuild, then the finding sets are equal — asserted per code, so a future check that forgets the rule fails here.
- Given a stale entity, when `qdev get` reads it, then it is returned with `stale: true` — the derivation rule does not leak into reads.
- Given a live row, when each computed check runs, then it reports exactly what it reported before this change.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are clean, and the sweep benchmark stays within its 30 ms budget.

## Implementation Notes

- **`EntityPresence { Live, Stale, Absent }` rather than a boolean**, because the two deferred-work
  checks must tell a stale row (skip — the file's own parse failure is already the actionable
  finding) from a missing one (report — a `deferred_work` row with no `entities` row is a broken
  cache, and dropping it would let `qdev validate` exit 0 on a real problem). The boolean
  `entity_exists_for_derivation` sits on top for the sites that do not care why.
- **The rule is written once, in `EntityPresence::from_stale_flag`, and
  `entity_presence_for_derivation` is a *provided* trait method defined in terms of it.** An
  implementor cannot spell the rule differently; `SqliteStore` overrides it only to read the flag
  alone instead of the whole row. The first implementation made it required, which left exactly
  the hole this story closes open for the next `Store`.
- **Honest scope of "enforceable".** The rule is now *centralised* — one implementation, one name
  that says which question it answers — but it is not *mechanically* enforced: nothing stops a
  future site writing `get_entity(..).is_some()`. A clippy `disallowed-methods` entry would, at
  the cost of annotating every legitimate read; that trade is filed rather than taken.
- **A better message was tried and reverted.** Naming a stale origin differently from an absent
  one ("could not be parsed" vs "does not exist") reads better and is more actionable — and it
  broke the convergence test immediately, because the message is part of the finding and a sweep
  has the stale row where a rebuild has none. Convergence is the stronger property. The attempt
  and the reason are recorded in the code.
- **Deliberate exceptions, each of which looks like the bug**: `deferred_work_path` asks for a
  path to report against (a row's `source_path` is right whatever its staleness); `ids_in_use`
  asks about id ownership (a stale row's id is still taken); and `validate_relations_graph` filters
  in SQL because it is whole-graph, where a per-row helper would trade one query for N.

## Spec Change Log

- 2026-09-10 — Created from the epic 1 cross-story review pass 2 (finding NEW-5). Third of the three invariants that review named. No Open Questions: the rule already exists and is already written down in `architecture.md`; this story only makes it enforceable rather than remembered.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | `EntityPresence::Absent` — the entire justification for three states over a boolean — was tested nowhere: every deferred-work fixture upserts a matching entity row, so collapsing `Absent` into `Stale` would silence both DW checks on a broken cache and leave the suite green (all three reviewers, two independently). | high | patch | Confirmed. New `test_deferred_work_with_no_entity_row_is_still_reported` asserts both codes fire and the placeholder path, plus the `Absent` mapping at the store. |
| 2 | `entity_presence_for_derivation` was a *required* trait method with only prose telling implementors not to restate the rule — so a second `Store` could spell it wrong, which is the defect this story fixes (blind, edge). | high | patch | Confirmed. Now a provided method defined in terms of `from_stale_flag`; `SqliteStore` overrides it only for the cheaper single-column read. |
| 3 | The new store API had no direct test: it was exercised only transitively through `run_validation` (blind). | medium | patch | Confirmed. `test_entity_presence_for_derivation_maps_all_three_states` asserts all three states and that reads still return the stale row. |
| 4 | A stale origin story yields a finding saying it "does not exist", which sends the reader after a missing file instead of the parse failure already reported (edge). | medium | rejected | Real as UX, and **the convergence test rejected it**: differentiating the message makes a sweep describe the workspace differently from a rebuild, since the message is part of the finding. Convergence is the stronger property. Tried, reverted, and recorded in the code — the invariant test doing its job on my own change. |
| 5 | Nothing mechanically prevents the next `get_entity(..).is_some()` derivation site, so "enforceable rather than remembered" overstates what shipped (blind). | medium | patch | Fair. The Implementation Notes now say plainly that the rule is centralised rather than mechanically enforced, and the clippy `disallowed-methods` option is filed with its cost. `list_entities` gained the symmetric warning it lacked. |
| 6 | `EntityPresence` derived `Serialize`/`Deserialize` with no consumer — committing an internal decision value to a wire format, for a type whose point is that it must not appear in payloads (blind). | low | patch | Confirmed by deleting the derives and rebuilding clean. Dropped. |
| 7 | `EntityRecord::presence_for_derivation` is public and cannot return `Absent`, so a caller matching on it writes a dead arm (blind). | low | patch | Documented at the signature, pointing callers at the boolean. |
| 8 | `deferred_work_path`'s documented exception is unreachable as stated: both callers skip a stale row before reaching it, so it only ever sees `Live` or a missing row (verification-gap). | low | patch | Correct, and the justification was overstated. The comment now says what is actually true — the unfiltered read matters for the *missing* case, not the stale one. |
| 9 | The convergence table had no case where a check's own subject *and* the row it refers to are both stale — the precedence the two rules create (blind). | medium | patch | Added, plus a `dw_missing_rationale` case with a rationale present, which pins that check's negative side rather than mere silence. |
| 10 | The CLI test asserted the whole findings list was empty as a precondition, and compared only `(code, path)` where the core test compares the message too (blind). | low | patch | Both tightened; the precondition now filters to the code under test. |
| 11 | Stale line citations in this spec, and "three deliberate exceptions" against a task saying "two" (blind). | low | patch | Citations replaced with symbol names; every exception is now named in both the code and the notes. |
| 12 | `qdev relate` decides existence with `get_entity` and then reads `target.kind` off a possibly stale row, so a kind-pair refusal can be decided from pre-edit content (blind). | medium | defer | Real, and not in this story's inventory: it is a *write gate*, not a derived finding, so the rule may legitimately differ there — but nobody has decided which. Filed. |
| 13 | `origin_story_id` pointing at a non-story entity passes the origin check, which never verified kind (edge). | low | defer | Pre-existing and independent of staleness. Filed. |
| 14 | `ids_in_use`'s deliberate unfiltered read has no test pinning the stale case, so a later "consistency" pass could adopt the helper there and hand out a stale row's id (verification-gap, other). | medium | defer | Good catch — the architecture text makes that mistake tempting. Filed as a test to add. |

## Design Notes

- The two exceptions are real and worth naming in the code, because they look like the bug: `deferred_work_path` asks `get_entity` for a *path to report against*, where a stale row's path is still correct, and `ids_in_use` asks for *id ownership*, where a stale row's id is still taken. Both would be wrong to filter. That is exactly why a bare `get_entity(..).is_some()` at a derivation site is indistinguishable from a legitimate read at a glance — and why the helper needs a name that says which question it answers.
- The convergence test is the right home for the invariant, and its blind spot was specific: fixtures made the *subject* of a check stale but never something a check *refers to*. One case per finding code closes that shape.
