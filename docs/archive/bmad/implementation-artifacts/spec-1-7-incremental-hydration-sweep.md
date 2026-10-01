---
title: 'Incremental Hydration Sweep'
type: 'feature'
created: '2026-09-07'
status: 'done'
route: 'dispatch'
baseline_commit: 'd00b4360ad6c7dc87f3e7cc3b197a014d2dcda55'
review_loop_iteration: 1
context:
  - 'docs/architecture.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** The SQLite cache is only rebuilt when missing or schema-mismatched; on a healthy boot the working tree is never re-scanned, so hand edits, external changes, and branch switches never reach queries, removed files linger in the cache forever, and the ≤30 ms hydration target (AD-6, architecture §11) is unmet.

**Approach:** Add an incremental boot sweep inside `ensure_cache`: stat every file under the configured spec/state directories plus `qdev.toml`, compare mtime+size against `sync_state`, hash only changed candidates, re-parse and upsert only files whose hash changed (one transaction, under the advisory write lock), purge cache rows of removed files, always re-parse rows qdev marked dirty, and record per-file validation findings (`merge_conflict`, `schema_violation`) in a new `findings` table while retaining invalid entities' previous rows flagged stale. The cache schema moves to v2 to carry the `findings` table and `entities.stale` column.

## Boundaries & Constraints

**Always:**
- Sweep exactly the file set the full rebuild scans: `*.md` under `specs_dir`/`state_dir` (recursive), `*.jsonl` under `state_dir/scratch`, `*.json` under `state_dir/evidence`, plus root `qdev.toml` (gates refresh).
- A file is a re-hash candidate only if its mtime or size differs from `sync_state`, its `sync_state` row is missing, or its entity is in `dirty_entities`. Re-parse only when the hash changed (or row dirty/missing). Update `sync_state` (mtime, size, hash) for every scanned file.
- All sweep mutations commit in a single transaction under the advisory write lock (`<cache_dir>/write.lock`, 5 s timeout; timeout → exit 5 `conflict`), matching the full rebuild.
- A file containing `<<<<<<<` anywhere is never parsed: it yields a `merge_conflict` finding for that path only; hydration of every other file continues; its previous entity row (if any) is retained and flagged stale.
- A file that fails frontmatter extraction, lacks a non-empty `id`, or fails its kind's JSON-schema validation yields a `schema_violation` finding (message carries the parse/validator errors); the previous entity row is retained and flagged stale; the flag clears on the next successful parse of that entity.
- Findings are per-path current state: the sweep replaces a path's findings every pass; purging a path removes its findings. Both codes carry severity `error`.
- A full rebuild and a sweep of the same tree produce identical `findings`/`stale` state: the per-file parse/upsert/finding logic is shared by both paths.
- `qdev-core` stays free of terminal I/O and forbidden dependencies; no new dependencies.
- **Benchmark decision (2026-09-07):** the ≤30 ms bound is proven as the median of repeated warm sweeps (after a warm-up pass), with a generous ≤500 ms ceiling enforced on every run as a regression backstop.

