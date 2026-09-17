---
title: 'Story 2.1: State Machine & Lifecycle Hooks'
type: 'feature'
created: '2026-09-12'
baseline_commit: '23ca087881599ace0441221ab446732cc3a19e70'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Story status updates currently occur via general frontmatter mutations (`qdev update`) without enforcing lifecycle rules, readiness prerequisites, dependency blockers, or lifecycle hooks for downstream systems like leases and verification gates.

**Approach:** Introduce a dedicated story transition engine and CLI command (`qdev transition story <id> <target_status> [--justification <reason>]`) with registered synchronous pre- and post-transition hooks, validating legal forward edges, enforcing readiness gates on `draft → ready`, blocking transitions on `ready → in-progress` while unmet dependencies exist, requiring justification for terminal `superseded`/`abandoned` moves, and automatically resolving `closes_dw` targets on `done`.

## Boundaries & Constraints

**Always:**
- Execute status mutations exclusively through the atomic write path (`apply_entity_update`), acquiring the workspace write lock and bumping entity versions.
- Evaluate legal forward edges: `draft → ready`, `ready → in-progress`, `in-progress → review`, and `review → done`.
- Run registered `pre_transition` hooks synchronously in order; abort immediately on the first hook error and execute zero mutations.
- Enforce `draft → ready` prerequisites: refuse with exit 1 (`readiness_criteria_unmet`) if the story lacks an acceptance criterion heading section, a valid non-empty `appetite`, or at least one `target_modules` entry.
- Enforce `ready → in-progress` dependencies: refuse with exit 3 (`story_blocked`) if `blocked` is true, citing all blocking story IDs in the error message and details.
- Require `--justification` for `superseded` and `abandoned` transitions from any non-terminal state; refuse with exit 3 (`needs_justification`) if omitted.
- Disallow any transition out of terminal states (`done`, `superseded`, `abandoned`), exiting with exit 1 (`invalid_transition`).
- Automatically set all target entities listed under `relations.closes_dw` to `status: "done"` with `resolution: "<story_id>"` via the write path when a story reaches `done`.
- Emit standard AD-13 JSON envelopes with exit codes (0: success, 1: logical failure, 2: usage, 3: policy refusal, 4: infrastructure failure, 5: conflict).

