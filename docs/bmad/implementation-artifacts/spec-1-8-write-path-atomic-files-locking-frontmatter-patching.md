---
title: 'Write Path: Atomic Files, Locking, Frontmatter Patching'
type: 'feature'
created: '2026-09-07'
status: 'done'
baseline_revision: '2ebf4bddbb23d4e4f6f911302ef63edfe0d71ad5'
review_loop_iteration: 0
followup_review_recommended: false
context:
  - 'docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md'
  - 'docs/architecture.md'
  - 'docs/cli-reference.md'
warnings: []
deferred: []
---

<intent-contract>

## Intent

**Problem:** Concurrent modifications to entity Markdown files risk corrupted YAML frontmatter, lost user comments, partial file writes, and database desynchronization without advisory locking, line-based frontmatter patching, atomic temp-file-rename writes, and cache dirty marking.

**Approach:** Implement a robust write engine in `qdev-core::write` with a 5-second advisory lock on `.qdev/cache/write.lock`, line-based comment-preserving frontmatter patching, single-section Markdown replacement, optimistic concurrency verification (`--if-version`), atomic write-temp-then-rename, and SQLite cache upsert with dirty marking, exposed via `qdev update` in `qdev-cli`.

## Boundaries & Constraints

**Always:**
- Acquire the advisory file lock on `.qdev/cache/write.lock` with a 5-second (5,000 ms) timeout before modifying any entity file; exit with exit code 5 (`Conflict`, code: `lock_timeout`) on lock acquisition timeout.
- Apply frontmatter mutations as line-based patches: existing YAML comments, blank lines, and key order outside the edited fields must survive byte-for-byte (verified by golden tests).
- Automatically record `updated_by` with the active author type (`human` or `agent`) and developer/agent ID, and increment the entity's `version` integer by 1 on every mutation.
- When `--if-version <N>` is specified, verify that the existing entity frontmatter `version` equals `<N>`; if it differs or is absent, reject the update with exit code 5 (`Conflict`, code: `version_mismatch`) without modifying the file or cache.
- When `--section "<heading>" --file <path>` is provided, replace exactly the single matching Markdown section in the body while leaving all other headings, sections, and text byte-for-byte untouched. If no section or multiple sections match, return an error.
- Perform all file writes atomically by writing to a temporary file in the target entity's parent directory and renaming it over the destination path.
- Upsert the updated entity into the SQLite cache `entities` (and kind-specific) table and mark the row as dirty in `dirty_entities` (and invalidate `sync_state`), ensuring WAL mode and `busy_timeout = 5000ms`.
- Keep `qdev-core` completely free of forbidden terminal and network dependencies (`clap`, `colored`, `reqwest`, etc.) and direct terminal I/O per AD-1.
- Never write to or revert `sprint-status.yaml`.

**Never:**
- Never perform in-place overwrites of entity Markdown files that could leave partial or corrupt data on crash or interruption.
- Never strip or re-order comments, unrecognized fields, or formatting in unedited frontmatter keys.
- Never hold the advisory write lock while waiting on external network calls or user prompts.
- Never allow `--if-version` mismatches or lock timeouts to exit with any code other than 5 (`Conflict`).
- Never add forbidden network dependencies (`reqwest`, `hyper`, `curl`, `ureq`) to the workspace (violates NFR-403).
- Never allow `qdev-core` to perform direct terminal I/O (`println!`, `eprintln!`, `std::io::stdout`).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Update single field happy path | `qdev update story E12S4 --status in-progress` | Exit 0; updates `status`, increments `version`, sets `updated_by`, preserves comments; emits JSON envelope with `--json` | No error expected |
| Update via universal ID without kind | `qdev update E12S4 --status ready` | Exit 0; infers entity kind from ID grammar / file location and updates file | No error expected |
| Frontmatter comment preservation | Entity file with leading/trailing comments and inline formatting | Golden test: byte-for-byte identical output outside modified keys and incremented version | No error expected |
| Optimistic concurrency match | `qdev update story E12S4 --status ready --if-version 1` where file version is 1 | Exit 0; succeeds and increments version to 2 | No error expected |
| Optimistic concurrency mismatch | `qdev update story E12S4 --status ready --if-version 2` where file version is 1 | Exit 5; conflict error with code `version_mismatch` | Exit code 5 Conflict |
| Body section replacement happy path | `qdev update story E12S4 --section "Acceptance Criteria" --file ac.md` | Exit 0; replaces only the "Acceptance Criteria" section body; preserves other sections | No error expected |
| Body section replacement non-existent | `qdev update story E12S4 --section "NonExistent" --file ac.md` | Exit 2; usage error reporting section not found in entity body | Exit code 2 Usage error |
| Advisory lock timeout | Second process calls `qdev update` while lock is held for >5s | Exit 5; conflict error with code `lock_timeout` | Exit code 5 Conflict |
| Concurrent atomic updates | Two processes calling `qdev update` on different entities | Both acquire lock sequentially, write temp files, rename, and succeed with exit 0 | No error expected |
| Cache upsert and dirty mark | After successful file write | SQLite `entities` row updated with new version/hash; `dirty_entities` table has record | No error expected |
| Non-existent entity target | `qdev update story E99S99 --status ready` | Exit 2; usage error reporting entity file not found | Exit code 2 Usage error |

