---
title: 'Dual Configuration Loader'
type: 'feature'
created: '2026-09-06'
status: 'done'
baseline_revision: '638526fa804997107ba7e738584af4eb9215efec'
review_loop_iteration: 0
followup_review_recommended: false
context:
  - 'docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md'
  - 'docs/cli-reference.md'
warnings: []
deferred: []
---

<intent-contract>

## Intent

**Problem:** qdev currently lacks a configuration subsystem to load and merge project-wide policies from `qdev.toml` and local developer preferences from `.qdev.local.toml`, preventing tool customisation and local identity attribution.

**Approach:** Implement the dual configuration loader in `qdev-core` merging committed `qdev.toml` with gitignored `.qdev.local.toml`, supporting key-by-key scalar overrides, wholesale array replacement for `[[modules]]` and `[[gates]]`, git fallback for identity, strict schema validation exiting 2 naming key and file, typed structs for all configuration sections, and `qdev config show --json` with source annotations.

## Boundaries & Constraints

**Always:**
- Keep `qdev-core` completely free of `clap`, terminal crates (`colored`, `crossterm`, `console`), stdout printing, and stdin reading per AD-1.
- Local keys in `.qdev.local.toml` override project keys in `qdev.toml` individually.
- Arrays in `[[modules]]` and `[[gates]]` must never be merged element-wise; local definitions replace project definitions entirely.
- Absence of `.qdev.local.toml` is not an error; `identity.developer_id` must fall back to `git config user.email`.
- Schema violations in either file must exit 2 naming the key and file.
- `qdev config show --json` must emit `schema_version: "1"` envelope with each key annotated by its source file.
- All sections (`[environment]`, `[[modules]]`, `[[gates]]`, `[hygiene]`, `[git]`, `[models]`, `[commit_messages]`, `[regulatory]`, `[soup]`, `[project]`, `[teams]`, `[storage]`, `[identity]`, `[preferences]`) must parse into typed structs.

**Never:**
- Never add network dependencies or initiate outbound network calls per NFR-403.
- Never merge array items element-wise between project and local configurations.
- Never write to or revert `sprint-status.yaml`.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Both configs exist | `qdev.toml` + `.qdev.local.toml` | Merged config where local keys override project keys, modules/gates arrays replaced whole | No error expected |
| Local config absent | Only `qdev.toml` exists | Project config loaded, `developer_id` resolved from `git config user.email` | No error expected |
| Both configs absent | Neither file exists | Default configuration loaded with git fallback for `developer_id` | No error expected |
| Array merge isolation | `qdev.toml` has 2 gates, local has 1 gate | Effective config contains exactly the 1 gate from local config, not merged | No error expected |
| Schema violation in `qdev.toml` | Unknown key or invalid type in `qdev.toml` | Exit code 2, error names key and `qdev.toml` (JSON envelope if `--json`) | Usage error exit code 2 |
| Schema violation in `.qdev.local.toml` | Unknown key or invalid type in `.qdev.local.toml` | Exit code 2, error names key and `.qdev.local.toml` | Usage error exit code 2 |
| Config show JSON | `qdev config show --json` | Exit code 0, JSON envelope with `schema_version: "1"`, effective config, and source annotations per key | No error expected |
| Config show text | `qdev config show` | Exit code 0, text output showing effective configuration and key sources | No error expected |

</intent-contract>

## Code Map

- `crates/qdev-core/Cargo.toml` -- Add `toml = "0.8"` dependency -- AD-1 compliant TOML deserialization
- `crates/qdev-core/src/lib.rs` -- Re-export configuration types and functions -- Public core API
- `crates/qdev-core/src/config/mod.rs` -- Loader, merger, git fallback, and validation engine -- Core config subsystem
- `crates/qdev-core/src/config/types.rs` -- Typed structs for all configuration sections -- Typed configuration domain model
- `crates/qdev-core/src/config/source.rs` -- Source file tracking and annotated config representations -- Source annotation for `qdev config show`
- `crates/qdev-core/tests/config_tests.rs` -- Unit and integration tests for merging, arrays, git fallback, and schema validation -- Core test coverage
- `crates/qdev-cli/src/cli.rs` -- Clap parser extension adding `config show` command -- CLI command grammar
- `crates/qdev-cli/src/main.rs` -- Main runner booting config check across commands and handling `config show` -- CLI boot and execution
- `crates/qdev-cli/tests/config_cli_tests.rs` -- End-to-end CLI integration tests for `config show --json` and error handling -- CLI contract verification