**Never:**
- Never bypass the advisory write lock or mutate Markdown frontmatter outside the canonical write path.
- Never run `post_transition` hooks if `pre_transition` hooks or the write path fail.
- Never transition entities other than `story` (refuse with exit 2 `usage_error`).
- Never allow backward transitions or terminal jumps without valid justification.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Legal forward transition | `qdev transition story E12S4 review` (status: `in-progress`) | State updated to `review`, version bumped, cache synced, post-hooks executed | Exit 0 |
| `draft → ready` missing AC section | Story lacks markdown heading `## Acceptance Criteria` | Transition rejected, entity unchanged | Exit 1 `readiness_criteria_unmet` |
| `draft → ready` missing appetite | Story has no `appetite` in frontmatter | Transition rejected, entity unchanged | Exit 1 `readiness_criteria_unmet` |
| `draft → ready` missing target modules | Story has empty or absent `target_modules` | Transition rejected, entity unchanged | Exit 1 `readiness_criteria_unmet` |
| `ready → in-progress` blocked | Story has unmet `depends_on` target (not `done`) | Transition refused citing blocking story IDs | Exit 3 `story_blocked` |
| Transition to terminal without justification | `qdev transition story E12S4 abandoned` (no `--justification`) | Transition refused | Exit 3 `needs_justification` |
| Transition to terminal with justification | `qdev transition story E12S4 abandoned --justification "Superseded by bet"` | Status updated to `abandoned`, version bumped | Exit 0 |
| Transition out of terminal state | Target is `done` / `abandoned` / `superseded` | Transition rejected | Exit 1 `invalid_transition` |
| Transition to `done` closes DW | Story with `relations.closes_dw: ["DW-7f3a"]` transitions to `done` | Story updated to `done`; `DW-7f3a` updated to `status: done`, `resolution: "<story_id>"` | Exit 0; DW write errors fail transition |
| Pre-transition hook refusal | A registered `pre_transition` hook returns error | Aborts transition immediately, frontmatter untouched | Exits with hook's error code and message |
| Lock contention | Lock file held by another process for > 5000ms | Aborts with lock timeout | Exit 5 `lock_timeout` |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/workflow/mod.rs` (or `crates/qdev-core/src/transition.rs`) -- New module defining `StoryState`, `TransitionContext`, `PreTransitionHook`, `PostTransitionHook`, `TransitionEngine`, and transition execution.
- `crates/qdev-core/src/write.rs` -- Write path integration: reuse `apply_entity_update`, `resolve_entity_file`, `parse_heading_line`, and `acquire_write_lock`.
- `crates/qdev-core/src/query.rs` -- Dependency / `blocked` resolution: reuse `get_live_entity_for_derivation` and relation inspect routines.
- `crates/qdev-core/src/lib.rs` -- Re-export transition types and engine from `qdev_core`.
- `crates/qdev-cli/src/cli.rs` -- Add `Transition(TransitionArgs)` subcommand to `Commands`.
- `crates/qdev-cli/src/main.rs` -- Implement `handle_transition`, wiring CLI arguments, author resolution, and emitting text/JSON envelope outputs.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/transition.rs` -- Implement story state machine, readiness validator, blocking dependency checker, pre/post hook runner, and DW closure logic.
- [x] `crates/qdev-core/src/lib.rs` -- Export `transition` module types (`StoryState`, `TransitionEngine`, `TransitionPayload`, hooks).
- [x] `crates/qdev-cli/src/cli.rs` -- Define `TransitionArgs` (`kind`, `id`, `target_status`, `--justification`).
- [x] `crates/qdev-cli/src/main.rs` -- Add `Commands::Transition` dispatch, author attribution, and JSON/text formatting.
- [x] `crates/qdev-core/tests/transition_tests.rs` -- Unit tests for transition states, readiness checks, blocked checks, DW closing, and pre/post hooks.
- [x] `crates/qdev-cli/tests/transition_cli_tests.rs` -- CLI integration tests for `qdev transition` covering success, readiness failure (exit 1), blocked refusal (exit 3), missing justification (exit 3), and JSON envelopes.

**Acceptance Criteria:**
- Given a story in `in-progress`, when `qdev transition story E12S4 review` is run, then status becomes `review` and registered pre/post hooks run.
- Given a story in `draft`, when transitioning to `ready` without AC, appetite, or target modules, then command fails with exit 1.
- Given a story in `ready` with unmet dependencies, when transitioning to `in-progress`, then command refuses with exit 3 citing blocking IDs.
- Given any non-terminal story, when transitioning to `superseded` or `abandoned` without `--justification`, then command refuses with exit 3 `needs_justification`.
- Given a story with `relations.closes_dw` targets transitioning to `done`, all referenced DW entities are updated to `status: done` with resolution set to the story ID.

### Review Findings

_Code review of story 2-1, 2026-09-16 — diff `23ca087..14a34d4` (commit `14a34d4`). Layers: edge-case-hunter, verification-gap, acceptance-auditor (blind-hunter failed at runtime — review may be incomplete). All line references are to the working tree at review time._

