---
title: 'qdev validate'
type: 'feature'
created: '2026-09-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: 'f7941f373250072c7a64c23021cb85d470484309'
context:
  - 'docs/architecture.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Hydration already records `schema_violation`, `dangling_relation`, `invalid_relation_kind`, `dependency_cycle`, and `merge_conflict` findings into the cache, but nothing aggregates or surfaces them to a human, and several integrity problems (duplicate planning IDs, orphan/under-rationale deferred work, stories citing unregistered modules) are never checked anywhere.

**Approach:** Add `qdev validate [--json] [--changed] [--fix-ids [--yes]]`: surface the cache's existing findings via `list_findings()`, layer on four new checks computed fresh from `list_entities`/`list_deferred_work`/a direct file scan (not persisted to the `findings` table), filter by `--changed` against a `git merge-base` diff, exit 1 iff any `error`-severity finding survives, and offer `--fix-ids` as a guided duplicate-ID renumber.

## Boundaries & Constraints

**Always:**
- Reuse `Store::list_findings()` (`sqlite.rs:2713`) verbatim for hydration-derived codes; never duplicate their computation.
- Compute the four new codes fresh at request time; never write them into the `findings` table (that table's lifecycle belongs to hydration only). All four are severity `"error"`: `duplicate_planning_id` (2+ files under `storage.specs_dir` declare the same frontmatter `id` — found via a direct scan, since `entities.id` is a PRIMARY KEY and the last-hydrated file silently wins in cache, making duplicates invisible there), `orphan_deferred_work` (DW's `origin_story_id` set but `get_entity` returns `None`), `dw_missing_rationale` (DW `safety_risk` in `{acceptable_with_mitigation, unacceptable}` and `rationale` null/empty), `target_module_not_registered` (a story's `target_modules` entry absent from `config.modules[].id`). For the two DW-derived codes, `FindingRecord.path` comes from `get_entity(dw.id)?.source_path` — `DeferredWorkRecord` itself carries no path field.
- `--changed`: run `git merge-base HEAD <config.git.integration_branch>` then `git diff --name-only <merge-base>` via `std::process::Command`, mirroring `resolve_git_email`'s shell-out (`config/mod.rs:749`); keep only findings whose `path` is in that set, except a multi-path `dependency_cycle` (one row per participating file) is kept or dropped as a whole group if *any* of its participant paths is in the changed set — never show a partial cycle. Missing git binary/repo -> `ExitCode::InfrastructureFailure`.
- Exit `ExitCode::LogicalFailure` (1) iff any post-filter finding has `severity == "error"`; `ExitCode::Success` (0) otherwise.
- `--fix-ids` requires `Interactivity::resolve(...).is_interactive()` or `--yes`; otherwise refuse with `ExitCode::PolicyRefusal`, no writes (mirror `handle_init`'s non-interactive gate).
- `--fix-ids`'s guided renumber rewrites the duplicate file's `id`, every relation referencing the old id (via the existing write path), and citations under the union of `config.modules[].paths` — no new config field; this ties the module registry and citation-pattern config together for the first time. `hygiene.citation_pattern` matches *any* bracket citation kind (`[E1S2]`, `[AD-3]`, `[FR-12]`, `[DW-a1b2]`, ...); rewriting must only touch occurrences where the captured id equals the exact pre-renumber duplicate id, never every match of the pattern. A workspace with no `[[modules]]` configured skips citation rewriting (file + relations still rewritten).
- JSON payload items match `{code, severity, path, message}` (AC-mandated shape = `FindingRecord`'s fields).

**Never:**
- No enforcement of `modules[].may_depend_on` layering — 1.11 checks registry *membership* only.
- No hygiene linter / citation scanning beyond what `--fix-ids` needs for its own renumber (Story 3.9 stays out of scope).
- No new git crate dependency (git2/gix) — shell out to the `git` binary only.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Happy path | Clean workspace | `{findings: []}` | exit 0 |
| Duplicate IDs | Two files declare `id: E1S2` | `duplicate_planning_id` on each path, message names the other path(s) | exit 1 |
| Cache-native findings | Hydration already recorded `dependency_cycle`/`merge_conflict` | Both surfaced via `list_findings()` | exit 1 |
| Orphan DW | DW's `origin_story_id: E9S9` (no such story) | `orphan_deferred_work` on the DW's path | exit 1 |
| DW missing rationale | DW `safety_risk: unacceptable`, no `rationale` | `dw_missing_rationale` | exit 1 |
| Unregistered module | Story `target_modules: ["ghost"]`, not in `config.modules` | `target_module_not_registered` | exit 1 |
| `--changed`, none touched | Findings exist outside the merge-base diff | `{findings: []}` | exit 0 |
| `--fix-ids`, non-interactive, no `--yes` | Duplicates present, `--non-interactive` | Refused, no writes | exit 3 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs:2713` `list_findings` -- reuse for hydration-derived findings
- `crates/qdev-core/src/store/sqlite.rs:4890` `collect_markdown_files` (private) -- make `pub(crate)`, reuse for the duplicate-ID disk scan
- `crates/qdev-core/src/store/mod.rs:214,126,237` `FindingRecord`/`DeferredWorkRecord`/`EntityFilter`; `Store::list_entities`/`list_deferred_work`/`get_entity` -- data sources for the 4 new checks
- `crates/qdev-core/src/schema.rs:242` `extract_frontmatter` -- reuse to read `id` per file in the duplicate scan
- `crates/qdev-core/src/config/types.rs:44-48` `GitConfig.integration_branch`; `:131-132` `ModuleConfig.id`; `crates/qdev-core/src/config/mod.rs:749` `resolve_git_email` -- pattern to mirror for the new git shell-out
- `crates/qdev-core/src/interactivity.rs` `Interactivity::resolve`/`is_interactive` -- reuse for `--fix-ids` gate
- `crates/qdev-core/src/errors.rs:12` `ExitCode` -- reuse `LogicalFailure`/`PolicyRefusal`/`InfrastructureFailure`
- `crates/qdev-core/src/envelope.rs` `JsonEnvelope` -- reuse for `--json`
- `crates/qdev-core/src/validate.rs` (new) -- `run_validation`, `find_duplicate_planning_ids`, `git_changed_files`, `filter_by_changed`
- `crates/qdev-cli/src/cli.rs:27-51` `Commands` enum, `RelateArgs`-style structs -- add `Validate(ValidateArgs)`
- `crates/qdev-cli/src/main.rs:161-232` dispatch match, `:1343` `handle_relate` pattern, `:925,939` `open_query_store`/`ensure_query_workspace` -- add `handle_validate`, thread `interactivity` like `handle_init`
- `crates/qdev-core/tests/dag_tests.rs`, `crates/qdev-cli/tests/relate_cli_tests.rs` (new siblings) -- coverage per I/O matrix

## Tasks & Acceptance

**Execution:** `validate.rs` first (pure logic, unit-testable without the CLI), then CLI wiring.
- [x] `crates/qdev-core/src/store/sqlite.rs` -- mark `collect_markdown_files` `pub(crate)`
- [x] `crates/qdev-core/src/validate.rs` -- new module: the 4 new checks + `git_changed_files` + `filter_by_changed` + `run_validation` merging with `list_findings()`
- [x] `crates/qdev-cli/src/cli.rs` -- `Commands::Validate(ValidateArgs)` with `--changed`, `--fix-ids`, `--yes` (`-y`)
- [x] `crates/qdev-cli/src/main.rs` -- dispatch + `handle_validate`: ensure workspace, open store, call `run_validation`, apply `--changed`, emit JSON/text, compute exit code; `--fix-ids` renumber (file + relations + citations under `config.modules[].paths` matching `hygiene.citation_pattern`)
- [x] `crates/qdev-core/tests/validate_tests.rs` -- unit coverage per I/O matrix (duplicate IDs, orphan DW, missing rationale, unregistered module, changed-filter); `--changed`/`--fix-ids` tests `git init` a throwaway repo fixture with its own commits/branches rather than depending on the ambient CI checkout
- [x] `crates/qdev-cli/tests/validate_cli_tests.rs` -- CLI-level coverage: `--json` shape, exit codes, `--fix-ids` non-interactive refusal

**Acceptance Criteria:**
- [x] Given a workspace with seeded defects, when `qdev validate --json` runs, then it reports findings with `code`, `severity`, `path`, `message` for all eight problem classes in the story text, and exits 1 iff any is `error`-severity.
- [x] Given `--changed`, when findings exist outside the merge-base diff against `config.git.integration_branch`, then they are excluded from the report.
- [x] Given `--fix-ids` in non-interactive mode without `--yes`, when duplicates exist, then the command refuses (exit 3) and writes nothing.
- [x] Given the entire test suite, `cargo test` passes with zero failures.

## Implementation Notes

- `crates/qdev-core/src/validate.rs` is the new pure-logic module: `run_validation` merges `Store::list_findings()` verbatim with the four freshly computed checks (`find_duplicate_planning_ids`, `find_orphan_deferred_work`, `find_dw_missing_rationale`, `find_unregistered_target_modules`), none of which are written back to the `findings` table.
- `scan_duplicate_planning_ids` re-reads every file under `specs_dir` directly (via the now-`pub(crate)` `collect_markdown_files` + `extract_frontmatter`), since the cache's `entities.id` PRIMARY KEY makes a second file declaring an already-hydrated id invisible to any cache-only check.
- `git_changed_files` shells out to `git merge-base HEAD <integration_branch>` then `git diff --name-only <merge-base>`, mirroring `resolve_git_email`'s `std::process::Command` usage but propagating failures (missing git binary, no such branch, not a repo) as `InfrastructureFailure` instead of swallowing them.
- `filter_by_changed` groups `dependency_cycle` findings by their shared cycle message (hydration writes one row per participant path, all sharing an identical `"depends_on cycle: ..."` message) so a cycle is kept or dropped as a whole group, never partially.
- `--fix-ids` is gated exactly like `handle_init`'s non-interactive flag requirements (`Interactivity::resolve(...).is_interactive() || --yes`, else `PolicyRefusal` with no writes) before any duplicate scan runs. The guided renumber:
  1. Keeps the lexicographically-first path in each duplicate group as-is; every other path is offered a renumber (prompted interactively, or applied automatically under `--yes`).
  2. Rewrites the duplicate file's `id` via a new narrowly scoped primitive, `rewrite_frontmatter_id` (a line-based edit of only the top-level `id:` block), since `id` is a managed field everywhere else in the write path (`patch_frontmatter` rejects it via `custom_fields`). `patch_frontmatter` is then applied on top (no custom fields) purely to bump `version`/`updated_by` and reuse schema validation + the atomic-write + cache-upsert steps `apply_entity_update` uses.
  3. Rewrites every relation in the workspace whose `target_id` is the old id via `apply_relation_change` twice per affected `(source, relation)` pair (`unrelate` old, `relate` new) -- literally reusing `qdev relate`/`unrelate`'s own write path, per spec. The renumbered file's own outgoing relations need no separate rewrite: their `source_id` is re-derived from the file's own frontmatter at the next hydration sweep.
  4. Rewrites bracket citations of the old id to the new id under the union of `config.modules[].paths` (skipped entirely when no `[[modules]]` are configured), using `hygiene.citation_pattern` (or the built-in default), and only where the captured id inside the brackets exactly equals the pre-renumber id (`rewrite_citations`).
  5. `next_available_id` only supports the sequentially numbered kinds that can appear under `specs_dir` (Epic, Story, ADR, FR, NFR, Hazard, PRD); DW/Decision/Constraint identifiers are rejected as unsupported, since they cannot arise from `scan_duplicate_planning_ids` by construction (DW/Decision live under `state_dir`, and constraints are not top-level frontmatter ids).
  6. A minimal, dependency-free glob matcher (`glob_match`) was added since `config.modules[].paths` had no path-matching implementation anywhere in the codebase before this story (spec explicitly notes this ties the module registry and citation-pattern config together for the first time).
- `regex` moved from `qdev-core`'s `[dev-dependencies]` to `[dependencies]` (it was already a transitive workspace dependency via dev-deps/tests) since citation rewriting needs it at runtime; re-exported as `qdev_core::regex` for the CLI layer.
- `write::current_iso8601` was made `pub` (post-review; originally `pub(crate)`) and is reused directly by the CLI layer's cache-upsert path in `renumber_duplicate_file` — no second copy of the timestamp algorithm exists.

## Risks / follow-ups (not blocking, flagged for visibility)

- `rewrite_citations_under_modules` walks the entire workspace tree (skipping only `.git`/`.qdev`) and attempts a UTF-8 read of every file whose relative path matches a configured module glob; this is fine for the fixture-sized workspaces this story's tests exercise but is an O(files) walk with no `.gitignore` awareness (e.g. it would also open a `target/` or `node_modules/` directory if a module's glob were broad enough to include one). Acceptable per spec's explicit scope ("Story 3.9 stays out of scope" for a full hygiene/citation linter), but worth tightening if module globs ever get broad in practice.
- The duplicate-id renumber's "keep the lexicographically-first path" tie-break is deterministic but arbitrary (not based on git history, mtime, or which file is "more canonical"); this matches the spec's silence on tie-break policy but is worth a human decision if it ever surprises someone in practice.
- `--fix-ids`'s per-item loops (relation rewrite, citation rewrite, the outer duplicate-group loop) still abort on the first I/O or write error without rollback (Review Triage Log #11): rare in practice, and a correct fix needs transactional/resumable semantics beyond a direct correction, so it was rejected rather than patched.

## Post-implementation review fixes (bmad-build step-04)

Three independent review layers (blind-hunter, edge-case-hunter, verification-gap) found 16 candidate issues; see the `## Review Triage Log` above for the full triage. 10 were routed `patch` and applied directly (no re-derivation subagent was available to re-engage, so these were applied by the orchestrating session):
1. `git_changed_files` now also includes untracked files (`git ls-files --others --exclude-standard`), so a freshly created duplicate file is no longer invisible to `--changed`.
2. `rewrite_relations_to` now adds the new relation edge before removing the old one, so a mid-sequence failure leaves an extra edge instead of losing the relation entirely.
3. `handle_fix_ids`'s post-renumber rescan now calls `run_validation`/`has_error_finding` (the full validation surface) instead of only re-checking for remaining duplicate ids, matching the documented exit-code rule.
4. `qdev validate --changed --fix-ids` is now rejected with a usage error instead of silently ignoring `--changed`.
5. `renumber_duplicate_file`'s kind inference now reuses hydration's own `determine_entity_kind` (newly exposed as `pub`) instead of a narrower, independently maintained copy that missed several entity kinds' directory conventions.
6. `write::current_iso8601` was made `pub` and reused, deleting the duplicated date-math function in the CLI layer.
7. `render_validate_text` now strips embedded newlines from a finding's message before rendering, preserving its one-line-per-finding contract.
8. `rewrite_frontmatter_id` now preserves a trailing inline YAML comment on the `id:` line, if any, instead of discarding it.
9. Added `test_fix_ids_rewrites_relations_pointing_at_renumbered_id` (CLI test) closing the verification-gap on relation rewriting.
10. Added `test_fix_ids_rewrites_citations_under_configured_module_paths` (CLI test) closing the verification-gap on citation rewriting.

The remaining 6 findings were rejected after verification (false claims, or real-but-low-severity findings whose fix would require more than a direct correction, or out-of-scope feature requests) — see rows 11-16 of the Review Triage Log for evidence per finding. All fixes were re-verified: `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cargo test --workspace` all pass with zero warnings and zero failures (including the two new tests).

## Spec Change Log

## Review Triage Log

| # | Finding | Verdict | Evidence | Route |
|---|---------|---------|----------|-------|
| 1 | `git_changed_files` (`validate.rs:269-280`) uses `git diff --name-only <merge-base>`, which never lists untracked (not yet `git add`ed) files, so a brand-new duplicate file is silently excluded from `--changed`-filtered findings | medium | Verified: `git diff --name-only` only reports tracked-file changes; no call to `git ls-files --others --exclude-standard` or equivalent exists anywhere in `git_changed_files`. The most common real trigger for `duplicate_planning_id` (a freshly created file) is exactly the case `--changed` would miss. | patch |
| 2 | `rewrite_relations_to` (`main.rs:2237-2263`) does `unrelate(old)` then `relate(new)` per affected relation; if the `relate` call fails after `unrelate` succeeds, the relation is permanently dropped rather than retargeted | medium | Verified both calls use `?` with no compensating action; a mid-sequence failure (OCC conflict, disk error) leaves neither the old nor new edge. Reordering to `relate(new)` then `unrelate(old)` makes a partial failure leave an extra (safe, visible) edge instead of none. | patch |
| 3 | `handle_fix_ids`'s post-renumber rescan (`main.rs:2016-2024`) only re-runs `scan_duplicate_planning_ids`, not full `run_validation`, despite its own comment claiming to follow "the shared exit-code rule" (exit 1 iff any error-severity finding survives) | medium | Verified: the rescan ignores `orphan_deferred_work`/`dw_missing_rationale`/`target_module_not_registered`/hydration findings entirely, so `--fix-ids` can exit 0 while a plain `qdev validate` immediately after would exit 1 — contradicting the frozen Boundaries' exit-code rule, which this story's own comment invokes. | patch |
| 4 | `handle_validate` (`main.rs:1779-1788`) dispatches to `handle_fix_ids` before ever inspecting `validate_args.changed`, so `qdev validate --changed --fix-ids` silently renumbers the whole workspace instead of a changed-scoped subset, with no warning | medium | Verified the `fix_ids` branch returns unconditionally; `changed` is read nowhere in `handle_fix_ids`. A mutating command silently expanding its blast radius beyond what the combined flags suggest is real harm, and the fix (reject the combination with a usage error) is a trivial guard. | patch |
| 5 | `renumber_duplicate_file`'s kind-inference fallback, `infer_kind_from_path` (`main.rs:2199-2216`), is a narrower reimplementation of the existing (but module-private) `determine_entity_kind` (`sqlite.rs:4930-4988`) — missing sprints/releases/dw/decisions/scratch/soup/evidence directories and the identifier-kind-parse fallback | medium | Verified line-by-line: `determine_entity_kind` covers 13 directory patterns plus an `Identifier`-kind fallback and a `Story` default; `infer_kind_from_path` covers only 6 and returns `None` otherwise. A duplicate DW/Decision/Sprint/etc. file without an explicit `kind:` field would fail `--fix-ids` with `unknown_entity_kind` even though hydration infers it correctly. | patch |
| 6 | `chrono_placeholder_now` (`main.rs:2173-2198`) reimplements the same civil-calendar algorithm `write::current_iso8601` already has, solely because that function was made `pub(crate)` instead of `pub` | low | Confirmed by the diff's own doc comment on `chrono_placeholder_now`. Two independent copies of non-trivial date math can silently drift; fix is a one-line visibility change plus deleting the duplicate. | patch |
| 7 | `render_validate_text` (`main.rs:1738-1757`) doesn't guard an embedded newline in `FindingRecord.message`, breaking its own documented "one line per finding" contract if one ever occurs | low | Verified no current message-producing code (existing hydration codes or the four new ones in `validate.rs`) embeds a newline today, so this is latent, not triggered. Fix (`.replace('\n', " ")`) is trivial, so not rejected despite low everyday likelihood. | patch |
| 8 | `rewrite_frontmatter_id` (`validate.rs:390-450`) fully replaces the matched `id:` line, discarding any trailing inline YAML comment, contradicting architecture.md's stated design goal that qdev's frontmatter patches "preserve comments and ordering" | low | Verified `result_lines.push(format!("id: {}{}", new_id, newline))` drops everything after the original value. Uncommon in practice (hand-written inline comments on `id:` specifically) but a direct, self-contained fix (preserve trailing `# ...` text) is available. | patch |
| 9 | No end-to-end test verifies `--fix-ids` rewrites relations targeting the renumbered id to the new id | medium | verification-gap layer (pre-verified): read all three `--fix-ids` CLI tests plus `validate_tests.rs`'s inline tests — none seeds a relation to the duplicated id or asserts a post-renumber target. `rewrite_relations_to` is CLI-crate-private, so `qdev-core`'s test crate cannot reach it either. | patch |
| 10 | No end-to-end test verifies `--fix-ids` rewrites bracket citations under a configured `[[modules]]` path | medium | verification-gap layer (pre-verified): confirmed no test configures `[[modules]]` + places a citation file + runs `--fix-ids` end-to-end; only the low-level `rewrite_citations` regex primitive is unit-tested in isolation. | patch |
| 11 | `rewrite_citations_under_modules`/`handle_fix_ids`'s per-item loops abort on the first I/O or write error mid-sequence, leaving a partially rewritten/renumbered workspace with no completion report | low | Verified (`?` propagation with no rollback in both the citation-rewrite file loop and the outer duplicate-group loop). Real but requires an actual I/O/write failure mid-run to trigger; a correct fix needs transactional or resumable semantics, which is more than a direct correction. | reject (low, unlikely trigger, fix beyond direct correction) |
| 12 | `--fix-ids --json` in an interactive terminal without `--yes` still issues a blocking `y/N` prompt, which could stall a scripted caller expecting machine-readable, non-interactive output | low | Verified this exactly mirrors `handle_init`'s existing, pre-existing pattern (`main.rs:390,426,459,499` all prompt regardless of `cli.json`) — the spec explicitly instructed mirroring `handle_init`'s interactivity gate. Not a defect introduced by this story; a codebase-wide `--json`-implies-non-interactive rule is out of this story's scope. | reject (matches established pre-existing pattern, out of scope) |
| 13 | `scan_duplicate_planning_ids` silently skips a file it cannot read (`Err(_) => continue`) or whose `id` is a valid frontmatter value that isn't a string, potentially missing a real duplicate | low | Verified both `continue` branches exist. Matches the codebase's existing scanning convention (`id.rs`'s `allocate_next_story_id`/`hex_id_collides` also silently skip unreadable/non-matching entries). A non-string `id` would already surface as `schema_violation` via existing hydration, covering the malformed file by another code path. Unlikely in everyday use; not a direct/trivial fix without broader scanning-convention changes. | reject (low, unlikely, covered by existing schema_violation path) |
| 14 | The `(path, code)` sort comparator is duplicated between `main.rs::sort_findings` and `validate.rs::filter_by_changed` | false | Verified both implementations are currently byte-identical in behavior; no divergence exists today. "Could drift in the future" names no current harm — a style preference, not a defect. | reject |
| 15 | `--fix-ids` always resolves the acting author via `resolve_author(None, None, ...)` with no CLI flag to override it, unlike `qdev update`/`relate` | low | Verified `ValidateArgs` has no `--author-type`/`--author-id` fields and the story's AC never mentions author override for `validate`. This is a feature request beyond the captured intent, not a bug in what was asked for. | reject (out of scope per intent) |
| 16 | `find_orphan_deferred_work`/`find_dw_missing_rationale` (`validate.rs`) silently produce no finding when `store.get_entity(&dw.id)?` returns `None`, even though the DW row exists in `deferred_work` | false | Verified `delete_kind_detail_row` (`sqlite.rs:3775-3778`) deletes a DW's `deferred_work` row in the same purge transaction that removes its `entities` row, and both are inserted together during hydration — so a `deferred_work` row with no matching `entities` row is not reachable through any code path this story or the existing cache lifecycle touches. | reject |

### 2026-09-09 — Cross-story review of stories 1.9-1.13 (bmad-review)

**`--fix-ids` reference rewriting — reviewed, boundary reaffirmed, behaviour kept.**

A cross-story review challenged Boundaries §"Always" — that the guided renumber rewrites "every
relation referencing the old id (via the existing write path), and citations under the union of
`config.modules[].paths`" — on the grounds that it is ambiguous in the duplicate case.

The argument: the renumber keeps the duplicated id on the group's lexicographically-first file
and renumbers the others, so a reference naming that id could have meant either file, and the
keeper still holds it. Redirecting every reference to the new id therefore points references
that meant the keeper at the renumbered entity instead. Nothing in the workspace records which
file a given reference meant, so the tool cannot infer it.

**Decision (human, 2026-09-09): keep the specified behaviour.** References follow the renumbered
entity. This is the boundary as approved, and it is what a renumber means everywhere else in the
tool. The ambiguity is real but unresolvable from the data, and the alternative (leaving
references on the old id) silently strands the renumbered entity instead — a coin flip either
way, so the specified behaviour stands.

What that means in practice, now documented at every layer (`rewrite_relations_to`,
`payload-fix-ids.json`, and the two CLI tests): after a renumber, incoming references point at
the renumbered file, and the keeper — which still holds the original id — has none. A human who
needs the other split reviews `renumbered[].relations_rewritten` and
`renumbered[].citations_rewritten` in the payload and moves them back.

Also changed in this pass, and *not* a boundary question: the renumber now takes the advisory
write lock for each write (it took none), the reference rewriting runs outside that lock because
`apply_relation_change` acquires it itself and it is not reentrant, the confirmation prompt is
collected before any lock is taken, a partial failure reports what it already wrote inside the
payload rather than as a second JSON document on stdout, and a reconciling sweep runs afterwards
so the keeper's id is hydrated back into the cache.

## Design Notes

- The duplicate-ID scan is independent of the cache (it re-reads every file under `specs_dir`), because the `entities.id` PRIMARY KEY means hydration's `ON CONFLICT(id) DO UPDATE` silently makes the last-processed file win — a second file declaring the same `id` leaves no trace in the cache to detect against.
- `run_validation` returns a flat `Vec<FindingRecord>` mixing cache-native and freshly-computed findings so the CLI layer has one shape to sort, filter, and serialize.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all unit, integration, and CLI tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
