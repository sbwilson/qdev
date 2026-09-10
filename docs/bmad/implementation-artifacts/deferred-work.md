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

## Scheduled from: epic 1 cross-story review (2026-09-10)

Full evidence and reproductions in `epic-1-cross-story-review-2026-09-10.md`. The eight high
findings are **blocking for epic 1 acceptance** and are not deferred — they cluster into three
stories named at the end of that document. The medium and low findings are recorded here.

- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: `qdev unrelate` with a misspelled relation name exits 0 reporting `changed: false` while the real edge survives; `relate` reports the same input as `invalid_relation_kind` with a message that misdescribes it.
  evidence: Re-verified by the reviewing session. `dag.rs:14-27` returns an empty slice for an unrecognised relation, so `main.rs:1485` cannot distinguish "unknown relation" from "disallowed kind pair", and `handle_unrelate` (`main.rs:1605-1630`) does no relation-name check at all. Every other enum-valued argument is an exit-2 usage error. A cleanup script reads exit 0 as "edge removed". Review by: with the identity-resolution story, which already touches the relation write path.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: `duplicate_planning_id` scans `specs_dir` only, so id collisions among sprints, DW, decisions, releases and SOUP in `state_dir` are never reported — and then trigger the purge defect when one file is deleted.
  evidence: Reproduced: two files in `docs/state/sprints/` both declaring `sprint-1` yield `findings: []`. `validate.rs:38` joins `specs_dir` alone where hydration collects from both directories (`sqlite.rs:2922`). Review by: with the purge/re-hydration story — the two defects compound.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: Schema validation before a write resolves the entity kind by directory-then-grammar; hydration resolves it by frontmatter `kind:` first, so `update` can exit 0 on a file the next boot records a `schema_violation` for.
  evidence: Reproduced with an ADR file declaring `kind: story`. `--fix-ids` is the only writer that resolves kind the way hydration does, and its comment at `main.rs:2469` names the invariant the others break. Blast radius is limited to the `appetite`/`safety_class` enums today and widens as the schemas differentiate. Review by: with the identity-resolution story.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: "Entity does not exist" carries three error codes across two exit classes, and a malformed entity file gets five different answers from five commands — `relate` reporting "not found" for an entity that exists with a corrupt file.
  evidence: Reproduced across `get`/`update`/`relate`/`sync`/`validate`. A `relate` target that does not exist is exit 1 `dangling_relation` while a missing source is exit 2 `usage_error`, so one mistyped id lands in either "your invocation was wrong" or "the workspace has a defect". Review by: epic 3 story 3.2, which freezes the failure taxonomy — but the `relate` source/target split is worth fixing sooner.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: `find_workspace_root` accepts `.git` as a root marker while the workspace guards require `qdev.toml`, so a submodule or vendored clone inside a workspace becomes a shadow root that `create story` will write duplicate ids into.
  evidence: Reproduced: from a nested directory containing a `.git` file, `qdev list stories` exits 2 while `qdev create story E1` exits 0 and writes a duplicate id no sweep will see. Two guards at the same seam disagree about the same cwd. Review by: with the configuration-merge story, which owns root and layout resolution.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: `qdev init` treats an unparseable `qdev.toml` as absent where every other command treats it as fatal, so it reports success on a workspace no command can use and scaffolds the default layout over a configured one.
  evidence: Reproduced. `init.rs:44-49` swallows the parse error; `config/mod.rs:1216` makes it an exit-2 usage error before dispatch — which also means `doctor` can never diagnose a bad config. Review by: with the configuration-merge story.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: `qdev doctor` exits 5 on advisory-lock contention, contradicting the "a report, never a gate" promise added to `cli-reference.md` on 2026-09-10.
  evidence: Reproduced by holding `write.lock`: `get`, `list` and `doctor` all exit 5 after the 5 s timeout, because boot's sweep takes the lock. Read-only commands blocking on a writer was adjudicated as intended by spec 1.7, but the doctor promise post-dates that and is now false. Either doctor skips the boot sweep or the doc is corrected. Review by: before the first tagged release — a diagnostic that fails when the system is busy is the diagnostic you need most.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: Eight low findings — `create_story`'s weaker cache predicate, `init` reporting hardcoded default paths, `payload-fix-ids.json` missing its `error` field, `cli-reference.md` self-contradictions on payload schemas and `--json` coverage, `sync`'s partial `findings=` count, `delete_entity` not being the purge cascade, off-convention files being readable but not writable, and story-id allocation outside the lock.
  evidence: Each reproduced or read directly; see the Low section of the cross-story review for sites and scenarios. Grouped as one ledger entry because they are cheap individually and none loses state. Review by: epic 2 planning, to be swept up with whichever stories touch those files.