- [x] [Review][Patch] Concurrent transitions race: stale-read decisions overwrite each other, and a terminal state can be silently revived [crates/qdev-core/src/transition.rs:486] — Two `qdev transition` runs on the same story without `--if-version` both validate against their pre-lock reads; `apply_entity_update` re-reads the file under the write lock but patches the stale target status, so the second commit silently overwrites the first. E.g. a justified `ready → abandoned` commits, then a stale `ready → in-progress` lands `in-progress` — terminal immutability bypassed, both commands exit 0, and `validate` cannot detect it afterwards (file, version, and cache are all internally consistent). `--if-version` (exit 5 `version_mismatch`) is the only mitigation and is optional; the spec is silent on concurrency. **Resolved 2026-09-16: re-read the story status under the write lock and re-run `classify_transition` before patching** (requires exposing an in-lock re-validation seam in the shared write path). — **implemented 2026-09-17.**
- [x] [Review][Patch] DW closure runs after the story commits to terminal `done`; a DW write failure leaves the story `done` with DWs open and no retry path [crates/qdev-core/src/transition.rs:623] — Step 9 commits the story to `done` before step 10 `close_deferred_work`; a DW-side failure (IO error, lock timeout, DW schema validation failure) exits non-zero, but the story is immutable-terminal and the DW stays open — no command can retry (`invalid_transition` from terminal), and open-DW consumers (pulse, close-outs) see a closing story that is already `done`. Spec internal contradiction: the I/O matrix says "DW write errors fail transition" while the spec's own Design Notes prescribe this story-first ordering. Pre-verified companion gap: no test pins the partial-failure path — a swallowed DW error (`let _ =`) would ship undetected. **Resolved 2026-09-16: reorder — close `closes_dw` targets first, then commit the story** (failure becomes retryable: re-closing an already-done DW is idempotent, so re-running the transition self-heals), plus the pinning test for the partial-failure path. The spec's Design Notes (non-frozen) prescribe the old ordering and must be updated to match. — **implemented 2026-09-17.**
- [x] [Review][Patch] Terminal-jump `--justification` is ephemeral [crates/qdev-core/src/transition.rs:658] — the justification for `abandoned`/`superseded` is validated (exit 3 when missing) then discarded; no DEC- record or scratchpad entry records why a story was killed. Story 2.2 (since committed) gave backward transitions DEC- records; terminal jumps got none and remain unrecorded at current HEAD. The governance audit-trail intent in `epic-2-context.md` is unmet for the highest-consequence transition class. **Resolved 2026-09-16: mirror story 2.2 — DEC- record + scratchpad entry on terminal jumps** (new `decision_type` value for terminal jumps; `decision.json` amended if it's a closed enum). — **implemented 2026-09-17.**
- [x] [Review][Patch] Schema-invariant test omits the new `transition` payload [crates/qdev-cli/tests/schema_payload_cli_tests.rs:640] — `test_every_advertised_payload_name_has_a_compilable_schema` hardcodes `["story", "error", "validate", "fix_ids", "list", "sync", "doctor"]`; a semantically broken `payload-transition.json` would ship undetected. Pre-verified. — **implemented 2026-09-17.**
- [x] [Review][Patch] No round-trip test validates `qdev transition --json` output against `payload-transition.json` [crates/qdev-cli/tests/schema_payload_cli_tests.rs:607] — every other payload kind has a round-trip sibling test; drift in either direction (extra field vs `additionalProperties: false`, wrong type in the schema) ships undetected. Pre-verified. — **implemented 2026-09-17.**
- [x] [Review][Patch] Readiness gate's fenced-code-block contract for the AC heading is untested [crates/qdev-core/src/write.rs:653] — `has_markdown_heading`'s FenceTracker branch is only exercised through `replace_markdown_section`; if the fence skipping regressed, a story with `## Acceptance Criteria` only inside a fenced block would pass the `draft → ready` gate. Pre-verified. — **implemented 2026-09-17.**
- [ ] [Review][Patch] Post-transition hook ordering ("after closing DW") is not pinned by any test [crates/qdev-core/src/transition.rs:704] — the documented `PostTransitionHook` contract runs after DW closure, but the only hook-ordering test uses a story with no `closes_dw`; Epic 3 gate/audit hooks that observe DW state could silently see un-closed DW after a reordering. Pre-verified.
- [x] [Review][Patch] Cache-absent `story_blocked` error claims unmet dependencies without mentioning the missing cache [crates/qdev-core/src/transition.rs:362] — when `cache.sqlite` is absent, `validate_dependencies` refuses citing all `depends_on` IDs as unmet without checking their status and without naming the missing cache or pointing at `qdev sync`; a story whose dependencies are all `done` on disk gets a false "blocked" message. (The fail-closed behaviour itself follows the explicit-sync architecture — ruled `false` for the implicit-sync alternative in the Review Triage Log; only the misleading message is actioned here.) — **implemented 2026-09-17.**

**Rejected:**

- Readiness gate accepts any heading level vs the matrix's literal `## Acceptance Criteria` — `false`: the normative Always-list constraint is "an acceptance criterion heading section", level-agnostic; `has_markdown_heading` matches the exact title case-insensitively at any level with fence/frontmatter awareness, satisfying the constraint's wording; the matrix's `##` is fixture shape, and no named harm exists.
- CLI `qdev transition` registers zero hooks; AC #1's "registered pre/post hooks run" is only exercisable against the library API — `low`, not worth fixing: CLI hook configuration is explicitly Epic 3 scope per the Review Triage Log, no present user or developer impact, and the fix is not a direct correction.

### Review Findings (pass 2, 2026-09-17)

_Re-review of the same diff `23ca087..14a34d4` (commit `14a34d4`). Every item below was verified against the working tree at review time (`develop` @ `7eabcfc`, which contains stories 2.2–2.8), not only against the diff — several pass-1 items were marked "Resolved 2026-09-16" but the corresponding code is still absent, so they are re-filed here._

#### decision-needed — all three resolved 2026-09-17, now pending implementation

- [x] [Review][Patch] Transitions still commit on a decision made before the write lock — and the re-validation seam built to fix it is dead code [crates/qdev-core/src/transition.rs:638, crates/qdev-core/src/write.rs:1960] — `TransitionEngine::transition` resolves and reads the story, extracts frontmatter, runs `classify_transition`, the readiness gate, the dependency check and the `closes_dw` existence check, and only then calls `apply_entity_update` with a pre-computed `status: Some(target)`. `write.rs` now exposes `apply_entity_update_checked`, whose doc-comment names this exact caller ("a transition classified from a pre-lock read would otherwise commit on top of a concurrent commit"), but `grep` finds zero callers — the pass-1 resolution was implemented as a seam nobody uses. Consequence is unchanged: two `qdev transition` runs on one story (or an edit landing between read and write) both exit 0 and the second patches over the first, so a justified `ready → abandoned` can be revived to `in-progress` and `validate` cannot detect it afterwards. Options: (a) wire `transition()` to `apply_entity_update_checked` and re-run `classify_transition` plus the readiness/dependency/DW checks against the fresh content (the unimplemented pass-1 resolution); (b) make `--if-version` mandatory so a stale attempt fails with exit 5 `version_mismatch`; (c) accept last-writer-wins and record that concurrency is out of scope. **Resolved 2026-09-17: option 1 — route `transition()` through `apply_entity_update_checked` and re-run `classify_transition` plus the readiness, dependency and DW checks against the content read under the write lock.** — **implemented 2026-09-17.**
- [x] [Review][Patch] `closes_dw` still runs after the story commits to terminal `done` — partial failure leaves the story `done` with the DW open and no retry path [crates/qdev-core/src/transition.rs:623-646] — step 9 commits the story, step 10 calls `close_deferred_work`, which propagates any DW-side failure with `?` (`apply_entity_update(&dw_opts)?`). Verified in pass 1 with a mode-`000` DW file: `qdev --json transition story E12S4 done` exits 4 (`io_error`) with the story left at `status: done`, and re-running hits `invalid_transition` from terminal, so the debt can never be closed by this path. Options: (a) the agreed-but-unimplemented reorder — close `closes_dw` first, then commit the story (re-closing an already-`done` DW is idempotent, so a retry self-heals); (b) keep the order but record the unclosed DW as a finding / report it in the payload; (c) accept and document it. Note the spec's Design Notes (steps 5-6) still prescribe the current ordering, so (a) also needs those rewritten — which is why this is a decision, not a mechanical patch. **Resolved 2026-09-17: option 1 — close `closes_dw` targets first, then commit the story (re-closing an already-`done` DW is idempotent, so a retry self-heals); rewrite the Design Notes to match.** — **implemented 2026-09-17.**
- [x] [Review][Patch] Terminal jumps (`superseded` / `abandoned`) still leave no audit trail [crates/qdev-core/src/transition.rs:659-699] — story 2.2 added `append_scratchpad_entry` and `create_backward_transition_decision`, but both sit behind `if transition_kind == TransitionKind::Backward`. A `TerminalJump` validates `--justification` (exit 3 `needs_justification` without it) and then discards it: `TransitionPayload` carries no justification field, `update_opts.custom_fields` is empty, and nothing writes a DEC- or scratchpad record. Implementing the pass-1 resolution needs a `decision_type` value — `crates/qdev-core/schemas/decision.json` keeps `decision_type` a closed enum (`human_ruling`, `agent_assumption`, `cross_team_override`, `pivot`, `review_rejection`, `lease_override`), so a new value means amending the schema. Options: (a) extend the record path to `TerminalJump` with a new `decision_type` (e.g. `story_abandoned`); (b) reuse `pivot` or `review_rejection`; (c) accept that terminal jumps are unrecorded and renegotiate the AC / `epic-2-context.md` audit-trail intent. **Resolved 2026-09-17: option 1 — extend the DEC-record and scratchpad path to `TerminalJump` with a new `decision_type` (`story_abandoned`), amending `decision.json`'s closed enum.** — **implemented 2026-09-17.**

#### patch

- [x] [Review][Patch] The new `transition` payload is still outside the schema-invariant test [crates/qdev-cli/tests/schema_payload_cli_tests.rs:639-641] — `test_every_advertised_payload_name_has_a_compilable_schema` still enumerates `["story", "error", "validate", "fix_ids", "list", "sync", "doctor"]`; `PayloadKind` now has eight entries (`transition`, `claim`, `release` missing). Add `"transition"` and any other missing names. Coverage exists only as an ad-hoc assertion at `crates/qdev-cli/tests/transition_cli_tests.rs:904`, added by story 2.2 — not the project's canonical round-trip location. Same as the two open pass-1 bullets ("Schema-invariant test omits…", "No round-trip test validates…"), still unresolved. — **implemented 2026-09-17.**
- [x] [Review][Patch] Readiness gate's fenced-code-block contract is still untested [crates/qdev-core/src/write.rs:653-670] — no fixture in `transition_tests.rs` or `transition_cli_tests.rs` puts `## Acceptance Criteria` inside a fenced or indented block (`grep '```'` → zero hits), and `write.rs` has no unit test for `has_markdown_heading`; the naive rewrite `content.contains("## Acceptance Criteria")` would keep the suite green. Add a case beside `test_transition_draft_to_ready_frontmatter_comment_does_not_satisfy_ac` asserting exit 1 `readiness_criteria_unmet` with `missing: ["acceptance_criteria"]`. Same as the open pass-1 bullet. — **implemented 2026-09-17.**
- [x] [Review][Patch] Cache-absent `story_blocked` still claims unmet dependencies without naming the missing cache [crates/qdev-core/src/transition.rs:362-374] — when `cache.sqlite` is absent the engine refuses with "Story 'X' is blocked by unmet dependencies: …" citing every `depends_on` ID without checking any of them, and never says the cache was never built or points at `qdev sync`; `test_transition_ready_to_in_progress_blocked_when_cache_absent` pins exactly that message. The fail-closed behaviour is correct per the explicit-sync architecture (already ruled `false` in the pass-1 triage); the misleading message is the part that was actioned and still is not fixed. — **implemented 2026-09-17.**

