---
title: '`qdev doctor` reports the computed validation checks'
type: 'bugfix'
created: '2026-09-10'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
baseline_commit: '3827acfeb97aec141f6d20375d0bd6759ba613ee'
context: [ '{project-root}/docs/architecture.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-11-qdev-validate.md', '{project-root}/docs/bmad/implementation-artifacts/spec-1-12-qdev-sync-cache-diagnostics.md' ]
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** `qdev doctor` reports `finding_count: 0` on a workspace `qdev validate` finds defects in. Epic 1's retrospective reproduced it (finding C3): two `duplicate_planning_id` findings from `validate`, zero from `doctor`.

The cause is a split source of truth. Hydration writes its findings to the `findings` table, and `CacheDoctorSection` reads that table via `store.list_findings()` (`doctor.rs:99`). But four of the eight checks — `find_duplicate_planning_ids`, `find_orphan_deferred_work`, `find_dw_missing_rationale`, `find_unregistered_target_modules` — are computed fresh by `run_validation` and deliberately never written back (`validate.rs:219-238`). `doctor` cannot see them, so the command a user runs to ask "is my workspace healthy?" answers from half the evidence, and answers `0`.

Under-reporting is worse than not reporting: a user who runs `doctor` and reads `finding_count: 0` has been told their workspace is clean.

**Approach:** `doctor` runs the same validation `qdev validate` runs, rather than reading a table that structurally cannot hold half the answer. `run_validation` stays the single definition of "what is wrong with this workspace", and stays read-only — computed findings are not persisted, because a persisted computed finding outlives the defect it describes and would be wiped by the next sweep.

## Boundaries & Constraints

**Always:**
- `run_validation` remains the one definition of the check set, called by both `qdev validate` and `qdev doctor`. Neither command gets its own copy or its own subset.
- `run_validation` stays read-only with respect to the `findings` table. Computed findings are never persisted.
- `qdev validate`'s output, exit codes and JSON payload are unchanged — it already reports all eight checks.
- `doctor`'s existing cache fields (`cache_schema_version`, `expected_cache_schema_version`, `schema_status`, `missing_tables`, `entity_count`, `last_synced_at`) keep their names, types and meaning. The section stays serialized in its current deterministic field order, with new fields appended.
- `doctor` remains a diagnostic that survives a broken workspace: if validation cannot run, that is reported in the section rather than aborting the command, exactly as the counts already do (`doctor.rs:91-92`).
- **Decision (2026-09-10, Simon):** option C — a new `validation` section reports the total from `run_validation` with a per-code breakdown. The `cache` section is untouched, so its `finding_count` keeps meaning cache-native findings; `cli-reference.md` must say so explicitly, since that field reading `0` is what misled the reader in the first place.
- `default_doctor_sections` stays the single place sections are wired in, and stays usable by later epics adding their own sections.

**Never:**
- No new validation checks, and no change to what any existing check reports.
- No change to the `findings` table schema and no new writer of it.
- `qdev doctor` does not gain flags, and does not change its exit code because of findings — it is a report, not a gate. `qdev validate` remains the command that exits 1 on findings.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| The reproduced defect | workspace with two duplicate planning ids | `doctor` and `validate` report the same non-zero count | N/A |
| Clean workspace | no defects | both report zero; no new noise in `doctor`'s output | N/A |
| Hydration findings only | a `schema_violation` in the cache, no computed defects | counted exactly once — not double-counted from both sources | N/A |
| Both kinds present | a `schema_violation` plus an orphan DW entry | both appear; the count is their sum | N/A |
| Section order | any workspace | `cache` then `validation`, deterministic across runs | N/A |
| Broken cache | a table missing, so validation cannot complete | `schema_status: mismatch` and the finding fields report unavailable rather than aborting | N/A |
| Not a git repository | a check that shells out to git cannot run | `doctor` still completes and reports the rest | N/A |

</frozen-after-approval>


## Code Map

