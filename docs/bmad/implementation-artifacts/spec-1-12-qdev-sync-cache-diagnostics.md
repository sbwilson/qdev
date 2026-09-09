---
title: 'qdev sync & Cache Diagnostics'
type: 'feature'
created: '2026-09-09'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
context: []
baseline_commit: 'e4f6ebdd18037edd952bd1ece2ad75532a5db91a'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** There is no way to force a hydration sweep, rebuild the cache from scratch, or inspect cache health. Developers can only recover from a corrupt or stale cache by manual file deletion, and have no visibility into schema version, entity count, sync freshness, or finding counts.

**Approach:** Add `qdev sync` (optionally `--rebuild`) to explicitly run/rebuild the incremental hydration sweep and print its counts, and `qdev doctor` to report structured diagnostics through a small extensible section registry in `qdev-core`, starting with one cache section.

## Boundaries & Constraints

**Always:** Reuse `Store::sweep_workspace`/`SqliteStore::reset_and_rebuild` rather than re-implementing hydration; both `sync` and `doctor` support `--json` with the existing envelope/exit-code conventions; the cache stays fully rebuildable from Markdown with zero data loss.

**Never:** No new migrations mechanism — new cache state is added as a `CREATE TABLE IF NOT EXISTS` addition to the existing v2 DDL, detected via the existing table-presence mismatch check (same mechanism that already handles adding `findings` without a version bump). No hygiene/gate/lease doctor sections yet — only the cache section ships now; the registry itself is the deliverable for extension.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Plain sync | Cache has 1 unchanged, 1 modified file | `parsed=1, unchanged=1, purged=0, findings=N` | N/A |
| Rebuild | `qdev sync --rebuild` on any cache state | Cache dropped + recreated from disk; counts reflect the full re-parse | N/A |
| Uninitialized workspace | No `qdev.toml` | `qdev sync`/`qdev doctor` refuse with existing usage error | exit 2 |
| Doctor healthy | Valid schema, N entities, M findings | Cache section reports schema version, entity count, `last_synced_at`, finding count | N/A |
| Doctor no prior sync | Freshly rebuilt cache, `sync_meta` empty | `last_synced_at` reported as `null`/absent, not an error | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/store/sqlite.rs:26-42` `ALL_TABLE_NAMES` + `SCHEMA_V2_DDL` (~44) -- add `sync_meta(id INTEGER PRIMARY KEY CHECK (id=1), last_synced_at TEXT NOT NULL)` as a new `CREATE TABLE IF NOT EXISTS`; add `"sync_meta"` to `ALL_TABLE_NAMES` so `inspect_cache_schema`'s existing table-count/presence check (sqlite.rs:3303-3356) flags old caches as `Mismatch` and triggers the existing auto-rebuild path in `ensure_cache` -- no version bump, mirrors how `findings` was previously added.
- `crates/qdev-core/src/store/sqlite.rs:2771-3120` `sweep_workspace` -- upsert `sync_meta` (via `write::current_iso8601()`) inside the existing transaction, right before the `findings` COUNT query (~3096), so every boot-time and explicit sweep stamps freshness.
- `crates/qdev-core/src/store/sqlite.rs:351-380,385-...` `reset_and_rebuild`/`rebuild_from_workspace` -- change both to return `Result<SweepSummary, QdevError>` (currently `Result<(), QdevError>`); `parsed` = total entity+scratch+evidence files rebuilt, `unchanged`/`purged` = 0, `findings` via the same COUNT pattern; stamp `sync_meta` inside `rebuild_from_workspace`'s own transaction. Only 3 call sites total (sqlite.rs:3423 `ensure_cache`, and 2 tests) -- update `ensure_cache` to keep discarding via `?`, tests to `.unwrap()` the summary or ignore it.
- `crates/qdev-core/src/store/mod.rs:222-233` `SweepSummary` -- reuse verbatim for both sync paths; add `get_last_synced_at(&self) -> Result<Option<String>, QdevError>` to the `Store` trait (mod.rs, near other accessors) for doctor's cache section.
- `crates/qdev-core/src/doctor.rs` (new) -- `DoctorSectionReport { name: String, fields: Vec<(String, serde_json::Value)> }` (`Vec` not `HashMap`, for deterministic JSON ordering per epic requirement); `trait DoctorSection { fn name(&self) -> &'static str; fn run(&self, store: &dyn Store) -> Result<DoctorSectionReport, QdevError>; }`; `CacheDoctorSection` reporting `schema_version` (via `inspect_cache_schema`-style pragma read or the known `CACHE_SCHEMA_VERSION` constant), `entity_count` (`list_entities(&EntityFilter::default())?.len()`), `last_synced_at`, `finding_count` (`list_findings()?.len()`); `pub fn default_doctor_sections() -> Vec<Box<dyn DoctorSection>>` returning just `[CacheDoctorSection]` for now -- later epics append their own section here.
- `crates/qdev-core/src/lib.rs` -- `pub mod doctor;` + re-export `DoctorSection`, `DoctorSectionReport`, `default_doctor_sections`.
- `crates/qdev-cli/src/cli.rs:27-53` `Commands` enum -- add `Sync(SyncArgs)` and `Doctor` variants after `Validate`; `SyncArgs { #[arg(long)] rebuild: bool }` mirroring `ValidateArgs` (cli.rs:55-69).
- `crates/qdev-cli/src/main.rs:232-239` dispatch block -- add `Commands::Sync`/`Commands::Doctor` arms calling new `handle_sync`/`handle_doctor`, following `handle_validate`'s exact shape (main.rs:1764-1859): `find_workspace_root`, `ensure_query_workspace` (main.rs:947), `open_query_store` (main.rs:933), JSON via `JsonEnvelope::new` + `output.emit_envelope`, text via a new `render_sync_text`/`render_doctor_text` next to `render_validate_text` (main.rs:1738).
- `crates/qdev-cli/tests/validate_cli_tests.rs` -- sibling pattern for new `sync_cli_tests.rs`/`doctor_cli_tests.rs`.

## Tasks & Acceptance

**Execution:** core changes first (schema, `Store` trait, `doctor.rs`), then CLI wiring, then tests.
- [x] `crates/qdev-core/src/store/sqlite.rs` -- add `sync_meta` table + `ALL_TABLE_NAMES` entry; stamp it in `sweep_workspace` and `rebuild_from_workspace`; change `reset_and_rebuild`/`rebuild_from_workspace` to return `SweepSummary`; add `get_last_synced_at`
- [x] `crates/qdev-core/src/store/mod.rs` -- add `get_last_synced_at` to the `Store` trait
- [x] `crates/qdev-core/src/doctor.rs` -- new module: `DoctorSection` trait, `DoctorSectionReport`, `CacheDoctorSection`, `default_doctor_sections`
- [x] `crates/qdev-core/src/lib.rs` -- export the above
- [x] `crates/qdev-cli/src/cli.rs` -- `Commands::Sync(SyncArgs)`, `Commands::Doctor`
- [x] `crates/qdev-cli/src/main.rs` -- dispatch + `handle_sync` (plain sweep, or `reset_and_rebuild` under `--rebuild`) + `handle_doctor` (iterate `default_doctor_sections()`), plus `render_sync_text`/`render_doctor_text`
- [x] `crates/qdev-core/tests/sweep_tests.rs` or new `doctor_tests.rs` -- unit coverage: `sync_meta` stamped after sweep and after rebuild; `get_last_synced_at` returns `None` on a freshly created (never-synced) schema
- [x] `crates/qdev-cli/tests/sync_cli_tests.rs` (new) -- `qdev sync`/`--rebuild` counts, `--json` shape, uninitialized-workspace usage error
- [x] `crates/qdev-cli/tests/doctor_cli_tests.rs` (new) -- cache section fields present, `--json` shape

**Acceptance Criteria:**
- Given any cache state, when `qdev sync --json` runs, then it reports `parsed`/`unchanged`/`purged`/`findings` matching `SweepSummary` and exits 0
- Given `qdev sync --rebuild`, then the cache file's tables are dropped and repopulated from Markdown before counts are reported
- Given a healthy cache, when `qdev doctor --json` runs, then the cache section reports `schema_version`, `entity_count`, `last_synced_at` (or `null` if never synced), and `finding_count`
- Given an uninitialized directory, when either command runs, then it exits 2 with the existing "not a qdev workspace" usage error

## Implementation Notes

- `sync_meta` added to `SCHEMA_V2_DDL`/`ALL_TABLE_NAMES` (16 tables now) with no version bump, exactly per plan: `inspect_cache_schema`'s existing table-count/presence check flags pre-existing caches missing it as `Mismatch`, triggering the existing auto-rebuild path.
- `DoctorSectionReport` serializes via a hand-written `Serialize` impl (a flat map: `name` then each `(key, value)` pair in `Vec` order) rather than deriving it, since a derived struct-of-`Vec` or a `serde_json::Map` would either nest `fields` under its own key or (without the `preserve_order` feature) alphabetize it — both wrong for a flat, insertion-ordered JSON object.
- Risk/limitation (not blocking, flagged for visibility): every `qdev` invocation in an initialized workspace already runs the boot-time `ensure_cache` sweep (spec-1-6/1-7) before command dispatch. A plain `qdev sync` therefore almost always reports `parsed=0`/everything `unchanged` in a single CLI invocation, since the boot sweep moments earlier in the same process already absorbed any on-disk changes. This is architecturally inherent, not something this spec's Code Map asked to change, and is exactly what `test_sync_settles_to_all_unchanged_once_the_boot_time_sweep_has_run` documents and asserts; the "plain sync reports non-zero parsed" I/O matrix row is instead covered directly at the `Store` level (`test_sync_reports_summary_counts_matching_sweep_summary_shape`), bypassing the boot sweep. `--rebuild` is unaffected and always deterministically reprocesses everything. A follow-up wanting literal "sync reports the delta from the user's own edits" end-to-end would need `ensure_cache` to surface its sweep summary to `handle_sync` instead of discarding it, or to skip the boot sweep specifically for the `Sync` command.
- Verified: `cargo test --workspace` (all tests pass), `cargo clippy --workspace --all-targets -- -D warnings` (zero warnings), `cargo fmt --check` (clean).

## Spec Change Log

## Review Triage Log

| # | Finding | Verdict | Evidence | Route |
|---|---------|---------|----------|-------|
| 1 | `rebuild_from_workspace`'s `parsed` count (`entity_files.len() + scratch_files.len() + evidence_files.len()`, sqlite.rs:531) is a raw directory-listing length, not a count of files actually hydrated: (a) a `fs::read_to_string` failure inside the per-file loops (`Err(_) => continue`, sqlite.rs:470-497) still leaves that file counted, unlike `sweep_workspace` which only increments `parsed` on confirmed successful hydration; (b) `qdev.toml` is parsed and upserted (sqlite.rs:459-466) but never added to the count at all, unlike `sweep_workspace`'s `SweepFileRole::Config` arm which does increment `parsed` for it | medium | Verified both asymmetries by reading `rebuild_from_workspace` (sqlite.rs:459-497, 531) against `sweep_workspace`'s per-role counting (sqlite.rs:3047-3080). `SweepSummary.parsed`'s own doc comment says "Files re-parsed and upserted this pass" — the rebuild path violates that contract on any read failure, and undercounts qdev.toml on every rebuild. Both surface through the identical `qdev sync`/`qdev sync --rebuild` JSON shape, so the same field means different things depending on which path produced it. | patch |
| 2 | No `qdev doctor` test seeds a validation finding, so the `finding_count` field's actual non-zero value path is unverified — all existing doctor tests use a workspace with zero findings | low | Verified: `doctor_cli_tests.rs`'s two JSON tests and the text test all use clean workspaces; `finding_count` is only ever asserted as present/zero, never as a specific positive count reflecting a real finding. The underlying code (`list_findings()?.len()`) is already well-exercised by `validate`, so risk is low, but the fix (one more test) is trivial. | patch |
| 3 | Neither `sync_cli_tests.rs` nor `doctor_cli_tests.rs` covers the human-readable (non-`--json`) text rendering of the uninitialized-workspace exit-2 usage error for either command — both files only assert the error shape under `--json` | low | Verified: `test_sync_uninitialized_workspace_exits_2` and `test_doctor_uninitialized_workspace_exits_2` both pass `--json`; no sibling test omits it. Trivial fix (one more assertion per command). | patch |
| 4 | `DoctorSectionReport`'s hand-written `Serialize` impl (doctor.rs:923-935) emits `name` first, then every `(key, value)` from `fields` as sibling map entries with no collision guard — a future `DoctorSection` (the exact extension point this story's registry exists for) that happens to name one of its own fields `"name"` would produce a JSON object with a duplicate `name` key | low | Verified by reading the `Serialize` impl: `map.serialize_entry("name", ...)` is unconditional, then the `fields` loop calls `map.serialize_entry(key, value)` with no check against `"name"`. Not reachable by the current `CacheDoctorSection` (its fields are `schema_version`/`entity_count`/`last_synced_at`/`finding_count`), but this is precisely the shared mechanism "later epics (gates, leases, hygiene)" (per this spec's own Design Notes) are meant to build on, so a landmine here is in this story's scope, not theirs. Fix is a trivial guard (e.g. `debug_assert!(key != "name", ...)` in the loop). | patch |
| 5 | The I/O matrix's "Doctor no prior sync" row (`last_synced_at` reported `null`) is only exercised at the `Store` level (`test_get_last_synced_at_none_on_freshly_created_schema`, using `SqliteStore::open_in_memory()`), never through the `qdev doctor` CLI | false | Verified this scenario is architecturally unreachable via the CLI: `main.rs` runs `qdev_core::ensure_cache(...)` unconditionally before any command dispatches (main.rs:151-158), and `ensure_cache` always takes either the `reset_and_rebuild` or `sweep_workspace` path — both of which now stamp `sync_meta` (per this story's own changes) before `handle_doctor` ever opens the store. There is no code path by which `qdev doctor` can observe an un-stamped cache; the Store-level test is the only test that *can* cover this row, and it does. | reject (false) |
| 6 | `CacheDoctorSection::run` (doctor.rs:954-983) makes three independent, non-atomic `Store` calls (`list_entities`, `get_last_synced_at`, `list_findings`) rather than reading one consistent snapshot; a concurrent writer between calls could make one `doctor` report mix state from before and after that write | low | Verified no transaction wraps the three calls — each goes through its own `with_conn`/lock acquisition. Real but requires an actual concurrent writer landing in the narrow window between three fast reads (further narrowed by the existing advisory write-lock), and a correct fix (a single-transaction multi-read snapshot) is a real design addition, not a direct correction. | reject (low, unlikely trigger, fix beyond a direct correction) |
| 7 | `docs/cli-reference.md` was not updated to document the new `qdev sync`/`qdev doctor` commands | false | Verified `docs/cli-reference.md:46-48` and `:365-378` already document both commands, including a `qdev doctor` example spanning Git/Modules/Gates/Hooks/Skills/MCP/Leases sections this story never claimed to implement — the doc already describes the full end-state this and later stories build toward incrementally (per epics.md's own text: "later stories extend doctor with their own sections through a registry"), so there is nothing stale to fix. | reject (false) |
| 8 | `render_doctor_text`'s flat `key = value` rendering has no visual separation tested between multiple sections | false | Verified only one section (`cache`) exists today; there is no current multi-section output to be unreadable, and no defect exists to fix — a concern about a state the code cannot yet reach. | reject (false) |
| 9 | `get_last_synced_at`'s doc comment doesn't describe a transient state during a legacy v1/pre-`sync_meta` cache's auto-rebuild | false | No code path demonstrated where `sync_meta` is queried before it exists: `inspect_cache_schema`'s table-presence check flags any cache missing `sync_meta` as `Mismatch`, and `ensure_cache` always rebuilds (which creates the table) before returning a usable store. No bad outcome shown. | reject (false) |
| 10 | `Commands::Doctor` has no args struct / forward-compat flag (e.g. a future `--section` filter) for the registry's stated future extensibility | false | This is a feature request for functionality no acceptance criterion or later-epic story currently specifies; the registry itself (the actual extension point) is present and does not require a CLI flag to be extensible. | reject (out of scope per intent) |
| 11 | Plain `qdev sync` (no `--rebuild`) almost always reports `parsed=0`/everything `unchanged` because the pre-existing boot-time `ensure_cache` sweep (spec-1-6/1-7) already absorbs on-disk changes before `handle_sync`'s own sweep runs | medium (unverified severity, pre-existing) | Real per `test_sync_settles_to_all_unchanged_once_the_boot_time_sweep_has_run` (already in the diff) and the spec's own Implementation Notes. Root cause predates this story (`ensure_cache`'s unconditional boot sweep, main.rs:151-158); a correct fix (surface the boot sweep's summary to `handle_sync`, or skip the boot sweep for `Sync`) touches shared boot dispatch order, not a direct correction scoped to this story. | defer |



`last_synced_at` is stored as an absolute ISO8601 timestamp (via the existing `write::current_iso8601`), not a precomputed relative age string — keeps the stored/JSON value deterministic and testable; any human-readable "Xm ago" rendering is computed from it at text-output time only, never persisted or asserted on in tests.

The doctor registry is intentionally minimal: a `Vec<Box<dyn DoctorSection>>` built by one function in `qdev-core`, no dynamic plugin loading. Later epics (gates, leases, hygiene) add their own `DoctorSection` impl and append it inside `default_doctor_sections`.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all unit, integration, and CLI tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