#### defer

- [x] [Review][Defer] Post-transition hook contract is unpinned and unused [crates/qdev-core/src/transition.rs:650, 704-706] — the documented `PostTransitionHook` runs "after updating the story frontmatter and closing DW" but the only ordering test (`test_transition_pre_and_post_hooks`) uses a story with no `closes_dw`; and a post-hook error after a committed write still reports failure for work that succeeded. Neither is reachable from the CLI (nothing registers hooks), so nothing is hurt today. — deferred: blocked on Epic 3, which is where hook registration and hook configuration land.
- [x] [Review][Defer] CLI `qdev transition` registers zero hooks; AC #1 ("registered pre/post hooks run") is only exercisable against the library API [crates/qdev-cli/src/main.rs:1721] — `handle_transition` uses `qdev_core::TransitionEngine::new()` with empty registries and no config surface exists. — deferred: already ruled Epic 3 scope in the pass-1 triage log; unchanged and still true.

#### Rejected

- Already-`done` / `superseded` DW is force-set to `done` and its existing `resolution` overwritten; DW writes pass `if_version: None` — `low`: needs a DW already resolved by another story, the overwrite is arguably the intended semantics, and the fix adds status checks and a threaded version guard rather than correcting anything.
- Self-referential `depends_on` blocks a story forever — `low`: only reachable if someone authors a story that depends on itself, and the result is fail-closed.
- Malformed relation shapes (`depends_on: 123`, mapping, nested array; `closes_dw` as a mapping) are swallowed and treated as "no dependencies / no DW to close" — `low`: `story.json` declares `relations.*` as string arrays with `additionalProperties: false`, so this requires hand-authored frontmatter that `qdev validate` already rejects.
- Readiness gate accepts any heading level and may search frontmatter when extraction fails — `false`: `has_markdown_heading` strips frontmatter first (`extract_frontmatter_str`, whole-content fallback only when that fails) and matches the title case-insensitively at any level, which is what the level-agnostic spec constraint asks for.
- "Is this story blocked?" has two sources of truth (`validate_dependencies` vs `compute_blocked`), and neither consults a stored `blocked` field — `false`: `blocked` is computed, never authored (`story.json` has no such property); the two implementations differ only for files edited after the last `sync`, which the project's explicit-sync rule already covers.
- Readiness / dependency gates only fire on `draft → ready` and `ready → in-progress`, so a hand-authored `status: ready` reaches `in-progress` unchecked — `false`: the spec requires the gates on exactly those two edges, and hand-authoring status bypasses the write path the spec forbids.
- `TransitionOptions` keeps `entity_kind`, `target_status` and `storage` untyped — `false`/`low`: the CLI always passes configured storage, so the `.qdev/cache` fallback is library-only and matches the documented default; no caller is demonstrably hurt.
- Spec bookkeeping (`status: 'done'` with six findings unchecked, `review_loop_iteration: 0`, empty Spec Change Log, Code Map paths that were never touched, Design Notes that contradict the pass-1 resolutions) — rejected: the fix is to edit the spec under review.
- Five files in the diff are rustfmt-only churn; nothing wires the new test files into CI — `low`: no named harm, and formatting-only diffs in an already-merged commit cost nothing to fix.
- `PayloadKind::parse` accepts the plural alias `"transitions"` only for this kind — `false`: `"claim" | "claims"`, `"release" | "releases"`, etc. already do the same.

