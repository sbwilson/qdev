# Deferred Work

Items deferred from reviews and implementation, kept out of the story they surfaced in.

**Every entry carries a `Review by:` point** — a story, an epic-planning moment, or a measurable
condition ("the first time the 30 ms boot budget is missed"). Recording an item without one is
what let two items deferred after the story 1.7 review go on to cause both of epic 1's blocking
defects: they were correctly diagnosed, then never looked at again. An item nobody has to revisit
is not deferred, it is forgotten. Resolved entries are struck through in place with a
`RESOLVED <date>` note and what closed them, rather than deleted — the ledger is the record of
what was known and when.

## Deferred from: code review of spec-1-7-incremental-hydration-sweep (2026-09-08)

- ~~**`PRAGMA schema_version` used as an application version, and a future `user_version` silently rebuilt on the boot path**~~ — **RESOLVED by story 1.14** (`spec-1-14-cache-version-stamp-hardening.md`, 2026-09-10): the cookie is no longer read or written, `PRAGMA user_version` is the sole stamp, and `ensure_cache` now raises the same `schema_version_mismatch` conflict as `init::check_cache_status` for a newer-than-supported cache. Original entry: `create_schema_v2` writes SQLite's internal `schema_version` cookie and `inspect_cache_schema` compares it to `CACHE_SCHEMA_VERSION`; any future DDL touch would bump it and convert a healthy cache into a destructive full rebuild. Separately, `ensure_cache` rebuilds a newer-than-supported cache without asking, while `init.rs::check_cache_status` returns a `schema_version_mismatch` conflict for the same condition. Pre-existing from story 1.6 (verbatim at baseline `d00b436`). `crates/qdev-core/src/store/sqlite.rs:3248, :3289`
- **No indexes; the sweep does two full `entities` scans per pass** — the v2 DDL declares no `CREATE INDEX` (nor did v1), yet the sweep runs a `SELECT ... WHERE source_path = ?` per re-parsed file and two full `SELECT id, source_path FROM entities` scans per pass. The benchmark meets the 30 ms budget at N=1,000, so this is a scaling concern. An index on `entities(source_path)` plus `WHERE id IN (SELECT id FROM dirty_entities)` would replace both scans. `crates/qdev-core/src/store/sqlite.rs:2870` Review by: the first time the 30 ms boot budget is missed, or before a workspace exceeds ~5,000 entities — whichever comes first.
- ~~**`docs/architecture.md` §10/§11 still documents the v1 cache schema**~~ — **RESOLVED 2026-09-10** (epic 1 retrospective action item 7): §10 now lists all 16 tables including `findings`, `dirty_entities` and `sync_meta`, carries `entities.stale`, names `ALL_TABLE_NAMES` as the authority, and states the cache version; §11 documents the whole-graph relation revalidation pass. The §11.2 half of this entry was **refuted** on re-check: hydration does record `dangling_relation` findings — `validate_relations_graph` is called from both `sweep_workspace` (`sqlite.rs:3204`) and `rebuild_from_workspace` (`:504`) — so the original claim that story 1.7 scoped it out is stale, presumably superseded by story 1.10. Original entry: §10 lists 14 tables with no `findings` table and no `entities.stale` column. `docs/architecture.md:245`
- **Two files declaring the same frontmatter `id` silently overwrite each other** — the second file wins via `ON CONFLICT(id)`, and later deleting the loser purges nothing. Pre-existing: `rebuild_from_workspace` has always been last-writer-wins and story 1.7 does not change it. `crates/qdev-core/src/store/sqlite.rs:3901` Review by: epic 2 story 2.1 (state machine) — silent overwrite of an id is a data-loss path, and `qdev validate --fix-ids` already depends on duplicate ids being visible rather than collapsed.

## Deferred from: spec-1-10 scope split (2026-09-09)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-10-relations-dag-computed-blocked.md`
  summary: `qdev graph --dot [--epic E12]` — render the story dependency graph as Graphviz DOT, nodes colored by status, edges labelled by relation.
  evidence: Spec 1.10 combined relation validation (kind-pairs, dangling/cycle findings, `relate`/`unrelate` write path) with DOT graph rendering at ~2811 tokens, well over the 900–1600 target. Graph rendering is a read-only visualization layered on top of the relation/DAG work and isn't required by any downstream story (1.11 `qdev validate` and Epic 2 only need relations, cycle detection, and computed `blocked`), so it was split out as the smaller, independently shippable piece. Review by: epic 4 story 4.9 (`qdev graph` rendering options), which owns this surface.

## Deferred from: code review of spec-1-10-relations-dag-computed-blocked (2026-09-09)