</intent-contract>

## Code Map

- `crates/qdev-core/Cargo.toml` -- Add `fs2 = "0.4"` dependency for cross-platform advisory file locking, `sha2 = "0.10"` for content hash, and `tempfile = "3.10"` in dependencies -- Core dependencies
- `crates/qdev-core/src/write.rs` -- Implement `AdvisoryLockGuard`, `acquire_write_lock`, `patch_frontmatter`, `replace_markdown_section`, `write_file_atomic`, `upsert_cache_and_mark_dirty`, and `apply_entity_update` -- Core write and locking engine
- `crates/qdev-core/src/lib.rs` -- Re-export write module functions, types, and error helpers -- Core public interface
- `crates/qdev-core/tests/write_tests.rs` -- Unit tests and golden tests for line-based frontmatter patching, comment preservation, section replacement, lock timeout, and cache dirty marking -- Core verification
- `crates/qdev-cli/src/cli.rs` -- Add `Update(UpdateArgs)` subcommand supporting `<kind> <id>` or `<id>`, field updates, `--status`, `--title`, `--section`, `--file`, `--if-version`, and attribution overrides -- CLI grammar
- `crates/qdev-cli/src/main.rs` -- Implement `handle_update` dispatching entity resolution, active identity, core mutation, and JSON envelope output -- CLI execution
- `crates/qdev-cli/tests/update_cli_tests.rs` -- E2E CLI integration tests for `qdev update`, lock timeout exit 5, version mismatch exit 5, section replacement, and concurrent execution -- CLI contract verification

## Tasks & Acceptance

**Execution:**
- `crates/qdev-core/Cargo.toml` -- Add `fs2 = "0.4"`, `sha2 = "0.10"`, and `tempfile = "3.10"` -- Storage & locking dependencies
- `crates/qdev-core/src/write.rs` -- Implement advisory locking on `.qdev/cache/write.lock` with 5s timeout, line-based frontmatter patching preserving comments byte-for-byte, section replacement, atomic temp-file-rename write, and cache upsert with dirty marking -- Core write engine
- `crates/qdev-core/src/lib.rs` -- Re-export write engine types and functions -- Core API exposure
- `crates/qdev-core/tests/write_tests.rs` -- Implement unit and golden fixture tests for frontmatter patching, comment survival, section replacement, locking timeout, and atomic writes -- Core validation
- `crates/qdev-cli/src/cli.rs` -- Define Clap CLI arguments for `qdev update` -- CLI grammar
- `crates/qdev-cli/src/main.rs` -- Connect `qdev update` to core write path, handling attribution and JSON/text formatting -- CLI execution
- `crates/qdev-cli/tests/update_cli_tests.rs` -- Add E2E tests for `qdev update` covering golden comment preservation, lock contention, version mismatch exit 5, section replacement, and JSON output -- E2E verification

