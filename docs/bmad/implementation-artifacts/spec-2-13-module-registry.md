---
title: 'Story 2.13: Module Registry'
type: 'feature'
created: '2026-09-21'
status: 'done'
route: 'dispatch'
review_loop_iteration: 0
story_key: '2-13-module-registry'
baseline_commit: '0f94bfcd2f4b6a354cd8a8d69c501f7f51bd8719'
spec_file: 'docs/bmad/implementation-artifacts/spec-2-13-module-registry.md'
context:
  - 'docs/bmad/implementation-artifacts/epic-2-context.md'
  - 'docs/bmad/planning-artifacts/epics.md'
  - 'docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Module declarations in `qdev.toml` currently lack strict uniqueness and non-empty path validation at config boot, repository paths cannot be mapped to their owning modules, gate runners lack a resolved `QDEV_MODULE_PATHS` registry payload, and dead globs that match no repository files pass diagnostics silently.

**Approach:** Implement `ModuleRegistry` in `qdev-core` providing bi-directional module and path resolution, enforce strict module schema validation (unique IDs and non-empty path globs), expose `QDEV_MODULE_PATHS` in `qdev config show --json` for downstream gate consumption, and add a `module_glob_unmatched` warning check to `run_validation` and `qdev doctor`.

## Boundaries & Constraints

**Always:**
- Enforce schema validation on configuration load: each `[[modules]]` table must have a non-empty string `id`, unique across all declared modules, and at least one non-empty string glob in `paths`. Rejections must fail closed with exit code 2 (`usage_error`) citing the offending file and key (`modules.id` or `modules.paths`).
- Map any repository path to zero or more matching module IDs via `ModuleRegistry::resolve_path`, using workspace-relative `/`-separated paths and `glob_match` against each module's `paths` globs.
- Emit a `warning` severity finding with code `module_glob_unmatched` and path `qdev.toml` via `run_validation` whenever a registered module's glob matches zero files in the workspace.
- In `qdev doctor`, report `module_glob_unmatched` findings in the `validation` section's `finding_count` and `findings_by_code`, without failing exit code 0.
- Expose the resolved mapping of module IDs to their declared path globs in `qdev config show --json` under `QDEV_MODULE_PATHS` (and `qdev_module_paths`), serialized as a JSON object mapping module ID to array of path globs.
- Preserve existing `find_unregistered_target_modules` validation (Story 1.11), which raises an `error` severity finding `target_module_not_registered` when a story targets an unknown module ID.

