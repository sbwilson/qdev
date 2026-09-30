---
title: 'Story 4.3: Hygiene Directive & Citation Template'
type: 'feature'
created: '2026-09-29'
status: 'done'
route: 'dispatch'
baseline_commit: 792645fdae96aabf63844068003efd2c9bd4950a
review_loop_iteration: 0
context: []
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Agents generating code often write long, conversational comments and historical memoirs directly in source files rather than compact, verifiable citations. Additionally, code reviewers lack immediate visibility of comment-hygiene violations on the pull-request diff during the review phase.

**Approach:** Extend `[hygiene]` configuration to support an overridable `directive` text, base `citation_template` (with `citation_format` alias), and per-language `citation_templates`. Inject the directive text and resolved per-language citation templates into the `hygiene` section of `qdev context` for both `develop` and `review` phases. Add a `hygiene_findings` section to the `review` phase context that runs the comment linter on the current diff and reports any violations so reviewers can enforce hygiene standards.

## Boundaries & Constraints

**Always:**
- Derive citation templates for all configured languages in `hygiene.languages` (defaulting to Rust, Swift, and Python).
- If `directive` is not set in `[hygiene]`, default to `HYGIENE_DIRECTIVE` verbatim from `docs/compliance-and-safety.md` §1.
- In both `develop` and `review` context projections, the `hygiene` section must contain the effective directive text followed by the per-language citation templates.
- In the `review` phase context projection, include a dedicated `hygiene_findings` section (priority 10, between `diff` and `gate_receipts`) containing findings on the diff from `check_hygiene` (or `(none)` if clean).
- Deterministic output: section names, priority order, and language template ordering must remain completely deterministic.
- Safe execution: outside a git repository or when a diff baseline cannot be resolved, `hygiene_findings` returns an empty reason note (`(empty: ...)`) without failing the context projection.

**Never:**
- Never modify or write files during `qdev context` (read-only command).
- Never fail the context projection when hygiene check detects findings; findings must be reported as section content for reviewer inspection.
- Never hardcode comment syntax as `//` for languages that use `#` (such as Python) when deriving templates.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Default hygiene config in develop | `qdev context E12S4 --phase develop --json` | `hygiene` section contains `HYGIENE_DIRECTIVE` and templates for Rust (`// [{id}] {summary}`), Swift (`// [{id}] {summary}`), Python (`# [{id}] {summary}`) | exit 0 |
| Custom directive in qdev.toml | `[hygiene] directive = "Custom rules."` | `hygiene` section uses "Custom rules." followed by citation templates | exit 0 |
| Custom per-language templates | `[hygiene.citation_templates] rust = "// [{entity_id}]"` | Rust template uses custom template; other languages use default derived syntax | exit 0 |
| Base citation_template configured | `[hygiene] citation_template = "[{id}]"` | Rust/Swift prefix `// [{id}]`, Python prefixes `# [{id}]` | exit 0 |
| Review phase with clean diff | `qdev context E12S4 --phase review --json` with no hygiene violations | `hygiene_findings` section has content `(none)` | exit 0 |
| Review phase with diff violations | `qdev context E12S4 --phase review --json` with narrative memoir comment in diff | `hygiene_findings` section contains formatted `file:line: [rule_id] excerpt` lines | exit 0 |
| Review phase outside git worktree | `qdev context E12S4 --phase review` in non-git directory | `hygiene_findings` section has `(empty: not inside a git work tree)` | exit 0 |
| Hygiene disabled in config | `[hygiene] enabled = false` in review | `hygiene_findings` section has content `(none)` | exit 0 |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/config/types.rs` -- `HygieneConfig`: add `directive: Option<String>`, `citation_template: Option<String>` (aliased `citation_format`), `citation_templates: BTreeMap<String, String>`, and helper `resolved_citation_templates(&self) -> BTreeMap<String, String>`
- `crates/qdev-core/src/config/mod.rs` -- `validate_hygiene_section`: allow `directive`, `citation_template`, `citation_format`, `citation_templates` and validate types; update config loader to populate fields and source annotations
- `crates/qdev-core/src/config/source.rs` -- add formatting for `hygiene.directive`, `hygiene.citation_template`, and `hygiene.citation_templates` in `qdev config show`
- `crates/qdev-core/src/context.rs` -- update `hygiene_section` to emit directive text plus formatted citation templates; implement `hygiene_findings_section` via `check_hygiene` on diff; add `hygiene_findings` to review phase drafts at priority 10
- `crates/qdev-core/tests/context_tests.rs` -- update existing assertions to reflect enhanced `hygiene` content and review section list; add tests for custom directive, custom templates, and review hygiene findings
- `crates/qdev-core/tests/config_tests.rs` -- add tests for `[hygiene]` configuration parsing, validation, and source tracking
- `crates/qdev-cli/tests/context_cli_tests.rs` -- assert CLI `--json` output contains updated `hygiene` section and review phase `hygiene_findings`
- `docs/cli-reference.md` -- document new `[hygiene]` configuration fields and updated context sections for develop and review
- `docs/compliance-and-safety.md` -- update hygiene directive and citation template documentation

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/config/types.rs` -- Add `directive`, `citation_template`, `citation_templates` to `HygieneConfig` with derivation logic -- Provide typed schema and default resolution for comment templates
- [x] `crates/qdev-core/src/config/mod.rs` & `source.rs` -- Update `validate_hygiene_section`, loader, and `ConfigAnnotated` display -- Ensure config validation and tracking accept new hygiene keys
- [x] `crates/qdev-core/src/context.rs` -- Integrate directive, citation templates into `hygiene` section and add `hygiene_findings` to review phase -- Fulfill Story 4.3 acceptance criteria for develop and review payloads
- [x] `crates/qdev-core/tests/context_tests.rs` & `config_tests.rs` -- Add unit tests for hygiene directive, citation template derivation, and review diff hygiene findings -- Verify core context behavior and edge cases
- [x] `crates/qdev-cli/tests/context_cli_tests.rs` -- Add CLI integration tests for `qdev context` with custom hygiene config and review findings -- Verify end-to-end CLI behavior
- [x] `docs/cli-reference.md` & `docs/compliance-and-safety.md` -- Update documentation for hygiene configuration, citation templates, and review context payload -- Maintain accurate developer references