- source_spec: `docs/bmad/implementation-artifacts/spec-1-10-relations-dag-computed-blocked.md`
  summary: No test exercises hydration finding two disjoint `depends_on` cycles in a single sweep.
  evidence: `validate_relations_graph`'s cycle-repeat loop (`crates/qdev-core/src/store/sqlite.rs`) removes only the found cycle's closing edge from the in-memory working list and re-runs `find_dependency_cycle`, so it is designed to report every disjoint cycle in one pass — but `test_hydration_records_dependency_cycle_finding_on_every_participant` sets up only one 2-node cycle. If the "keep looping" behavior regressed to stop after the first cycle, no existing test would catch it. Worth a follow-up test: two disjoint `depends_on` cycles (e.g. `E1S1<->E1S2` and `E1S3<->E1S4`) in one workspace, asserting all four participants get a `dependency_cycle` finding.

## Deferred from: code review of spec-1-12-qdev-sync-cache-diagnostics (2026-09-09) Review by: epic 3 story 3.10 (impact analysis), the first consumer that reasons over the whole dependency graph.

- source_spec: `docs/bmad/implementation-artifacts/spec-1-12-qdev-sync-cache-diagnostics.md`
  summary: A plain `qdev sync` (no `--rebuild`) almost always reports `parsed=0`/everything `unchanged` in a real CLI invocation, because the boot-time `ensure_cache` sweep (spec-1-6/1-7) already runs and absorbs any on-disk changes before `handle_sync`'s own explicit `sweep_workspace` call gets a chance to see them.
  evidence: `crates/qdev-cli/src/main.rs` runs `qdev_core::ensure_cache(...)` unconditionally before command dispatch and discards its `SweepSummary`; `handle_sync` then calls `store.sweep_workspace(...)` again in the same process and finds nothing left outstanding. Verified end-to-end by `crates/qdev-cli/tests/sync_cli_tests.rs::test_sync_settles_to_all_unchanged_once_the_boot_time_sweep_has_run`. This is pre-existing architecture (not something spec-1-12 was scoped to change) but undermines `qdev sync`'s main diagnostic purpose ("see what just changed"); a real fix needs `ensure_cache` to surface its sweep summary for `handle_sync` to report directly, or to skip the boot sweep specifically for the `Sync` command.

## Deferred from: spec-1-13-qdev-schema scope (2026-09-09) Review by: epic 2 planning — `qdev sync`'s contract should be settled before more commands depend on its counts.

- source_spec: `docs/bmad/implementation-artifacts/spec-1-13-qdev-schema.md`
  summary: Hand-author and ship `qdev schema payload context` once `qdev context` (Epic 2/3 scope) has a live command to verify the schema against.
  evidence: Spec 1.13 intentionally scoped payload schemas to `story`, `error`, and `validate` — the three payload kinds with a live command today. `context` has no command yet, so there is no real output to round-trip test against; hand-authoring it now would be unverifiable. Review by: with the story that implements `qdev context` (epic 2/3 scope).

- source_spec: `docs/bmad/implementation-artifacts/spec-1-13-qdev-schema.md`
  summary: Hand-author and ship `qdev schema payload next` once `qdev next` (Epic 2/3 scope) has a live command to verify the schema against.
  evidence: Same rationale as `context` above — `qdev next` is not implemented yet, so its payload schema cannot be round-trip verified and was deferred rather than hand-authored blind. Review by: with the story that implements `qdev next` (epic 2 story 2.11).

- source_spec: `docs/bmad/implementation-artifacts/spec-1-13-qdev-schema.md`
  summary: Hand-author and ship `qdev schema payload gate_run` (as an output payload, distinct from the existing `EntityKind::Evidence` frontmatter schema aliased to `gate_run`) once `qdev gate run` (Epic 3 scope) has a live command whose `--json` output payload can be verified.
  evidence: Same rationale as `context`/`next` above — Story 1.13 only ships payload schemas with a real command to round-trip test against; `gate run`'s output-payload shape (as opposed to its evidence-record frontmatter, already schema-printable via `qdev schema gate_run`) is Epic 3 scope.

## Deferred from: code review of spec-1-14-cache-version-stamp-hardening (2026-09-10) Review by: with the story that implements `qdev gate run` (epic 3 story 3.1).

- source_spec: `docs/bmad/implementation-artifacts/spec-1-14-cache-version-stamp-hardening.md`
  summary: No test reaches `ensure_cache`'s under-lock `NewerThanSupported` arm — the refusal that fires when another process stamps the cache newer between the pre-lock inspection and the write lock.
  evidence: Verified by the verification-gap review: replacing that arm (`crates/qdev-core/src/store/sqlite.rs:3630-3637`) with `=> true` — the pre-change destructive behaviour — leaves the entire suite green, because every existing test stamps the future version before calling `ensure_cache` and so is caught by the pre-lock arm. Closing it needs a two-process harness (a second `qdev` stamping the cache while the first blocks on `write.lock`), which this repo's single-process test style has no precedent for. The pre-lock arm covers every non-racing case. Review by: the first story that touches multi-process coordination — epic 2 story 2.3 (story leases).