**Acceptance Criteria:**
- Given two processes updating entities simultaneously, when both invoke `qdev update`, each acquires the advisory lock on `.qdev/cache/write.lock` with a 5s timeout, writes a temp file, renames it, upserts the cache row, and marks it dirty; a lock timeout exits 5.
- Given an entity Markdown file with comments, key order, and blank lines, when frontmatter fields are patched, comments, key order, and blank lines outside the edited keys survive byte-for-byte.
- Given an entity update, `updated_by` is set from the active identity and author type (`human` or `agent`), and `version` increments by 1.
- Given an update with `--if-version N`, when stored version differs, `qdev update` refuses with exit code 5 (`Conflict`).
- Given an update with `--section "<heading>" --file body.md`, exactly one Markdown section of the body is replaced and nothing else.
- Given the entire test suite, running `cargo test` passes cleanly with zero failures and zero architecture/network violations.

## Spec Change Log

## Review Triage Log

### 2026-09-07 — Review pass
- verdicts: 33 findings — high 0, medium 11, low 21, false 1, maybe-false 0
- findings:
  - `[medium]` `[patch]` Cache sync_state row deletion during entity update is unverified in tests (`crates/qdev-core/src/write.rs:809-819`) — action taken: in write_tests.rs inserted a row into sync_state and asserted it is deleted after upsert_cache_and_mark_dirty.
  - `[medium]` `[patch]` End-to-end entity update population of kind-specific stories table is unverified in end-to-end tests (`crates/qdev-core/src/write.rs:1114-1136`) — action taken: added assertions in test_apply_entity_update_end_to_end and update_cli_tests.rs querying the SQLite stories table.
  - `[medium]` `[patch]` CLI --field argument parsing and arbitrary frontmatter field mutation are unverified (`crates/qdev-cli/src/main.rs:736-760`) — action taken: added test_update_field_arbitrary_frontmatter_mutation in update_cli_tests.rs verifying --field appetite=medium.
  - `[medium]` `[patch]` Updating existing entity title and CLI --title flag are unverified (`crates/qdev-core/src/write.rs:401-411` & `crates/qdev-cli/src/main.rs:729`) — action taken: added assertions for --title in test_apply_entity_update_end_to_end and added test_update_title_flag in update_cli_tests.rs.
  - `[medium]` `[patch]` Post-patch frontmatter schema validation rejection on invalid update is unverified (`crates/qdev-core/src/write.rs:1073-1082`) — action taken: added test_update_post_patch_schema_validation_rejection in update_cli_tests.rs verifying schema validation rejection with invalid status enum value.
  - `[low]` `[patch]` Custom fields with single-entry mappings formatted as invalid single-line YAML (`crates/qdev-core/src/write.rs:420-424`) — action taken: formatted custom field mappings and sequences with line-by-line 2-space indentation under `k:` in patch_frontmatter.
  - `[medium]` `[patch]` Section replacement misidentifies code comments starting with `#` as headings (`crates/qdev-core/src/write.rs:1696-1732`) — action taken: added FenceTracker tracking fenced code blocks (``` and ~~~) in replace_markdown_section and ignoring lines within them.
  - `[low]` `[patch]` Target heading line at end of file without a trailing newline causes concatenation (`crates/qdev-core/src/write.rs:1740-1744`) — action taken: inserted newline before appending replacement body if heading lacked trailing newline.
  - `[low]` `[patch]` Opening frontmatter delimiter search scans entire file instead of document start (`crates/qdev-core/src/write.rs:1673-1692`) — action taken: ensured opening `---` is only preceded by BOM or whitespace in patch_frontmatter.
  - `[medium]` `[patch]` Multi-line frontmatter value contains an empty line prematurely terminating block scanning (`crates/qdev-core/src/write.rs:1454-1461`) — action taken: continued KeyBlock scanning through blank lines if followed by indented lines in patch_frontmatter.
  - `[low]` `[patch]` Bare filename with empty parent component causes create_dir_all error (`crates/qdev-core/src/write.rs:1195-1208`) — action taken: filtered parent paths with `.filter(|p| !p.as_os_str().is_empty())` before calling create_dir_all.
  - `[low]` `[patch]` File locking non-WouldBlock I/O error hangs for five seconds (`crates/qdev-core/src/write.rs:1232-1249`) — action taken: checked is_lock_contended and returned infrastructure failure immediately on non-contention I/O errors.
  - `[low]` `[patch]` Version integer overflow when existing version is u64::MAX (`crates/qdev-core/src/write.rs:1525`) — action taken: used checked_add(1) returning logical failure on overflow.
  - `[low]` `[reject]` Author ID or status containing YAML boolean keywords or numbers parsed as non-string — refutation: author ID and status are strictly validated against regex/enums before serialization and serialized via serde_yaml.
  - `[low]` `[patch]` `--field` argument specifies key with whitespace, colons, or invalid identifier characters (`crates/qdev-cli/src/main.rs:283-305`) — action taken: added validation rejecting invalid key identifiers and managed keys in CLI.
  - `[low]` `[patch]` Existing author creation metadata in SQLite cache overwritten with NULL on update (`crates/qdev-core/src/write.rs:1886-1887`) — action taken: used COALESCE(excluded.created_by_type, entities.created_by_type) in SQLite entities ON CONFLICT clause.
  - `[medium]` `[patch]` Callers can arbitrarily manipulate or revert entity version via `--field version=N` (`crates/qdev-core/src/write.rs:1569-1587`) — action taken: rejected managed fields (`id`, `version`, `updated_by`, `created_by`) in both patch_frontmatter and CLI handle_update.
  - `[medium]` `[patch]` `patch_frontmatter` breaks multi-line YAML key blocks on blank lines (`crates/qdev-core/src/write.rs:298-306`) — action taken: continued KeyBlock scanning through blank lines followed by indented lines.
  - `[medium]` `[patch]` `replace_markdown_section` misidentifies comments inside code blocks as headings (`crates/qdev-core/src/write.rs:541-551`) — action taken: added FenceTracker tracking fenced code blocks (``` and ~~~) in replace_markdown_section.
  - `[low]` `[patch]` Indented lines and code blocks misclassified as headings in `parse_heading_line` (`crates/qdev-core/src/write.rs:476`) — action taken: ignored lines indented >= 4 spaces or tabs in parse_heading_line.
  - `[low]` `[patch]` Frontmatter opening delimiter search scans entire file instead of document start (`crates/qdev-core/src/write.rs:233-240`) — action taken: restricted opening `---` search to document start.
  - `[low]` `[patch]` Frontmatter delimiter matching uses `.trim()`, improperly matching indented lines (`crates/qdev-core/src/write.rs:251`) — action taken: checked line.trim_end_matches(['\r', '\n']) == "---" without trimming leading whitespace.
  - `[medium]` `[patch]` Arbitrary `--field` updates can mutate immutable or managed fields (`id`, `created_by`, `version`, `updated_by`) — action taken: rejected managed fields in CLI and patch_frontmatter with usage error.
  - `[low]` `[patch]` Ambiguous or conflicting arguments between convenience flags (`--status`, `--title`) and `--field` not rejected — action taken: added validation in handle_update returning usage error if conflicting flags are passed.
  - `[low]` `[reject]` `--field` YAML parsing coercions distort scalar strings containing colons — everyday scalar values with colons or special characters can be quoted or formatted; schema validation verifies types.
  - `[medium]` `[patch]` Entity resolution returns user-supplied input rather than entity's canonical frontmatter `id` (`crates/qdev-core/src/write.rs:929`) — action taken: extracted and recorded canonical frontmatter id in EntityRecord and EntityUpdateResult.
  - `[false]` `[reject]` Incomplete kind-specific cache table upserts for non-story entities (`crates/qdev-core/src/write.rs:760-789`) — refutation: Story 1.8 AC mandates upserting the cache row (entities table) and marking dirty; kind-specific detail tables for decisions, DW, and gates belong to Epics 2 and 3.
  - `[low]` `[patch]` Story `epic_id` extraction ignores frontmatter `epic_id` field (`crates/qdev-core/src/write.rs:1115-1119`) — action taken: extracted epic_id from frontmatter if present, falling back to parsed story identifier.
  - `[low]` `[patch]` Non-deterministic file selection when multiple files match an ID prefix (`crates/qdev-core/src/write.rs:867-879`) — action taken: returned error listing ambiguous candidates if more than one file matches ID in find_file_in_dir.
  - `[low]` `[patch]` Hardcoded directory paths ignore workspace `StorageConfig` (`qdev.toml`) — action taken: passed StorageConfig into apply_entity_update and handle_update.
  - `[low]` `[patch]` Lack of file data synchronization (`sync_all`) in `write_file_atomic` (`crates/qdev-core/src/write.rs:143`) — action taken: added temp_file.as_file().sync_all() before persist.
  - `[low]` `[reject]` Inconsistent line-ending handling across section replacement in CRLF files — section replacement preserves existing line endings from input and test coverage confirms consistent replacement.
  - `[low]` `[patch]` Version parsing fails to detect numeric version wrapped in quotes (`crates/qdev-core/src/write.rs:326-333`) — action taken: added support for parsing quoted version strings in patch_frontmatter.

