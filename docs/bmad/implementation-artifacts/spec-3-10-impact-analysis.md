---
title: 'Story 3.10: Impact Analysis'
type: 'feature'
created: '2026-09-26'
status: 'done'
baseline_commit: '51a0f5a0f74335ecce15f98567d6fd7e877cbfbc'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/cli-reference.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Reviewers and developers making or reviewing changes lack a fast, deterministic view of the blast radius across active concurrent stories, requirements, hazards, ADRs, dependent stories, overlapping verification gates, and inline code citations.
**Approach:** Implement `qdev impact [STORY_ID] [--paths <PATHS...>]` in `crates/qdev-core` and `crates/qdev-cli`, resolving affected modules via `ModuleRegistry`, discovering active stories in `in-progress` or `review` that touch those modules, computing transitive dependents with depth via reverse `depends_on` and `extends`, tracing linked requirements, hazards (`mitigates`), ADRs (`governed_by`), finding gates whose `verifies` overlap, and extracting compact entity citations from changed files.

## Boundaries & Constraints

**Always:**
- Support query invocation via story ID (`qdev impact E12S4`), explicit path arguments (`qdev impact --paths crates/bridge/src/lib.rs`), or both; if neither is provided, fall back to the currently active story lease from `crates/qdev-core/src/lease.rs`, or return a usage error (exit 2) if no lease exists.
- Resolve modules using `ModuleRegistry::resolve_path` for paths and `resolve_story_target_modules` for stories.
- Report all stories in the SQLite cache whose `target_modules` overlap with resolved modules and whose status is `in-progress` or `review`.
- Compute dependents by traversing reverse `depends_on` and `extends` relations from the target story/stories with depth tracking and cycle detection.
- Trace linked requirements via outgoing `traces_to` relations and gate `verifies`, linked hazards via `mitigates`, and linked architectural decision records via `governed_by`.
- Identify gates in configuration (`config.gates`) whose `verifies` list overlaps with the reached requirements or that are configured on the target story.
- Scan changed files (either explicitly passed via `--paths` or detected via git diff when in a git worktree) for inline entity citations matching `hygiene.citation_pattern` (or `DEFAULT_CITATION_PATTERN`) and report unique IDs as `cited_entities`.
- Support `--json` output conforming to `crates/qdev-core/schemas/payload-impact.json` wrapped in the standard `JsonEnvelope`, and human-readable text output by default.
- Register `PayloadKind::Impact` with `crates/qdev-core/src/schema.rs` and make `qdev schema payload impact` return the JSON schema.