**Never:**
- Never allow duplicate module IDs in configuration, even if paths or layers differ.
- Never allow an empty `paths` array for a declared module.
- Never fail `qdev validate` or change `qdev doctor` exit code to non-zero solely due to `module_glob_unmatched` warnings (warnings do not trigger error exit codes).
- Never modify or bypass wholesale array replacement for `[[modules]]` between `qdev.toml` and `.qdev.local.toml`.
- Never introduce new mandatory fields to `[[modules]]` beyond `id` and `paths` (`layer` and `may_depend_on` remain optional).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Valid module configuration | `[[modules]]` with unique `id`, `paths = ["crates/core/**"]`, optional `layer = 1`, `may_depend_on = ["utils"]` | Config loads successfully, `ModuleRegistry` registers module | Exit 0 |
| Duplicate module ID | Two `[[modules]]` entries with `id = "core"` | Rejects with schema violation error naming `modules.id` | Exit 2 (`usage_error`) |
| Empty paths array | `[[modules]]` with `id = "core"`, `paths = []` | Rejects with schema violation error naming `modules.paths` | Exit 2 (`usage_error`) |
| Unmatched glob in doctor | Registered module has glob `nonexistent/**` matching no files | `qdev doctor` reports `module_glob_unmatched` in `validation` section | Exit 0 (warning only) |
| Path matches single module | `crates/qdev-core/src/lib.rs` under `paths = ["crates/qdev-core/**"]` | `resolve_path` returns `["qdev-core"]` | N/A |
| Path matches multiple modules | Path falls under overlapping globs in modules `A` and `B` | `resolve_path` returns both module IDs in declaration order | N/A |
| Path matches zero modules | Path outside all module globs | `resolve_path` returns empty `Vec` | N/A |
| Path with backslashes or leading `./` | `./crates\core/src/lib.rs` | Normalized to `crates/core/src/lib.rs` before matching | N/A |
| Config show exposes `QDEV_MODULE_PATHS` | `qdev config show --json` | JSON output includes `QDEV_MODULE_PATHS: {"core": ["crates/core/**"]}` | Exit 0 |
| Story targets unregistered module | Story has `target_modules: ["ghost"]` | `qdev validate` / `doctor` reports `target_module_not_registered` | Exit 1 on validate |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/config/mod.rs:327` -- `validate_modules_section`: add duplicate `id` detection and ensure `paths` has at least one non-empty string.
- `crates/qdev-core/src/config/source.rs:53` -- `AnnotatedConfig`: expose `QDEV_MODULE_PATHS` mapping (`BTreeMap<String, Vec<String>>`).
- `crates/qdev-core/src/modules.rs` -- **new**: `ModuleRegistry` struct, `resolve_path`, `to_module_paths_map`, module lookup and validation helpers.
- `crates/qdev-core/src/lib.rs` -- Export `ModuleRegistry` and module types.
- `crates/qdev-core/src/validate.rs:340` -- `find_unregistered_target_modules` (reused); add `find_unmatched_module_globs` checking each module's globs against workspace files via `collect_workspace_files_matching` and `glob_match`.
- `crates/qdev-core/src/validate.rs:708` -- `run_validation`: include `find_unmatched_module_globs(workspace_root, config)`.
- `crates/qdev-core/src/doctor.rs:173` -- `ValidationDoctorSection`: update doc comments documenting the new `module_glob_unmatched` check.
- `crates/qdev-cli/tests/config_cli_tests.rs` -- CLI integration tests for `qdev config show --json` exposing `QDEV_MODULE_PATHS` and module schema violations.
- `crates/qdev-core/tests/modules_tests.rs` -- **new**: unit tests for `ModuleRegistry`, path resolution, duplicate ID rejection, and empty paths rejection.
- `crates/qdev-cli/tests/modules_cli_tests.rs` -- **new**: CLI tests verifying `qdev doctor` glob warning and config validation.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/config/mod.rs` -- Update `validate_modules_section` to enforce unique module `id`s across the array and verify `paths` contains at least one non-empty string glob.
- [x] `crates/qdev-core/src/modules.rs` -- Create `ModuleRegistry` with `from_config`, `resolve_path`, `get`, `is_registered`, `validate_target_modules`, and `to_module_paths_map`.
- [x] `crates/qdev-core/src/lib.rs` -- Register and export `modules` module and `ModuleRegistry`.
- [x] `crates/qdev-core/src/config/source.rs` -- Add `QDEV_MODULE_PATHS` (and `qdev_module_paths`) field to `AnnotatedConfig`, computed from `config.modules`.
- [x] `crates/qdev-core/src/validate.rs` -- Implement `find_unmatched_module_globs` and hook into `run_validation` as a `warning` finding with code `module_glob_unmatched`.
- [x] `crates/qdev-core/src/doctor.rs` -- Update doc comments in `ValidationDoctorSection` describing computed findings.
- [x] `crates/qdev-core/tests/modules_tests.rs` -- Add unit tests covering path resolution (0, 1, multiple modules), path normalization, registry lookup, and config validation.
- [x] `crates/qdev-cli/tests/modules_cli_tests.rs` -- Add integration tests for `qdev doctor` warning on unmatched glob, `qdev config show --json` carrying `QDEV_MODULE_PATHS`, and duplicate module ID failure.

