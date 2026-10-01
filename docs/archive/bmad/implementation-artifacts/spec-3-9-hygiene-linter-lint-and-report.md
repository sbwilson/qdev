---
title: 'Story 3.9: Hygiene Linter (Lint and Report)'
type: 'feature'
created: '2026-09-26'
status: 'done'
baseline_commit: 'b7df4744c401af0ae255987eaf807acddd379ef6'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/cli-reference.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** LLMs and human developers frequently inject narrative memoirs, review rounds, and story banner comments into source code, cluttering files and obscuring business logic.
**Approach:** Implement a language-aware lexical comment tokenizer for Rust, Swift, and Python to extract line, block, and doc comments without false positives from string literals; lint comments against `max_inline_comment_lines`, `forbid_patterns`, and narrative markers (story banners, review rounds, waves); exempt compact entity citations matching `citation_pattern`; report findings with `file:line`, rule ID, and excerpt; and enforce lint-and-report only (`--fix` exits 2 with a message that automatic fixing is deferred).

## Boundaries & Constraints

**Always:**
- Extract comments using a language-aware lexical tokenizer for Rust (`.rs`), Swift (`.swift`), and Python (`.py`), correctly parsing line comments (`//`, `#`), block comments (`/* ... */` with nesting in Rust and Swift), and doc comments (`///`, `//!`, `/** ... */` in Rust/Swift; docstrings `"""` / `'''` in Python).
- Ignore comment syntax characters (`//`, `/*`, `#`, `"""`) when they occur inside string literals (standard, escaped, multiline, raw strings `r#"..."#`, `#"..."#`).
- Exempt compact citations matching `citation_pattern` (or `DEFAULT_CITATION_PATTERN`) so that valid citations (`[E12S4]`, `[AD-43]`, `[DEC-2b91]`, `[HAZ-14]`, `[DW-7f3a]`, `[E12S4/NG-2]`) are never flagged as findings or counted against inline comment block limits.
- Flag non-doc inline comment blocks exceeding `max_inline_comment_lines` (default 6) with rule ID `max_inline_comment_lines`.
- Flag comments matching any pattern in `config.hygiene.forbid_patterns` with rule ID `forbid_patterns`.
- Flag story banners (e.g. `STORY \d+`, `STORY E\d+S\d+`, `⭐ STORY`) with rule ID `story_banner` and review-round narratives or wave markers (e.g. `Review round \d+`, `Wave \d+`, `Pass \d+`) with rule ID `review_round`.
- Exit with code 1 (`ExitCode::LogicalFailure`) when any hygiene finding is detected.
- Exit with code 0 (`ExitCode::Success`) when zero findings are detected or when hygiene is disabled.
- Exit with code 2 (`ExitCode::UsageError`) when `--fix` is passed, outputting a clear message that automatic fixing is deferred, without modifying any source file.
- Support `--diff` to restrict linting to files changed in the git diff against the integration branch (falling back to staged/working tree changes).
- Support explicit target paths (`PATHS...`) to lint specific files or directories.
- Support `--json` emitting an envelope validating against `schemas/payload-hygiene.json` with `status`, `summary`, and structured `findings` (`file`, `line`, `location`, `rule_id`, `excerpt`).