## Implementation Notes

- Implemented `StoryState` in `crates/qdev-core/src/transition.rs` covering all 7 states (`draft`, `ready`, `in-progress`, `review`, `done`, `superseded`, `abandoned`), with terminal state identification and forward ranking.
- Implemented `TransitionEngine` with pluggable `PreTransitionHook` and `PostTransitionHook` registries and synchronous execution.
- Enforced legal forward transitions (`draft -> ready`, `ready -> in-progress`, `in-progress -> review`, `review -> done`), rejecting forward skips with exit 1 `invalid_transition`.
- Enforced `draft -> ready` prerequisites (Acceptance Criteria section heading, non-empty appetite in enum, and non-empty `target_modules`), rejecting with exit 1 `readiness_criteria_unmet`.
- Enforced `ready -> in-progress` dependency blocking checks against live SQLite store and story frontmatter relations, rejecting blocked transitions with exit 3 `story_blocked` and citing blocking story IDs.
- Enforced `--justification` requirement for terminal jumps (`abandoned`, `superseded`) and backward transitions, rejecting missing justification with exit 3 `needs_justification`.
- Enforced terminal state immutability, rejecting transitions out of `done`, `superseded`, or `abandoned` with exit 1 `invalid_transition`.
- Implemented automatic closure of `relations.closes_dw` targets on transition to `done`, setting `status: done` and `resolution: "<story_id>"` via `apply_entity_update`.
- Integrated `Transition(TransitionArgs)` into `crates/qdev-cli` with `--justification`, `--author-type`, and `--author-id` support, emitting AD-13 JSON envelopes and clean human-readable text output.