- source_spec: `docs/bmad/implementation-artifacts/spec-1-14-cache-version-stamp-hardening.md`
  summary: `docs/architecture.md` §10's abbreviated DDL is still the v1 shape, so the cache-version paragraph this story added describes an `entities.stale` column the listing below it does not show.
  evidence: Pre-existing and already owned by epic-1 retrospective action item 6 ("Reconcile architecture.md sections 10 and 11 with the shipped 16-table cache"); duplicated here because story 1.14's review surfaced it again from the same file it edits. This story added the version paragraph, it did not create the stale listing.

## Scheduled from: epic 1 retrospective section D verification pass (2026-09-10)

Confirmed by re-checking every section-D claim against the tree at `a706c85`; see the
"Verification pass — 2026-09-10" table in `epic-1-retro-2026-09-09.md` for the refutations. Review by: **resolved 2026-09-10** by retrospective action item 7; §10 now lists all 16 tables and `entities.stale`.

- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: A frontmatter fence closed with a trailing space (`"--- "`) hydrates but cannot be patched — `schema.rs:332` trims the line, `write.rs:311`, `write.rs:691` and `validate.rs:518` do not.
  evidence: Confirmed. The three scanners must agree on what closes a fence; today a file readable by the sweep is rejected by `qdev update`. Review by: before epic 2 story 2.1 (lifecycle hooks patch frontmatter on every transition).
- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: Malformed frontmatter exits 4, 1, or 2 depending on which reader hits it; AD-13 reserves 4 for infrastructure and 2 for malformed input.
  evidence: Confirmed. `parse_error` is an `infrastructure_failure` at `write.rs:1335`, `write.rs:1732`, `main.rs:2537`; `yaml_parse_error` is a `logical_failure` at `write.rs:1570`. Review by: before epic 3 story 3.2 (gate result contract / failure taxonomy) freezes the exit-code contract.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: `write.rs:877-920` hand-writes five raw statements duplicating four `Store` methods, and `epic_id`/`seq` are derived in three places (`sqlite.rs:4512`, `write.rs:1428`, `main.rs:2560`).
  evidence: Confirmed as duplication only — no behavioural difference today (the `stale`-column half of the original claim is refuted). Same class of hazard that produced defects B1/B2: two sources of truth kept in step by hand. Review by: whenever the cache schema next changes.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: The AD-1 forbidden-dependency guard inspects only qdev-core's direct dependencies, so a forbidden crate arriving transitively passes.
  evidence: Confirmed and latent (`cargo tree` is currently clean). `architecture_tests.rs:56-70` iterates the qdev-core package's own `dependencies` array from `cargo metadata` rather than walking the resolved graph. Review by: before epic 3 story 3.12 (SOUP audit / SBOM), which needs the resolved graph anyway.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: `_QDEV_MOCK_TTY` (`main.rs:114`) is compiled into release builds, so the env var makes a piped stdin look interactive in a shipped binary.
  evidence: Confirmed — no `cfg(debug_assertions)` gate. Interacts with the non-interactive-first / closed-fail policy. Review by: before the first tagged release.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: Not-found errors carry a typed `entity_not_found` code only on the `query.rs` pair; `main.rs:1524` and `write.rs:1121,1173` emit bare usage errors with no stable code.
  evidence: Downgraded from the original claim — every path exits 2, so there is no exit-code split. Code consistency only. Review by: before epic 4 story 4.1 (context projection consumes error codes programmatically).
- source_spec: `docs/bmad/implementation-artifacts/epic-1-retro-2026-09-09.md`
  summary: Unsettled — 21 `println!` sites in `main.rs` bypass `OutputEmitter::emit_text`, which swallows `BrokenPipe`; claim is that `qdev relate | head -1` exits 4 where `qdev list | head -1` exits 0.
  evidence: Still unverified after two attempts (the retro's and this pass's). Settling it needs one pipe-closing test, not an investigation — write that test before deciding whether there is anything to fix. Review by: epic 2 planning.

## Deferred from: code review of spec-create-story-via-write-path (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-create-story-via-write-path.md`
  summary: Story-id allocation runs outside the advisory lock, so two concurrent `qdev create story E12` runs can both allocate `E12S1` and the loser exits 5 `file_exists` instead of getting `E12S2`.
  evidence: Confirmed by two reviewers. `allocate_next_story_id_in` is called in `handle_create_story` before `create_story` takes the lock — the frozen Code Map deliberately kept it there, so fixing it is a design change (allocate inside the lock, or retry once on `file_exists`) rather than a patch. Review by: epic 2 story 2.9 (sprints as assignments), the first feature that creates entities from more than one session.
