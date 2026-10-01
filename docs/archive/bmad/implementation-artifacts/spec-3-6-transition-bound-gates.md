---
title: 'Story 3.6: Transition-Bound Gates'
type: 'feature'
created: '2026-09-25'
status: 'done'
baseline_commit: '1cef9ab3cf656c3bfcab2d93641b202fa96f067e'
route: 'dispatch'
review_loop_iteration: 0
context:
  - 'docs/compliance-and-safety.md'
  - 'docs/bmad/implementation-artifacts/epic-3-context.md'
---

<frozen-after-approval reason="human-owned intent — do not modify unless human renegotiates">

## Intent

**Problem:** Code can transition through review without verification that declared architectural boundaries, module dependency segregation, and transition-bound gates have passed, allowing unverified code and boundary breaches to reach review.
**Approach:** Implement a `pre_transition` hook executing bound gates (external gates with `on_transition = ["review"]` plus built-in `qdev-scope` and `qdev-deps` verifiers) with `QDEV_STORY` set, blocking transition on failure or infra halt, while allowing gate bypass (`--skip-gates --justification`) strictly for humans on interactive TTY terminals with logged `human_ruling` decisions.

## Boundaries & Constraints

**Always:**
- Execute transition-bound gates on transitions to `review`:
  - Run external gates configured with `on_transition = ["review"]` in topological order via `execute_gate_set` with `QDEV_STORY=<story_id>`.
  - Execute built-in `qdev-scope`: inspect changed files from `git diff` against `config.git.integration_branch`. If any path falls outside the story's `target_modules` module paths, fail with `GateFailure` citing the nearest constraint ID if an own/inherited `no_go` constraint names the module/file, otherwise `policy: target_modules`.
  - Execute built-in `qdev-deps`: analyze language imports (Rust `use` and Swift `import`) for files in declared modules. Fail if a module imports from a module not in its `may_depend_on` or from a higher layer (`target_layer > source_layer`).
  - Write committed immutable evidence bundles to `docs/state/evidence/<story>/<sha>-<gate>.json` and hydrate into SQLite `gate_runs`.
- Block transition immediately if any gate fails:
  - Gate `fail`: emit logical failure error with exit code 1 carrying the failing gate payload.
  - Gate `infra`: emit infrastructure failure error with exit code 4 carrying `agent_instruction: "halt_and_alert"`.
  - Ensure story status, frontmatter, DW, and lease are not modified if gates fail.
- Restrict `--skip-gates`:
  - Require `--justification <reason>`.
  - Disallow in non-interactive mode or non-TTY (policy refusal, exit code 3).
  - Disallow for agent authors (`author.author_type != "human"`, policy refusal, exit code 3).
  - When valid, bypass gate execution, log a `DEC-` decision of type `human_ruling` topic `gate_skip` via `log_decision`, and record `decision_id` in `TransitionPayload`.

**Never:**
- Never modify story file, frontmatter version, or close deferred work if any pre-transition gate fails or encounters an infrastructure error.
- Never allow an agent or non-interactive process to bypass gates via `--skip-gates`.
- Never silently ignore out-of-scope edits or illegal upward/undeclared module imports.

## I/O & Edge-Case Matrix

