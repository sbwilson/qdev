---
title: 'Story 3.5: Evidence Bundles'
type: 'feature'
created: '2026-09-23'
status: 'done'
baseline_commit: 'fd7e5b4d0f73c985a9dbdde884cdaef9a10218c8'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Verification records and audit trails are lost or unversioned when gate execution results only print transient terminal receipts without committing persistent, tamper-evident verification evidence per commit.
**Approach:** Implement immutable, committed JSON evidence bundles written to `docs/state/evidence/<story-or-_workspace>/<sha>-<gate>[-<N>].json` upon completion of every gate run, hydrated into the SQLite `gate_runs` table, and exposed via `qdev get story <id> --expand evidence` listing the latest run per gate.

## Boundaries & Constraints

**Always:**
- Write an immutable JSON evidence file matching `docs/compliance-and-safety.md` §4 whenever a gate run completes:
  - Directory: `<state_dir>/evidence/<story-or-_workspace>/` where `<story-or-_workspace>` is the story ID if resolved (from options or active lease), otherwise `_workspace`.
  - Filename: `<sha>-<gate>.json` where `<sha>` is the 7-character short commit SHA (falling back to `unknown` if no git commit / git unavailable). If `<sha>-<gate>.json` exists, append `-2.json`, `-3.json`, etc., without ever modifying or overwriting existing evidence files.
  - JSON payload fields:
    - `schema_version`: `"1"`
    - `gate`: gate ID string
    - `story`: optional story ID (`null` if workspace-level)
    - `commit`: 7-character short commit SHA string (or `unknown`)
    - `status`: `"pass"` | `"fail"` | `"infra"`
    - `exit_code`: integer exit code
    - `duration_ms`: duration in milliseconds
    - `metric`: numeric metric value or `null`
    - `summary`: summary string
    - `output_sha256`: SHA-256 hex digest of the gate output (stdout concatenated with stderr, or empty string hash if no output)
    - `run_by`: author attribution `{ "type": "human" | "agent", "id": "<id>" }` resolved via `resolve_author`
    - `ran_at`: ISO 8601 UTC timestamp
    - `verifies`: array of requirement/hazard IDs copied from `gate_config.verifies`
    - `skipped_locally`: boolean, true if gate has `skip = true`
- Hydrate every evidence file into the SQLite `gate_runs` table:
  - On write, insert/upsert the record into `gate_runs` if the cache database exists.
  - In `hydrate_evidence_file` (used by `rebuild_from_workspace` and `sweep_workspace`), parse evidence JSON supporting both canonical schema fields (`gate`, `story`, `commit`, `output_sha256`, `metric`) and legacy aliases (`gate_id`, `story_id`, `commit_sha`, `output_hash`, `metric_value`), recording `sync_state`.
- In `qdev get story <id> --expand evidence`:
  - Accept `evidence` as a valid `--expand` value alongside `relations`, `constraints`, and `scratch`.
  - Query `gate_runs` for the story and select the latest run per gate (using `(ran_at, id)` ordering).
  - Format output in text mode under an `evidence:` section, and in JSON mode as an `evidence` array in the story payload.
- Update `GateRunOutcome` receipt formatting for `pass` status to include `| evidence <path>` matching `compliance-and-safety.md` §2.

