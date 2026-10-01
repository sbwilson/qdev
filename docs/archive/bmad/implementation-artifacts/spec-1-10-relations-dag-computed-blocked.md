---
title: 'Relations, DAG & Computed Blocked'
type: 'feature'
created: '2026-09-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '565fbde1aa2549a88e9e0fe650a0c0d7aa817777'
context:
  - 'docs/architecture.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Relations are read-only today — hydration upserts them from frontmatter and `qdev get`/`list` can display them, but nothing validates them (dangling targets, wrong kind pairs, `depends_on` cycles), and nothing writes them from the CLI.

**Approach:** Add `qdev relate`/`qdev unrelate` (patch `relations:` frontmatter via the existing atomic write path, with synchronous kind-pair/dangling/cycle validation before writing), and extend hydration to validate every stored relation against `docs/architecture.md` §8's allowed kind pairs, emitting persistent findings for dangling targets and `depends_on` cycles. `qdev graph --dot` is deferred (see `docs/bmad/implementation-artifacts/deferred-work.md`).

## Boundaries & Constraints

**Always:**
- Reuse the existing `relations` table and `Store` methods as-is — no schema changes.
- Kind-pair table covers exactly the 8 relations in `docs/architecture.md` §8.
- Dangling target, invalid kind pair, and `depends_on` cycles are non-halting `findings` (reuse `findings`/`record_finding`), matching story 1.7's `merge_conflict`/`schema_violation` pattern. New codes: `dangling_relation`, `invalid_relation_kind`, `dependency_cycle`.
- `relate`/`unrelate` reuse `apply_entity_update`'s primitives and merge into the existing `relations:` mapping — never replace the whole map.
- `relate` validates kind pair, target existence, and would-be cycles *before* writing, refusing (exit 1) rather than writing a bad edge; hydration findings are the backstop for edits made outside `qdev`.
- Cycle detection traverses `depends_on` edges only, with a visited/recursion-stack guard.

**Never:**
- No `Gate` `EntityKind` variant — `verifies` (Gate→Requirement) is unreachable until Epic 3 adds one.
- No `qdev validate` (story 1.11) — findings are recorded, not aggregated/reported.
- No `qdev graph` (deferred, see `deferred-work.md`).
- No batching of `compute_blocked`'s per-relation lookup (rejected in story 1.9 review).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Relate happy path | `qdev relate E1S2 depends_on E1S1 --json` | `relations.depends_on` gains `E1S1`; other relations untouched; cache updated | N/A |
| Relate refused (any of: would-cycle / wrong kind pair / dangling target) | e.g. E1S2 depends_on E1S1 where E1S1 already depends_on E1S2 | Refused, no write | exit 1, `dependency_cycle` / `invalid_relation_kind` / `dangling_relation` |
| Unrelate | `qdev unrelate E1S2 depends_on E1S1` (present or absent) | Entry removed if present; idempotent no-op if absent | exit 0 |
| Out-of-band relations problem | Merge/raw edit introduces a cycle or dangling target across files, next hydration runs | Matching finding recorded per affected file (cycle finding names the cycle); relation kept, hydration continues | N/A |

</frozen-after-approval>

## Code Map