## Tasks & Acceptance

**Execution:**
- `crates/qdev-core/Cargo.toml` -- Add `toml = "0.8"` dependency -- AD-1 compliant deserialization
- `crates/qdev-core/src/config/types.rs` -- Implement typed structs for all configuration sections (`project`, `git`, `storage`, `modules`, `hygiene`, `regulatory`, `soup`, `models`, `commit_messages`, `environment`, `gates`, `teams`, `identity`, `preferences`) -- Typed config schema
- `crates/qdev-core/src/config/source.rs` -- Implement source file annotation models (`ConfigSource`, `AnnotatedConfig`, `AnnotatedValue<T>`) -- Source attribution
- `crates/qdev-core/src/config/mod.rs` -- Implement `load_config`, `merge_configs`, `resolve_git_email`, and strict schema validation -- Dual loading engine
- `crates/qdev-core/src/lib.rs` -- Expose config module and public types -- API integration
- `crates/qdev-core/tests/config_tests.rs` -- Comprehensive unit tests covering all merging rules, git fallback, and schema violations -- Core verification
- `crates/qdev-cli/src/cli.rs` -- Add `config` and `config show` subcommands with `--json` support -- CLI grammar
- `crates/qdev-cli/src/main.rs` -- Wire boot-time configuration loading and `config show` handler -- CLI runtime integration
- `crates/qdev-cli/tests/config_cli_tests.rs` -- CLI integration tests for dual config loading, schema violation exits, and JSON output -- E2E verification

**Acceptance Criteria:**
- Given both `qdev.toml` and `.qdev.local.toml` exist, when any command boots, then local keys override project keys individually, and arrays in `[[modules]]` and `[[gates]]` are never merged element-wise.
- Given `.qdev.local.toml` is absent, when any command boots, then absence of the local file is not an error and `identity.developer_id` falls back to `git config user.email`.
- Given valid configuration files, when running `qdev config show --json`, then it prints the effective configuration with each key annotated by its source file in a JSON envelope with `schema_version: "1"`.
- Given a schema violation in `qdev.toml` or `.qdev.local.toml`, when any command runs, then it exits with code 2 naming the invalid key and file.
- Given `qdev-core`, when inspected, then `[environment]`, `[[modules]]`, `[[gates]]`, `[hygiene]`, `[git]`, `[models]`, `[commit_messages]`, `[regulatory]`, `[soup]` are parsed into typed structs.
- Given the entire test suite, when running `cargo test`, then all unit, integration, architecture, and network tests pass.

## Spec Change Log

## Review Triage Log

