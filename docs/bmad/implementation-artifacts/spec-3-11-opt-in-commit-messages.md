---
title: 'Story 3.11: Opt-In Commit Messages'
type: 'feature'
created: '2026-09-26'
status: 'done'
route: 'dispatch'
baseline_commit: '8a59ec7ec7b193cdf592eb4473c4c35e21cfdc4e'
review_loop_iteration: 0
context:
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
  - 'docs/cli-reference.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Git history has no consistent contextual summary of the leased work or the decisions that shaped it. Teams need useful drafts without forcing a message convention or ever replacing a message a developer already wrote.

**Approach:** When `[commit_messages]` is enabled, make the existing `prepare-commit-msg` hook draft a commit message from the active story lease, canonical story title, and recent decision or tradeoff scratchpad entries. Keep the existing hook installation and legacy-hook chaining behavior.

## Boundaries & Constraints

**Always:**
- Keep `[commit_messages] enabled = false` as the default no-op behavior.
- Draft only when Git's message file has no nonblank, non-comment content; preserve Git comments/templates and never overwrite supplied content.
- Resolve story context through the current worktree's active lease and the authoritative story file, respecting configured storage paths.
- Include the story ID as scope, the story title, and at most the last three qualifying scratchpad entries (`decision` or `tradeoff`) in chronological order.
- Support only the configured `simple` and `conventional` formats, validate the setting, and retain existing legacy `prepare-commit-msg` chaining and exit propagation after qdev's work.
- Treat a missing active lease or an unreadable/missing leased story as a structured error so an enabled contextual commit cannot silently lose its required context.
- Render the simple header as `E12S4: Story title` and the conventional header as `feat(E12S4): Story title`; conventional remains the default and preferred format.

**Never:**
- Do not alter hook shims, installation ownership, Git state, leases, story files, or scratchpad records.
- Do not use the SQLite cache as the source of the title or scratchpad history.
- Do not turn disabled configuration, an already-written message, or ordinary Git comment/template lines into a failure.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|---|---|---|---|
| Conventional draft | Enabled config, empty Git message file, active lease | Writes the configured conventional header with story ID scope and title, then the three latest qualifying entries | Exit 0 unless legacy hook fails |
| Simple draft | Enabled config with `format = "simple"`, empty message file | Writes the selected simple header and the same contextual body | Exit 0 unless legacy hook fails |
| Existing message | Any nonblank, non-comment line in the message file | Leaves every byte of the file unchanged and chains legacy hook | Legacy exit is propagated |
| Disabled default | Missing or disabled section | Makes no qdev edit and chains legacy hook | Legacy exit is propagated |
| Scratchpad filtering | Mixed scratchpad kinds, including more than three eligible entries | Excludes non-decision/tradeoff entries; retains only the latest three qualifying entries in chronological order | Missing/empty scratchpad behavior follows the approved no-lease/data policy |
| Invalid format | Enabled config with a format other than `simple` or `conventional` | Rejects configuration before hook drafting | Structured configuration error |
| Missing context | Enabled config with no active lease or an unreadable/missing leased story | Do not draft a contextless message | Structured error blocks the commit |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/hook.rs` -- Replace `run_prepare_commit_msg`'s enabled placeholder; retain empty-message detection, workspace-relative message file resolution, and `run_legacy_hook` ordering/propagation.
- `crates/qdev-core/src/config/types.rs` -- `CommitMessagesConfig` supplies the disabled default and format value.
- `crates/qdev-core/src/config/mod.rs` -- `validate_commit_messages_section` and merging/provenance are the validation seam for the allowed format enum.
- `crates/qdev-core/src/lease.rs` -- Reuse `find_active_lease_with_storage` to obtain current-worktree story context.
- `crates/qdev-core/src/scratch.rs` -- Reuse `read_scratch_entries`; filter `decision` and `tradeoff` records rather than the broader summary helper.
- `crates/qdev-core/src/write.rs` and `crates/qdev-core/src/schema.rs` -- Resolve the canonical story file and extract its frontmatter title without cache dependency.
- `crates/qdev-cli/src/handlers/hook.rs` and `crates/qdev-cli/src/cli.rs` -- Existing `qdev hook prepare-commit-msg <git args...>` dispatch; do not expand the CLI surface.
- `crates/qdev-cli/tests/hook_cli_tests.rs` -- Extend the established real-Git integration coverage for enabled drafting, preservation, scratchpad selection, and legacy chaining.
- `crates/qdev-core/tests/config_tests.rs` -- Cover accepted and rejected commit-message format configuration.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/config/mod.rs` and `crates/qdev-core/tests/config_tests.rs` -- Validate `commit_messages.format` as `simple` or `conventional`, with tests for accepted defaults and rejected values.
- [x] `crates/qdev-core/src/hook.rs` -- Build and write contextual drafts only for an enabled, empty message file; resolve lease/story/scratchpad data through existing disk-backed APIs; preserve comments, supplied text, and legacy chaining.
- [x] `crates/qdev-cli/tests/hook_cli_tests.rs` -- Add end-to-end hook fixtures for both exact header formats, last-three chronological scratchpad selection, disabled/preserved-message behavior, missing-context failure, and legacy-hook exit propagation.
- [x] `docs/cli-reference.md` -- Document `[commit_messages]`, the two supported formats, opt-in behavior, and the non-overwrite guarantee.