- [x] `crates/qdev-core/src/store/sqlite.rs:78-83,1139-1268` -- `relations` table + CRUD methods -- reuse as-is
- [x] `crates/qdev-core/src/store/sqlite.rs:4181-4199` -- hydration relation upsert from frontmatter -- extend: kind-pair + dangling checks per relation via `dag.rs`, call `record_finding`
- [x] `crates/qdev-core/src/store/sqlite.rs:189,2629-3520` -- `findings` table + CRUD -- reuse; new codes `dangling_relation`, `invalid_relation_kind`, `dependency_cycle`
- [x] `crates/qdev-core/src/schema.rs:11-26` -- `EntityKind` enum (no `Gate` variant) -- source for kind-pair table, no enum changes
- [x] `crates/qdev-core/src/query.rs:129-140` -- `compute_blocked` -- unchanged
- [x] `crates/qdev-core/src/dag.rs` (new) -- kind-pair table (architecture.md §8), `depends_on` cycle detection (DFS + recursion stack), whole-graph scan after sweep -- used by hydration and `relate`
- [x] `crates/qdev-core/src/write.rs:230-236,249,1153` -- `FrontmatterPatchOptions`/`patch_frontmatter`/`apply_entity_update` -- add `apply_relation_change(store, id, relation, target, add: bool)`: merge into existing `relations:` map
- [x] `crates/qdev-cli/src/cli.rs:28-45,87-108` -- `Commands` enum, `UpdateArgs` pattern -- add `Relate`/`Unrelate`; positional `source_id relation target_id` (+ `--if-version` on `Relate`)
- [x] `crates/qdev-cli/src/main.rs:722-775,879` -- `handle_update` pattern -- add `handle_relate`/`handle_unrelate`
- [x] `crates/qdev-core/tests/dag_tests.rs`, `crates/qdev-cli/tests/relate_cli_tests.rs` (new) -- coverage per I/O matrix

## Tasks & Acceptance

**Execution:** one task per Code Map entry, in the order listed there (`dag.rs` first — hydration, `write.rs`, and the CLI all depend on it).

**Acceptance Criteria:**
- [x] Given relations declared in frontmatter, when hydration runs, then every relation is stored with source/target kinds checked against the allowed pairs, a dangling target produces a `dangling_relation` finding (relation still stored), and a `depends_on` cycle produces a `dependency_cycle` finding naming the cycle.
- [x] Given `qdev relate E1S2 depends_on E1S1`, when the edit is valid, then the source file's `relations:` map is patched via the write path and the cache reflects the new edge; when the edit is a wrong kind pair, dangling target, or would form a cycle, then it is refused with exit 1 before any write.
- [x] Given `qdev unrelate E1S2 depends_on E1S1`, then the entry is removed from the map; unrelating an absent relation is an idempotent exit-0 no-op.
- [x] Given a story with an undone `depends_on` target, `blocked` is `true` (existing `compute_blocked` behavior, unchanged).
- [x] Given the entire test suite, `cargo test` passes with zero failures.

## Implementation Notes