- `crates/qdev-core/src/doctor.rs:99` (`finding_count`) -- `store.list_findings()`, the half-answer -- the defect
- `crates/qdev-core/src/doctor.rs:60-71` (`DoctorSection` trait) -- `run(&self, store: &dyn Store)` has no workspace root and no `Config`, which `run_validation` both need. Give the section those at construction rather than widening the trait — the trait is public API that later epics implement -- key design constraint
- `crates/qdev-core/src/doctor.rs:73` (`CacheDoctorSection`), `:150-154` (`default_doctor_sections`) -- a unit struct built with no arguments, and the single wiring point; both change shape if the section needs context -- construction
- `crates/qdev-core/src/validate.rs:224-238` (`run_validation`) -- `list_findings()` plus the four computed checks, explicitly read-only; the function `doctor` must call -- the fix
- `crates/qdev-core/src/validate.rs` (`find_duplicate_planning_ids`, `find_orphan_deferred_work`, `find_dw_missing_rationale`, `find_unregistered_target_modules`) -- the four checks invisible to `doctor` today -- what becomes visible
- `crates/qdev-cli/src/main.rs:2035-2080` (`handle_doctor`) -- iterates `default_doctor_sections()` and renders; it already holds `annotated_config` and `current_dir`, so it can supply what the section needs -- the caller
- `crates/qdev-cli/src/main.rs:1946` (`render_doctor_text`) -- flat text rendering, one line per field; new fields appear here automatically -- output
- `crates/qdev-core/src/doctor.rs:30-57` (`DoctorSectionReport` serializer) -- deterministic field order with a `debug_assert` against duplicate keys; new fields must not collide -- serialization contract
- `crates/qdev-cli/tests/doctor_cli_tests.rs:70-130,250-315` -- existing assertions on the cache section's fields and on the stale-database case -- the contract to keep
- `crates/qdev-cli/tests/validate_cli_tests.rs` -- fixtures that produce each computed finding; reuse them to build the doctor test rather than inventing new ones -- verification

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/doctor.rs` -- give the section the workspace root and `Config` it needs at construction, leaving the `DoctorSection` trait signature unchanged -- context without breaking public API
- [x] `crates/qdev-core/src/doctor.rs` -- add a `validation` section reporting `run_validation`'s total plus a per-code breakdown, with a failed validation reported as unavailable rather than aborting -- the fix
- [x] `crates/qdev-core/src/doctor.rs` (`default_doctor_sections`) -- take the context and pass it through; keep it the single wiring point -- construction
- [x] `crates/qdev-cli/src/main.rs` (`handle_doctor`) -- supply the workspace root and config it already holds -- the caller
- [x] `crates/qdev-cli/tests/doctor_cli_tests.rs` -- add the matrix rows: the reproduced duplicate-id case, hydration-only findings counted once, both kinds present, and a broken cache still reporting -- verification
- [x] `docs/cli-reference.md` -- document the `validation` section, that it reports the same checks `validate` runs, that `doctor` never exits non-zero because of them, and that the `cache` section's `finding_count` means cache-native findings only -- keep docs authoritative

**Acceptance Criteria:**
- Given the retrospective's exact reproduction — a workspace with two duplicate planning ids — when `qdev doctor --json` runs, then it reports the same number of findings `qdev validate --json` reports, and that number is not zero.
- Given a workspace whose only defect is a `schema_violation` in the cache, when `doctor` runs, then that finding is counted exactly once.
- Given a clean workspace, when `doctor` runs, then the `validation` section reports zero and the `cache` section's fields are byte-identical to today's output.
- Given any workspace, when `qdev doctor --json` runs, then the sections appear in a deterministic order with `cache` first and `validation` after it, and no field key is duplicated within a section.
- Given a workspace with findings, when `qdev doctor` runs, then it exits 0 — reporting findings never changes `doctor`'s exit code.
- Given the workspace, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` are all clean.

## Implementation Notes

- `ValidationDoctorSection` (`crates/qdev-core/src/doctor.rs`) owns a `PathBuf` workspace root and
  a cloned `Config`, given at construction. `DoctorSection::run(&self, store: &dyn Store)` is
  unchanged, so nothing later epics implement breaks.
- Its fields, in serialized order: `status` (`ok` | `unavailable`), `finding_count`,
  `findings_by_code`. The breakdown is built in a `BTreeMap` and emitted as a
  `serde_json::Value::Object` (itself `BTreeMap`-backed without `preserve_order`), so the codes
  come out sorted and identical across runs. A clean workspace reports `0` and `{}`, not absent
  fields.
- A `run_validation` that returns `Err` is swallowed into `status: "unavailable"` with both other
  fields `null` — `doctor` never aborts because validation could not run, matching how the cache
  section already degrades its counts to `null`.
- No new field name collides with the cache section's, and the two sections are separate JSON
  objects, so the serializer's duplicate-key `debug_assert` is untouched. `finding_count` appears
  in both sections with related but distinct meanings — cache-native rows vs. the full validation
  total — which is exactly the ambiguity the frozen decision asks `cli-reference.md` to spell out,
  and it now does.
