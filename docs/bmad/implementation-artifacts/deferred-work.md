# Deferred Work

Items deferred from reviews and implementation, kept out of the story they surfaced in.

## Deferred from: code review of spec-1-7-incremental-hydration-sweep (2026-09-08)

- **`PRAGMA schema_version` used as an application version, and a future `user_version` silently rebuilt on the boot path** — `create_schema_v2` writes SQLite's internal `schema_version` cookie and `inspect_cache_schema` compares it to `CACHE_SCHEMA_VERSION`; any future DDL touch would bump it and convert a healthy cache into a destructive full rebuild. Separately, `ensure_cache` rebuilds a newer-than-supported cache without asking, while `init.rs::check_cache_status` returns a `schema_version_mismatch` conflict for the same condition. Pre-existing from story 1.6 (verbatim at baseline `d00b436`). `crates/qdev-core/src/store/sqlite.rs:3248, :3289`
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