- `dag.rs` provides `allowed_kind_pairs`/`is_valid_kind_pair` (the architecture.md §8 table; `verifies` maps to an empty pair set since no `Gate` `EntityKind` exists), `find_dependency_cycle` (whole-graph DFS + recursion-stack guard, used by hydration) and `would_create_cycle` (BFS reachability from the target, used by `relate`'s pre-write refusal check).
- Hydration's `validate_relations_graph` runs after both the full rebuild and every incremental sweep, before commit. It clears and fully recomputes the three finding codes from the current `entities`/`relations` tables each run (not just files touched that pass), so an out-of-band fix or break anywhere is caught on the next hydration; it repeats cycle detection after removing each found cycle's closing edge from the in-memory working list (never from storage) so multiple disjoint cycles are all reported.
- `write.rs` adds `apply_relation_change`/`RelationChangeOptions`/`RelationChangeResult`, mirroring `apply_entity_update`'s pipeline (resolve file, lock, patch, schema-validate, atomic write, cache upsert+dirty), merging into the existing `relations:` map. An absent-entry `unrelate` is an idempotent no-op that writes nothing. The story-detail-field extraction was refactored into a shared `story_detail_fields` helper used by both `apply_entity_update` and `apply_relation_change`.
- CLI adds `Relate`/`Unrelate` (`source_id relation target_id`, `--if-version` only on `Relate`). `handle_relate` validates target existence, kind pair, and would-be `depends_on` cycles against the cache before writing, refusing with exit 1 and the matching code; `handle_unrelate` applies the change directly.
- Verified with `cargo test --workspace` (all tests pass, zero failures), `cargo clippy --workspace --all-targets -- -D warnings` (zero warnings), and `cargo fmt --check` (clean). All four I/O & Edge-Case Matrix rows are covered by passing tests in `crates/qdev-core/tests/dag_tests.rs` and `crates/qdev-cli/tests/relate_cli_tests.rs`.
- **Superseded 2026-09-11** by `spec-relation-command-contracts.md` (epic 1 cross-story review findings H8 and M1), which changed three of the notes above: `dag.rs` now keeps the relation names and their kind pairs in one table and exposes `is_known_relation`/`relation_names`, so an unknown name and `verifies` are distinguishable; an unknown relation name is a usage error (exit 2) from `relate` *and* `unrelate`, before any other check; `unrelate` takes `--if-version`, and `apply_relation_change` compares it before it can report the idempotent no-op.
- `purge_entity_with_children` (pre-existing, from story 1.7) deletes target-side relation rows when the target entity's file is removed, so a relation naturally disappears in that specific case rather than going dangling; left as-is since the spec scopes relation-table handling as "reuse as-is." Dangling/invalid-kind/cycle findings are otherwise fully exercised.

## Spec Change Log

## Review Triage Log

| # | Finding | Verdict | Evidence | Route |
|---|---------|---------|----------|-------|
| 1 | `resolve_author` silently defaults an invalid `QDEV_AUTHOR_TYPE` env value to `human` instead of erroring | false | Verified byte-for-byte identical to the pre-existing `handle_update` author-resolution block (`main.rs:833-861`, unmodified by this diff) — the doc comment even states it mirrors `qdev update`; not a regression. | reject |
| 2 | `handle_relate`'s text-output branch always prints "Related ..." even when `res.changed == false` (edge already existed) | low | Verified: unlike `handle_unrelate`, which branches on `res.changed` to print "No-op: ...", `handle_relate`'s else-branch is unconditional. Misleading text output only (JSON payload's `changed` field is correct); trivial one-line fix mirroring `handle_unrelate`'s existing pattern. | patch |
| 3 | `handle_unrelate` never calls `ensure_query_workspace`, unlike `handle_relate` | low | Verified by reading both handlers: `handle_relate` calls it before touching the store, `handle_unrelate` does not. Running `unrelate` outside a qdev workspace gets a less clean error than `relate`/`get`/`list` give in the same situation. Trivial fix (one added call). **Resolved since, differently: 2026-09-11** — the guard was centralised into the boot-path `requires_workspace` check, which classifies `Unrelate` as requiring a workspace, so neither handler needs its own call. Verified: `qdev unrelate` outside a workspace exits 2 with the same message `get`/`list` give and creates nothing. | patch (resolved by centralisation) |
| 4 | No self-relation guard for non-`depends_on` relations (e.g. `extends E1S1 extends E1S1`) | low | Verified `would_create_cycle` is only invoked when `relation == "depends_on"`. Not covered by the spec's kind-pair/cycle boundaries, unlikely in everyday use, and a general fix would add guard logic across 6 additional relation kinds not scoped by this story. | reject (low, everyday-use unlikely, fix beyond direct correction) |
| 5 | `unrelate` has no `--if-version` flag, asymmetric with `relate` | false | Spec Code Map explicitly scopes `--if-version` to `Relate` only ("positional source_id relation target_id (+ `--if-version` on `Relate`)"); this is by design, not a gap. **Superseded 2026-09-11** by `spec-relation-command-contracts.md`: the asymmetry was re-filed as review finding H8 and `unrelate` now takes `--if-version` with the same meaning. | reject (superseded) |
| 6 | `--if-version` conflict path on `relate` has no CLI-level test | low | Verified no such test exists in `relate_cli_tests.rs`. The underlying OCC mechanism (`patch_frontmatter`) already has dedicated coverage in `write_tests.rs` (`test_patch_frontmatter_optimistic_concurrency_mismatch`); only the one-line CLI wiring (`if_version: relate_args.if_version`) is untested. Low regression risk, not user-facing. | reject (low, redundant with existing primitive coverage) |
| 7 | `handle_relate`'s `depends_on` cycle pre-check does an unscoped `store.list_relations()` full-table scan | n/a | Matches the exact performance pattern the spec explicitly excludes: "No batching of `compute_blocked`'s per-relation lookup (rejected in story 1.9 review)," and the existing `deferred-work.md` "No indexes" entry already tracks this class of issue project-wide. | reject (out of scope per spec boundary) |
| 8 | `relations:` map key order is not preserved across `relate`/`unrelate` (silently re-sorted alphabetically), producing spurious diff noise on every call | medium | Verified: `serde_json::Map` defaults to `BTreeMap` ordering (no `preserve_order` feature enabled anywhere in the dependency tree — confirmed via `cargo tree -e features -p serde_json`), so building `relations_obj` from `extract_frontmatter`'s `serde_json::Value` and round-tripping through `serde_yaml::to_value` loses original key order on every single-edge change. Directly contradicts architecture.md's stated design goal ("Frontmatter edits by qdev are line-based patches that preserve comments and ordering"). Self-contained fix within `apply_relation_change`. | patch |
| 9 | `apply_relation_change` uses `extract_frontmatter(&existing_content).ok()`, silently discarding a frontmatter parse failure and defaulting `relations_obj` to an empty map | medium | Verified the `.ok()` call exists (mirrors a pre-existing pattern in `apply_entity_update`, but that function never reconstructs a full nested map from the parsed value the way `apply_relation_change` does). Unlike the pre-existing usage, this one is actively used to rebuild and overwrite the entire `relations:` block, so a parse failure on an otherwise-patchable file (intact delimiters, malformed YAML body) silently drops every existing relation on the next `relate`/`unrelate`. Narrow trigger, but real data-loss consequence; self-contained fix (propagate the error instead of swallowing it). | patch |
| 10 | Non-string elements in an existing relation's target array are silently dropped by `filter_map` during merge | low | Verified `story.json`'s `relations.*` schema declares `items: {type: string}`, so a non-string target can only exist via a hand-edit that bypasses qdev's own write-time schema validation — already an out-of-band scenario hydration's `schema_violation` finding covers. Unlikely in everyday use; fix would add defensive handling beyond direct correction. | reject |
| 11 | `validate_relations_graph` would misreport `dangling_relation` if a target's `kind` column failed `EntityKind::from_str_loose` | false | `entities.kind` is populated exclusively via `EntityKind::as_str()` at every qdev write path, which always round-trips through `from_str_loose`; this state is unreachable through any code path this story touches. | reject |
| 12 | No test asserts a repeated `qdev relate <same edge>` is idempotent (`changed: false`, version unchanged, no duplicate array entry) | pre-verified (verification-gap) | Verification-gap layer read both new test files in full and confirmed no test exercises this path; the dedup logic itself (`targets.iter().any(...)`) is correct by inspection, only untested. | patch (add test) |
| 13 | No test exercises hydration finding *two* disjoint `depends_on` cycles in one sweep | pre-verified (verification-gap) | Verification-gap layer confirmed `test_hydration_records_dependency_cycle_finding_on_every_participant` covers only a single 2-node cycle; the multi-cycle removal loop in `validate_relations_graph` is otherwise unexercised. | defer |

## Design Notes

- **Cycle-finding placement**: dangling/invalid-kind findings attach to the source file's path. A `depends_on` cycle spans multiple files — record one `dependency_cycle` finding per participating file's path, same message naming the full cycle (e.g. `E1S1 -> E1S2 -> E1S1`), so `delete_findings_for_path`'s per-path lifecycle still works.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all unit, integration, and CLI tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