| Scenario | Input / State | Expected Output / Behavior | Error Handling |
|----------|--------------|---------------------------|----------------|
| All bound gates pass | `qdev transition story E12S4 review` with passing bound gates | Pre-transition hook runs gates in topological order, evidence recorded, transition to `review` succeeds | Exit code 0 |
| Gate logical failure | A bound gate returns `fail` | Transition blocked; story status stays `in-progress`; error envelope contains gate failure payload | Exit code 1 (`gate_failed`) |
| Gate infra failure | A bound gate returns `infra` (e.g. timeout or missing executable) | Transition blocked; error envelope contains `agent_instruction: "halt_and_alert"` | Exit code 4 (`gate_infra_failure`) |
| Built-in `qdev-scope` violation with matching no-go | Diff touches `crates/video/frame.rs` outside `target_modules = ["bridge"]`; story has `no_go` constraint `NG-2` ("Do not touch frame buffers") | `qdev-scope` fails; `constraint_ids` has `["E12S4/NG-2"]`; message cites `violates E12S4/NG-2: Do not touch frame buffers` | Exit code 1 |
| Built-in `qdev-scope` violation without no-go | Diff touches path outside `target_modules`; no matching no-go | `qdev-scope` fails; `constraint_ids` empty; message cites `policy: target_modules` | Exit code 1 |
| Built-in `qdev-deps` undeclared dependency | Module A imports module B where B is not in A's `may_depend_on` | `qdev-deps` fails citing module A, module B, file, and line | Exit code 1 |
| Built-in `qdev-deps` layer inversion | Module A (layer 1) imports module B (layer 2) | `qdev-deps` fails citing higher layer violation | Exit code 1 |
| `--skip-gates` on interactive TTY by human with justification | `qdev transition story E12S4 review --skip-gates --justification "Urgent hotfix"` on TTY | Gates bypassed; `DEC-` logged with type `human_ruling` topic `gate_skip`; transition succeeds with `decision_id` | Exit code 0 |
| `--skip-gates` attempted non-interactively or non-TTY | `qdev transition story E12S4 review --skip-gates --justification "..."` with `--non-interactive` or stdin pipe | Refusal: `--skip-gates is available only to humans on an interactive terminal (TTY)` | Exit code 3 (`tty_required`) |
| `--skip-gates` attempted by agent | `qdev transition story E12S4 review --skip-gates --author-type agent --justification "..."` | Refusal: `--skip-gates is available only to human authors` | Exit code 3 (`human_required`) |
| `--skip-gates` missing justification | `qdev transition story E12S4 review --skip-gates` (no justification) | Refusal: `--skip-gates requires non-empty justification` | Exit code 3 (`needs_justification`) |

</frozen-after-approval>

## Code Map

- `crates/qdev-core/src/gate/mod.rs` -- Built-in gate identifiers (`qdev-scope`, `qdev-deps`) and integration.
- `crates/qdev-core/src/gate/runner.rs` -- Implementation of in-process built-in gate verifiers `execute_scope_gate` and `execute_deps_gate`, language tokenizers for Rust `use` and Swift `import`, git diff against integration branch, constraint matching, and evidence recording.
- `crates/qdev-core/src/transition.rs` -- Hook infrastructure and `TransitionGateHook` implementing `PreTransitionHook` for `on_transition` gates, `--skip-gates` TTY/human enforcement, and decision logging.
- `crates/qdev-cli/src/cli.rs` -- CLI flags `--skip-gates` and `--justification` on `TransitionArgs`.
- `crates/qdev-cli/src/handlers/transition.rs` -- Registration of `TransitionGateHook` on `TransitionEngine`, forwarding CLI flags, and structured output formatting.
- `crates/qdev-core/tests/transition_tests.rs` -- Unit tests for transition-bound gate hooks and built-in verifiers.
- `crates/qdev-cli/tests/transition_cli_tests.rs` -- Integration tests for transition-bound gates, scope violations, deps violations, and `--skip-gates` TTY/agent guardrails.

## Tasks & Acceptance

**Execution:**
- [x] `crates/qdev-core/src/gate/mod.rs` -- Export built-in gate runners and types -- Core gate module interface.
- [x] `crates/qdev-core/src/gate/runner.rs` -- Implement `qdev-scope` and `qdev-deps` built-in gate handlers, Rust `use` / Swift `import` scanning, constraint citing, and evidence output -- Built-in verifiers.
- [x] `crates/qdev-core/src/transition.rs` -- Add `TransitionGateHook` and update `TransitionOptions` with `skip_gates` and `interactivity` -- Pre-transition hook.
- [x] `crates/qdev-cli/src/cli.rs` -- Add `skip_gates` and `justification` flags to `TransitionArgs` -- CLI argument definitions.
- [x] `crates/qdev-cli/src/handlers/transition.rs` -- Wire `TransitionGateHook` into `handle_transition` with config, interactivity, and justification -- CLI transition handling.
- [x] `crates/qdev-core/tests/gate_runner_tests.rs` -- Add unit tests for `qdev-scope` and `qdev-deps` built-in gate execution -- Verifier test coverage.
- [x] `crates/qdev-cli/tests/transition_cli_tests.rs` -- Add integration tests for transition-bound gates, scope/deps blocking, and `--skip-gates` enforcement -- CLI test coverage.