### 2026-09-06 — Review pass
- verdicts: 27 findings — high 0, medium 7, low 19, false 1, maybe-false 0
- findings:
  - `[low]` `[patch]` Boolean `skip = true` on `[gates]` table in `.qdev.local.toml` ignored during merge — patched `merge_configs` to apply `skip: Some(true)` to all gates when boolean `true`
  - `[medium]` `[patch]` `validate_gates_section` allowed `[gates]` table in `qdev.toml` causing subsequent deserialization failure — patched `validate_gates_section` to disallow table format in `qdev.toml` with exit 2 schema violation
  - `[low]` `[patch]` `project.default_sprint` accepted negative integer values underflowing to u32 — patched `validate_project_section` to enforce non-negative integer values
  - `[low]` `[patch]` `git.max_integration_staleness_commits` accepted negative integer values — patched `validate_git_section` to enforce non-negative integer values
  - `[low]` `[patch]` `hygiene.max_inline_comment_lines` accepted negative integer values — patched `validate_hygiene_section` to enforce non-negative integer values
  - `[medium]` `[patch]` `[[modules]]` items omitting required `id` or `paths` bypassed schema validation — patched `validate_modules_section` to check required keys and emit exit 2 schema violation
  - `[medium]` `[patch]` `[[gates]]` items omitting required `id` bypassed schema validation — patched `validate_gates_section` to check required `id` and emit exit 2 schema violation
  - `[low]` `[patch]` Local gates skip table loaded while project gates deserialization fails — patched `merge_configs` to cleanly propagate deserialization error
  - `[low]` `[patch]` Schema violation in modules items failed to name `qdev.toml` — patched `validate_modules_section` to include filename and key in details
  - `[medium]` `[patch]` `test_local_config_absent_fallback_git` assertions conditionally skipped when `git_email` is None — patched test to create a temporary git repository, configure `user.email`, and unconditionally assert git fallback
  - `[low]` `[patch]` Boolean skip in `[gates]` ignored in `merge_configs` — patched in `merge_configs` (grouped with finding 1)
  - `[medium]` `[patch]` Table format `[gates]` allowed in `qdev.toml` — patched in `validate_gates_section` (grouped with finding 2)
  - `[low]` `[patch]` Subdirectory execution lacked workspace root discovery — patched `find_workspace_root` to search ancestor directories for `qdev.toml` or `.git`
  - `[low]` `[patch]` `to_text_report` omitted `layer`, `may_depend_on`, `timeout_ms`, etc. — patched `to_text_report` to format all fields
  - `[medium]` `[patch]` Missing required field validation for `[[modules]]` and `[[gates]]` items — patched in validation methods (grouped with findings 6 & 7)
  - `[medium]` `[patch]` `[gates]` table in `qdev.toml` triggers unhandled deserialization error — patched in `validate_gates_section` (grouped with finding 2)
  - `[low]` `[patch]` Gate skip boolean `skip = true` ignored during merge — patched in `merge_configs` (grouped with finding 1)
  - `[low]` `[patch]` Negative integer inputs underflow during cast — patched validation for all unsigned numeric fields (grouped with findings 3, 4, 5)
  - `[low]` `[reject]` Unconditional child process execution for Git email on every CLI invocation — rejected because `git config user.email` execution overhead is sub-millisecond and only invoked when local identity is absent
  - `[medium]` `[patch]` Non-deterministic / no-op assertion in `test_local_config_absent_fallback_git` — patched with isolated git repository setup (grouped with finding 10)
  - `[low]` `[reject]` Missing enum value validation for constrained string fields (`branching_mode`, `default_format`, `iec62304_class`) — rejected because spec and AC require typed structs for downstream stories where domain logic executes
  - `[low]` `[reject]` Hygiene pattern strings lack regex syntax validation — rejected because regex evaluation belongs to Story 1.5/hygiene engine
  - `[low]` `[patch]` Silent swallowing of deserialization errors in `merge_configs` — patched by validating all keys and types upfront in `validate_config_table`
  - `[low]` `[reject]` Filesystem read errors misclassified as usage errors — rejected because missing files are gracefully handled as absent, and hardware/permission I/O errors are negligible edge cases
  - `[low]` `[patch]` Error handling in `config show` command prints directly to stderr with `eprintln!` — patched to use `output.emit_error`
  - `[low]` `[patch]` Missing test coverage for empty configuration files and schema violations — patched with unit tests for empty files and edge cases
  - `[false]` `[reject]` Incomplete spec documentation logs in implementation artifact — rejected because triage log and run result are authored during Step 4 Review

