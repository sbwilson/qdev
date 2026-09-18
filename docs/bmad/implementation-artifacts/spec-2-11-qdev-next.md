---
title: 'Story 2.11: qdev next'
type: 'feature'
created: '2026-09-18'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
story_key: '2-11-qdev-next'
baseline_commit: '2f2ec16a7d430d930442ff107ff68b985961e07c'
spec_file: 'docs/bmad/implementation-artifacts/spec-2-11-qdev-next.md'
context:
  - 'docs/bmad/implementation-artifacts/epic-2-context.md'
  - 'docs/cli-reference.md'
  - 'docs/governance-and-teams.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Unattended loops (and Story 2.12's pulse) need one command that answers "what should I work on next?" without human judgement. Today every story choice is manual; nothing derives the next eligible story from sprint assignments, blocked status, leases, and ownership.

**Approach:** Add the workspace command `qdev next [--sprint N] [--owner me]`, with selection logic in a new `qdev-core` module (`next.rs`). Candidates are stories assigned to active sprints (or to the `--sprint N` sprint); selection skips blocked and leased stories and otherwise follows the ordering in `docs/cli-reference.md` §5 exactly. Output carries the selected story payload plus `reason` fields explaining every filter applied; when nothing is eligible it exits 0 with `next: null` and the nearest blockers.

## Boundaries & Constraints

**Always:**
- `qdev next` requires an initialized workspace (`requires_workspace`; outside one, exit 2), supports `--json`, and never mutates anything: no lease is created or broken, no entity or cache row is written, no decision record is logged.
- Candidate scope: with `--sprint N`, the stories assigned to sprint N (whether or not it is `active`; unknown sprint → exit 2 `sprint_not_found`). Without it, the stories assigned to every sprint with `status: active` — if several sprints are active, sprint id breaks that tie first. Stories in no sprint assignment are never candidates. With `--owner me`, candidates are further restricted to stories owned by the current identity (user id, git email, or a team they belong to — reuse the `governance.rs` ownership helpers and the `resolve_author` chain); with no `--owner`, any ownership is eligible but current-user-or-team matches sort ahead of other-owned stories.
- "Blocked" comes from the existing `depends_on` computation (`query.rs` `compute_blocked` / `transition.rs` `validate_dependencies` semantics: a dependency that is not live-`done`, dangling, or stale blocks the story). "Leased" comes from `lease.rs` — a story leased in another worktree is skipped; a story leased **by this worktree** remains eligible and is returned (finish what you started — `docs/cli-reference.md` §5 shows exactly this).
- Sort order for eligible candidates: active-sprint membership first, then ownership match, then epic `phase`, then story sequence, tie-broken by story id — independent of cache row or fixture order (property test: shuffled inputs give identical output).
- Nothing eligible → exit 0 with `next: null` and a `blockers` list naming the nearest reasons (e.g. blocked stories and their unmet dependency ids, leased stories and their holders, or "no active sprints").
- Add `PayloadKind::Next` with `schemas/payload-next.json`; `qdev schema payload next` must then print it (update the deferred-names test).