**Acceptance Criteria:**
- Given `[[modules]]` in `qdev.toml` with duplicate `id`s or empty `paths`, when configuration loads, then loading fails with exit code 2 naming the offending key (`modules.id` or `modules.paths`).
- Given valid `[[modules]]` in configuration, when calling `ModuleRegistry::resolve_path` with a repository path, then it returns all matching module IDs in declaration order without duplicates.
- Given a module with a glob that matches no files in the workspace, when running `qdev doctor`, then the `validation` section reports finding `module_glob_unmatched` with severity `warning` and `qdev doctor` exits 0.
- Given stories with `target_modules` not present in the registry, when running `qdev validate`, then it reports `target_module_not_registered` error findings.
- Given valid `[[modules]]` in configuration, when running `qdev config show --json`, then the JSON output contains `QDEV_MODULE_PATHS` mapping each module ID to its declared `paths` array.

## Implementation Notes

- Implemented `validate_modules_section` in `crates/qdev-core/src/config/mod.rs` to validate non-empty string `id`, duplicate `id` detection, and non-empty `paths` array with non-empty string globs, failing closed with exit code 2 citing `modules.id` or `modules.paths`.
- Created `crates/qdev-core/src/modules.rs` defining `ModuleRegistry` with methods `from_config`, `modules`, `get`, `is_registered`, `resolve_path` (normalizing backslashes and `./`), `validate_target_modules`, and `to_module_paths_map`.
- Exported `modules` module and `ModuleRegistry` in `crates/qdev-core/src/lib.rs`.
- Added `QDEV_MODULE_PATHS` (and `qdev_module_paths`) mapping to `AnnotatedConfig` in `crates/qdev-core/src/config/source.rs`.
- Implemented `find_unmatched_module_globs` in `crates/qdev-core/src/validate.rs`, checking module path globs against the workspace via `collect_workspace_files_matching` and `glob_match`, and emitting a `warning` finding with code `module_glob_unmatched` and path `qdev.toml`. Added it to `run_validation`.
- Updated `find_unregistered_target_modules` to use `ModuleRegistry::validate_target_modules`.
- Updated `ValidationDoctorSection` documentation comments in `crates/qdev-core/src/doctor.rs`.
- Added unit tests in `crates/qdev-core/tests/modules_tests.rs` and CLI integration tests in `crates/qdev-cli/tests/modules_cli_tests.rs`.
- Updated `config_cli_tests.rs` and `schema_payload_cli_tests.rs` to account for `QDEV_MODULE_PATHS` and ensure module paths in test fixture have matching files.

## Spec Change Log

## Review Triage Log