## Deferred from: code review of spec-identity-resolution-seam (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-identity-resolution-seam.md`
  summary: `--fix-ids` reports `relations_rewritten: []` / `citations_rewritten: 0` for edges and citations it had already redirected when a later step aborts, and `skipped` now carries three causes (declined, unparseable id, attempted-and-failed) under one shape.
  evidence: Raised by two reviewers. `rewrite_relations_to` accumulates its progress and discards it with `?` on the first failure, so the payload understates what is on disk — the same reporting gap as the H3 abort this story fixed, one level down. No state is lost; only the report is incomplete. The fix carries partial progress out of the error path (a `Result<Vec<_>, (Vec<_>, QdevError)>` or an `&mut Vec`) and splits or documents `skipped`'s third cause. Review by: the next time `--fix-ids` is opened, or epic 2 story 2.9, whichever comes first.
- source_spec: `docs/bmad/implementation-artifacts/spec-identity-resolution-seam.md`
  summary: `--fix-ids` can plan a renumber to an id whose canonical filename is already occupied, so the user confirms a rename that is then refused; the planner checks declared ids but not file names.
  evidence: `next_available_id` skips ids in use, which is why this is now the only route to the `rename_target_exists` refusal at all — it needs a file *named* for an id nothing declares. Having the planner skip such an id turns a refusal into a repair, and would make the refusal path genuinely unreachable rather than merely rare. Review by: with the deferred reporting fix above, same command.
- source_spec: `docs/bmad/implementation-artifacts/spec-identity-resolution-seam.md`
  summary: An entity file outside every standard entity directory is still readable but not writable; `qdev validate` now names it as a `warning`, but the write path cannot resolve it.
  evidence: Cross-story review finding L7, deliberately only *reported* by this story: resolving an arbitrary location needs the cache's `source_path`, which the frozen option-B decision rules out for the write path. The warning tells the user the file's expected name and path, so it is actionable. Review by: whenever option A (cache-driven write resolution) is reconsidered — the two are alternatives, not a sequence.
- source_spec: `docs/bmad/implementation-artifacts/spec-identity-resolution-seam.md`
  summary: The kind-aware cache purge added by this story is untested for kinds that have detail rows, because `--fix-ids` cannot renumber any of them.
  evidence: `next_available_id` supports only Epic/Story/ADR/FR/NFR/Hazard/PRD, and of those only Story has a detail table — so the `deferred_work`/`decisions`/`sprints`/`soup_dependencies`/`gate_runs` branches of the purge are reachable only through the other caller: `upsert_cache_and_mark_dirty`'s stale-path purge, after a hand-edited id plus a `qdev update`. A core-level test on that path would pin it. Review by: epic 2 story 2.8 (deferred work as entities), the first story that writes a kind with a detail row through the CLI.

## Deferred from: code review of spec-sweep-rebuild-convergence (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md`
  summary: `handle_fix_ids` seeds `used_ids` from the cache as well as the duplicate scan, and nothing pins why — the scenario it appears to protect against turns out to be unconstructible.
  evidence: The test named for it (`validate_cli_tests.rs::test_fix_ids_does_not_allocate_an_id_already_used_outside_specs_dir`) no longer tests it, because the scan now walks `state_dir`. Building the real case — an entity in the cache whose file is under no scanned directory — showed the next sweep purges that row, correctly, after which reusing the id is right. Either the seeding guards something not yet identified, or it is dead and should go with a note saying why. Review by: epic 2 planning, alongside the `--fix-ids` reporting work below.