**Never:**
- Never mutate workspace files, cache tables, or git state during impact analysis (`qdev impact` is strictly a read-only query command).
- Never report cyclical dependents in an infinite loop; visited nodes must be tracked.
- Never crash when a path is binary or unreadable; skip unreadable files gracefully when scanning for citations.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Impact by Story ID | `qdev impact E12S4` with story in cache | Resolves target modules, active stories in modules, requirements, hazards, ADRs, overlapping gates, reverse dependents with depth, and citations from diff | Exit 0; text or JSON envelope |
| Impact by Paths | `qdev impact --paths crates/core/src/lib.rs` | Resolves module for path, discovers active stories in module, traces relations and citations in path | Exit 0; text or JSON envelope |
| Impact with Lease Fallback | `qdev impact` with active story lease | Automatically uses leased story ID as target | Exit 0 |
| Neither Story nor Paths nor Lease | `qdev impact` with no arguments and no lease | Explains missing target; lists usage options | Exit 2 usage error |
| Non-existent Story ID | `qdev impact NONEXISTENT` | Reports unknown story error | Exit 1 entity not found |
| Circular Dependencies | Story A depends on B and B depends on A | Traverses dependents without infinite recursion, marking correct depths | Handled gracefully |
| JSON Schema Query | `qdev schema payload impact` | Prints embedded JSON schema | Exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/impact.rs` -- Core impact analysis engine: `run_impact`, `ImpactOptions`, `ImpactOutcome`, `ImpactStory`, `ImpactDependent`, and `format_impact_text`.
- `crates/qdev-core/schemas/payload-impact.json` -- JSON Schema for `qdev impact --json` payload.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Impact` mapping to `payload-impact.json`.
- `crates/qdev-core/src/modules.rs` -- `ModuleRegistry::resolve_path` and `validate_target_modules` for module resolution.
- `crates/qdev-core/src/store/sqlite.rs` -- Relation queries (`get_relations_for_source`, `get_relations_for_target`, `list_relations`), entity queries, and story details.
- `crates/qdev-core/src/preflight.rs` -- Reuse/reference `resolve_story_target_modules` for story module resolution.
- `crates/qdev-core/src/config/types.rs` -- `DEFAULT_CITATION_PATTERN`, `GateConfig`, and `HygieneConfig`.
- `crates/qdev-core/src/lib.rs` -- Export `impact` module, types, and functions.
- `crates/qdev-cli/src/cli.rs` -- Add `Impact(ImpactArgs)` variant to `Commands` enum.
- `crates/qdev-cli/src/handlers/impact.rs` -- CLI command handler for `qdev impact`.
- `crates/qdev-cli/src/handlers/mod.rs` -- Export `impact::handle_impact`.
- `crates/qdev-cli/src/main.rs` -- Dispatch `Commands::Impact` in main CLI entry point.
- `crates/qdev-core/tests/impact_tests.rs` -- Unit tests for core impact calculations, graph traversals, and citation extraction.
- `crates/qdev-cli/tests/impact_cli_tests.rs` -- CLI integration tests for `qdev impact` with story ID, `--paths`, `--json`, and schema payload.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/payload-impact.json` -- Create JSON Schema for impact payload with modules, stories, dependents, requirements, hazards, adrs, gates, and cited_entities.
- [x] `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Impact` with `payload-impact.json`.
- [x] `crates/qdev-core/src/impact.rs` -- Implement `run_impact`, relationship traversals, module mapping, gate overlap logic, and text formatting.
- [x] `crates/qdev-core/src/lib.rs` -- Export impact types and functions.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `ImpactArgs` and `Commands::Impact`.
- [x] `crates/qdev-cli/src/handlers/impact.rs` -- Implement `handle_impact` handling JSON and text output.
- [x] `crates/qdev-cli/src/handlers/mod.rs` -- Re-export `impact` handler.
- [x] `crates/qdev-cli/src/main.rs` -- Wire `Commands::Impact` into CLI dispatch.
- [x] `crates/qdev-core/tests/impact_tests.rs` -- Add unit tests for impact module covering all acceptance criteria and edge cases.
- [x] `crates/qdev-cli/tests/impact_cli_tests.rs` -- Add CLI tests for `qdev impact` flag combinations, JSON schema output, and error conditions.

**Acceptance Criteria:**
- Given a story ID (`qdev impact E12S4`) or paths (`qdev impact --paths crates/bridge/src/lib.rs`), when run, modules are resolved via `ModuleRegistry`.
- Active stories with those `target_modules` in `in-progress` or `review` are reported with their status and title.
- Requirements reached via `traces_to` and gate `verifies`, hazards reached via `mitigates`, and ADRs reached via `governed_by` are reported.
- Dependents via reverse `depends_on` and `extends` are reported with numeric depth.
- Gates whose `verifies` overlap with reached requirements are reported.
- Inline entity citations `[E...]`, `[AD-...]`, `[DW-...]` found in changed files are reported as `cited_entities`.
- `qdev schema payload impact` prints the valid payload schema.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---|---|---|---|---|
| Whitespace-only story string bypasses target validation and lease fallback | edge-case-hunter / verification-gap / blind-hunter | `medium` | In `run_impact`, `options.story.as_deref().map(str::trim).filter(|s| !s.is_empty())` was not applied before the `if let Some(ref s)`, skipping both lease fallback and `usage_error`. | patch |
| Target story included in its own active concurrent stories list | edge-case-hunter / blind-hunter | `medium` | If the target story has status `in-progress` or `review`, it is collected under `stories`; should filter out `target_story`. | patch |
| Git worktree diff-based citation detection is unverified when paths are omitted | verification-gap | `medium` | Pre-verified gap: existing tests either specify `--paths` or do not test git diff citation extraction without paths. | patch |
| Inappropriate bypass of `ensure_cache` in CLI boot dispatch | blind-hunter | `medium` | `Commands::Impact` was exempted from boot `ensure_cache` in `main.rs:252`; removing the exemption guarantees the SQLite cache is populated/verified on boot. | patch |
| Directory file collection lacks symlink loop guard and file size limit | edge-case-hunter / blind-hunter | `low` | Recursive descent in `collect_files_recursive` should skip symlink directories to prevent infinite loops, and skip files > 10MB during text reading. | patch |
| Update `docs/cli-reference.md` for `impact` and payload schema | blind-hunter | `low` | Add `impact` to `qdev schema payload` doc and document `qdev impact [STORY_ID] [--paths <PATHS...>]`. | patch |
| Ensure only valid story entities with derivation presence populate `dependents` | blind-hunter | `low` | Add `ent.kind == EntityKind::Story` and derivation existence check when enriching dependents. | patch |
| Git diff remote-tracking fallback for integration branch | blind-hunter | `low` | Check `origin/<integration_branch>` when local ref `<integration_branch>` is absent. | patch |
| Missing `--story` option flag and argument ordering hazard | blind-hunter | `false` | Positional story ID is explicitly required by AC (`qdev impact E12S4`); changing to `--story` would break the requirement. | reject |
| Inline entity citations not connected to overlapping gates | blind-hunter | `false` | The AC specifies `cited_entities` as an independent field; gate overlap is defined by requirement traces. | reject |
| Pre-populating visited hides dependencies between active seed stories | blind-hunter | `false` | In path queries, seed stories are already reported in `active_stories`; re-reporting them in dependents would be redundant. | reject |


## Design Notes

### Transitive Dependent Traversal
Reverse dependencies are traversed using a queue-based breadth-first search:
- Start with seed stories (target story or active stories found).
- At each step, query `store.get_relations_for_target(current_id)` for relations matching `depends_on` and `extends`.
- Each matching source story is recorded with `depth = current_depth + 1`.
- A `HashSet<String>` ensures visited stories are not re-queued, eliminating risk of infinite loops in cyclical or diamond dependency graphs.

### Overlapping Gates
A gate is considered overlapping if:
- Any requirement ID in `gate.verifies` is present in the set of requirements reached via `traces_to` or story gates.
- Or if the gate ID is directly declared in the story's `gates` array.

## Verification

**Commands:**
- `cargo test --test impact_tests` -- expected: Core impact unit tests pass.
- `cargo test --test impact_cli_tests` -- expected: CLI integration tests for `qdev impact` pass.
- `cargo test` -- expected: Entire workspace test suite passes.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero warnings.
