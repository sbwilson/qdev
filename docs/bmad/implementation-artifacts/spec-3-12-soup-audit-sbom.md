---
title: 'Story 3.12: SOUP Audit & SBOM'
type: 'feature'
created: '2026-09-26'
status: 'done'
route: 'dispatch'
baseline_commit: '0d8075096c6514c7d4da8b2ca16d13a503130fab'
review_loop_iteration: 0
context:
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** qdev already accepts configured SOUP audit, deny, and SBOM commands, but it has no command surface that executes them or preserves their release evidence. Compliance managers therefore cannot establish a dependency-audit trail or link a generated SBOM to a release.

**Approach:** Add deterministic `qdev soup` audit and SBOM commands that reuse the existing external-command/evidence pipeline, hydrate supported cargo-audit findings as source-controlled SOUP records, and store the configured SBOM artifact path on the explicitly selected release. `qdev soup sbom` requires `--release <version>`; this story exposes a tested SOUP summary API for Story 3.13 rather than adding a partial review command.

## Boundaries & Constraints

**Always:** Execute every configured audit/deny/SBOM command through the shared gate-runner semantics; write immutable `_workspace` evidence for every attempted configured command, including failures; keep SOUP Markdown records as the durable source of truth; parse cargo-audit JSON conservatively without inventing license or CVE facts; preserve the CLI's standard JSON/text and exit-code contracts.

**Never:** Bypass the runner with ad-hoc shell execution; treat generic command output as dependency metadata; modify dependency manifests, install audit tools, or implement the wider Epic 3.13 sprint-review command beyond a reusable SOUP summary seam.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|---------------|----------------------------|----------------|
| Audit succeeds | Configured audit and deny commands; `--release 0.1.0` | Both commands run; `_workspace` evidence is recorded; cargo-audit vulnerabilities become release-tagged SOUP records | Generic successful output remains evidence-only |
| Audit fails | A configured command exits non-zero | Evidence still records the run and the command reports the runner's logical/infra result | No partial fabricated SOUP records |
| SBOM succeeds | Configured command and resolvable release | Command runs through runner and its produced artifact path is recorded on that release | Missing artifact or release is a surfaced error with no release mutation |
| Missing configuration | Requested command lacks its configuration key | No external command runs | Usage/configuration error explains the missing key |

</frozen-after-approval>

## Code Map