**Never:**
- Never rebuild the whole cache on a healthy boot; never hash or parse a file whose mtime, size, and stored hash all match (except dirty/missing rows).
- Never fail a command because of a `merge_conflict` or `schema_violation` finding — findings are recorded, not fatal; the command's own exit code is unaffected.
- Never migrate v1 cache data in place: v1 caches are detected as a schema mismatch and rebuilt losslessly from files via the existing mechanism.
- Never consult Git to detect changes — the filesystem sweep alone covers branch switches and hand edits.
- Never emit findings for malformed scratch `.jsonl` lines or malformed evidence `.json` (out of scope; current silent-skip behavior is kept).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Warm boot, no changes | 1,000-entity warmed cache; next boot | No file reads/hashes/parses; sweep no-ops; cache identical | N/A |
| Single modified file | One entity's content changed | Only that file read+hashed+re-parsed; its rows (entities, kind table, constraints, relations) replaced; sync_state updated | N/A |
| Touch, same content | File re-touched or same-size edit, content identical | Hashed; hash unchanged → no re-parse; sync_state mtime/size updated | N/A |
| In-place id edit | One entity's frontmatter `id` changed | Old-id row (and cascade) purged before the new-id row is upserted; no orphan rows | N/A |
| Branch switch | Another branch checked out: files changed, added, removed | Changed/added re-parsed; removed files' entities rows, child rows, sync_state, findings purged | N/A |
| Merge conflict | One file contains `<<<<<<<` | `merge_conflict` finding for that path only; previous row retained, stale=1; all other files hydrated | Non-fatal |
| Schema violation | One file's frontmatter fails its schema (or YAML/`id` missing) | `schema_violation` finding with error detail; previous row retained, stale=1; flag clears once fixed | Non-fatal |
| qdev write (dirty) | `qdev update` ran, then next boot | Written entity re-parsed even if mtime/size match; dirty row cleared; sync_state restored | N/A |
| Gates edited | `[[gates]]` in `qdev.toml` changed | Gates table re-upserted from the file on next boot | N/A |
| v1 cache present | Existing v1 `cache.sqlite` | Boot detects mismatch → lossless full rebuild to v2; state equals a fresh sweep | N/A |
| Lock contention | Another process holds `write.lock` >5 s during sweep | Boot fails exit 5 `lock_timeout`, same semantics as rebuild | Conflict envelope |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs` -- `SqliteStore` (:193); constants `CACHE_SCHEMA_VERSION`/`CACHE_USER_VERSION`/`BUSY_TIMEOUT_MS` (:20-22); `ALL_TABLE_NAMES` (:24-39); `SCHEMA_V1_DDL` (:41); `rebuild_from_workspace` (:370-1270; per-file flow :503-1023 to extract — mtime/size :515-522, sync_state upsert :525-542, frontmatter :544, id check :549, kind :554, entities+kind upserts :584-1022; scratch :1025-1133; evidence :1135-1257); `reset_and_rebuild` (:338); `create_schema_v1` (:3340); `drop_all_user_tables` (:3366); `inspect_cache_schema` (:3481-3554, pragmas + exact table set); `ensure_cache` (:3559-3623; healthy path :3620-3622 is the sweep hook point); `collect_markdown_files`/`collect_files_with_ext` (:3625-3663); `determine_entity_kind` (:3665-3725); private `sha256_digest` (:3727); `delete_entity` (:1554, child-row cascade); `list_sync_state` (:3190); `get_dirty_entities` (:3249)
- `crates/qdev-core/src/store/mod.rs` -- `Store` trait (:212-321); `EntityRecord` (:17); `SyncStateRecord` (:190); `DirtyEntityRecord` (:199) -- gains `FindingRecord`, `SweepSummary`, findings CRUD, `sweep_workspace`, `stale` on `EntityRecord`
- `crates/qdev-core/src/schema.rs` -- `EntityKind` (:11); `extract_frontmatter` (:242); `validate_frontmatter_detailed` (:305) → `Vec<ValidationError{path,message}>` (:173); `SchemaError` (:141) -- reused as-is for sweep findings
- `crates/qdev-core/src/write.rs` -- `acquire_write_lock` (:62); public `sha256_digest` (:799); `upsert_cache_and_mark_dirty` (:808-953: marks entity dirty + DELETEs its sync_state row — the sweep's missing-row trigger)
- `crates/qdev-core/src/init.rs` -- re-exports `create_schema_v1` (:7-9); `check_cache_status` (:69) and `initialize_cache` (:334) compare `user_version` against `CACHE_SCHEMA_VERSION` — the v1→v2 migration rides this existing comparison
- `crates/qdev-core/src/lib.rs` -- public re-exports (e.g. `sha256_digest` :47)
- `crates/qdev-cli/src/main.rs` -- boot hook :148-155 calls `ensure_cache`; CLI stays unchanged (sweep lives in core)
- `crates/qdev-core/tests/store_tests.rs` -- setup pattern (TempDir + fs::write + ensure_cache, :522-552); `dump_all_tables` (:601); table-by-table identity test (:641) -- extend for v2 + findings
- `crates/qdev-cli/tests/cache_cli_tests.rs` -- assert_cmd + `current_dir` + `qdev_core::rusqlite` cache-inspection idiom (:5-49); dirty-entity assertion pattern in `crates/qdev-cli/tests/update_cli_tests.rs:76-96`
- `docs/architecture.md` -- §10 cache schema (:245-332); §11 storage & hydration (:338-349, includes the 30 ms target)

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/store/sqlite.rs` -- add v2 DDL (`entities.stale INTEGER NOT NULL DEFAULT 0`; `findings(path, code, severity, message, found_at; PK(path,code))`), bump `CACHE_SCHEMA_VERSION`/`CACHE_USER_VERSION` to 2, extend `ALL_TABLE_NAMES`, rename `create_schema_v1` → `create_schema_v2`, update `inspect_cache_schema` (pragmas=2 + 15-table set) -- v2 cache schema
- [x] `crates/qdev-core/src/store/mod.rs` -- add `FindingRecord`, `SweepSummary {parsed, unchanged, purged, findings}`, findings CRUD, `sweep_workspace`, `stale: bool` on `EntityRecord` -- public sweep API
- [x] `crates/qdev-core/src/store/sqlite.rs` -- extract the rebuild's per-file parse/upsert (:503-1023) into a shared helper returning an outcome (`Parsed` / `MergeConflict` / `SchemaViolation{errors}` / `NoId`), used by both `rebuild_from_workspace` and the sweep; rebuild clears and repopulates `findings` so rebuild state == sweep state -- single source of hydration logic
- [x] `crates/qdev-core/src/store/sqlite.rs` -- implement `sweep_workspace`: walk the same 4+1 file sets; stat vs `sync_state` (+ missing rows + `dirty_entities`); hash candidates; re-parse hash-changed in one `Immediate` transaction under `acquire_write_lock`; purge removed files (entities cascade via `delete_entity`, target-side relations, `scratchpad_entries`/`gate_runs` by path, sync_state, findings); when re-parsing a file, first purge entity rows claiming the same `source_path` under a different `id` (cascade + target-side relations) so in-place id edits leave no orphans; set/clear `stale`; replace per-path findings; refresh gates when `qdev.toml` hash changes; clear consumed dirty rows; return `SweepSummary` -- the sweep
- [x] `crates/qdev-core/src/store/sqlite.rs` -- call the sweep from `ensure_cache` on the healthy path (after `open`, when no rebuild happened) -- boot integration
- [x] `crates/qdev-core/src/init.rs` -- follow the `create_schema_v2` rename (re-export + call sites) -- DDL rename
- [x] `crates/qdev-core/tests/sweep_tests.rs` -- new: warm no-op boot; single-file re-parse; same-content touch; branch-switch add/change/remove purge; `<<<<<<<` finding + stale + others continue; schema-violation finding + stale retained + cleared on fix; dirty-row forced re-parse + clear; gates refresh; v1→v2 auto-rebuild; rebuild/sweep findings equality; benchmark test (warm-up pass, then 25 warm sweeps; assert median ≤30 ms and every run ≤500 ms) -- sweep verification
- [x] `crates/qdev-cli/tests/sweep_cli_tests.rs` -- new E2E: `qdev status` boots the sweep (modified file visible in cache, removed file gone, findings rows present, lock-timeout exit 5) -- CLI contract
- [x] `crates/qdev-core/tests/store_tests.rs` + `crates/qdev-core/tests/init_tests.rs` -- update v2 assertions (15 tables, pragmas=2, `stale` column, findings in `dump_all_tables`) and add v1→v2 migration via `qdev init --yes` -- schema-bump coverage
- [x] `crates/qdev-core/src/lib.rs` -- re-export the new public types -- API surface