- source_spec: `docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md`
  summary: An unreadable `qdev.toml` is a fifth sweep/rebuild divergence — the rebuild truncates `gates` and never repopulates it, the sweep retains the previous rows, and `gates` has no `stale` flag to signal either state.
  evidence: Unreachable through the CLI today: config loading fails hard before dispatch (`config/mod.rs:1217`), confirmed independently by two reviewers, so it needs `reset_and_rebuild` called directly. It becomes reachable the moment anything hydrates without loading config first. Review by: epic 3 story 3.1 (gate runner), the first consumer that executes gate definitions.
- source_spec: `docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md`
  summary: `deferred_work_is_stale` issues a `get_entity` per deferred-work row from two checks, and `deferred_work_path` a third for the same id — N per-row queries on every `qdev validate` and `qdev doctor`.
  evidence: `list_deferred_work` already joins `entities`, so threading a `stale` field into `DeferredWorkRecord` removes all of them. A public struct change, which is why it was not done inline. Notable because the sweep changes in the same story went out of their way to avoid per-file queries. Review by: whenever `DeferredWorkRecord` is next opened, or if `validate` latency is ever measured.
- source_spec: `docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md`
  summary: Three convergence-test gaps — `unchanged` counts are not asserted alongside `parsed == 0`; nothing asserts that repeated sweeps over a duplicate-id pair settle on the same winner a rebuild picks; nothing covers three files declaring one id.
  evidence: The winner-stability property is the valuable one: the sweep's re-parse bookkeeping is what makes "last sorted path owns the row" hold across passes, and it was verified by hand (three consecutive sweeps, deterministic `parsed: 2`, same winner as a rebuild) rather than by a test. Three-way duplicates make the displacement bookkeeping genuinely non-trivial. Review by: before epic 1 is accepted — this is coverage of the invariant the epic's acceptance now rests on.
- source_spec: `docs/bmad/implementation-artifacts/spec-sweep-rebuild-convergence.md`
  summary: `qdev sync` counts an unreadable file as `unchanged` in its summary, and its `findings=` count remains cache-native only.
  evidence: Pre-existing wording, untouched by this story; the `read_error` finding carries the real signal, but the summary line actively says "unchanged" about a file that could not be read. Pairs with the earlier ledger entry about `sync`'s partial finding count. Review by: epic 2 planning, with the other `sync` summary item.

## Deferred from: code review of spec-init-uses-the-merged-config (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-init-uses-the-merged-config.md`
  summary: `load_project_storage` re-reads and re-validates both config files immediately after `load_config` already did, so the effective and project layouts can come from different content if a file is edited between the two reads — and `.gitignore` is then written from the mismatch.
  evidence: Confirmed by inspection. The fix is for the loader to return both layouts from one read, which means `AnnotatedConfig` carrying the project-only `StorageConfig` — a public struct change, which is why it was not done inline. The window is one process start and both files are small, so the practical risk is doubled I/O rather than a wrong answer. Review by: whenever `AnnotatedConfig` is next opened, or epic 2 story 2.13 (module registry), which adds another consumer of the merged config.
- source_spec: `docs/bmad/implementation-artifacts/spec-init-uses-the-merged-config.md`
  summary: Every write path resolves its advisory-lock directory as `storage.as_ref().map(..).unwrap_or(".qdev/cache")`, so a caller that omits `storage` locks a different file than one that supplies it — and two locks means no mutual exclusion.
  evidence: Four call sites in `crates/qdev-core/src/write.rs`. The CLI always supplies `storage`, so no caller diverges today; this is the last place a hardcoded default layout survives after `8220a63` and this story. Making `WriteOptions::storage` non-optional is a public API change across the write path. Review by: epic 2 story 2.3 (story leases), the first feature with a second lock to coordinate.