## Spec Change Log

- 2026-09-17 (pass 2 review): implemented the three decisions recorded under `### Review Findings (pass 2, 2026-09-17)` — `TransitionEngine::transition` now runs through `apply_entity_update_checked` (the decision and its gates are re-judged against the content read under the write lock), `closes_dw` is closed **before** the story commits with already-`done` DWs skipped, and a terminal jump records its justification as a `DEC-` record plus a scratchpad entry using two new `decision_type` values (`story_abandoned`, `story_superseded`) added to `decision.json`, `VALID_DECISION_TYPES`, the cache `decisions` CHECK, and the CLI help — with the cache stamp raised from 3 to 4 so a cache built against the narrower constraint rebuilds. `create_backward_transition_decision` / `record_backward_transition_decision` became `create_transition_decision` / `record_transition_decision`. Also: the schema-invariant test now iterates `PayloadKind::all()`; the fenced and indented code-block readiness cases, the DW partial-failure case, the already-closed-DW case, and the stale-decision case are pinned in `crates/qdev-core/tests/transition_tests.rs`; and the cache-absent `story_blocked` refusal names the missing cache and points at `qdev sync`.

## Review Triage Log

- `crates/qdev-core/src/write.rs:638`: `medium` -- `has_markdown_heading` matches lines without skipping frontmatter delimited by `---`, treating YAML comments matching `# Acceptance Criteria` as body headings.
- `crates/qdev-core/src/transition.rs:326`: `medium` -- `validate_dependencies` ignores `SqliteStore::open` failure with `if let Ok(store)`, silently allowing blocked stories to transition if cache open fails.
- `crates/qdev-core/src/transition.rs:599`: `medium` -- `close_deferred_work` runs after the story is committed to terminal `done` without pre-validating DW target existence, risking permanent lock in `done` if a DW write fails.
- `crates/qdev-cli/src/main.rs:1247`: `low` -- `handle_transition` text output uses `println!` instead of `output.emit_text`, bypassing broken-pipe error handling.
- `crates/qdev-core/src/schema.rs:149`: `medium` -- `TransitionPayload` lacks `payload-transition.json` and registration in `PayloadKind`, violating the invariant that every shipped `--json` payload has a schema.
- `crates/qdev-core/src/transition.rs:511`: `low` -- Corrupted status on disk parsed via `StoryState::from_str` returns usage error (exit 2) instead of logical failure (exit 1).
- `crates/qdev-cli/tests/transition_cli_tests.rs`: `low` -- Pre-verified gap: `--author-type` and `--author-id` overrides on `qdev transition` lack assertion verifying updated story frontmatter attribution.
- `crates/qdev-cli/tests/transition_cli_tests.rs`: `low` -- Pre-verified gap: Closed deferred work text emission unasserted in text-mode transition to `done`.
- `crates/qdev-core/tests/transition_tests.rs`: `low` -- Pre-verified gap: Dependency blocking when `cache.sqlite` is absent on disk unasserted.
- `crates/qdev-cli/src/cli.rs:230`: `low` -- `TransitionArgs` lacks `--if-version` flag for optimistic concurrency.
- `crates/qdev-core/src/transition.rs:518`: `false` -- Persisting `--justification` into `DEC-` records and scratchpad entries is explicitly scheduled in Story 2.2.
- `crates/qdev-cli/src/main.rs:878`: `false` -- `qdev update` retaining `--status` is existing behaviour; restricting update is outside Story 2.1 scope.
- `crates/qdev-core/src/transition.rs:320`: `false` -- Implicit sync before reading cache violates explicit-sync architecture across all commands.
- `crates/qdev-core/src/transition.rs:419`: `false` -- Per-entity write locking is standard across all multi-entity operations in qdev-core.
- `crates/qdev-cli/src/main.rs:1227`: `false` -- CLI hook configuration from external gates/plugins is Epic 3 scope.
- `crates/qdev-core/src/transition.rs:188`: `false` -- Post-hook signature receives context and update result; closed DW list is not required by Story 2.1.
- `crates/qdev-core/src/transition.rs:616`: `false` -- Post-transition hooks execute after write by definition; failures cannot revert atomic writes.
- `crates/qdev-core/src/transition.rs:248`: `false` -- Target module registry validation is explicitly scheduled in Story 2.13.
- `docs/bmad/implementation-artifacts/sprint-status.yaml`: `false` -- Sprint status is updated to `in-progress` in step-03 and `review` in step-05 as required by the workflow.