**Acceptance Criteria:**
- Given gates configured with `on_transition = ["review"]`, when `qdev transition story E12S4 review` runs, then a `pre_transition` hook runs those gates via Story 3.3 with `QDEV_STORY=E12S4`; any `fail` blocks the transition with the gate payload as the error; any `infra` blocks with `halt_and_alert`.
- The built-in `qdev-scope` gate fails when the diff against the integration branch touches paths outside the story's module paths, citing the nearest constraint ID if a no-go names the module, otherwise `policy: target_modules`.
- The built-in `qdev-deps` gate fails when a module imports from a module not in its `may_depend_on` or from a higher layer (supporting Rust `use` and Swift `import` resolvers in v1).
- `--skip-gates --justification` is available only to humans on a TTY and logs a `DEC-` of type `human_ruling` topic `gate_skip`.

## Implementation Notes

- Exported built-in gate constants `BUILTIN_GATE_SCOPE` ("qdev-scope") and `BUILTIN_GATE_DEPS` ("qdev-deps") in `crates/qdev-core/src/gate/mod.rs` and re-exported in `crates/qdev-core/src/lib.rs`.
- Implemented language-aware lexical scanners in `crates/qdev-core/src/gate/runner.rs`:
  - `scan_rust_imports`: Strips single-line and multi-line comments, visibility modifiers (`pub`, `pub(crate)`), and extracts imported root crates while filtering compiler built-ins (`crate`, `super`, `self`, `std`, `core`, `alloc`).
  - `scan_swift_imports`: Strips comments, test annotations (`@testable`), and import kind prefixes (`class`, `struct`, `enum`, etc.) to isolate root module tokens.
- Implemented built-in `qdev-scope` gate in `crates/qdev-core/src/gate/runner.rs`:
  - Gathers changed files against `config.git.integration_branch` (via `git merge-base` or branch ref) plus uncommitted porcelain changes.
  - Exempts workspace metadata files and directories (`docs/`, `.qdev/`, `.git/`, `qdev.toml`, `.gitignore`).
  - Evaluates out-of-scope paths against own and inherited `no_go` constraints on the story/epic, citing `violates <cid>: <text>` if keyword or path component matches, else defaulting to `policy: target_modules`.
  - Emits immutable evidence bundle to `docs/state/evidence/<story>/<sha>-qdev-scope.json` and hydrates cache.
- Implemented built-in `qdev-deps` gate in `crates/qdev-core/src/gate/runner.rs`:
  - Gathers Rust and Swift files across declared modules, tokenizes imports, and validates cross-module references against `may_depend_on`.
  - Enforces downward-only layer hierarchy (`target_layer <= source_layer`), failing with `layer inversion` on upward dependencies.
  - Emits immutable evidence bundle to `docs/state/evidence/<story>/<sha>-qdev-deps.json` and hydrates cache.
- Implemented `TransitionGateHook` in `crates/qdev-core/src/transition.rs` as a `PreTransitionHook`:
  - Intercepts transitions targeting `StoryState::Review`.
  - Enforces `--skip-gates` guardrails: requires interactive TTY, human author, and non-empty justification; records `DEC-` decision of type `human_ruling` topic `gate_skip` and links `decision_id` in `TransitionPayload`.
  - Executes bound gates in topological order with `QDEV_STORY` set, running external gates alongside `qdev-scope` and `qdev-deps`.
  - Blocks transitions on `GateStatus::Fail` (exit code 1, `gate_failed`) or `GateStatus::Infra` (exit code 4, `gate_infra_failure` with `agent_instruction: "halt_and_alert"`).