## Scheduled from: epic 1 cross-story review pass 2 (2026-09-10)

Full evidence in `epic-1-cross-story-review-2026-09-10-pass-2.md`. The eight high and medium-high
findings there are **blocking for epic 1 acceptance** and are not deferred. Recorded here are the
two pass-1 findings that were dropped by mistake and the mediums.

- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: H8 — `--if-version` is not a fence on `qdev relate`: a no-op relate exits 0 reporting the current version without comparing it, and `unrelate` does not accept the flag at all.
  evidence: **Triaged as high and blocking in pass 1, then never scheduled or filed** — a tracking failure, not a missed defect. Re-verified as reproducing verbatim at pass 2. `apply_relation_change` returns before `patch_frontmatter` on the no-op path, so the version is never checked; an agent using the flag as a compare-and-swap fence reads exit 0 as confirmation. Also undocumented: `cli-reference.md` documents `--if-version` on `update` only. Review by: **before epic 1 acceptance** — it is one of pass 1's eight blocking findings.
  - source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10.md`
  summary: M1 — `qdev unrelate` with an unknown relation name exits 0 reporting `changed: false` while the real edge survives; `relate` reports the same input as `invalid_relation_kind` with a message that misdescribes it.
  evidence: Filed in pass 1 with "review by: with the identity-resolution story"; that story shipped as `61cebe7` without touching it, so the review-by point elapsed silently. Re-verified as reproducing at pass 2. Review by: **before epic 1 acceptance**, with H8 — both are in the relation write path.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md`
  summary: `qdev validate --fix-ids` exits 0 on a workspace whose plain `qdev validate` exits 1, whenever there are no duplicate ids to fix.
  evidence: Early `return ExitCode::Success` when the duplicate scan is empty (`main.rs:2198`), contradicting the exit rule stated in that function's own comment. A CI step running `--fix-ids --yes` as self-healing validation goes green on a workspace with error-severity findings. Review by: with the H8/M1 contract work — same command family.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md`
  summary: `qdev update --field schema_version=99` emits a JSON document with two `schema_version` keys, so a lenient parser reads the envelope's contract version as 99.
  evidence: `MANAGED_FIELDS` blocks `id`/`version`/`updated_by`/`created_by` but not `schema_version`, and the update payload is `#[serde(flatten)]`ed into the envelope. `DoctorSectionReport`'s serializer has a duplicate-key `debug_assert` for exactly this hazard. Review by: epic 3 story 3.2, which freezes the payload contract — or sooner, since the fix is one entry in a list.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md`
  summary: `qdev init` is the only cache mutator that does not take `<cache_dir>/write.lock`, so its drop-and-recreate is serialized against concurrent writers only by SQLite's own locking.
  evidence: Reported independently by two reviewers; verified that `init` proceeds immediately while every other command exits 5 `lock_timeout` against a held lock. No wrong end state was constructed — the Markdown files are the source of truth and `init` clears `sync_state` — so this is an unenforced invariant rather than a demonstrated defect. Review by: with the NEW-1/NEW-2 migration work, which is already in `initialize_cache`.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md`
  summary: A `./`-prefixed layout value defeats the normalization added by `60f509d` — `.gitignore` gets entries git cannot honour, and `parent_dir_of("./cache")` scaffolds `gates/` at the repo root where `"cache"` puts it under `.qdev`.
  evidence: Verified with `git status`: the live cache is untracked-but-committable. The loader trims whitespace and a trailing `/` but not a leading `./`, and `parent_dir_of`'s fallback differs between the two spellings of one directory. Review by: with the remaining layout work — it is the same class as the empty/absolute guard already added.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md`
  summary: `--fix-ids` renames within the file's current directory, ignoring the directory half of the identity rule, so it reports a repair that leaves the entity readable but not writable.
  evidence: Reproduced: a duplicate in `docs/specs/stories/epic1/` is renamed to `epic1/E1S2.md` and reported at exit 0, after which `get E1S2` works and `update E1S2` fails "Entity file not found". This is the state `61cebe7` says the rename removes; the off-convention warning names the correct destination, so the information is available. Review by: with NEW-3, which is the same story's blind spot on the other side.