**Acceptance Criteria:**
- Given 1,000 entity fixtures with one modified, when a command boots, then the sweep compares mtime+size to `sync_state`, hashes only changed candidates, re-parses only hash-changed files, and the benchmark test proves the bound: median of repeated warm sweeps ≤30 ms with every run ≤500 ms.
- Given a hydrated cache, when the working tree switches to another branch (files changed, added, removed), then changed/added files are re-parsed and removed files' rows (entities, child rows, sync_state, findings) are purged.
- Given a hydrated cache, when one file contains `<<<<<<<`, then a `merge_conflict` finding exists for that path only, its previous row is retained flagged stale, and every other file hydrates normally with the command exiting 0.
- Given a hydrated cache, when one file's frontmatter is schema-invalid, then a `schema_violation` finding with the validator errors exists, the previous row is retained flagged stale, and the flag clears after the file is fixed and a command boots.
- Given a `qdev update` (dirty row written, sync_state deleted), when the next command boots, then that entity is re-parsed regardless of mtime/size and its dirty row is cleared.
- Given an existing v1 cache, when any command boots, then it is rebuilt losslessly to v2 (pragmas 2, 15 tables) and the resulting state equals a fresh sweep of the same tree.
- Given the workspace, when running `cargo test`, then all suites pass including the architecture tests (no forbidden deps, no terminal I/O in core), with `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.

### Review Findings

_Code review 2026-09-08 (blind-hunter, edge-case-hunter, verification-gap, acceptance-auditor)._

- [x] [Review][Patch] Unreadable file is swept as `unchanged` and its dirty marker is cleared [crates/qdev-core/src/store/sqlite.rs:2939, :3013] — `fs::read_to_string` failure in `sweep_workspace` does `unchanged += 1; continue;` without touching `sync_state` and without recording a finding, while step 3 still deletes the file's `dirty_entities` row (clearing is keyed on path, not on hydrate outcome). **Decision 2026-09-08 (Simon): add a `read_error` finding code** (severity `error`) for the failure, and clear dirty rows only for paths whose hydrate actually returned `Parsed`. Note: this adds a third code beyond the frozen spec's `merge_conflict` / `schema_violation` — recorded here rather than by editing the frozen section.
- [x] [Review][Patch] `create_schema_v2` can stamp a pre-v2 database as v2 without adding `entities.stale` [crates/qdev-core/src/store/sqlite.rs:3069, :3248] — the DDL is all `CREATE TABLE IF NOT EXISTS`, so on a real 14-table v1 cache it creates `findings` and sets both pragmas to 2 while `entities` still lacks `stale`. `inspect_cache_schema` checks only pragmas and table names, so that database reports `Valid` and every subsequent sweep fails with `no such column: stale`. Reachable if a rebuild is interrupted between `SqliteStore::open` and `reset_and_rebuild`. **Decision 2026-09-08 (Simon): have `inspect_cache_schema` verify the `entities.stale` column**, so a half-migrated database reports `Mismatch` and self-heals via rebuild.

- [x] [Review][Patch] Sweep re-parse never replaces an entity's child rows [crates/qdev-core/src/store/sqlite.rs:3859] — `hydrate_markdown_file` only deletes rows when the file's `id` changed; otherwise constraints, relations, sprint rows and sprint assignments are re-inserted with `ON CONFLICT DO UPDATE` / `DO NOTHING` and nothing removes entries that disappeared from the file. The rebuild path is immune only because it `DELETE`s every table first. Removing a `constraints:` entry, a `relations:` target or a sprint `assignments:` entry leaves the row in the cache forever, and an id-preserving `kind` change leaves the old kind-detail row (`stories`/`sprints`/`deferred_work`/…) alongside the new one. Violates the I/O matrix row "Single modified file → its rows (entities, kind table, constraints, relations) replaced" and the Always-constraint that rebuild and sweep produce identical state. Fix: `DELETE FROM constraints WHERE owner_id = ?`, `DELETE FROM relations WHERE source_id = ?`, `DELETE FROM sprint_assignments WHERE sprint_id = ?` (and the stale kind-detail row) before the re-inserts.
- [x] [Review][Patch] `test_rebuild_and_sweep_findings_equal` is vacuous [crates/qdev-core/tests/sweep_tests.rs:590] — it snapshots after a rebuild, then sweeps the *unchanged* tree, so every file hits `!meta_changed && !is_dirty` and no hydration code runs at all; the two snapshots are identical by construction. It cannot detect the divergence above. Fix: mutate the tree (or rebuild a second workspace from the same tree after sweeping) and compare full `dump_all_tables` output.
- [x] [Review][Patch] `test_v1_cache_auto_rebuilds_to_v2` does not use a real v1 cache [crates/qdev-core/tests/sweep_tests.rs:543] — it creates an empty database with only `PRAGMA user_version = 1; PRAGMA schema_version = 1;`. A genuine v1 cache has 14 tables and an `entities` table without `stale`, which is the actual migration risk. The test also asserts only pragmas, table count and two entity rows — never AC-6's "the resulting state equals a fresh sweep of the same tree".
- [x] [Review][Patch] The benchmark never times the AC's "one modified" case [crates/qdev-core/tests/sweep_tests.rs:660] — AC 1 reads "1,000 entity fixtures with one modified", but the test loops 25 sweeps over a completely untouched tree and asserts `parsed == 0`. No test asserts that at N=1,000 exactly one file is re-parsed, nor times that path.
- [x] [Review][Patch] Findings CRUD has neither a caller nor a test [crates/qdev-core/src/store/mod.rs:355] — `upsert_finding`, `get_finding` and `delete_findings_for_path` are unused anywhere; `list_findings`/`get_findings_for_path` are only read against rows written by the internal `record_finding` helper. Swapping two bound parameters in `upsert_finding` would ship green. Every other one of the 14 tables has a `test_store_*_crud` in `store_tests.rs`; add `test_store_findings_crud`.
- [x] [Review][Patch] Removal purge is verified only for a story `.md` [crates/qdev-core/tests/sweep_tests.rs:241] — the only removal tests delete a childless story. The sprint (`sprints` + `sprint_assignments`), deferred-work, decision, soup, scratch `.jsonl`, evidence `.json` and `qdev.toml`-gates branches of `purge_removed_path` / `purge_entity_with_children` are unexercised; deleting any of those arms leaves the suite green.
- [x] [Review][Patch] The sweep is never tested against scratch `.jsonl` or evidence `.json` files [crates/qdev-core/tests/sweep_tests.rs:69] — every sweep-test workspace contains only `qdev.toml` and story Markdown, so `SweepFileRole::Scratch` / `Evidence` never execute under test. Dropping their `collect_files_with_ext` calls would not only stop syncing them but *purge* their rows (they would fall out of `disk_paths`), with no test failing.
- [x] [Review][Patch] `Store::upsert_entity` silently drops `EntityRecord.stale` [crates/qdev-core/src/store/sqlite.rs:502] — the new field is decoded on read but the insert omits the column and the conflict clause hard-codes `stale = 0`, so a record round-tripped through the trait loses the flag with no error. Bind `record.stale` (the write path in `write.rs` has its own SQL and keeps its clear-on-write semantics).
- [x] [Review][Patch] No test asserts that a write clears `stale` [crates/qdev-core/src/write.rs:859] — `stale = 0` was added to `upsert_cache_and_mark_dirty`'s conflict clause, but no test pre-sets `stale = 1` on a row and writes to it. Removing the line leaves the suite green while making staleness permanently sticky for entities repaired through `qdev update`.
- [x] [Review][Patch] `get_entity` swallows a `stale` decode error [crates/qdev-core/src/store/sqlite.rs:693] — `row.get(18).unwrap_or(false)` where `list_entities` uses `row.get(18)?`. `get_entity` is exactly what the sweep tests' stale assertions read, so the swallow sits underneath the AC checks. Make it `?`.
- [x] [Review][Patch] `parse_sprint_number` and the hydrate path disagree on sprint ids [crates/qdev-core/src/store/sqlite.rs:3490] — purge accepts only `sprint-N`, while `hydrate_markdown_file` falls back to a bare `id.parse()` and then to `0`. The sprint schema puts no pattern on `id`, so a sprint with id `"1"` gets rows that purge can never remove, and one with id `"s1"` is written under sprint number 0. Share one id→number helper between the two paths.
- [x] [Review][Patch] Redundant `clear_findings_for_path` calls [crates/qdev-core/src/store/sqlite.rs:3796] — it runs once near the top of `hydrate_markdown_file` and again as the first statement of all four failure branches (:3799, :3814, :3835, :3849). Dead work; drop the repeats.

- [x] [Review][Defer] `PRAGMA schema_version` is used as an application version, and a future `user_version` is silently rebuilt on the boot path [crates/qdev-core/src/store/sqlite.rs:3248, :3289] — deferred: pre-existing from story 1.6 (present verbatim at baseline `d00b436`); `schema_version` is SQLite's DDL-change counter, and `ensure_cache` rebuilds a newer-than-supported cache where `init.rs::check_cache_status` correctly returns a `schema_version_mismatch` conflict.
- [x] [Review][Defer] No indexes; the sweep does two full `entities` scans per pass [crates/qdev-core/src/store/sqlite.rs:2870] — deferred: the v2 DDL declares no `CREATE INDEX` (as v1 did not), and the benchmark meets the 30 ms budget at N=1,000, so this is a scaling concern rather than a current defect. An index on `entities(source_path)` and a `WHERE id IN (SELECT id FROM dirty_entities)` query would replace both scans.
- [x] [Review][Defer] `docs/architecture.md` §10/§11 still documents the v1 cache schema [docs/architecture.md:245] — deferred: fix edits a shared context document, not this story's code. §10 lists 14 tables with no `findings` table and no `entities.stale`; §11.2 still promises dangling-relation findings that this story deliberately scoped out.
- [x] [Review][Defer] Two files declaring the same frontmatter `id` silently overwrite each other [crates/qdev-core/src/store/sqlite.rs:3901] — deferred: pre-existing; `rebuild_from_workspace` has always had the same last-writer-wins behaviour and this story does not change it.

#### Rejected

- `low` — "The benchmark test asserts wall-clock timings inside the default `cargo test` run (and the lock-contention test burns the full 5 s timeout)." **Decision 2026-09-08 (Simon): keep as-is** — the spec's Verification section expects `cargo test` to cover the benchmark, and the 500 ms ceiling is the intended regression backstop.
- `false` — "Nothing surfaces the sweep's `SweepSummary` or findings to the user." The spec's Code Map states the CLI stays unchanged and the Design Notes assign findings surfacing to stories 1.11 (`qdev validate`) / 1.12 (`qdev doctor`, `qdev sync` counts).
- `false` — "A same-size edit inside one mtime second is never re-hashed." Real, but recorded verbatim as an accepted trade-off in the spec's Design Notes ("mtime has 1-second granularity … missed until the next change"); changing it would edit the frozen spec.
- `false` — "Every command now takes the exclusive write lock, so a read-only command can stall 5 s and exit 5." The frozen I/O matrix makes exit 5 `lock_timeout` on sweep lock contention the required behaviour, and `sweep_cli_tests` asserts it.
- `false` — "Purging a removed file drops inbound relations from unchanged files that the sweep never rebuilds." The frozen spec's purge cascade explicitly prescribes deleting target-side relations (`relations WHERE source_id = ?1 OR target_id = ?1`).
- `false` — "`entity_rows_for_source_path` swallows an unknown kind via `unwrap_or(EntityKind::Story)`, orphaning child rows." `entities.kind` is only ever written from `EntityKind::as_str()`, so `from_str_loose` cannot fail on a qdev-written row; no reachable input was shown.
- `false` — "Removing `create_schema_v1` / `SCHEMA_V1_DDL` breaks external callers." Both were internal to this workspace; every call site was updated and the workspace compiles.
- `false` — "The sweep only covers `.md` / `.jsonl` / `.json` / `qdev.toml`, not every file." That is precisely the 4+1 file set the frozen spec prescribes, matching the full rebuild.
- `false` — "Malformed evidence `.json` or scratch `.jsonl` produces no finding." The frozen spec's Never list explicitly excludes those from findings and keeps the silent-skip behaviour.
- `false` — "A schema-invalid file now loses its entity row entirely on a full rebuild." This follows the frozen spec (the previous row is retained; a fresh rebuild has no previous row). Worth knowing operationally: because v1 caches rebuild on first boot, any entity whose file fails validation drops out of the cache until fixed — but changing it would edit the spec.

## Implementation Notes

- Purge cascade in `sweep_workspace` uses a transaction-local `purge_entity_with_children` helper (same cascade as `delete_entity`: entity + stories/constraints/relations incl. target-side) rather than calling the `delete_entity` trait method — a `&self` trait call cannot participate in the sweep's single `Immediate` transaction. Behavior is identical to the spec's described cascade.
- Matrix row "qdev write (dirty)" is covered by two core tests: `test_dirty_row_forced_reparsed_and_cleared` (dirty flag alone forces re-parse with sync_state intact — the strongest case) and `test_dirty_row_with_deleted_sync_state_restored` (mimics the exact write-path side effects: dirty row + deleted sync_state; asserts re-parse, dirty cleared, sync_state row restored).

## Spec Change Log

- 2026-09-07: Implementation complete and verified (all tasks `[x]`; cargo test/clippy/fmt all pass). Added Implementation Notes (transaction-local purge helper; two-test coverage of the dirty-row matrix row).
- 2026-09-08: Code review applied. Sweep now replaces an entity's child rows on re-parse (`clear_owned_child_rows`); records a `read_error` finding and keeps the dirty row when a file cannot be read; clears dirty rows only for files that actually parsed; `inspect_cache_schema` verifies the `entities.stale` column so a half-migrated cache rebuilds; `upsert_entity` round-trips `stale`; one shared sprint-id→number helper. Seven new tests (child-row replacement, non-vacuous rebuild≡sweep, every-kind purge, scratch/evidence sweep, unreadable file, real v1 cache, half-migrated cache, one-modified benchmark) plus findings CRUD, stale round-trip and write-path stale coverage.

## Review Triage Log

- **2026-09-08 — code review (4 layers: blind-hunter, edge-case-hunter, verification-gap, acceptance-auditor).** 3 decision-needed (all resolved by Simon), 14 patches (all applied), 4 deferred to `deferred-work.md`, 10 rejected. See `### Review Findings` above.
- **Known spec tension (no code change).** The Always-constraint "a full rebuild and a sweep of the same tree produce identical `findings`/`stale` state" cannot hold for a file that was *valid before and is conflicted or schema-invalid now*: the sweep retains the previous entity row flagged stale (as the matrix requires), while a full rebuild of that tree has no previous row to retain and produces none. `test_rebuild_and_sweep_findings_equal` therefore proves convergence for changed, added, removed, newly-conflicted and newly-invalid files; the retained-stale-row case is covered separately by `test_merge_conflict_finding_and_stale` and `test_schema_violation_finding_stale_and_cleared_on_fix`. Renegotiate the wording when the spec is next opened.
- **Finding vocabulary widened.** Per Simon's decision, the sweep now also records `read_error` (severity `error`) for a file it cannot read, beyond the frozen spec's `merge_conflict` / `schema_violation`.