- `crates/qdev-cli/src/cli.rs` and `crates/qdev-cli/src/main.rs` -- add and dispatch the `Soup` command tree while preserving existing output plumbing.
- `crates/qdev-cli/src/handlers/mod.rs`, `crates/qdev-cli/src/handlers/gate.rs` -- establish a focused SOUP handler beside the closest command/evidence integration pattern.
- `crates/qdev-core/src/config/types.rs` and `crates/qdev-core/src/config/mod.rs` -- reuse the already validated, merged `SoupConfig`; do not add another configuration format.
- `crates/qdev-core/src/gate/runner.rs` and `crates/qdev-core/src/gate/mod.rs` -- extract or expose a narrow configured-command runner/evidence seam rather than duplicating subprocess, taxonomy, or `_workspace` evidence behavior.
- `crates/qdev-core/src/store/mod.rs`, `crates/qdev-core/src/store/sqlite.rs`, `crates/qdev-core/src/write.rs` -- preserve SOUP Markdown entities and cache hydration/upserts through the established source-of-truth write path.
- `crates/qdev-core/schemas/soup.json`, `crates/qdev-core/schemas/release.json` -- retain SOUP fields and add a validated release field for the SBOM artifact path.
- `crates/qdev-core/src/soup.rs` (new) -- hold cargo-audit JSON interpretation and future review-summary data, isolated from CLI rendering.
- `crates/qdev-cli/tests/evidence_cli_tests.rs`, `crates/qdev-cli/tests/gate_cli_tests.rs`, `crates/qdev-core/tests/store_tests.rs` -- patterns for workspace evidence, execution outcomes, and durable state.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-cli/src/cli.rs`, `crates/qdev-cli/src/main.rs`, and `crates/qdev-cli/src/handlers/{mod.rs,soup.rs}` -- add audit/SBOM command parsing, dispatch, structured payloads, and actionable errors; require `--release <version>` for SBOM.
- [x] `crates/qdev-core/src/gate/{mod.rs,runner.rs}` -- provide a reusable configured-command path that records stable synthetic SOUP gate evidence under `_workspace` without copying runner behavior.
- [x] `crates/qdev-core/src/soup.rs`, `crates/qdev-core/src/lib.rs`, `crates/qdev-core/src/{store/mod.rs,store/sqlite.rs,write.rs}` -- parse cargo-audit vulnerabilities, atomically persist supported dependency records, and expose a summary seam; generic output must not create records.
- [x] `crates/qdev-core/schemas/release.json` and the release frontmatter allowlist -- validate and persist an SBOM artifact path only after a successful run and selected-release resolution.
- [x] `crates/qdev-core/tests/soup_tests.rs` and `crates/qdev-cli/tests/soup_cli_tests.rs` -- cover parser input, audit/deny evidence, success/failure behavior, missing configuration, generic pass-through, SBOM release mutation, and lookup ambiguity.

**Acceptance Criteria:**
- Given configured audit and deny commands, when `qdev soup audit --release 0.1.0` runs, then each command uses the shared runner and emits `_workspace` evidence; cargo-audit JSON findings are upserted to `docs/state/soup/` with only supplied license/CVE data and the release tag.
- Given a configured SBOM command and `--release <version>`, when the SBOM command succeeds, then its artifact path is durably recorded on that release and validates against the release schema.
- Given command failures, malformed audit output, absent configuration, or ambiguous/no release resolution, when a SOUP command runs, then qdev preserves accurate evidence where execution began, returns the established error class, and does not fabricate or partially mutate release/dependency records.

## Implementation Notes

## Spec Change Log

## Review Triage Log

- medium — `soup audit --release` previously accepted a missing or ambiguous release and could persist dangling release metadata; it now resolves the selected release before running commands.
- high — an absolute, parent-traversing, or symlink-escaping SBOM output path could be persisted outside the workspace; `record_sbom_artifact` now accepts only a canonical path contained by the workspace.
- false — the SBOM write does not directly refresh SQLite, but qdev's next command boot-sweeps its Markdown source of truth and this command has no subsequent cache read.
- medium — the JSON field named `dependencies` represented only vulnerable findings; it is now accurately named `findings`.
- false — an audit record is historical release evidence, so an absent vulnerability in a later audit does not invalidate or delete the earlier recorded result.
- high — distinct package identities could normalize to one SOUP record ID and overwrite each other; batch persistence now rejects collisions before creating records.
- medium — a user-defined gate with a synthetic SOUP identifier was silently replaced; the shared runner now rejects reserved-ID collisions.
- low — JSON success rendering has no dedicated assertion, but its typed envelope and corrected `findings` count are exercised through the command’s existing integration path; no separate runtime defect was demonstrated.
- medium — cargo-audit aliases could contain multiple CVEs while only one was retained; the parser now keeps the sorted, deduplicated supplied aliases.
- false — a no-release audit intentionally preserves an earlier release evaluation rather than claiming that historic release evidence was erased.
- high — duplicate normalized identifiers could overwrite a different record in the same batch; collision validation now aborts before any durable write.
- high — SBOM artifact paths could escape via absolute or parent paths; containment validation now rejects them.
- low — release linkage and supplied CVE data were not asserted by the audit integration test; the test now verifies both durable frontmatter fields.
- low — a failed deny command had no focused persistence test; coverage now verifies evidence for both commands and no SOUP write.
- low — a failing SBOM command that prints an artifact path had no immutability test; coverage now verifies the release remains unchanged.

## Design Notes

Use stable synthetic gate identifiers such as `qdev-soup-audit`, `qdev-soup-deny`, and `qdev-soup-sbom` so evidence remains queryable and never collides with user-configured gates. The runner seam owns command execution and receipt construction; the SOUP module owns only interpretation and durable domain records.

## Verification

**Commands:**
- `cargo test -p qdev-core soup` -- expected: cargo-audit parser and SOUP persistence tests pass.
- `cargo test -p qdev-cli --test soup_cli_tests` -- expected: command, evidence, release, and failure-path tests pass.
- `cargo test --workspace` -- expected: full workspace regression suite passes.