### Pass 2 triage (2026-09-17)

- `crates/qdev-core/src/transition.rs:486`: `medium` -- Re-filed from pass 1 and implemented: the engine now re-validates under the write lock via `apply_entity_update_checked`.
- `crates/qdev-core/src/transition.rs:623`: `medium` -- Re-filed from pass 1 and implemented: `closes_dw` is closed before the story commits, and an already-`done` DW is skipped, so a DW write failure leaves the story retryable.
- `crates/qdev-core/src/transition.rs:658`: `medium` -- Re-filed from pass 1 and implemented: terminal jumps record `story_abandoned` / `story_superseded` DEC entries and a scratchpad line.
- `crates/qdev-cli/tests/schema_payload_cli_tests.rs:640`: `low` -- Implemented: the invariant test derives its list from `PayloadKind::all()`.
- `crates/qdev-core/src/write.rs:653`: `low` -- Implemented: fenced and indented code-block cases added to `transition_tests.rs`.
- `crates/qdev-core/src/transition.rs:362`: `low` -- Implemented: the refusal now says the cache is missing and to run `qdev sync`.
- `crates/qdev-core/src/transition.rs:433`: `low`, rejected -- re-stamping a DW that another story already closed needs an unusual fixture, and overwriting `resolution` is arguably the intent; handled incidentally by the skip above.
- `crates/qdev-core/src/transition.rs:650`: `low`, deferred -- post-hook ordering after DW closure, and error-after-commit reporting, are unreachable while nothing registers hooks; revisit with Epic 3.
- `crates/qdev-cli/src/main.rs:1721`: `low`, deferred -- CLI hook registration stays Epic 3 scope, as ruled in pass 1.
- `crates/qdev-core/src/transition.rs:300`: `low`, rejected -- `depends_on: 123`, a mapping, or nested arrays require hand-authored frontmatter that `story.json` already declares invalid.
- `crates/qdev-core/src/transition.rs:334`: `low`, rejected -- a story that depends on itself is authoring noise, and the outcome is fail-closed.
- `crates/qdev-core/src/transition.rs:233`: `false` -- the spec asks for "an acceptance criterion heading section" (level-agnostic), and `has_markdown_heading` strips frontmatter before matching; an empty-but-present section satisfies the constraint as written.
- `crates/qdev-core/src/transition.rs:297`: `false` -- `blocked` is computed, never authored (`story.json` has no such property), and the two blocked-ness routines differ only for files edited after the last `sync`, which the explicit-sync rule already covers.
- `crates/qdev-core/src/transition.rs:574`: `false` -- the spec requires the readiness and dependency gates on exactly the two edges the engine checks; hand-authoring `status: ready` bypasses the write path the spec forbids.
- `crates/qdev-core/src/transition.rs:206`: `false` -- the CLI always passes configured storage, so the `.qdev/cache` fallback is library-only and matches the documented default.
- spec/sprint bookkeeping (`status: 'done'` with findings unchecked, empty Spec Change Log, Code Map paths never touched, Design Notes contradicting the resolutions): rejected -- the fix is to edit the spec under review.
- rustfmt-only churn in five files, and no CI wiring for the new test files: `low`, rejected -- already the deferred formatting-drift entry, with no named harm.
- `crates/qdev-core/src/schema.rs:246`: `false` -- `claim|claims` and `release|releases` already carry plural aliases, so `transition|transitions` is consistent, not novel.