**Never:**
- Never modify or overwrite an existing evidence file; always increment suffix (`-2`, `-3`, ...) on rerun for the same commit.
- Never write evidence files outside the configured `storage.state_dir/evidence/` path.
- Never omit `output_sha256`, `run_by`, `verifies`, or `skipped_locally` from evidence JSON files.
- Never fail gate execution if cache database is absent or uninitialized; evidence file writing must succeed independently of SQLite cache hydration.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Gate Run with Story | Gate `c-abi-round-trip` completes for story `E12S4` on commit `8f1b2c4` | Writes `docs/state/evidence/E12S4/8f1b2c4-c-abi-round-trip.json`, hydrates into `gate_runs` | Atomic file write under workspace lock |
| Gate Run without Story | Gate `lint` completes with no active lease and no `--story` | Writes `docs/state/evidence/_workspace/<sha>-lint.json` with `"story": null` | Directory `_workspace` created if missing |
| Gate Rerun Same Commit | Gate rerun on same commit where evidence file already exists | Writes `docs/state/evidence/<target>/<sha>-<gate>-2.json`; subsequent rerun writes `-3.json` | Existing file never modified |
| Gate with verifies | Gate with `verifies = ["FR-102", "HAZ-01"]` runs | Evidence JSON carries `"verifies": ["FR-102", "HAZ-01"]` | Empty array if none configured |
| Skipped Gate Evidence | Gate with `skip = true` runs | Writes evidence with `"status": "pass"`, `"skipped_locally": true`, `"duration_ms": 0` | Hydrated with `skipped_locally = true` |
| `qdev get story --expand evidence` (Text) | Story `E12S4` has runs for `fmt` and `lint` | Displays `evidence:` section listing latest run for `fmt` and `lint` | `(none)` if no runs exist |
| `qdev get story --expand evidence` (JSON) | Story `E12S4` queried with `--expand evidence --json` | JSON envelope contains `evidence` array in story payload | Validated against `payload-story.json` |
| Multiple Runs Per Gate | Gate `lint` ran 3 times on story `E12S4` | `--expand evidence` returns only the single latest run for `lint` | Deduplicated by gate |
| Cache Hydration Sweep | `qdev sweep` or `rebuild_from_workspace` runs | Scans `docs/state/evidence/**/*.json`, upserting all gate runs into `gate_runs` | Sync state recorded per evidence file |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/schemas/evidence.json` -- Update schema to match `docs/compliance-and-safety.md` §4 evidence bundle specification.
- `crates/qdev-core/schemas/payload-story.json` -- Add `evidence` property to story payload schema for `--expand evidence`.
- `crates/qdev-core/src/schema.rs` -- Register and validate updated `evidence.json` and `payload-story.json`.
- `crates/qdev-core/src/gate/mod.rs` -- Define `EvidenceBundle` model, serialization, receipt formatting with evidence path.
- `crates/qdev-core/src/gate/runner.rs` -- Integrate evidence bundle generation into `execute_gate`: path resolution, immutable `-2` collision handling, `output_sha256` hashing, cache upsert.
- `crates/qdev-core/src/store/sqlite.rs` -- Update `hydrate_evidence_file` to support schema fields (`gate`, `story`, `commit`, `output_sha256`, `metric`, `verifies`, `skipped_locally`) and add query helper for story gate runs.
- `crates/qdev-core/src/query.rs` -- Add `expand_evidence` to `QueryOptions`, add `evidence: Option<Vec<EvidenceProjection>>` to `EntityProjection`, query latest run per gate for stories.
- `crates/qdev-cli/src/main.rs` -- Accept `evidence` in `--expand`, pass `expand_evidence`, render `evidence:` in `render_get_entity_text`.
- `crates/qdev-core/tests/evidence_tests.rs` -- Unit tests for evidence writing, filename collision suffixing, output hashing, schema validation, and cache hydration.
- `crates/qdev-cli/tests/evidence_cli_tests.rs` -- CLI integration tests for `qdev gate run`, evidence file creation, and `qdev get story --expand evidence` in text and JSON.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/schemas/evidence.json` -- Update JSON schema for evidence files matching compliance & safety §4 -- Contract definition.
- [x] `crates/qdev-core/schemas/payload-story.json` -- Add `evidence` array property to story payload schema -- Schema extension.
- [x] `crates/qdev-core/src/gate/mod.rs` -- Define `EvidenceBundle` struct, path resolution helpers, and updated receipt formatting -- Evidence data models.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Implement evidence file writing in `execute_gate`, short SHA derivation, `-N` collision resolution, and store hydration -- Core runner integration.
- [x] `crates/qdev-core/src/store/sqlite.rs` -- Enhance `hydrate_evidence_file` to parse evidence JSON properties and add `get_gate_runs_for_story` -- SQLite store integration.
- [x] `crates/qdev-core/src/query.rs` -- Add `expand_evidence` option and `EvidenceProjection` to `EntityProjection` -- Query engine expansion.
- [x] `crates/qdev-cli/src/main.rs` -- Enable `--expand evidence` validation and render evidence text section in `qdev get` -- CLI handler integration.
- [x] `crates/qdev-core/tests/evidence_tests.rs` -- Comprehensive unit tests for evidence file creation, collision numbering, output hashing, and hydration -- Verification.
- [x] `crates/qdev-cli/tests/evidence_cli_tests.rs` -- Integration tests for `qdev gate run` evidence generation and `qdev get story --expand evidence` -- Verification.