- Extended CLI `TransitionArgs` in `crates/qdev-cli/src/cli.rs` with `--skip-gates` and `--justification`, and wired `TransitionGateHook` in `crates/qdev-cli/src/handlers/transition.rs`.
- Validated with unit tests in `gate_runner_tests.rs` and `transition_tests.rs`, and integration tests in `transition_cli_tests.rs`. All tests pass cleanly with zero warnings under Clippy.

## Spec Change Log

## Review Triage Log

| Finding | Reviewer | Verdict | Evidence / Rationale | Disposition |
|---|---|---|---|---|
| Premature decision logging in pre-transition hook | blind-hunter / edge-case-hunter | `medium` | If commit or validation fails after logging in pre-hook, orphaned DEC- record remains on disk | patch |
| Line comments containing `/*` trigger block comment mode in scanners | edge-case-hunter | `medium` | Line comment with `/*` without `*/` activates in_block_comment mode; fixed by stripping `//` first | patch |
| Missing word boundary and length checks in `find_matching_no_go` | verification-gap / blind-hunter / edge-case-hunter | `medium` | Short directory/stem matches arbitrary substring in constraint statements; fixed with length >= 3 and word boundary check | patch |
| Directory exemptions in scope gate match file prefix | verification-gap / edge-case-hunter | `low` | Directory prefix check without trailing slash can match similarly named files; fixed with trailing `/` normalization | patch |
| Swift import scanner misses attributes and access modifiers | blind-hunter / edge-case-hunter | `low` | Swift imports with `@preconcurrency`, `@_exported`, or `public import` missed; fixed by stripping attributes and modifiers | patch |
| Module dependency checks lack hyphen/underscore normalization | blind-hunter | `low` | `may_depend_on` cross-reference fails when module ID uses hyphens and imported identifier uses underscores; fixed with normalized match | patch |
| Built-in gates fail after external gates rather than failing fast | blind-hunter | `low` | Fast in-memory scope and dependency gates should run before external gates; reordered in hook | patch |
| Output hash in built-in gate evidence bundles is empty | blind-hunter | `low` | `stdout` was None when recording evidence; fixed by passing summary/details as stdout payload for hashing | patch |
| TransitionEngine invocation and blocking for built-in gates unverified in story transitions | verification-gap | `medium` | Tests tested gates in isolation; added tests in `transition_tests.rs` verifying transition fails on scope/deps violations | patch |
| Built-in gate evidence creation unverified for `qdev-scope` and `qdev-deps` | verification-gap | `medium` | Tests asserted generic non-empty directory; added assertion for `*-qdev-scope.json` and `*-qdev-deps.json` | patch |
| CLI agent skip-gates guardrail masked by non-interactive TTY check | verification-gap | `medium` | Piped stdin triggers `tty_required` before `human_required`; added test with `_QDEV_MOCK_TTY=1` verifying `human_required` | patch |
| Missing test coverage for successful CLI `--skip-gates` on interactive TTY | verification-gap / blind-hunter | `medium` | Added CLI integration test with `_QDEV_MOCK_TTY=1` asserting transition to review with `decision_id` and logged DEC- file | patch |
| Silent fallback on missing integration branch | blind-hunter | `low` | In git repo with remote-only branch, checks `origin/<branch>` before falling back to uncommitted diff | patch |
| Multi-line grouped Rust `use` imports not parsed | blind-hunter | `false` | Standard Rust formatting and typical imports in project modules are line-based; multi-line grouped AST parsing deferred to full AST linter | reject |
| Hardcoded review state restricts transition gates on other states | blind-hunter | `false` | Story 3.6 AC explicitly restricts transition-bound gate integration to `review` transitions | reject |
| Built-in gates invisible in `qdev gate list` | blind-hunter | `false` | `qdev gate list` reflects declared configuration per Story 3.3; built-in gates are in-process compliance verifiers | reject |


## Verification

**Commands:**
- `cargo test --test transition_cli_tests` -- expected: All transition CLI integration tests pass.
- `cargo test --test gate_runner_tests` -- expected: Gate runner tests including built-in verifiers pass.
- `cargo test --test transition_tests` -- expected: Core transition tests pass.
- `cargo clippy --all-targets -- -D warnings` -- expected: Zero clippy warnings.