**Never:**
- Never use raw regex over unparsed source code to extract comments.
- Never modify or mutate any source file (v1 is strictly lint-and-report).
- Never flag compact entity citations matching `citation_pattern`.
- Never flag legitimate public API doc comments under `max_inline_comment_lines` (which applies to inline comments).

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| Clean source file | `qdev hygiene check path/to/clean.rs` | Exit 0; `Hygiene check: 0 findings` (or JSON `status: "pass"`) | N/A |
| Memoir banner comment | Comment `// ⭐ **STORY 2.10 — FIRST import**` | Exit 1; finding at `file:line` with rule ID `story_banner` and excerpt | ExitCode::LogicalFailure |
| Review round narrative | Comment `// Review round 2 findings: fixed bug` | Exit 1; finding at `file:line` with rule ID `review_round` and excerpt | ExitCode::LogicalFailure |
| Comment block over max lines | 7 contiguous lines of `// ...` with `max_inline_comment_lines = 6` | Exit 1; finding at block start line with rule ID `max_inline_comment_lines` | ExitCode::LogicalFailure |
| Configured forbid pattern | Comment matching `forbid_patterns` regex | Exit 1; finding at line with rule ID `forbid_patterns` and excerpt | ExitCode::LogicalFailure |
| Valid compact citation | Comment `// [E12S10] Rust-driven mount (see AD-43)` | Exit 0; citation exempted, no finding produced | N/A |
| Nested block comments | Rust/Swift `/* outer /* inner */ still comment */` | Extracted correctly as one continuous block comment with depth tracking | N/A |
| Comment markers in string literals | `let s = "Hello // not a comment /* neither */";` | Zero comments extracted from inside the string; no false findings | N/A |
| Raw string literals | `let r = r#"Raw string // not comment"#;` | Zero comments extracted from raw string; no false findings | N/A |
| Python triple quotes | Python docstring `""" ... """` | Extracted as doc comment; string inside `" # not comment "` ignored | N/A |
| `--fix` invoked | `qdev hygiene check --fix` | Exit 2; message that auto-fix is deferred; zero files modified | ExitCode::UsageError |
| `--diff` filter | `qdev hygiene check --diff` | Lints only files modified in git diff; clean diff exits 0 | If git fails, infrastructure error |
| Hygiene disabled | `[hygiene] enabled = false` in `qdev.toml` | Exit 0; 0 findings reported | N/A |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/hygiene/mod.rs` -- Hygiene module entry point, types (`HygieneFinding`, `HygieneCheckOutcome`), and `check_hygiene` orchestrator.
- `crates/qdev-core/src/hygiene/tokenizer.rs` -- Language-aware lexical comment tokenizers for Rust (`.rs`), Swift (`.swift`), and Python (`.py`), tracking line numbers, comment kinds, and string literal boundaries with nesting depth support.
- `crates/qdev-core/src/hygiene/linter.rs` -- Comment rule enforcement engine (`max_inline_comment_lines`, `forbid_patterns`, `story_banner`, `review_round`) with compact citation whitelist matching `citation_pattern`.
- `crates/qdev-core/schemas/payload-hygiene.json` -- JSON Schema for `qdev hygiene check --json` output envelope.
- `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Hygiene` with `payload-hygiene.json`.
- `crates/qdev-core/src/gate/runner.rs` -- Register `BUILTIN_GATE_HYGIENE` (`qdev-hygiene`) and wire hygiene runner into gate execution.
- `crates/qdev-core/src/lib.rs` -- Export `hygiene` module and types.
- `crates/qdev-cli/src/cli.rs` -- Add `--fix` flag to `HygieneCheckArgs`.
- `crates/qdev-cli/src/handlers/hygiene.rs` -- Full CLI handler for `qdev hygiene check [--diff] [--fix] [--json] [PATHS...]`.
- `crates/qdev-core/tests/hygiene_tests.rs` -- Comprehensive unit and property tests for comment tokenization, nesting, string literal shielding, citation exemption, and rule evaluation.
- `crates/qdev-cli/tests/hygiene_cli_tests.rs` -- CLI integration tests for `qdev hygiene check` with `--diff`, `--fix`, paths, `--json`, and exit codes.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/hygiene/tokenizer.rs` -- Implement lexical tokenizers for Rust, Swift, and Python handling line, block (with nesting), and doc comments while ignoring strings.
- [x] `crates/qdev-core/src/hygiene/linter.rs` -- Implement rule evaluation for `max_inline_comment_lines`, `forbid_patterns`, `story_banner`, and `review_round` with citation whitelist.
- [x] `crates/qdev-core/src/hygiene/mod.rs` -- Implement hygiene check runner inspecting file sets or diff paths against config.
- [x] `crates/qdev-core/schemas/payload-hygiene.json` -- Add JSON Schema for hygiene payload.
- [x] `crates/qdev-core/src/schema.rs` -- Register `PayloadKind::Hygiene`.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Add `BUILTIN_GATE_HYGIENE` ("qdev-hygiene") built-in gate support.
- [x] `crates/qdev-core/src/lib.rs` -- Export hygiene module.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `--fix` flag to `HygieneCheckArgs`.
- [x] `crates/qdev-cli/src/handlers/hygiene.rs` -- Implement `handle_hygiene` supporting text and JSON output, `--diff`, `--fix` exit 2, and exit code 1 on findings.
- [x] `crates/qdev-core/tests/hygiene_tests.rs` -- Unit tests for tokenizers across Rust, Swift, Python, rule checks, and citations.
- [x] `crates/qdev-cli/tests/hygiene_cli_tests.rs` -- CLI integration tests for `qdev hygiene check`.

**Acceptance Criteria:**
- Given `[hygiene]` configuration and source in the configured languages, when I run `qdev hygiene check --diff`, then comments are extracted with a language-aware tokenizer for Rust, Swift, and Python (line, block, and doc comments), not by regex over raw source.
- Findings are produced for blocks over `max_inline_comment_lines`, matches of `forbid_patterns`, and story banners or review-round narratives, each with `file:line`, rule id, and excerpt; exit 1 on any finding.
- Compact citations matching `citation_pattern` are never flagged.
- No file is modified; `--fix` exits 2 with a message that automatic fixing is deferred.