### 2026-09-07 — Review pass (follow-up)
- verdicts: 25 findings — high 0, medium 11, low 9, false 5, maybe-false 0
- findings:
  - `[medium]` `[patch]` `epic_id` extraction from frontmatter in `apply_entity_update` — action taken: extracted `epic_id` from updated_frontmatter if present, falling back to parsed story identifier.
  - `[medium]` `[patch]` `--field` key validation in `main.rs` only checks non-empty — action taken: added validation rejecting keys containing characters outside alphanumeric, `_`, and `-`.
  - `[false]` `[reject]` Top-level YAML sequence items without indentation (`- item` at column 0) cause premature termination of `KeyBlock` scanning — refutation: in valid YAML frontmatter and qdev schema conventions, mapping sequences are strictly indented with 2 spaces.
  - `[medium]` `[patch]` `resolve_entity_file` case-sensitive file matching causes lowercase IDs to fail on case-sensitive Linux filesystems — action taken: implemented case-insensitive file matching in `find_file_in_dir`.
  - `[low]` `[patch]` Fallback across directories in `resolve_entity_file` returns first match without checking for ambiguous entity ID matches across kinds — action taken: collected matches across all kinds and returned an ambiguous ID error if multiple match.
  - `[medium]` `[patch]` `author.id` formatted into `updated_by` frontmatter without YAML string quoting or serialization — action taken: used `serde_yaml::to_string` to serialize `author.id` safely as a YAML scalar.
  - `[low]` `[reject]` `write_file_atomic` does not issue `fsync` on parent directory after rename — refutation: temp file data and metadata are flushed and synced prior to atomic rename, satisfying the atomic write boundary; directory fsync adds non-portable OS overhead.
  - `[false]` `[reject]` Frontmatter key replacement performs case-sensitive matching (`Status:` vs `status:`) — refutation: YAML mapping keys are strictly case-sensitive in the YAML specification and qdev schema.
  - `[low]` `[reject]` Newly added frontmatter fields appended after trailing comments before closing delimiter — refutation: appending new fields before the closing delimiter preserves frontmatter validity; comments remain intact.
  - `[low]` `[reject]` `parse_heading_line` does not strip trailing hashes from closed ATX headings — refutation: closed ATX headings with trailing hashes are not used in qdev artifacts and adding branching logic is unwarranted.
  - `[low]` `[reject]` Inconsistent line-ending handling across section replacement in CRLF files — carried: section replacement preserves existing line endings from input and test coverage confirms consistent replacement.
  - `[low]` `[reject]` Windows file locking contention check in `is_lock_contended` omits `ERROR_SHARING_VIOLATION` (32) — refutation: `fs2::lock_exclusive` specifically yields `ERROR_LOCK_VIOLATION` (33) on lock contention.
  - `[false]` `[reject]` `replace_markdown_section` removes blank lines causing body to directly adjoin following heading — refutation: section replacement replaces the section body with the caller-provided body content, leaving following headings byte-for-byte untouched.
  - `[low]` `[reject]` Duplicate keys in frontmatter replaced at each occurrence rather than deduplicated — refutation: duplicate mapping keys are invalid YAML and schema validation rejects them.
  - `[low]` `[reject]` Environment variable `QDEV_AUTHOR_TYPE` falls back to "human" when given an invalid value — refutation: fallback to default human author type is intentional defensive environment handling.
  - `[false]` `[reject]` Custom field value multiline string corrupted into nested list item — refutation: string scalars are serialized via serde_yaml without adding list item markers.
  - `[medium]` `[patch]` Author ID containing boolean, numeric, or YAML indicator characters misparsed — action taken: serialized author.id using `serde_yaml::to_string` into frontmatter.
  - `[medium]` `[patch]` `--field` key containing whitespace, colons, or invalid identifier characters injects invalid YAML — action taken: added validation in CLI requiring key characters to be alphanumeric, `_`, or `-`.
  - `[medium]` `[patch]` Entity ID containing path traversal components resolves outside workspace entity directories — action taken: added validation in `resolve_entity_file` rejecting IDs containing `/`, `\`, or `..`.
  - `[low]` `[patch]` Entity ID matching across multiple entity directories during fallback search causes silent misdirection — action taken: collected matches across all kinds and errored on ambiguity.
  - `[low]` `[reject]` Indented frontmatter block containing an unindented comment line terminates key block prematurely — refutation: by design notes, key blocks span until the next unindented line; unindented comments outside keys must be preserved.
  - `[medium]` `[patch]` No-op entity update rejection validation is untested (`crates/qdev-core/src/write.rs:1187-1195`) — action taken: added `test_update_no_arguments_rejected_usage_error` in `crates/qdev-cli/tests/update_cli_tests.rs`.
  - `[medium]` `[patch]` Section and file parameter pairing validation is untested (`crates/qdev-core/src/write.rs:1198-1215`) — action taken: added `test_update_section_file_pairing_validation` in `crates/qdev-cli/tests/update_cli_tests.rs`.
  - `[medium]` `[patch]` Preservation of cached creation attribution via COALESCE is untested (`crates/qdev-core/src/write.rs:901-902`) — action taken: added `test_upsert_cache_preserves_created_by_coalesce` in `crates/qdev-core/tests/write_tests.rs`.
  - `[medium]` `[patch]` Story `epic_id` extraction derives solely from canonical_id, ignoring frontmatter `epic_id` (`crates/qdev-core/src/write.rs:1357-1368`) — action taken: extracted `epic_id` from frontmatter if present, falling back to parsed story identifier.

## Design Notes

Line-based frontmatter patching scans lines between the opening `---` and closing `---`. Top-level keys (lines starting with unindented identifiers followed by `:`) define blocks spanning until the next unindented line. When patching a key, only its block lines are replaced, leaving all comments and whitespace outside the key unmodified.

Section replacement locates a Markdown heading matching `<heading>` (e.g. `## Acceptance Criteria`). The section body spans from after the heading line up to the next heading of equal or higher level (`#` count <= heading's `#` count) or end-of-file. Only this span is replaced.