**Acceptance Criteria:**
- Given an enabled configuration and an empty prepare-commit message, when Git runs the hook for a leased story, then `simple` renders `E12S4: Story title`, `conventional` renders `feat(E12S4): Story title`, and both include the three most recent decision/tradeoff entries.
- Given a supplied commit message, when the hook runs, then qdev leaves the message intact regardless of enabled configuration.
- Given default or explicit disabled configuration, when the hook runs, then it makes no qdev draft and preserves legacy hook behavior.
- Given a valid format setting, when configuration is loaded, then both documented formats are accepted; unsupported formats are rejected before drafting.
- Given enabled configuration with no active lease or no readable leased story, when the hook runs, then it returns a structured error and does not create a contextless draft.
- Given a legacy prepare-commit-msg hook, when qdev's hook completes or intentionally no-ops, then the legacy hook is still invoked and its nonzero exit code reaches Git.

## Implementation Notes

- Implemented disk-backed draft construction in `hook.rs`: active lease to canonical story frontmatter to filtered scratchpad entries. Existing Git comments remain below the generated draft, and legacy-hook chaining remains the final operation.
- Configuration now rejects `commit_messages.format` values other than `simple` and `conventional`.

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---|---|---|---|---|
| Non-`#` Git comment templates are misclassified as supplied content | blind-hunter | `medium` | `core.commentChar` can be configured, and the original `#`-only predicate skipped drafting for a template-only file. | patch — read Git's configured character and cover it |
| Missing hook argument or nonexistent message file silently skips drafting | blind-hunter | `low` | Direct `qdev hook` invocation can reach this path, but Git's `prepare-commit-msg` contract supplies an existing message file; changing that contract adds complexity to an ordinary user-controlled CLI invocation. | reject |
| Direct CLI invocation can target arbitrary absolute paths or symlinks | blind-hunter | `low` | The caller already controls the local qdev command and its file argument; Git-hook use supplies Git's message path. Restricting paths would require new worktree/GIT_DIR policy not required by the intent. | reject |
| Multiline story title injects content into the subject | blind-hunter | `medium` | Frontmatter title text was rendered directly into a one-line commit subject. | patch — reject line breaks |
| Multiline scratchpad text escapes its bullet | blind-hunter | `medium` | Scratchpad text was interpolated after `- ` without continuation indentation. | patch — indent continuation lines |
| Generated draft may omit a final newline | blind-hunter | `low` | Header-only drafts and empty templates could be written without a terminal newline. | patch — normalize generated content |
| Commit-template write may partially replace the file on I/O failure | blind-hunter | `medium` | `fs::write` truncates in place, conflicting with preservation of the original Git template. | patch — use atomic replacement |
| Non-default Git comment character is not honored | edge-case-hunter | `medium` | Independently confirmed the `#` predicate failed when `core.commentChar` was configured differently. | patch — covered by the Git comment-character fix |
| Message changes during draft preparation can be overwritten | edge-case-hunter | `medium` | The first read was used for the final write after lease/story/scratch resolution without a freshness check. | patch — reread and return `commit_message_changed` on mismatch |
| Nonexistent message-file path proceeds without draft context | edge-case-hunter | `low` | This is reachable only through a non-Git direct CLI invocation; the actual Git hook always receives its message file. | reject |
| Enabled drafting plus legacy-hook exit propagation is untested | verification-gap | `medium` | The generated-draft test had no legacy hook, so a future bypass could pass all prior hook tests. | patch — add combined assertion |
| Configured storage layout is untested through drafting | verification-gap | `medium` | All prior draft fixtures used default storage, leaving the declared storage-aware APIs unpinned. | patch — add configured-layout integration test |
| Header-only draft behavior is untested | verification-gap | `medium` | No enabled, leased fixture exercised an empty qualifying-scratchpad result. | patch — add exact-output integration test |

## Design Notes

### Draft assembly

The hook should remain a narrow adapter over existing authoritative data paths: lease -> story file/frontmatter -> scratch ledger. Filtering first, taking the final three entries, and then retaining sequence order makes the resulting body both bounded and chronologically readable. A small pure rendering helper is appropriate if it lets exact grammar and message-file preservation be tested independently of Git fixtures.

## Verification

**Commands:**
- `cargo test --test config_tests` -- expected: commit-message configuration accepts the two supported formats and rejects invalid ones.
- `cargo test --test hook_cli_tests` -- expected: real-Git prepare-commit-msg drafting and legacy chaining pass.
- `cargo test` -- expected: entire workspace test suite passes.
- `cargo clippy --all-targets -- -D warnings` -- expected: no warnings.