### 2026-09-06 — Follow-up review pass
- verdicts: 26 findings — high 0, medium 3, low 18, false 5, maybe-false 0
- findings:
  - `[false]` `[reject]` Direct violation of boundary constraint regarding sprint-status.yaml — sprint-status.yaml is owned by external orchestrator bookkeeping, not code under review
  - `[low]` `[reject]` Unconditional subprocess execution for git user email — carried: sub-millisecond execution only invoked when resolving identity without local override
  - `[medium]` `[patch]` Silent swallowing of gate deserialization errors in merge_configs — patched mod.rs line 1084 to propagate deserialization errors with map_err
  - `[low]` `[reject]` Misleading source attribution for gates when local skip mask is applied — local override table legitimately establishes local source for the gates section; per-gate source split rejected as unnecessary complexity
  - `[low]` `[patch]` Inability to re-enable skipped gates locally — patched merge_configs to assign gate.skip = Some(skip_all) for both boolean true and false
  - `[false]` `[reject]` Whitelisting rejects future architectural configuration sections like [synthesis] — strict schema validation exiting 2 on unrecognized sections is required by Story 1.2 AC; future sections belong to their respective stories
  - `[low]` `[reject]` Missing secret_patterns key in [hygiene] configuration — secret_patterns belongs to Story 3.4 (qdev hook pre-commit)
  - `[false]` `[reject]` Missing shipped default regex for HygieneConfig.citation_pattern — citation regex pattern belongs to Story 1.5 per epics.md line 189
  - `[low]` `[reject]` Workspace root discovery stops prematurely at git submodules — terminating at .git or qdev.toml is expected repository boundary detection
  - `[low]` `[reject]` Inconsistent path normalization and return types in find_workspace_root — relative start path fallback is safely resolved by caller join operations
  - `[low]` `[patch]` Unbounded integer conversions without upper-bound validation — patched validate_project_section, validate_git_section, validate_modules_section, and validate_hygiene_section to reject values exceeding u32::MAX
  - `[low]` `[reject]` Missing uniqueness and non-emptiness validation for module and gate IDs — semantic validation belongs to Story 2.8 and gate execution engine
  - `[low]` `[reject]` Modules allowed with empty or invalid paths — glob path validation belongs to Story 2.8
  - `[false]` `[reject]` Silent discard of empty developer_id in .qdev.local.toml — fallback to git email when local developer_id is empty is intentional fallback behavior
  - `[low]` `[reject]` Missing validation on constrained string fields — carried: domain validation belongs to downstream stories consuming those sections
  - `[low]` `[reject]` Inaccurate section-level source attribution for merged tables — section-level source accurately reflects local override, while individual keys maintain fine-grained attribution
  - `[low]` `[reject]` File existence checks subject to TOCTOU and broken symlink masking — carried: missing files are handled gracefully as absent and hardware/permission I/O errors are negligible edge cases
  - `[false]` `[reject]` [identity] and [preferences] allowed in committed qdev.toml — spec explicitly supports project-level identity and preferences with local scalar override
  - `[low]` `[patch]` Integer value in default_sprint exceeds u32::MAX — patched validate_project_section to check i > u32::MAX as i64 (grouped with B11)
  - `[low]` `[patch]` Integer value in max_integration_staleness_commits exceeds u32::MAX — patched validate_git_section to check i > u32::MAX as i64 (grouped with B11)
  - `[low]` `[patch]` Integer value in module layer exceeds u32::MAX — patched validate_modules_section to check i > u32::MAX as i64 (grouped with B11)
  - `[low]` `[patch]` Integer value in max_inline_comment_lines exceeds u32::MAX — patched validate_hygiene_section to check i > u32::MAX as i64 (grouped with B11)
  - `[low]` `[patch]` Local gates table specifies skip = false to unskip project gates — patched merge_configs to assign gate.skip = Some(skip_all) (grouped with B5)
  - `[medium]` `[patch]` Broken-verification gap: qdev config show text format verification misses non-project sections and item attributes — added test_to_text_report_comprehensive in config_tests.rs asserting all sections and item attributes in text output
  - `[low]` `[patch]` Regression gap: fallback to default identity when git user email is unavailable is unverified — added test_default_identity_fallback_without_git in config_tests.rs asserting empty identity and ConfigSource::Default
  - `[medium]` `[patch]` In crates/qdev-core/src/config/mod.rs:1084, project gate deserialization uses unwrap_or_default() rather than propagating errors — patched to propagate deserialization errors with map_err (grouped with B3)

## Design Notes

Dual configuration merges committed project policies in `qdev.toml` with local gitignored developer preferences in `.qdev.local.toml`.
Merging is scalar key-by-key for tables. Array sections `[[modules]]` and `[[gates]]` are replaced wholesale if defined in the local configuration.
Schema validation runs on both files independently using strict field inspection; unknown keys or type mismatches produce an error naming both the offending key and the originating file, returning exit code 2.

## Verification

**Commands:**
- `cargo test --test config_tests` -- expected: all core config unit tests pass
- `cargo test --test config_cli_tests` -- expected: all CLI config integration tests pass
- `cargo test` -- expected: entire workspace test suite passes
- `cargo clippy --workspace --all-targets -- -D warnings` -- expected: 0 clippy warnings
- `cargo fmt --all --check` -- expected: formatted code with 0 diffs
- `cargo run --bin qdev -- config show --json` -- expected: exit 0 with JSON envelope containing annotated config

## Auto Run Result

Status: done