**Acceptance Criteria:**
- Given any gate run, when it completes, `docs/state/evidence/<story-or-_workspace>/<sha>-<gate>.json` is written matching the schema in `docs/compliance-and-safety.md` §4, including `output_sha256`, `run_by`, `verifies` copied from gate config, and `skipped_locally`.
- The record is hydrated into `gate_runs` in SQLite cache.
- `qdev get story <id> --expand evidence` lists the latest run per gate in text and JSON mode.
- Evidence files are never modified after creation; reruns on the same commit append numerical suffixes (`-2`, `-3`, ...).

## Implementation Notes

- Updated `crates/qdev-core/schemas/evidence.json` to define the JSON contract for evidence bundles according to `docs/compliance-and-safety.md` §4 (`schema_version`, `gate`, `story`, `commit`, `status`, `exit_code`, `duration_ms`, `metric`, `summary`, `output_sha256`, `run_by`, `ran_at`, `verifies`, `skipped_locally`).
- Extended `crates/qdev-core/schemas/payload-story.json` with the optional `evidence` array property describing gate run evidence records.
- Added `EvidenceBundle` and `EvidenceProjection` data models in `crates/qdev-core/src/gate/mod.rs` and `crates/qdev-core/src/query.rs`.
- Updated `GateRunOutcome.receipt()` to append `| evidence <path>` for `pass` runs conforming to `docs/compliance-and-safety.md` §2.
- Integrated evidence bundle generation into `execute_gate` in `crates/qdev-core/src/gate/runner.rs`:
  - Determines storage directory (`<state_dir>/evidence/<story_or__workspace>/`).
  - Resolves short 7-char commit SHA from HEAD (or `unknown`).
  - Performs immutable collision resolution: `<sha>-<gate>.json`, then `<sha>-<gate>-2.json`, `-3.json`, etc.
  - Computes `output_sha256` as SHA-256 hex digest of combined stdout and stderr (or empty hash if no output).
  - Copies `verifies` requirement tags from gate configuration.
  - Resolves author attribution via `resolve_author`.
  - Atomically writes JSON file through `write_file_atomic`.
  - Hydrates record directly into SQLite `gate_runs` table when cache database exists.
- Enhanced `hydrate_evidence_file` in `crates/qdev-core/src/store/sqlite.rs` to support both canonical schema keys (`gate`, `story`, `commit`, `output_sha256`, `metric`) and legacy aliases (`gate_id`, `story_id`, `commit_sha`, `output_hash`, `metric_value`), and added `get_gate_runs_for_story` helper.
- Added `expand_evidence` to `QueryOptions` in `crates/qdev-core/src/query.rs`, selecting and deduplicating the latest run per gate ordered by `(ran_at, id)`.
- Updated `crates/qdev-cli/src/main.rs` to validate `evidence` under `--expand`, pass `expand_evidence` into `QueryOptions`, and format evidence output in text mode under `evidence:`.
- Authored comprehensive test coverage:
  - `crates/qdev-core/tests/evidence_tests.rs`: 12 unit tests covering model validation, workspace-level output, collision suffixing, legacy hydration, skipped locally runs, query deduplication, and file generation.
  - `crates/qdev-cli/tests/evidence_cli_tests.rs`: 6 integration tests covering `qdev gate run` evidence writing, receipt formatting, collision suffixing, and `qdev get story --expand evidence` in text and JSON modes.