- `default_doctor_sections(&Path, &Config)` gained the two context arguments and stays the single
  wiring point; `handle_doctor` passes the `root` and `annotated_config.config` it already held.
  `CacheDoctorSection` stayed a unit struct — it needs no context.
- `crates/qdev-core/schemas/payload-doctor.json` was extended with the three new fields and a
  `validation`-section `required` branch, alongside the existing `cache` one, so
  `qdev schema payload doctor` still describes the real output.
- The broken-cache matrix row is covered in a new `crates/qdev-core/tests/doctor_tests.rs` rather
  than in the CLI tests: boot-time `ensure_cache` rebuilds a cache with missing tables before any
  handler opens it, so the CLI cannot observe that state (the same reasoning spec-1-12's triage
  row 5 records). `SqliteStore::open` does not create the schema, which makes the state
  constructible at store level. The other four matrix rows are CLI tests in
  `crates/qdev-cli/tests/doctor_cli_tests.rs`, each asserting `doctor`'s validation total equals
  `validate`'s own finding count on the same workspace.
- Manual verification of the retrospective's reproduction (two files declaring `E1S1`):
  `qdev validate --json` reports 2 findings and exits 1; `qdev doctor --json` reports
  `validation.finding_count: 2`, `findings_by_code: {"duplicate_planning_id": 2}`, and exits 0.
  The `cache` section still reports `finding_count: 0` on that workspace, which is correct and now
  documented — a duplicate planning id is invisible to the `findings` table by construction.

## Spec Change Log

- 2026-09-10 — Created from epic 1 retrospective action item 6 (finding C3).
- 2026-09-10 — Open Question answered by Simon: option C, a new `validation` section. Recorded in the frozen block; matrix, tasks and AC extended, including that the `cache` section's `finding_count` is documented as cache-native only.
- 2026-09-10 — Implemented. All execution tasks complete; `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` clean.

## Review Triage Log

Pass 1 (2026-09-10) — layers: blind-hunter, edge-case-hunter, verification-gap.