## Design Notes

The transition engine is structured around a pluggable pipeline:
1. Parse & validate requested state transition against the story lifecycle state machine graph.
2. Check core invariants (`draft → ready` requirements, terminal justification).
3. Check dynamic preconditions (querying blocking dependencies for `ready → in-progress`).
4. Execute `pre_transition` hooks in registered sequence. First error aborts.
5. If target state is `done`, resolve every not-yet-closed `closes_dw` entity and set `status: done`, `resolution: "<story_id>"` via `apply_entity_update` — before the story commits, so a DW that cannot be written leaves the story where it was and the transition can be re-driven.
6. Mutate story frontmatter through `apply_entity_update_checked`, which re-runs steps 1-3 against the content it reads under the advisory write lock: the decision in steps 1-3 comes from a read made before the lock, and the patch carries a status computed from it.
7. If the move needed justification (a backward transition or a terminal jump), append a `transition` entry to the story's scratchpad and log a `DEC-` record — `story_abandoned` / `story_superseded` for terminal jumps, `review_rejection` or `pivot` for backward moves — then execute `post_transition` hooks in registered sequence.

## Verification

**Commands:**
- `cargo test --test transition_tests` -- expected: All core state machine and hook unit tests pass.
- `cargo test --test transition_cli_tests` -- expected: CLI integration tests verify exit codes, error envelopes, and DW closing.
- `cargo test` -- expected: Full test suite passes without regressions.