**Never:**
- Never claim, release, or fabricate a lease; never transition any story; never write to `docs/state/`, the cache, or the lease files.
- Never select a story that is blocked or leased elsewhere, even if it is the first candidate; skipping is absolute, ranking only orders survivors.
- Never pick `done`, `review`, `superseded`, or `abandoned` stories; `draft`/`ready`/`in-progress` are all eligible (readiness gating is Epic 3's job).
- Don't build Story 2.12's root pulse, gate runners (`gate_run` stays deferred), or token-budgeted context projection.
- Don't add a new glob, entity kind, or store migration — everything needed already exists in core.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|-------------|
| Happy path | Sprint 5 active; `E12S2` blocked (dep `E12S1` not done), `E12S3` leased by bot-9 in another worktree, `E12S4` ready/unblocked/unleased | `qdev next --json` → exit 0, `next` = `E12S4` payload with `reason`; `E12S2`/`E12S3` appear under blockers with dep ids / holder | N/A |
| Nothing eligible | All active-sprint candidates blocked or leased | Exit 0, `next: null`, `blockers` names each nearest cause | N/A |
| No active sprint | No sprint has `status: active`; no `--sprint` | Exit 0, `next: null`, reason "no active sprints" | N/A |
| Explicit closed sprint | `--sprint 4`, sprint 4 `status: closed` | Selection runs over sprint 4's assignments anyway (explicit scope) | Unknown id → exit 2 `sprint_not_found` |
| `--owner me` | Identity `simon` in `core-platform`; only story owned by `team:frontend` | That story filtered out; if it was the only candidate → `next: null`, blocker names its owner | N/A |
| Own lease | `E12S4` leased by this worktree | `E12S4` returned (continuation), reason says "leased by you here" | N/A |
| Shuffled fixtures | Same entities inserted in different order / different cache scan order | Byte-identical `--json` output | N/A (property test) |
| Text mode | Same happy fixture, no `--json` | Human summary: selected story, reason lines, blockers | N/A |

## Decisions

_Resolved at checkpoint 1 (2026-09-18); user approved with these decided._

- **D-1 (was OQ-1) — epic `phase` ranking:** sort by the optional epic-file `phase` frontmatter — numerics ascending, then strings lexicographic, epics without `phase` last, tie by epic id — so the CLI reference §5 chain is honoured literally; with no `phase` data the order collapses to epic id then story seq, identical to dropping the key.
- **Token budget:** spec stays as written (~3,390 tokens, above the 900–1600 guidance) — single-goal story, no secondary slice worth deferring; risk accepted by the user at checkpoint 1.

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/next.rs` — **new**. `select_next(options) -> NextSelection` (pure over store/lease/identity inputs), `NextSelection { selected, reason, blockers }`; takes `&StorageConfig`-derived paths, never writes.
- `crates/qdev-core/src/store/mod.rs` — `EntityRecord` (L24, has `epic_id`, `seq`, `status`, `owners`), `StoryRecord` (L120), `SprintRecord`/`SprintAssignmentRecord`, `list_sprints` (L409), `get_sprint_assignments` (L417), `get_live_entity_for_derivation` (L348) — **reuse as-is**; live-only reads skip stale rows (do not offer stale entities).
- `crates/qdev-core/src/query.rs:149` — private `compute_blocked` (single-entity, `depends_on` + live status + stale/dangling); extract/republish a reusable blocked-for-enumeration helper or reimplement in `next.rs` from the same cache queries — do **not** modify `query_entity`/`query_list` behaviour. `transition.rs` `validate_dependencies` (~L294) shows the `details.blocking_ids` shape to reuse for blockers.
- `crates/qdev-core/src/sprint.rs:57` — `resolve_sprint_selection` (explicit → `config.project.default_sprint` → unique active; errors on 0/multiple). **Do not reuse as-is** for the default case (multiple active sprints are legal here); use `store.list_sprints()` filtered to `status == "active"` + live entities instead.
- `crates/qdev-core/src/lease.rs` — `get_lease` (L229, per-story lookup, shared-then-local), `list_leases` (L260, all worktrees), `StoryLease` (L18, `worktree_path`/`holder`); no TTL — any record on disk is an active lease.
- `crates/qdev-core/src/governance.rs` — `extract_entity_owners` (L50), `resolve_user_teams` (L103), `is_user_owner` (L154; empty owners = claimable = match).
- `crates/qdev-core/src/schema.rs` — `PayloadKind` (L151) **does not have `Next`** — add variant + `include_str!("../schemas/payload-next.json")` + `as_str`/`from_str_loose` arms + bump `all()` array (11 → 12); update the L147/L238 "next … deferred" comments.
- `crates/qdev-cli/src/cli.rs:36` — `Commands` enum; add `Next(NextArgs { sprint: Option<u32>, owner: Option<String> })` — `--owner` takes `me` (resolve via current identity), other values are literal owner strings.
- `crates/qdev-cli/src/main.rs` — add `handle_next` near `handle_list` (L1908); store via `open_query_store` (L1787) + `ensure_query_workspace`; identity via `resolve_author` (L2156); register `Next` in `requires_workspace` (guard true, L463); emit through `OutputEmitter` (AD-13: text errors → stderr, JSON envelope → stdout).
- `crates/qdev-cli/tests/schema_payload_cli_tests.rs:230` — `test_schema_payload_deferred_names_are_usage_errors` lists `["context", "next", "gate_run"]`; remove `"next"` and add a `payload-next.json` round-trip beside the chore tests.
- `crates/qdev-core/src/init.rs` — untouched; no new directories needed (selection is read-only).

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/next.rs` — Create: scope resolution (sprint assignments), blocked/leased/ownership filters, deterministic comparator, `NextSelection`/`NextOptions` types; reuse `store`, `lease`, `governance` APIs above.
- [x] `crates/qdev-core/src/lib.rs` — Export the next API beside `chore`/`dw`/`lease` exports.
- [x] `crates/qdev-core/src/schema.rs` + `crates/qdev-core/schemas/payload-next.json` — Add `PayloadKind::Next`, schema file (record shape in `definitions`, `next: null | Record`, `reason`, `blockers`), update `all()`/arms/comments.
- [x] `crates/qdev-cli/src/cli.rs` + `crates/qdev-cli/src/main.rs` — Add `Next(NextArgs)`, `handle_next` (workspace guard, store open, identity resolution, text + `--json` output).
- [x] `crates/qdev-core/tests/next_tests.rs` — **new**: scope from active sprints, blocked skip w/ dep ids, leased-vs-own-worktree, `--owner` filter, phase ranking, and the shuffled-order property test (same fixture permuted → identical output).
- [x] `crates/qdev-cli/tests/next_cli_tests.rs` — **new**: real-`git init` + seeded cache fixture (pattern from `init_cli_tests.rs:586`): happy path, nothing-eligible (`next: null`, exit 0), `--sprint` closed/unknown cases, `--owner me`, no-active-sprint case.
- [x] `crates/qdev-cli/tests/schema_payload_cli_tests.rs` — Drop `"next"` from the deferred-names test; add `payload-next.json` round-trip; confirm the `PayloadKind::all()` invariant covers `next`.

**Acceptance Criteria:**
- Given stories across active sprints, when running `qdev next --json`, then the selection follows the §5 ordering, skips leased and blocked stories, and returns the story payload plus `reason` fields for every filter applied.
- Given identical entity/lease/sprint data presented in different fixture order, when selecting twice, then output is identical (property test).
- Given nothing eligible, when running `qdev next --json`, then exit is 0 with `next: null` and a non-empty `blockers` list.
- Given a story leased by this worktree, when running `qdev next`, then that story is returned; a story leased by another worktree or by another holder is never returned.
- Given any inputs, then no file, cache row, lease, or story state changed (assert in tests via directory listing / cache checksum).
- Given `qdev schema payload next`, then it prints the payload schema and exit is 0 (no longer a usage error).

## Implementation Notes

Done as specified; notable choices and post-review changes below.

**Decisions made while building:**
- Selection treats an empty `owners` list as claimable (`is_user_owner` returns true), so an unowned story remains eligible and ranks equal to own-owned stories; reason notes were reworded (post-review) so an unowned story is described as *claimable*, never *owned by the current identity*.
- `--owner` without `me` is a literal owner string compared via `normalize_team_name` (bare team names match `team:` owners); it hard-filters candidates, while an absent `--owner` only re-ranks.
- Sprint scope: explicit `--sprint N` selects over that sprint's assignments whether or not it is active (documented CLI contract); default scope is all live `active` sprints — `resolve_sprint_selection` was deliberately not reused because several active sprints are legal here.
- Blocked/lease/stale/owner mismatches never reach selection; they surface as `blockers` entries with the dep ids / holder names, so a `next: null` result always carries the nearest causes.

**Added at the reviewer's direction after the first review (2026-09-18):**
- `next` (plus `--sprint 5` / `--owner me` variants) added to `guarded_invocations()` — the workspace-guard file's "pins every arm" invariant was violated by an unpinned `Next` arm.
- Core tests added for the `stale`/`missing` branches (stale row + assignment to an id with no cache row) and for `no_candidates` (assignment-less sprint, default and explicit) — both branches shipped with zero coverage.
- `payload-next.json`: trailing newline added; `blocked` pinned to `const: false` per its own "Always false" description.
- `docs/cli-reference.md` §5 reworded to state the actual ordering chain, including the leased-by-this-worktree continuation and the `--sprint`/`--owner` semantics; and the null-result `reason.notes` no longer restate `reason.summary` — they carry the actionable detail instead.


## Spec Change Log

| Date | Source | What was flagged | What changed |
|------|--------|------------------|--------------|

## Review Triage Log

_Review 1 (three layers, `baseline_commit: 2f2ec16a7d430d930442ff107ff68b985961e07c`). One row per finding; `patch` entries are fixed in this cycle; `false` rows are recorded with what disproves them._

| # | Layer | Finding | Verdict | Evidence | Route |
|---|-------|---------|---------|----------|-------|
| 1 | blind-hunter | `docs/cli-reference.md` §5 keeps the "not leased" chain while `next.rs` keeps an own-worktree lease eligible; new `qdev next` behaviour (`reason` codes, `--owner` semantics) undocumented | low | Confirmed: §5 predates the implementation and its own example already shows a held-lease story being returned. Fixed: §5 reworded to state the real chain and flags | patch — fixed |
| 2 | blind-hunter | `stale` blocker kind claimed unreachable (no producer of `stale: true`); `missing` detail text claimed wrong for never-existing files | false | Retained-stale rows are a designed cache state (`store/mod.rs` `EntityPresence`, Story 1.20: a file that fails its last parse keeps its row with `stale`); unit fixtures construct it directly and the new tests pin both branches. `missing`'s "run `qdev sync`" advice is correct for an assignment to an id with no cache row | rejected |
| 3 | blind-hunter | `no_candidates` path never exercised | low | Same finding as row 21 — see there; fixed with the same tests | patch — fixed (dedup of 21) |
| 4 | blind-hunter | No validation of `--owner`; `--owner simon` doesn't expand teams | false | `--owner` with a non-`me` value is spec'd as a literal owner string; a typo surfaces loudly via `all_filtered` + owner blocker naming the owner; team-name normalisation already covers bare vs `team:` forms | rejected |
| 5 | blind-hunter | `config.project.default_sprint` never consulted | false | Spec (and Code Map) deliberately set default scope to all live `active` sprints and say not to reuse `resolve_sprint_selection`; no documented contract asks `next` to honour `default_sprint` | rejected |
| 6 | blind-hunter | `in_active_sprint` comparator key can never discriminate | false | The key mirrors the docs' ordering chain verbatim and never mis-ranks; removing it would deviate from the literal §5 contract without changing any result | rejected |
| 7 | blind-hunter | `NextBlocker.kind`/`NextReason.code` are plain `String`s instead of enums | false | No reachable wrong value: codes/kinds are written from a fixed set in one module and pinned by schema round-trip tests; no named harm | rejected |
| 8 | blind-hunter | `payload-next.json` missing trailing newline; invariant fields unconstrained by schema | low | Confirmed both; fixed: newline added, `blocked` pinned `const: false` | patch — fixed |
| 9 | blind-hunter | Missing tests: workspace-guard case for `next`, `--owner ''`, non-numeric `--sprint`, stale-row sprint | low | Guard case was a real unpinned arm — fixed (3 argvs added). `--owner ''` already refused by `reject_empty_filter_values`; a non-numeric `--sprint` fails in clap with exit 2 — nothing to pin | patch — fixed (partially; remainder rejected) |
| 10 | blind-hunter | Property test permutes a fixture with a single eligible candidate, so ranking is never reordered | false | The AC requires identical output for identical inputs under shuffled fixture order — satisfied; multi-candidate ranking is exercised by the phase-ranking and owner-sort tests | rejected |
| 11 | blind-hunter | `handle_next` calls `open_query_store` without `ensure_query_workspace`; purity only tested on a pre-built cache | false | The central `requires_workspace` guard (with `Next` registered) prevents the stray-DB path entirely — the finding's own demonstration requires deleting that guard; the writes-nothing test compares cache rows directly, which is the contract that matters | rejected |
| 12 | blind-hunter | `resolve_author` error arm unreachable; identity invented as `developer` without config | false | Defensive arms over a total chain; the `resolve_author` chain intentionally falls back to git email then `developer` — documented in the spec's Code Map | rejected |
| 13 | blind-hunter | Null-case `reason.notes` restate the summary; no-filter note says "ranked below" for stories that actually tie | low | Confirmed at `next.rs` — `is_user_owner` returns true on empty owners, so the printed explanation contradicted the comparator; and the early-return notes duplicated the summary | patch — fixed: unowned stories now described as claimable; notes carry the actionable detail |
| 14 | blind-hunter | Blocked detection reads only cache relations (not front-matter `depends_on`); no cycle hint | false | Post-`sync` the cache mirrors the front-matter relations — an unsynced edit is exactly what "stale cache, run sync" means and every read honours it; a two-story cycle honestly yields `next: null` with each story blocking on the other | rejected |
| 15 | blind-hunter | No second choice / `--limit` / structured filter echo for unattended loops | false | Out of intent: the story, CLI reference, and spec all define one selection plus blockers; nothing asks for alternates | rejected |
| 16 | blind-hunter | Spec bookkeeping: empty Implementation Notes, empty logs, `in-review` vs sprint-status `in-progress`, stale Code Map names | false | Same as Story 2.10's row 16: logs are filled by this very triage; sprint-status moves to `review` at step-05; Code Map records pre-implementation investigation, not shipped API | rejected |
| 17 | edge-case-hunter | Explicit `--sprint` skips the live-entity check the default path applies | false | Deliberate per spec/Code Map: explicit `--sprint` selects over that sprint's assignments "whether or not it is active"; a user naming a sprint means that scope | rejected |
| 18 | edge-case-hunter | Non-story assignment skipped without a blocker; could yield `all_filtered` with zero blockers | false | Only reachable by hand-writing a sprint file assigning a non-story id — not a state any qdev command produces; adding a guard branch for it is more than a direct correction | rejected |
| 19 | edge-case-hunter | Corrupt lease JSON treated as unleased, so two agents could pick the same story | false | A corrupt lease file is not a state this story's tools produce (claim writes valid JSON); the story still gets picked and the second claim collides at `claim` — failure stays loud | rejected |
| 20 | verification-gap | `next` missing from `guarded_invocations()` — the one `requires_workspace` arm with no pinning test | medium | Filed pre-verified; the guard file's own invariant is "pins every arm" — `next` was not run outside a workspace anywhere | patch — fixed: three `next` argvs added; test passes |
| 21 | verification-gap | `stale`/`missing` branches never produced by any test | medium | Filed pre-verified; every fixture used `stale: false` and real story files; contract ships in schema enum with zero coverage | patch — fixed: `test_stale_row_and_missing_assignment_are_blockers_not_candidates` |
| 22 | verification-gap | `no_candidates` path never produced by any test | medium | Filed pre-verified; every fixture's sprint had assignments; first-run workspaces hit this path | patch — fixed: `test_sprint_without_assignments_gives_no_candidates` (default and explicit scope) |
| 23 | verification-gap | Other: null-branch text output never run by a test (coverage note); `payload-next.json` missing trailing newline | low | Newline real and fixed; the text-null branch is a note only — no AC or matrix row requires it | patch — fixed (newline only) |

**No `intent_gap` or `bad_spec` entries: nothing required re-opening the frozen intent, so there was no loopback.** All `patch` rows are implemented; rows 2, 4–7, 10–12, 14–18 are rejected with their refutations.

## Design Notes

Comparator (eligible candidates), per cli-reference §5: `(in_active_sprint desc, then sprint id asc, then owner_match desc, then epic phase (per OQ choice), then story seq asc, then story id asc)` — blocked and leased-elsewhere stories never reach selection; they are recorded as blockers. Blockers entry: `{ story_id, kind: "blocked"|"leased"|"stale", detail }` with dependency ids for blocked and holder/worktree for leased.

Example: sprint 5 active, `E12S2` blocked by `E12S1` (not done), `E12S3` leased by bot-9 in `/wt/x`, `E12S4` ready → `{ "next": { "id": "E12S4", … }, "reason": { … }, "blockers": [ { "story_id": "E12S2", "kind": "blocked", "detail": "waiting on E12S1" }, { "story_id": "E12S3", "kind": "leased", "detail": "held by bot-9 in /wt/x" } ] }`.

## Verification

**Commands:**
- `cargo test --test next_tests` — expected: scope, filters, ownership, phase, and determinism property test green.
- `cargo test --test next_cli_tests` — expected: end-to-end selections against seeded fixtures, incl. `next: null` exit 0.
- `cargo test --test schema_payload_cli_tests` — expected: `payload-next.json` round-trips; deferred-names test passes with `next` removed; `all()` invariant covers `next`.
- `cargo test --workspace` — expected: full suite green.
- `cargo clippy --workspace --all-targets -- -D warnings` — expected: clean (CI gate).
- `cargo fmt --all -- --check` — expected: no diff in touched files.