- Verified all workspace test suites pass cleanly with zero failures and zero clippy warnings.

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---------|----------|---------|----------------------|-------------|
| Primary key collision in SQLite `gate_runs` across stories and workspace | blind-hunter | `high` | `<sha>-<gate>` ID stem collisions overwrite rows across stories or workspace | patch |
| Missing `verifies`, `skipped_locally`, and `run_by` in `EvidenceProjection` and `payload-story.json` | blind-hunter | `medium` | CLI JSON envelope omitted core evidence bundle fields from projection | patch |
| `ORDER BY ran_at DESC` in `get_gate_runs_for_story` selects `NULL` timestamps over newer runs | blind-hunter | `medium` | SQLite orders NULL first in DESC sorts, which would shadow actual latest runs | patch |
| Evidence bundle output hashing is asserted only by string length | verification-gap | `medium` | Missing test asserting exact SHA-256 digest over stdout and stderr | patch |
| Gate run failure and infrastructure failure evidence generation has no test coverage | verification-gap | `medium` | Missing tests executing failing or infra error gates to verify evidence file status and exit code | patch |
| Golden fixture validation bypassed for `EntityKind::Evidence` leaving schema attribution unpinned | verification-gap | `medium` | Golden fixture test bypassed EntityKind::Evidence without validating invalid attribution rejection | patch |
| Unsanitized `story_id` can cause incorrect directories or path traversal | edge-case-hunter / blind-hunter | `medium` | Empty/whitespace or slash-bearing story_id leads to invalid paths | patch |
| Gate result document produces an empty summary string | edge-case-hunter | `medium` | Empty summary fails minLength schema validation | patch |
| Incorrect cache path in `evidence_tests.rs` leaves live SQLite hydration unverified | blind-hunter | `medium` | Tests pointed to `.cache/qdev` instead of `.qdev/cache` | patch |
| `write_evidence_bundle` does not validate bundles before writing | blind-hunter | `low` | EvidenceBundle was not validated against constraints prior to disk write | patch |
| Empty summary causes column shift in `render_get_entity_text` | blind-hunter | `low` | Column layout shifted when summary was empty | patch |
| `receipt()` formats empty commit SHA when `commit_sha` is `Some("")` | blind-hunter | `low` | Empty string commit_sha fell through instead of using "unknown" | patch |
| `crates/qdev-cli/src/cli.rs` omits `--expand evidence` documentation | blind-hunter | `low` | Clap docstring omitted `evidence` from `--expand` description | patch |
| Missing database index on `gate_runs(story_id)` | blind-hunter | `low` | Index optimizes `get_gate_runs_for_story` queries | patch |
| Conflicting title in `crates/qdev-core/schemas/evidence.json` | blind-hunter | `false` | Title intentionally matches existing test in `schema_cli_tests.rs` | reject |
| Dependency-skipped gates in `execute_gate_set` produce no evidence bundle | blind-hunter | `false` | Spec only requires evidence for executed or locally-skipped gates, not aborted deps | reject |
| `sprint-status.yaml` status mismatch for Story 3.5 | blind-hunter | `false` | Status transitions to review at step-05 per BMAD workflow | reject |


## Verification

**Commands:**
- `cargo test --test evidence_tests` -- expected: All evidence unit tests pass.
- `cargo test --test evidence_cli_tests` -- expected: All evidence CLI integration tests pass.
- `cargo test --test gate_runner_tests` -- expected: Existing gate runner tests pass.
- `cargo test --test get_cli_tests` -- expected: Existing get CLI tests pass.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero clippy warnings.