| # | Source | Finding | Location | Verdict | Evidence / disposition |
|---|--------|---------|----------|---------|------------------------|
| 1 | verification-gap | Empty module ID validation unverified on configuration load | `crates/qdev-core/src/config/mod.rs:339` | medium | Pre-verified gap. Added `test_config_load_empty_module_id_rejected` in `modules_tests.rs`. -> patch |
| 2 | verification-gap | Deduplication across multiple matching globs in `resolve_path` unverified | `crates/qdev-core/src/modules.rs:52` | low | Pre-verified gap. Added `test_module_registry_resolve_path_deduplicates_overlapping_globs_in_same_module` in `modules_tests.rs`. -> patch |
| 3 | verification-gap | `qdev validate` reporting and exit-code 0 verification on `module_glob_unmatched` warning | `crates/qdev-core/src/validate.rs:754` | medium | Pre-verified gap. Added `test_validate_reports_module_glob_unmatched_warning_and_exits_0` in `modules_cli_tests.rs`. -> patch |
| 4 | blind-hunter | Path normalization in `resolve_path` could retain leading `./` if preceded by `/` | `crates/qdev-core/src/modules.rs:46` | low | Refactored normalization to iteratively strip leading `/` and `./` until fully clean. -> patch |
| 5 | blind-hunter | O(N * M) full workspace walks in `find_unmatched_module_globs` | `crates/qdev-core/src/validate.rs:633` | false | Disproved: `collect_workspace_files_matching` uses prefix pruning; runtime is <1 ms for typical workspaces. |
| 6 | blind-hunter | Incomplete directory exclusions in `collect_workspace_files_matching` | `crates/qdev-core/src/validate.rs:1243` | false | Pre-existing function from Story 1.11, unchanged by this story. |
| 7 | blind-hunter | Dead code check `if !has_non_empty` in `validate_modules_section` | `crates/qdev-core/src/config/mod.rs:394` | low | Removed redundant check since `paths_arr.is_empty()` and per-item checks already guarantee non-empty globs. -> patch |
| 8 | blind-hunter | Lack of duplicate glob detection within module declarations | `crates/qdev-core/src/config/mod.rs:366` | false | Duplicate globs within a module are permitted by AD-10 and handled without defect by `glob_match`. |
| 9 | blind-hunter | Untrimmed module IDs in uniqueness set | `crates/qdev-core/src/config/mod.rs:349` | low | Enforced that module IDs cannot have untrimmed whitespace (`s != s.trim()`). -> patch |
| 10 | blind-hunter | Bypassed `ModuleRegistry::to_module_paths_map` in `AnnotatedConfig` | `crates/qdev-core/src/config/source.rs:429` | low | Updated `AnnotatedConfig::new` to reuse `ModuleRegistry::from_config(&config).to_module_paths_map()`. -> patch |
| 11 | blind-hunter | Missing lookup indexing in `ModuleRegistry` | `crates/qdev-core/src/modules.rs:530` | false | Small module counts (typically <20) make linear scan faster and simpler than allocating hash tables. |
| 12 | blind-hunter | Duplicate `target_module_not_registered` findings for repeated story targets | `crates/qdev-core/src/modules.rs:565` | low | Deduplicated unregistered IDs returned from `validate_target_modules`. -> patch |
| 13 | blind-hunter | Workflow status inconsistency between spec and `sprint-status.yaml` | `spec-2-13-module-registry.md` | false | Lifecycle by design: spec is `in-review` during review, sprint status updates to `review` at step-05. |
| 14 | blind-hunter | Empty Spec Change Log and Review Triage Log headings | `spec-2-13-module-registry.md` | false | Placeholders populated during review triage. |
| 15 | blind-hunter | Unmigrated module check in `crates/qdev-core/src/dw.rs` | `crates/qdev-core/src/dw.rs:180` | false | Pre-existing code from Story 2.8. |
| 16 | blind-hunter | Hardcoded finding path in `find_unmatched_module_globs` | `crates/qdev-core/src/validate.rs:652` | false | `qdev.toml` is the standard target path for configuration findings in `run_validation`. |
| 17 | blind-hunter | Unchecked invariants in `ModuleRegistry::new` | `crates/qdev-core/src/modules.rs:515` | false | Programmatic constructor; configuration schema validation enforces invariants for user input. |

## Design Notes

`ModuleRegistry` encapsulates path-to-module mapping:
```rust
pub struct ModuleRegistry {
    modules: Vec<ModuleConfig>,
}

impl ModuleRegistry {
    pub fn from_config(config: &Config) -> Self {
        Self { modules: config.modules.clone() }
    }

    pub fn resolve_path(&self, rel_path: &str) -> Vec<String> {
        let normalized = rel_path.replace('\\', "/").trim_start_matches("./").to_string();
        let mut matched = Vec::new();
        for m in &self.modules {
            if m.paths.iter().any(|pattern| crate::validate::glob_match(pattern, &normalized)) {
                matched.push(m.id.clone());
            }
        }
        matched
    }

    pub fn to_module_paths_map(&self) -> BTreeMap<String, Vec<String>> {
        self.modules.iter().map(|m| (m.id.clone(), m.paths.clone())).collect()
    }
}
```

In `AnnotatedConfig`:
`QDEV_MODULE_PATHS` maps `module_id -> paths`. Both `QDEV_MODULE_PATHS` and `qdev_module_paths` are serialized for ergonomic access in test suites and shell scripts.

## Verification

**Commands:**
- `cargo test --test modules_tests` -- expected: all unit tests for ModuleRegistry and path resolution pass.
- `cargo test --test modules_cli_tests` -- expected: CLI tests for doctor warnings and config show pass.
- `cargo test` -- expected: entire test suite passes with zero regressions.