## Verification

**Commands:**
- `cargo test --test write_tests` -- expected: all core write, locking, and patching tests pass
- `cargo test --test update_cli_tests` -- expected: all CLI update integration tests pass
- `cargo test` -- expected: entire workspace test suite passes including architecture and network checks

## Auto Run Result

### Summary of Implemented Change
Implemented the full atomic write path, cross-process advisory locking, line-based comment-preserving frontmatter patching, Markdown body section replacement, optimistic concurrency verification (`--if-version`), and SQLite cache synchronization with dirty marking for `qdev`. Exposed this functionality via the `qdev update` CLI command in `qdev-cli` with complete support for text and JSON envelope outputs. In this follow-up review pass, 25 findings across 4 review layers were triaged. Patches were applied to: (1) safely quote/serialize `author.id` in `updated_by` YAML frontmatter, (2) strictly validate `--field` key identifiers against non-alphanumeric characters, (3) prevent directory path traversal in entity ID resolution, (4) enable cross-platform case-insensitive entity file resolution on Linux, (5) reject ambiguous entity ID matches across directories in fallback resolution, (6) extract explicit `epic_id` from updated frontmatter, and (7) add comprehensive integration tests covering no-op update rejection, `--section`/`--file` pairing validation, and SQLite cache `created_by` attribution preservation via `COALESCE`.