| # | Finding | Verdict | Route | Evidence |
|---|---------|---------|-------|----------|
| 1 | The `validation` section's workspace root and `Config` are unverified: replacing the config with `Config::default()` left every test green, because each fixture uses the default layout and runs from the root. On a configured `specs_dir`, or from a subdirectory, the duplicate-id scan walks a directory that does not exist and reports `finding_count: 0, status: ok` — the same confident zero this change removes (verification-gap, pre-verified). | high | patch | Accepted as filed. New `test_doctor_validation_honours_configured_specs_dir_and_runs_from_a_subdirectory`; re-running the reviewer's mutation now fails it. |
| 2 | `run_validation(...).ok()` discarded the error, so `status: unavailable` carried no reason anywhere in the payload, and the code comment claimed it covered "an unreadable specs directory" — which it does not (blind, edge). | medium | patch | Both confirmed. The section now emits `unavailable_reason` with the error code, and the comment says what the branch actually covers. |
| 3 | The check itself is blind, not just `doctor`: with the specs directory unreadable and duplicates present, the reviewer measured `status: ok, finding_count: 0` (edge, verified against the built binary). | medium | defer | Real and nastier than the reported symptom, but pre-existing in `find_duplicate_planning_ids` — `qdev validate` is blind identically, so the two commands still agree — and the frozen boundary forbids changing what an existing check reports. Filed against the check. |
| 4 | The shared `properties` bag in `payload-doctor.json` made `status: {enum: [ok, unavailable]}` constrain *every* future section, contradicting the schema's own promise that later epics add sections freely (blind). | medium | patch | Confirmed. Section-specific fields moved into their per-section `then` branches; only `name` stays common. |
| 5 | `cli-reference.md` claimed `doctor` "always exits 0", but `handle_doctor` still propagates a section's `Err` (blind). | medium | patch | Confirmed. Reworded to "findings never change its exit code", with the failure case stated. |
| 6 | "The same function, so the two commands cannot disagree" is overstated: `validate --changed` narrows its output, and `validate`'s exit gate keys on `error` severity while `finding_count` counts every severity (blind, edge). | medium | patch | Confirmed. Doc qualified with both differences. A `findings_by_severity` field would answer "would validate fail?" directly — filed as deferred rather than added here. |
| 7 | Schema conformance was only ever exercised on a clean workspace, so the non-empty-breakdown and all-null `unavailable` shapes were unvalidated (blind). | medium | patch | Confirmed. New `test_doctor_defective_workspace_output_matches_its_printed_schema`, and the core test now asserts the `unavailable` shape's fields. |
| 8 | The documented "code-sorted, stable across runs" guarantee was untested: no test produced two codes, and the order test's `for _ in 0..2` loop compared each run to a literal rather than to the other run (blind). | medium | patch | Confirmed. New multi-code test asserts the serialized key order and byte-identical output across two runs; the order test now compares runs to each other. |
| 9 | The order test asserted a global `finding_count` occurrence count of 2, which breaks the moment a third section is appended — the extension `default_doctor_sections` exists to allow (blind, edge). | low | patch | Confirmed. Now recovers each section's raw text by brace matching and asserts key uniqueness within each section. |
| 10 | `findings_by_code` rendered in text mode as a raw JSON blob on the value line (blind). | low | patch | Confirmed. `render_doctor_text` now gives an object one indented line per entry, and `none` when empty. |
| 11 | `read_error` was missing from the documented list of cache-native codes; the reviewer verified hydration emits it (edge). | low | patch | Confirmed. Added. |
| 12 | The "four of the eight checks" phrasing is wrong — hydration records six codes, so the set is ten (edge). | low | patch | Confirmed. Corrected in the doc comment and `cli-reference.md`; the instance inside `<frozen-after-approval>` is left for the human. |
| 13 | `cache` and `validation` read findings independently, so a concurrent write between the two sections can make `cache.finding_count` exceed `validation.finding_count` in one report (edge). | low | defer | Real but narrow: a diagnostic snapshot skew requiring a write to land mid-report. A shared snapshot is the fix, and it belongs to the section registry rather than this section. |
| 14 | `cache` spells health `schema_status` while `validation` introduces `status`; with the registry advertised as the epic 3/4 extension point, each new section will otherwise invent its own (blind, edge). | low | patch | Confirmed as a real drift risk. The convention is now stated on the `DoctorSection` trait itself, noting why `cache` keeps its published name. |
| 15 | `doctor` now costs a directory walk it did not before, unacknowledged anywhere (blind). | low | patch | Confirmed. Stated in `cli-reference.md`; a section-selection flag is the obvious follow-up if it is ever felt. |
| 16 | The frozen matrix's "Not a git repository" row is unreachable — `run_validation` shells out to git nowhere; only `validate --changed` does (blind). | false | rejected | Correct that no test covers it, but the row's stated behaviour ("doctor still completes") holds trivially. The row is inside the frozen block; flagged to the human rather than edited. |
| 17 | Spec bookkeeping: tasks never named `payload-doctor.json` or the new `doctor_tests.rs`, and the verification task said to reuse `validate_cli_tests.rs` fixtures where the diff hand-rolled its own (blind). | low | patch | Confirmed. Recorded in Implementation Notes as deviations rather than rewriting history. |

### Review pass adjustments (2026-09-10)

- Files touched beyond the task list, both justified: `crates/qdev-core/schemas/payload-doctor.json`
  (the payload schema must describe real output, and `qdev schema payload doctor` is round-trip
  tested) and a new `crates/qdev-core/tests/doctor_tests.rs` (the broken-cache matrix row is not
  constructible through the CLI, since boot rebuilds such a cache before any handler runs).
- The verification task suggested reusing `validate_cli_tests.rs` fixtures; the implementation
  hand-rolled `write_orphan_dw` / `write_schema_violation` in the doctor test file instead. Left
  as-is — they are three-line helpers and cross-test-file fixture sharing is not a pattern this
  repo uses.

## Design Notes

- **Why not persist the computed findings.** It is the other half of the retrospective's phrasing, and it is the wrong half. A row in `findings` describes a defect at hydration time; a computed check answers a question about the workspace *now*. Persisting one means it outlives the fix that resolved it until something recomputes, and the next `reset_and_rebuild` drops it anyway — so the table would be authoritative for four checks and stale for four others. Running the checks keeps one definition and no staleness.
- The trait signature is the real constraint: `DoctorSection::run(&self, store: &dyn Store)` is public API that epics 3 and 4 will implement. Widening it to carry a workspace root and config would break every future implementor for the benefit of one section, so the context belongs on the section.

## Verification

**Commands:**
- `cargo test --workspace` -- expected: all tests pass, zero failures
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: zero warnings
- `cargo fmt --check` -- expected: clean
- Manual: reproduce the retrospective's case — two files with the same planning id — then compare `qdev validate --json` and `qdev doctor --json` finding counts
