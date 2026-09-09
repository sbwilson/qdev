# Deferred Work

Items deferred from reviews and implementation, kept out of the story they surfaced in.

## Deferred from: code review of spec-1-7-incremental-hydration-sweep (2026-09-08)

- ~~**`PRAGMA schema_version` used as an application version, and a future `user_version` silently rebuilt on the boot path**~~ — **RESOLVED by story 1.14** (`spec-1-14-cache-version-stamp-hardening.md`, 2026-09-10): the cookie is no longer read or written, `PRAGMA user_version` is the sole stamp, and `ensure_cache` now raises the same `schema_version_mismatch` conflict as `init::check_cache_status` for a newer-than-supported cache. Original entry: `create_schema_v2` writes SQLite's internal `schema_version` cookie and `inspect_cache_schema` compares it to `CACHE_SCHEMA_VERSION`; any future DDL touch would bump it and convert a healthy cache into a destructive full rebuild. Separately, `ensure_cache` rebuilds a newer-than-supported cache without asking, while `init.rs::check_cache_status` returns a `schema_version_mismatch` conflict for the same condition. Pre-existing from story 1.6 (verbatim at baseline `d00b436`). `crates/qdev-core/src/store/sqlite.rs:3248, :3289`
- **No indexes; the sweep does two full `entities` scans per pass** — the v2 DDL declares no `CREATE INDEX` (nor did v1), yet the sweep runs a `SELECT ... WHERE source_path = ?` per re-parsed file and two full `SELECT id, source_path FROM entities` scans per pass. The benchmark meets the 30 ms budget at N=1,000, so this is a scaling concern. An index on `entities(source_path)` plus `WHERE id IN (SELECT id FROM dirty_entities)` would replace both scans. `crates/qdev-core/src/store/sqlite.rs:2870`
- **`docs/architecture.md` §10/§11 still documents the v1 cache schema** — §10 lists 14 tables with no `findings` table and no `entities.stale` column; §11.2 still promises that dangling relations are recorded as validation findings, which story 1.7 deliberately scoped out. Fix edits a shared context document rather than story code. `docs/architecture.md:245`
- **Two files declaring the same frontmatter `id` silently overwrite each other** — the second file wins via `ON CONFLICT(id)`, and later deleting the loser purges nothing. Pre-existing: `rebuild_from_workspace` has always been last-writer-wins and story 1.7 does not change it. `crates/qdev-core/src/store/sqlite.rs:3901`

## Deferred from: spec-1-10 scope split (2026-09-09)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-10-relations-dag-computed-blocked.md`
  summary: `qdev graph --dot [--epic E12]` — render the story dependency graph as Graphviz DOT, nodes colored by status, edges labelled by relation.
  evidence: Spec 1.10 combined relation validation (kind-pairs, dangling/cycle findings, `relate`/`unrelate` write path) with DOT graph rendering at ~2811 tokens, well over the 900–1600 target. Graph rendering is a read-only visualization layered on top of the relation/DAG work and isn't required by any downstream story (1.11 `qdev validate` and Epic 2 only need relations, cycle detection, and computed `blocked`), so it was split out as the smaller, independently shippable piece.

## Deferred from: code review of spec-1-10-relations-dag-computed-blocked (2026-09-09)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-10-relations-dag-computed-blocked.md`
  summary: No test exercises hydration finding two disjoint `depends_on` cycles in a single sweep.
  evidence: `validate_relations_graph`'s cycle-repeat loop (`crates/qdev-core/src/store/sqlite.rs`) removes only the found cycle's closing edge from the in-memory working list and re-runs `find_dependency_cycle`, so it is designed to report every disjoint cycle in one pass — but `test_hydration_records_dependency_cycle_finding_on_every_participant` sets up only one 2-node cycle. If the "keep looping" behavior regressed to stop after the first cycle, no existing test would catch it. Worth a follow-up test: two disjoint `depends_on` cycles (e.g. `E1S1<->E1S2` and `E1S3<->E1S4`) in one workspace, asserting all four participants get a `dependency_cycle` finding.