- source_spec: `docs/bmad/implementation-artifacts/spec-create-story-via-write-path.md`
  summary: `write_file_atomic`'s rename can clobber a file created between the in-lock existence check and the rename by a writer that does not hold the advisory lock.
  evidence: Real but narrow — every qdev writer holds the lock, so it needs an external process writing that exact path inside a microsecond window. The old `OpenOptions::create_new` was atomic against it; the in-lock `symlink_metadata` check is not. An exact fix means creating the destination exclusively (or `hard_link`-ing the temp file) rather than renaming over it. Review by: whenever the write path is next opened.
- source_spec: `docs/bmad/implementation-artifacts/spec-create-story-via-write-path.md`
  summary: A cache-upsert failure after a successful atomic write leaves the file on disk while the command exits non-zero, and nothing covers that partial state.
  evidence: Not introduced here — `apply_entity_update` propagates the same error the same way (`crates/qdev-core/src/write.rs:1393`), so `update` and `relate` share it. The cache is a rebuildable index (AD-3), so the defensible fix is to report the cache failure without failing the command; that is a contract decision for the whole write path. Review by: epic 3 story 3.2, which freezes the failure taxonomy.
- source_spec: `docs/bmad/implementation-artifacts/spec-create-story-via-write-path.md`
  summary: The 5-second advisory-lock timeout is hardcoded at five call sites across `write.rs` and `main.rs` while `cli-reference.md` now promises it as a contract.
  evidence: Pre-existing duplication (three sites predate this story). One `pub const` in `write.rs` used by core and the CLI. Review by: next time the timeout is questioned or made configurable.
- source_spec: `docs/bmad/implementation-artifacts/spec-create-story-via-write-path.md`
  summary: `docs/cli-reference.md`'s entity table advertises `qdev create epic|adr|requirement|hazard|prd`, none of which exist — `Story` is the only `CreateCommands` variant.
  evidence: Pre-existing documentation overclaim, untouched by this story and outside its intent. Either implement them or mark the row as planned. Review by: epic 2 planning, which decides whether those creates land.

## Deferred from: code review of spec-doctor-sees-computed-findings (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-doctor-sees-computed-findings.md`
  summary: `find_duplicate_planning_ids` reports `0` findings with no error when it cannot read the specs directory, so a blind scan is indistinguishable from a clean workspace — in `qdev validate` as much as in `qdev doctor`.
  evidence: Measured by the edge-case reviewer against the built binary: `chmod 000 docs/specs/stories` with two duplicate ids present yields `{"status":"ok","finding_count":0,"findings_by_code":{}}`. The scan skips unreadable files and directories and returns what it found. Pre-existing in the check (both commands share it), and the doctor story's frozen boundary forbids changing what an existing check reports. Fix: have the scan count what it could not read and surface that as a finding or a partial status. Review by: before epic 3 story 3.9 (hygiene linter), which makes validation output gate-relevant.
- source_spec: `docs/bmad/implementation-artifacts/spec-doctor-sees-computed-findings.md`
  summary: `doctor`'s `validation` section reports `finding_count` across all severities, so it cannot tell the reader whether `qdev validate` would actually exit 1 (which keys on `error` severity alone).
  evidence: Raised independently by two reviewers. A `findings_by_severity` breakdown, or an `error_finding_count`, answers "would this fail the gate?" — the question a user reads doctor to answer. Not added here because it is new reporting surface beyond the story's intent. Review by: epic 3 story 3.6 (transition-bound gates), the first consumer that cares whether validation blocks.
- source_spec: `docs/bmad/implementation-artifacts/spec-doctor-sees-computed-findings.md`
  summary: The `cache` and `validation` doctor sections each read findings independently, so a write landing between them can produce one report where `cache.finding_count` exceeds `validation.finding_count`.
  evidence: Real but narrow — a diagnostic snapshot skew needing a concurrent write mid-report. The fix (one store snapshot shared by every section, or a single `list_findings` passed in) belongs to the section registry rather than to one section. Review by: whenever a third section is added, which is when the registry is next opened.
- source_spec: `docs/bmad/implementation-artifacts/spec-1-14-cache-version-stamp-hardening.md`
  summary: Flaky test — `sweep_tests::test_real_v1_cache_rebuilds_to_current_schema_and_matches_a_fresh_sweep` failed once in a full `cargo test --workspace` run and did not reproduce in 3 isolated and 7 further full runs.
  evidence: Observed 2026-09-10 during story verification; the assertion text was not captured. The test compares a rebuilt cache against a fresh sweep of the same tree, so the likely suspects are mtime/size granularity in `sync_state` or a timestamp captured either side of a second boundary. A flake in exactly the "rebuild equals sweep" invariant is worth pinning down rather than re-running. Review by: first time it fails again, or epic 2 planning — whichever comes first.