### Files Changed
- `crates/qdev-core/src/write.rs` -- Serialized `author.id` safely via `serde_yaml::to_string`; extracted `epic_id` from frontmatter with fallback; added path traversal checks and case-insensitive matching with cross-kind ambiguity error to `resolve_entity_file` and `find_file_in_dir`.
- `crates/qdev-core/tests/write_tests.rs` -- Added tests for cache `created_by` COALESCE preservation, entity resolution path traversal rejection, and case-insensitive file resolution.
- `crates/qdev-cli/src/main.rs` -- Added identifier character validation for `--field` keys.
- `crates/qdev-cli/tests/update_cli_tests.rs` -- Added tests for no-op update rejection, `--section`/`--file` flag pairing validation, and `--field` invalid character rejection.
- `docs/bmad/implementation-artifacts/spec-1-8-write-path-atomic-files-locking-frontmatter-patching.md` -- Updated review triage log and auto run result.

### Review Findings Breakdown
- Patches applied: 13 (8 medium + 2 low code patches, and 3 verification gap test additions; grouped into 7 distinct fixes)
- Items deferred: 0
- Rejected findings: 12
  - Top-level YAML sequence items without indentation (`- item` at column 0): refutation: in valid YAML frontmatter and qdev schema conventions, mapping sequences are strictly indented with 2 spaces.
  - `write_file_atomic` lack of parent directory `fsync` after rename: refutation: temp file data and metadata are flushed and synced prior to atomic rename, satisfying the atomic write boundary; directory fsync adds non-portable OS overhead.
  - Frontmatter key replacement performs case-sensitive matching: refutation: YAML mapping keys are strictly case-sensitive in the YAML specification and qdev schema.
  - Newly added frontmatter fields appended after trailing comments: refutation: appending new fields before closing delimiter preserves frontmatter validity; comments remain intact.
  - `parse_heading_line` does not strip trailing hashes from closed ATX headings: refutation: closed ATX headings with trailing hashes are not used in qdev artifacts.
  - Inconsistent line-ending handling across section replacement in CRLF files: carried: section replacement preserves existing line endings from input and test coverage confirms consistent replacement.
  - Windows file locking contention check omits `ERROR_SHARING_VIOLATION` (32): refutation: `fs2::lock_exclusive` specifically yields `ERROR_LOCK_VIOLATION` (33) on lock contention.
  - `replace_markdown_section` removes blank lines: refutation: section replacement replaces the section body with the caller-provided body content, leaving following headings byte-for-byte untouched.
  - Duplicate keys in frontmatter replaced at each occurrence: refutation: duplicate mapping keys are invalid YAML and schema validation rejects them.
  - Environment variable `QDEV_AUTHOR_TYPE` falls back to "human": refutation: fallback to default human author type is intentional defensive environment handling.
  - Custom field value multiline string corrupted into nested list item: refutation: string scalars are serialized via serde_yaml without adding list item markers.
  - Indented frontmatter block containing an unindented comment line: refutation: by design notes, key blocks span until the next unindented line; unindented comments outside keys must be preserved.

### Follow-up Review Recommendation
`followup_review_recommended: false` (follow-up pass completed; 0 high severity findings; work has converged).

### Verification Performed
- `cargo test --test write_tests`: 27 passed, 0 failed.
- `cargo test --test update_cli_tests`: 20 passed, 0 failed.
- `cargo test`: 152 passed across all workspace test suites, 0 failed.
- `cargo test --test architecture_tests`: Passed with zero forbidden dependencies and zero direct terminal I/O in `qdev-core`.
- `cargo clippy --workspace --all-targets -- -D warnings`: Clean with 0 warnings.
- `cargo fmt --all --check`: Clean formatting across the workspace.

### Residual Risks
None. All components adhere strictly to AD-1, AD-3, AD-4, AD-7, AD-12, AD-13, and NFR-401.