## Deferred from: code review of spec-1-12-qdev-sync-cache-diagnostics (2026-09-09)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-12-qdev-sync-cache-diagnostics.md`
  summary: A plain `qdev sync` (no `--rebuild`) almost always reports `parsed=0`/everything `unchanged` in a real CLI invocation, because the boot-time `ensure_cache` sweep (spec-1-6/1-7) already runs and absorbs any on-disk changes before `handle_sync`'s own explicit `sweep_workspace` call gets a chance to see them.
  evidence: `crates/qdev-cli/src/main.rs` runs `qdev_core::ensure_cache(...)` unconditionally before command dispatch and discards its `SweepSummary`; `handle_sync` then calls `store.sweep_workspace(...)` again in the same process and finds nothing left outstanding. Verified end-to-end by `crates/qdev-cli/tests/sync_cli_tests.rs::test_sync_settles_to_all_unchanged_once_the_boot_time_sweep_has_run`. This is pre-existing architecture (not something spec-1-12 was scoped to change) but undermines `qdev sync`'s main diagnostic purpose ("see what just changed"); a real fix needs `ensure_cache` to surface its sweep summary for `handle_sync` to report directly, or to skip the boot sweep specifically for the `Sync` command.

## Deferred from: spec-1-13-qdev-schema scope (2026-09-09)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-13-qdev-schema.md`
  summary: Hand-author and ship `qdev schema payload context` once `qdev context` (Epic 2/3 scope) has a live command to verify the schema against.
  evidence: Spec 1.13 intentionally scoped payload schemas to `story`, `error`, and `validate` — the three payload kinds with a live command today. `context` has no command yet, so there is no real output to round-trip test against; hand-authoring it now would be unverifiable.
- source_spec: `docs/bmad/implementation-artifacts/spec-1-13-qdev-schema.md`
  summary: Hand-author and ship `qdev schema payload next` once `qdev next` (Epic 2/3 scope) has a live command to verify the schema against.
  evidence: Same rationale as `context` above — `qdev next` is not implemented yet, so its payload schema cannot be round-trip verified and was deferred rather than hand-authored blind.
- source_spec: `docs/bmad/implementation-artifacts/spec-1-13-qdev-schema.md`
  summary: Hand-author and ship `qdev schema payload gate_run` (as an output payload, distinct from the existing `EntityKind::Evidence` frontmatter schema aliased to `gate_run`) once `qdev gate run` (Epic 3 scope) has a live command whose `--json` output payload can be verified.
  evidence: Same rationale as `context`/`next` above — Story 1.13 only ships payload schemas with a real command to round-trip test against; `gate run`'s output-payload shape (as opposed to its evidence-record frontmatter, already schema-printable via `qdev schema gate_run`) is Epic 3 scope.

## Deferred from: code review of spec-1-14-cache-version-stamp-hardening (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-14-cache-version-stamp-hardening.md`
  summary: No test reaches `ensure_cache`'s under-lock `NewerThanSupported` arm — the refusal that fires when another process stamps the cache newer between the pre-lock inspection and the write lock.
  evidence: Verified by the verification-gap review: replacing that arm (`crates/qdev-core/src/store/sqlite.rs:3630-3637`) with `=> true` — the pre-change destructive behaviour — leaves the entire suite green, because every existing test stamps the future version before calling `ensure_cache` and so is caught by the pre-lock arm. Closing it needs a two-process harness (a second `qdev` stamping the cache while the first blocks on `write.lock`), which this repo's single-process test style has no precedent for. The pre-lock arm covers every non-racing case.
- source_spec: `docs/bmad/implementation-artifacts/spec-1-14-cache-version-stamp-hardening.md`
  summary: `docs/architecture.md` §10's abbreviated DDL is still the v1 shape, so the cache-version paragraph this story added describes an `entities.stale` column the listing below it does not show.
  evidence: Pre-existing and already owned by epic-1 retrospective action item 6 ("Reconcile architecture.md sections 10 and 11 with the shipped 16-table cache"); duplicated here because story 1.14's review surfaced it again from the same file it edits. This story added the version paragraph, it did not create the stale listing.