## Implementation Notes

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---|---|---|---|---|
| Pre-commit hook does not verify comment hygiene blocks commits | verification-gap | `medium` | Pre-verified gap: existing hook tests verify secrets and validation but no test asserts hygiene failures block pre-commit | patch |
| Bare `qdev hygiene check` workspace traversal unverified | verification-gap | `medium` | Pre-verified gap: tests use explicit paths or --diff; bare directory collection is untested | patch |
| Multi-language scanning for Swift and Python in check_hygiene unverified | verification-gap | `medium` | Pre-verified gap: unit tests tested tokenizer in isolation; end-to-end check_hygiene on .swift and .py unverified | patch |
| `crates/qdev-core/src/transition.rs` omits `execute_hygiene_gate` on review transition | verification-gap / blind-hunter | `medium` | Built-in gates qdev-scope and qdev-deps are invoked in transition.rs, but qdev-hygiene was omitted | patch |
| Rust char literal `'\''` escaped quote parsing desynchronizes tokenizer | blind-hunter | `medium` | In `tokenize_rust`, scanning for closing quote matched `chars[i+2]` which is the escaped `'` itself | patch |
| Overly broad `pass \d+` regex in `review_round` flags domain terms | blind-hunter | `medium` | `pass \d+` matched `render pass 1` and `low-pass 1 filter`; should be scoped to review/findings context | patch |
| Trailing comments on consecutive code lines grouped as comment block | edge-case-hunter | `medium` | Separate single-line comments annotating consecutive statements falsely triggered max_inline_comment_lines | patch |
| `--diff` combined with explicit paths ignores path filter | edge-case-hunter / blind-hunter | `medium` | `check_hygiene` took all diff files without filtering to specified paths when both provided | patch |
| Gate dependency validation flags `qdev-hygiene` as unknown gate | edge-case-hunter | `low` | `validate_gate_dependencies` ran before built-in injection and failed if configured gate depended on qdev-hygiene | patch |
| Redundant trailing newline in text output | blind-hunter | `low` | `handle_hygiene` appended newline to string passed to `emit_text` which uses `writeln!` | patch |
| `execute_hygiene_gate` hardcodes `diff: true` | blind-hunter | `low` | In non-git workspaces, hardcoded diff caused error; check git worktree before enabling diff | patch |
| `collect_dir_files` lacks symlink cycle guard and standard ignore directories | blind-hunter | `low` | Following symlinks risks circular loops; skip directory symlinks and ignore venv/build | patch |
| Configured language aliases (e.g. `rs`, `py`) not recognized | edge-case-hunter | `low` | Users may configure `languages = ["rs", "py"]`; map aliases to SupportedLanguage | patch |
| Swift string interpolation containing quotes terminates string early | blind-hunter | `low` | Quotes inside `\("id")` terminated string literal state; track interpolation parentheses depth | patch |
| Built-in gates omitted from `get_gate_list` | blind-hunter | `low` | `qdev gate list` did not include default built-in gates unless explicitly declared in qdev.toml | patch |
| `resolve_diff_files` fallback when merge-base is unavailable | blind-hunter | `false` | Fallback to uncommitted git status and staged changes matches existing codebase conventions in validate.rs | reject |


## Design Notes

### Lexical Tokenizer Architecture
The comment tokenizers operate as lightweight state machines scanning character streams:
- **Rust**: Tracks states (Code, LineComment, DocLineComment, BlockComment, QuotedString, RawString, CharLiteral). Tracks block comment nesting depth `/* ... /* ... */ ... */`. Raw strings track matching `#` counts (`r###"..."###`).
- **Swift**: Tracks states (Code, LineComment, DocLineComment, BlockComment, QuotedString, MultilineString, RawString). Block comments support recursive nesting.
- **Python**: Tracks states (Code, LineComment, SingleQuotedString, DoubleQuotedString, TripleSingleString, TripleDoubleString). Triple-quoted strings are captured as doc comments.

### Citation Exemption
Comments containing compact citations conforming to `citation_pattern` (e.g. `[E12S4]`, `[AD-43]`, `[DEC-2b91]`, `[HAZ-14]`, `[DW-7f3a]`, `[E12S4/NG-2]`) are exempt from narrative marker and forbid pattern checks, and lines containing citations do not count toward `max_inline_comment_lines`.

## Verification

**Commands:**
- `cargo test --test hygiene_tests` -- expected: Unit tests for Rust, Swift, and Python tokenizers and rule checks pass.
- `cargo test --test hygiene_cli_tests` -- expected: CLI integration tests for `qdev hygiene check` pass.
- `cargo test` -- expected: Full test suite passes.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero warnings.