**Acceptance Criteria:**
- Given `qdev context --phase develop`, when the payload is built, then the `hygiene` section includes the directive text (from `[hygiene].directive` or default `HYGIENE_DIRECTIVE`) and per-language citation templates derived from `[hygiene]` configuration for all configured languages.
- Given `qdev context --phase review`, when the payload is built, then the payload includes both the `hygiene` section and a `hygiene_findings` section containing findings from running the hygiene linter on the current diff (or `(none)` when no findings exist).
- Given custom `[hygiene]` configuration with custom `directive`, `citation_template`, or `citation_templates`, when `qdev context` runs, then the emitted payload reflects the custom configuration.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| # | Reviewer Layer | Location | Claim / Finding | Verdict | Route | Resolution |
|---|---|---|---|---|---|---|
| 1 | Blind Hunter | `crates/qdev-core/src/config/types.rs:427` | Language alias `"py"` missing from hash-comment leader match | `high` | `patch` | Add `"py"` to hash-comment match arm so `languages = ["py"]` gets `#` instead of `//`. |
| 2 | Edge Case Hunter / Blind Hunter | `crates/qdev-core/src/config/mod.rs:1231` | Project `citation_template` overrides local `citation_format` alias | `high` | `patch` | Inspect local config for either `citation_template` or `citation_format` before falling back to project table. |
| 3 | Edge Case Hunter / Blind Hunter | `crates/qdev-core/src/config/mod.rs:1270` | Case-sensitive deduplication during `citation_templates` merging allows project template to shadow local override | `medium` | `patch` | Use case-insensitive key check when merging project templates into local templates. |
| 4 | Blind Hunter | `crates/qdev-core/src/config/mod.rs:520` | Missing validation against simultaneously specifying both `citation_template` and `citation_format` in the same table | `medium` | `patch` | Return a schema violation usage error if both alias keys are defined in the same TOML file. |
| 5 | Blind Hunter | `crates/qdev-core/src/config/mod.rs:520` | Missing validation for empty or whitespace-only directive and citation templates | `medium` | `patch` | Return usage error if `directive`, `citation_template`, `citation_format`, or `citation_templates` values are empty/whitespace. |
| 6 | Blind Hunter | `crates/qdev-core/src/config/types.rs:430` | Doc-comment prefixes (`///` or `//!`) produce corrupted Python templates | `low` | `patch` | Strip all leading slashes and trim when converting comment leaders to `#`. |
| 7 | Blind Hunter / Verification Gap | `docs/cli-reference.md:990` | Double negative typo ("without an unresolvable baseline") | `low` | `patch` | Fix typo to "with an unresolvable baseline". |
| 8 | Blind Hunter | `crates/qdev-core/src/context.rs:78` | Stale doc comment on `HYGIENE_DIRECTIVE` claiming config-overridability arrives in later story | `low` | `patch` | Update doc comment to reflect current overridability via `[hygiene].directive`. |
| 9 | Blind Hunter | `sprint-status.yaml` | Story key status is `in-progress` instead of `review` | `false` | `reject` | Workflow rule: `sprint-status.yaml` advances to `review` at step-05, not step-04. |
| 10 | Verification Gap Reviewer | `crates/qdev-core/tests/context_tests.rs:628` | Pre-verified gap: unresolvable diff baseline for `hygiene_findings` in git worktree is unverified | `medium` | `patch` | Add test verifying `(empty: cannot resolve a diff baseline against 'develop')` in git worktree lacking integration branch. |
| 11 | Verification Gap Reviewer | `crates/qdev-core/tests/config_tests.rs:575` | Pre-verified gap: hierarchical config merging of `[hygiene.citation_templates]` from `.qdev.local.toml` is unverified | `medium` | `patch` | Add test verifying local `.qdev.local.toml` `citation_templates` override project `qdev.toml`. |
| 12 | Blind Hunter | `crates/qdev-core/src/config/source.rs:365` | Loss of per-language source provenance in `qdev config show` | `low` | `reject` | Standard across all small mapping sections in `source.rs`; per-entry provenance is only used for `[[teams]]` arrays. |

## Design Notes

- **Citation Template Derivation:**
  For each language configured in `config.hygiene.languages`:
  1. Check `config.hygiene.citation_templates` (case-insensitive). If present, use that string.
  2. Else, take the base template: `config.hygiene.citation_template.as_deref().unwrap_or("[{id}] {summary}")`.
  3. Prepend or adapt the comment leader for the language:
     - Rust / Swift / C / C++ / Go / JS / TS: `// <base>` (if base starts with `//`, keep as-is; if base starts with `#`, replace with `//`)
     - Python / Ruby / Shell / YAML / TOML: `# <base>` (if base starts with `#`, keep as-is; if base starts with `//`, replace with `#`)
- **Review Phase Section Order:**
  1. `story_spec`
  2. `constraints`
  3. `modules`
  4. `adr_excerpts`
  5. `requirements`
  6. `scratchpad`
  7. `gates`
  8. `hygiene`
  9. `diff`
  10. `hygiene_findings`
  11. `gate_receipts`
  12. `evidence`