## Design Notes

- **Why v2:** "flagged stale" (AC) and "recorded as validation findings" (§11.2) need a durable home, and the codebase's only DDL-evolution mechanism is version-bump-then-lossless-rebuild (`inspect_cache_schema` ⇒ mismatch ⇒ rebuild from files). Bumping is therefore the cheap path; every developer's cache rebuilds exactly once on next boot.
- **Purge cascade for a deleted entity path:** `SELECT id FROM entities WHERE source_path = ?` → `delete_entity(id)` (cascades stories/constraints/relations) + delete target-side relations + findings for the path. Removed scratch `.jsonl` → delete `scratchpad_entries` by story id; removed evidence `.json` → delete `gate_runs` by evidence_path; sync_state row deleted in all cases.
- **Accepted trade-off (per AD-6 / §11.1):** mtime has 1-second granularity, so an external edit landing in the same second with the same size is missed until the next change; qdev's own writes can never be missed because the write path deletes the entity's sync_state row and marks it dirty.
- **Conflicted files** update `sync_state` with the current hash, so a stable conflict is detected once (finding persists across no-change boots) and a resolved file re-parses on hash change.
- **In-place id edits** are handled by purging the entity row that claims the same `source_path` under a different `id` before the upsert — otherwise the old id would survive until a full rebuild and the sweep would diverge from rebuild state.
- **Findings shape** mirrors Story 1.11's planned finding record (`code`, `severity`, `path`, `message`) so `qdev validate` and `qdev doctor` (1.11/1.12) can read the table directly; `SweepSummary` fields mirror 1.12's `qdev sync` counts (parsed, unchanged, purged, findings).

## Verification

**Commands:**
- `cargo test` -- expected: entire workspace suite passes, including new sweep tests and the benchmark test
- `cargo clippy --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: zero diffs

**Results (2026-09-08, after code review):**
- `cargo test --workspace` -- PASS: 20 test binaries green, zero failures, including 21/21 `sweep_tests`.
- `cargo clippy --all-targets -- -D warnings` -- PASS: zero warnings.
- `cargo fmt --check` -- PASS: zero diffs.

**Results (2026-09-07):**
- `cargo test --workspace` -- PASS: all suites green, including 13/13 `sweep_tests` (warm no-op, single-modified, touch-same-content, in-place id edit, branch switch, merge-conflict, schema-violation, dirty×2, gates refresh, v1→v2, rebuild/sweep equality, benchmark) and 3/3 `sweep_cli_tests` (edits/adds/removes, findings non-fatal exit 0, lock contention exit 5); architecture tests (no forbidden deps, no terminal I/O in core) pass.
- `cargo clippy --all-targets -- -D warnings` -- PASS: zero warnings.
- `cargo fmt --check` -- PASS: zero diffs (after one `cargo fmt` pass on the new test code).