- source_spec: `docs/bmad/implementation-artifacts/epic-1-cross-story-review-2026-09-10-pass-2.md`
  summary: `payload-doctor.json`'s `finding_count` description still says four computed checks; there are five.
  evidence: The doctor doc comments were corrected when the fifth check landed and the schema description was not. Review by: next time a payload schema is touched.

## Deferred from: code review of spec-one-id-in-use-rule (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md`
  summary: The hex allocators (`DW-`, `DEC-`) still probe one flat directory case-sensitively via `id::hex_id_collides`, so the class this invariant closed for story ids is still open for them.
  evidence: Honest remaining instance, raised by the implementer and a reviewer. `next_available_id` rejects those kinds by design and their strategy is random-with-length-growth rather than sequential, so they are a different allocator — but the same class: a private, narrower answer to "is this id taken". Pointing them at `ids_in_use` is the fix. Review by: **before epic 1 acceptance** — the invariant is not fully established while a second allocator disagrees.
- source_spec: `docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md`
  summary: `--fix-ids`' `skipped` array carries no reason, and there are now two refusal causes plus a read failure; the actionable content (the expected path) goes only to stderr, and the advice can name a destination that is still off-convention on the filename axis.
  evidence: A JSON consumer sees a bare path and cannot tell an off-convention refusal from a prompt decline. The fix is a structured entry (`path`, `reason`, `expected_path`) plus naming the canonical `<id>.md` target — an additive payload-schema change this story did not scope. Review by: with the next `--fix-ids` change; it is the third reporting item filed against that command.
- source_spec: `docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md`
  summary: A *directory* named `<id>.md` contributes no carried id, so allocation hands out that id and `create_story` then refuses `file_exists` for an id the user never chose.
  evidence: Same shape as the `.MD` hole fixed in this story, but `collect_markdown_files` yields files only, and including directory names changes the shared collector every hydration path uses. Review by: with the directory-comparison item below — both are the walk's edges.
- source_spec: `docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md`
  summary: `off_convention_directory_target` compares directories by case-sensitive string equality, so a mis-cased `docs/Specs/stories/` is refused as off-convention although every writer resolves it on a case-insensitive filesystem.
  evidence: Narrow — it needs a directory the user typed with different case — but it is a case-sensitive comparison inside a story about matching hydration's case-insensitivity. Belongs with the wider path-normalization question (separators, `./` prefixes) rather than a one-off fix. Review by: with the `./`-prefix layout item already filed from pass 2.
- source_spec: `docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md`
  summary: `qdev create story` now walks both hydrated trees and parses every file's frontmatter on each invocation, with nothing measuring it; and `--fix-ids` re-reads each duplicate inside the refusal check.
  evidence: Bounded by the same walk the boot sweep already performs on every command (6 ms median at N=1000 on an idle machine), so the create path cannot cost more than a boot — but that is an argument, not a measurement. A benchmark row for allocation at N=1000 would settle it. Review by: if create latency is ever questioned, or with the next allocation change.
- source_spec: `docs/bmad/implementation-artifacts/spec-one-id-in-use-rule.md`
  summary: `--fix-ids` refusal coverage gaps — no test for a non-story kind (the `kind_for_write` path the check leans on), none for a state-tree duplicate, no exit-code assertion on the refusal, and the occupied-rename-target case narrowed to a directory occupant only.
  evidence: The narrowing is the interesting part and is now correct-by-construction: a *file* at the rename target puts that id in use, so it is never allocated, leaving only a directory able to occupy the name. Worth a test asserting that reasoning rather than assuming it. Review by: with the `--fix-ids` reporting work above.