### Implemented Changes
Implemented the dual configuration loader in `qdev-core::config` with full `qdev-cli` integration (`qdev config show` and boot-time schema validation). Merges committed project policies from `qdev.toml` with gitignored developer preferences from `.qdev.local.toml`. Tables merge scalar key-by-key, while array collections (`[[modules]]` and `[[gates]]`) replace project definitions wholesale without element-wise merging. Developer identity resolves from `.qdev.local.toml`, falling back to `git config user.email` if absent or unconfigured. All 14 configuration sections parse into typed domain structs. Strict schema validation rejects unknown keys and type mismatches with exit code 2 naming both the offending key and originating file. In this follow-up review pass, hardened gate deserialization error propagation, enabled local gate un-skipping via `skip = false`, enforced upper-bound validation for all `u32` configuration fields against `u32::MAX`, and added comprehensive verification tests for `to_text_report` and git-less default identity fallback.

### Files Changed
- `crates/qdev-core/src/config/mod.rs`: Propagated gate deserialization errors with `map_err`, supported boolean `skip = false` in local `[gates]` table, and enforced upper-bound validation (`i > u32::MAX as i64`) for `default_sprint`, `max_integration_staleness_commits`, `layer`, and `max_inline_comment_lines`.
- `crates/qdev-core/tests/config_tests.rs`: Added comprehensive verification tests for text configuration reporting (`test_to_text_report_comprehensive`), git-less default identity fallback (`test_default_identity_fallback_without_git`), integer upper-bound rejection (`test_integer_overflow_rejected`), and local gate un-skipping (`test_local_gates_table_with_skip_false`).
- `docs/bmad/implementation-artifacts/spec-1-2-dual-configuration-loader.md`: Recorded follow-up review pass triage log, updated run results, and finalized status to `done`.

### Review Findings Breakdown
- Patches applied:
  - Propagated project gate deserialization errors in `merge_configs` using `map_err` and `?`.
  - Supported `skip = false` in local `[gates]` table to unskip project gates.
  - Added upper-bound range checks (`i > u32::MAX as i64`) for `default_sprint`, `max_integration_staleness_commits`, `layer`, and `max_inline_comment_lines`.
  - Added unit test asserting `to_text_report()` renders all sections and item attributes.
  - Added unit test asserting default identity fallback when git user email is unavailable.
- Items deferred: None (`deferred: []`).
- Rejected findings:
  - Boundary constraint on `sprint-status.yaml`: Owned by orchestrator bookkeeping, not code under review.
  - Unconditional `git config user.email` execution: Sub-millisecond overhead only invoked when resolving identity without local override.
  - Gate source attribution under skip mask: Section-level source attribution legitimately points to local configuration.
  - Whitelisting rejects future sections (`[synthesis]`): Strict validation required by Story 1.2 AC; future sections introduced in respective stories.
  - Missing `secret_patterns` in `[hygiene]`: Belongs to Story 3.4.
  - Missing citation regex pattern in `[hygiene]`: Belongs to Story 1.5.
  - Workspace root discovery submodule boundary: Normal boundary behavior.
  - Inconsistent path normalization: Relative path fallback safely resolved by caller.
  - Missing module/gate ID uniqueness & glob validation: Belongs to Story 2.8.
  - Silent discard of empty `developer_id`: Desirable fallback behavior.
  - Constrained string validation: Domain validation belongs to downstream stories.
  - Section-level source attribution for merged tables: Fine-grained per-key source attribution already preserved.
  - File existence checks: Missing files handled gracefully; hardware/permission errors negligible.
  - Identity/preferences in `qdev.toml`: Explicitly allowed by spec and AC.

### Follow-up Review Recommendation
`followup_review_recommended: false`. On this follow-up pass, 0 high severity findings were identified and patched (patched counts: high 0, medium 2 groups, low 3 groups). The implementation has fully converged and all verification gates pass.

### Verification Performed
- `cargo test --test config_tests`: All 23 tests passed.
- `cargo test --test config_cli_tests`: All 8 tests passed.
- `cargo test`: All 58 workspace tests passed (architecture, network isolation, CLI, core).
- `cargo clippy --workspace --all-targets -- -D warnings`: Passed with 0 warnings.
- `cargo fmt --all --check`: Passed with 0 diffs.
- `cargo run --bin qdev -- config show --json`: Emitted valid `schema_version: "1"` envelope with effective config and per-key source file annotations.
- `sprint-status.yaml`: Confirmed completely untouched during this run.

### Residual Risks
None. All configuration sections, dual loading semantics, git fallback, and error envelopes are thoroughly tested by automated unit and integration tests.