## Deferred from: code review of spec-change-gate-compares-content (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-change-gate-compares-content.md`
  summary: The "cannot resolve the edit" predicate only recognises a *whole-second* stamp, so a filesystem with millisecond or centisecond timestamp granularity keeps the original hole — a same-length edit inside one coarse tick stays invisible.
  evidence: The predicate tests `stamp % 1_000_000_000 == 0`, which is the signature of one-second granularity (HFS+, some network mounts) and nothing else. The general fix is to detect the granularity — trailing-zero width of the stamp, or probing it once per workspace — rather than hardcoding one second. No filesystem in the support matrix has sub-second-but-coarse granularity today. Review by: if a coarse-granularity filesystem is ever added to the support matrix, or with the next change to the gate.
- source_spec: `docs/bmad/implementation-artifacts/spec-change-gate-compares-content.md`
  summary: A change stamp that saturates at `i64::MAX` compares equal to every other saturated stamp and is never second-aligned, so edits to a file with an absurd or corrupt mtime are never seen.
  evidence: `i64::try_from(d.as_nanos()).unwrap_or(i64::MAX)` — reachable only with an mtime beyond the year 2262 or a corrupted one. Treating a saturated stamp as "cannot resolve" would close it in one line. Filed with the granularity item; both are the stamp's edges. Review by: with the granularity work.
- source_spec: `docs/bmad/implementation-artifacts/spec-change-gate-compares-content.md`
  summary: The convergence test no longer compares `sync_meta` across a sweep and a rebuild, and nothing replaced the assertion, so a rebuild leaving that table empty or duplicated is uncaught.
  evidence: The exclusion itself is correct — `sync_meta` holds a wall-clock stamp that cannot be equal across two hydrations — but a shape assertion (exactly one row on both paths) would keep the coverage the exclusion dropped. Review by: next time the convergence test is edited.

## Deferred from: code review of spec-stale-means-absent (2026-09-10)

- source_spec: `docs/bmad/implementation-artifacts/spec-stale-means-absent.md`
  summary: `qdev relate` decides entity existence with the unfiltered `get_entity` and then reads `target.kind` off a possibly stale row, so a kind-pair refusal can be decided from pre-edit content.
  evidence: Not in this story's inventory of five sites, and arguably outside its rule: `relate` is a *write gate*, not a derived finding, so refusing on a stale row's kind may be the right conservative answer — but nobody has decided which, and the site looks exactly like the one this story fixed. Either route it through the helper or name it as a fourth deliberate exception. Review by: with the `--if-version`/`unrelate` contract work already filed against the relation write path (pass 1's H8 and M1).
- source_spec: `docs/bmad/implementation-artifacts/spec-stale-means-absent.md`
  summary: `ids_in_use`'s deliberate use of the unfiltered `list_entities` has no test pinning the stale case, so a later consistency pass could adopt the derivation helper there and hand a stale row's id to a second entity.
  evidence: The architecture text now says "every derivation site asks the helper", which makes exactly that mistake tempting; only a comment protects the exception. A test asserting that `create story` skips a *stale* row's id would make the exception enforceable. Review by: **before epic 1 acceptance** — it guards invariant 1's own rule.
- source_spec: `docs/bmad/implementation-artifacts/spec-stale-means-absent.md`
  summary: A `deferred_work` row whose `origin_story_id` names a non-story entity (an epic, another DW, a decision) passes the origin check, which never verified kind.
  evidence: Pre-existing and independent of staleness; the check asks only whether *something* with that id exists. Review by: epic 2 story 2.8 (deferred work as entities), which owns the DW model.
- source_spec: `docs/bmad/implementation-artifacts/spec-stale-means-absent.md`
  summary: The stale-means-absent rule is centralised but not mechanically enforced — nothing stops a future site writing `get_entity(..).is_some()` at a derivation point.
  evidence: A clippy `disallowed-methods` entry on `Store::get_entity` would turn the convention into a compile error, at the cost of annotating every legitimate read (query, doctor, CLI — roughly ten sites), which inverts the burden onto the common case. Renaming the read to `get_entity_including_stale` is the other option and touches the same call sites. Filed rather than taken because both trades deserve a decision, not a default. Review by: epic 2 planning, when the number of derivation sites is known.
