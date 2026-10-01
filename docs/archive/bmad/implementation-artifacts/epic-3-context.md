# Epic 3 Context: Gates, Evidence & Git

<!-- Compiled from planning artifacts. Edit freely. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Epic 3 delivers the verification engine, compliance evidence generation, and Git integration layer for qdev: an external gate execution runner with strict timeouts and process tree termination; a failure taxonomy distinguishing logical test failures from infrastructure halts; topologically ordered gate dependencies; ratchets with committed branch baselines that prevent metric regressions; immutable, per-commit evidence bundles hydrated into the SQLite index; transition-bound gates attaching to the state machine (including built-in scope and dependency segregation verifiers); a Git preflight guard enforcing scope and integration branch freshness; cross-platform Git hook shims; a language-aware comment hygiene linter; relationship- and module-aware change impact analysis; opt-in contextual commit message drafting; SOUP dependency audits and SBOM capture; and end-of-epic / end-of-sprint review reporting that produces requirements traceability matrices (`rtm.md`), residual anomaly reports (`anomalies.md`), baseline snapshots, and safe sprint close with carry-over. It matters because high-integrity development across parallel human and AI workers requires tamper-evident proof that code satisfies requirements, adheres to architectural boundaries, and preserves quality baselines before reaching review or release. The epic is complete when a transition to `review` runs bound gates, writes evidence, and blocks on a cited constraint violation, and sprint close emits a traceability matrix and anomaly report.

## Stories

- Story 3.1: Gate Runner: Sub-process, Environment, Timeouts
- Story 3.2: Gate Result Contract & Failure Taxonomy
- Story 3.3: Gate Dependencies & `--all`
- Story 3.4: Ratchets & Baselines
- Story 3.5: Evidence Bundles
- Story 3.6: Transition-Bound Gates
- Story 3.7: Git Preflight Guard
- Story 3.8: Git Hook Shims & `qdev hook`
- Story 3.9: Hygiene Linter (Lint and Report)
- Story 3.10: Impact Analysis
- Story 3.11: Opt-In Commit Messages
- Story 3.12: SOUP Audit & SBOM
- Story 3.13: Epic & Sprint Reviews, Sprint Close

## Requirements & Constraints

- Gates execute as external subprocesses (keeping project-specific verifiers out of the core binary), inheriting parent environment variables alongside declared `[environment]` overrides and qdev execution variables (`QDEV_STORY`, `QDEV_GATE`, `QDEV_COMMIT`, `QDEV_RESULT_FILE`, `QDEV_MODULE_PATHS`).
- Every gate run enforces `timeout_ms` with clean process tree termination across platforms (process groups on Unix, job objects on Windows). Stdout and stderr are captured into bounded 1 MB ring buffers. Missing executables are classified as infrastructure failures (`status: infra`) with the resolved path. Local gate skip configurations are honored and recorded as `skipped_locally` in evidence receipts.
- Gate results adhere to a strict JSON schema (written to stdout or `$QDEV_RESULT_FILE`) reporting `status`, `summary`, `failures` (location and message), `metric`, and `constraint_ids`. If no JSON result is produced, status is inferred from exit code and the last 40 lines of stderr. Shipped adapters (`cargo`, `xcodebuild`) extract failing test locations and assertion text from raw output.
- Gate outcomes follow a three-state taxonomy:
  - `pass`: Exit 0 or result `pass` (emits a concise single-line receipt).
  - `fail`: Non-zero exit, result `fail`, or ratchet regression (instructs agent to fix cited failures).
  - `infra`: Timeout, process kill, missing executable, or invalid JSON when `output_adapter = "json"` is declared (instructs agent to `halt_and_alert` the human without modifying code).
- Gates support `depends_on` relationships. Running `qdev gate run --all` or `--for-transition` executes gates in topological order; a failed dependency marks downstream dependents as `skipped`. Dependency cycles are rejected with exit code 2. Aggregate command exit code is 1 if any gate failed, 4 if any encountered an infrastructure failure without logical failures, else 0.
- Metric ratchets (`kind = "ratchet"`) compare numeric output against committed branch baselines (`docs/state/baselines/<branch>/<gate>.json`) using direction rules (`must_not_increase` or `must_not_decrease`). Regressions fail with the delta in the summary; missing baselines pass with a warning. Baselines are updated via `qdev gate baseline <id> --set`.
- Every gate run writes an immutable committed JSON evidence file to `docs/state/evidence/<story-or-_workspace>/<sha>-<gate>.json` capturing gate ID, story ID, commit SHA, status, exit code, duration, metric, summary, `output_sha256`, `run_by` attribution, `ran_at`, `verifies` requirement IDs, and `skipped_locally`. Reruns on the same commit append numerical suffixes (`-2`). Evidence files are hydrated into the `gate_runs` SQLite cache table.
- State transitions (specifically `review`) invoke bound gates declared in `on_transition`. Any failure blocks the transition. Built-in verifiers run alongside external gates:
  - `qdev-scope`: Fails if the git diff against the integration branch touches paths outside the story's target modules, citing the nearest constraint ID (or target modules policy).
  - `qdev-deps`: Fails if a module imports from a module not in its `may_depend_on` or violates layer hierarchy (supporting Rust `use` and Swift `import` resolvers).
  - Bypassing gates (`--skip-gates --justification`) is restricted to humans on a TTY and logs a `DEC-` decision of type `human_ruling`.
- `qdev preflight` verifies: uncommitted working tree changes fall strictly within the leased story's target modules (or chore paths); the local integration branch is not behind its remote; merge-base staleness is within `max_integration_staleness_commits`; and zero blocking validation findings exist.
- Git hook shims (`pre-commit`, `pre-push`, `prepare-commit-msg`) installed via `qdev install hooks` are lightweight platform-agnostic shims calling `qdev hook <name>`, preserving and chaining existing non-qdev hooks. Pre-commit executes hygiene diff checks, changed validation, and optional secret/PHI scans; pre-push executes preflight.
- The hygiene linter extracts comments using a language-aware tokenizer for Rust, Swift, and Python (never raw regex), flagging blocks exceeding `max_inline_comment_lines`, forbidden patterns, and narrative memoir banners. Compact entity citations (`[E12S4]`, `[AD-43]`, `[DEC-2b91]`, `[HAZ-14]`, `[DW-7f3a]`, `[E12S4/NG-2]`) are permitted. The linter operates strictly as lint-and-report (no automatic source code mutation).
- `qdev impact` calculates the blast radius of a story or changed paths, reporting affected stories in `in-progress` or `review`, requirements (`traces_to`, `verifies`), hazards (`mitigates`), ADRs (`governed_by`), overlapping gates, reverse dependencies (`depends_on`, `extends`), and inline entity citations.
- When `[commit_messages] enabled = true`, `qdev hook prepare-commit-msg` automatically drafts commit messages from the active lease (story ID and title) and recent scratchpad decisions/tradeoffs, never overwriting existing commit messages.
- `qdev soup audit` wraps configured dependency audit commands, hydrates records under `docs/state/soup/` with license and CVE status, and records SBOM artifact paths per release.
- Reviews and sprint close:
  - `qdev review epic` audits story statuses, gate evidence coverage, and open deferred work; exits 1 if any story is `done` without gate evidence.
  - `qdev review sprint` executes `sprint_close` transition gates and emits requirements traceability (`rtm.md`) and residual anomaly (`anomalies.md`) reports under `docs/state/releases/<version>/`.
  - `qdev sprint close` refuses to close (exit 3) if any `unacceptable` deferred work lacks rationale or if any story is in-progress with an active lease; otherwise it snapshots baselines, marks the sprint `completed`, and carries unfinished stories into the next sprint without renaming.

## Technical Decisions

- **Subprocess Isolation & Process Groups (AD-5, AD-14).** External gates execute without internal runner dependencies. Process tree termination cleanly kills all spawned child processes on timeout using OS-native mechanisms (`setpgid`/`killpg` on Unix, Job Objects on Windows), preventing orphaned runner processes from stalling headless agents.
- **Result Schema & Output Adapters.** Gates communicate via structured JSON payload or standard streams. Optional built-in adapters (`cargo`, `xcodebuild`) parse compiler/test outputs into structured failure locations and messages. Declared JSON adapter failures fall back safely to `infra` status.
- **Topological Gate Ordering & Cycle Prevention.** Gate dependencies form a directed acyclic graph evaluated prior to execution. Cycles fail validation immediately (exit 2). Upstream failures automatically cascade `skipped` status to dependent gates, saving execution time.
- **Committed Branch Baselines.** Metrics from ratchet gates compare against branch-specific baseline records in `docs/state/baselines/<integration-branch>/<gate>.json`. Baseline files are version-controlled, human-readable JSON updated through explicit write commands.
- **Committed, Append-Only Evidence Bundles (AD-3, NFR-402).** Gate executions generate immutable JSON evidence files under `docs/state/evidence/<story>/<sha>-<gate>.json`. Existing evidence files are never overwritten; reruns on the same commit append numerical suffixes (`-2`). SQLite `gate_runs` table indexes these files during hydration for fast querying.
- **In-Process Boundary Verifiers.** Structural boundary checks (`qdev-scope` and `qdev-deps`) run as built-in gates within core. Scope verification checks `git diff` paths against the story's declared `target_modules`. Dependency verification analyzes language ASTs/tokens (Rust `use`, Swift `import`) against the module registry's `may_depend_on` and layer hierarchy.
- **Non-Network Git Preflight Inspection.** Preflight evaluates local git state (`git merge-base`, `git rev-parse`, `git status --porcelain`) to confirm working tree hygiene, module scope, and remote tracking alignment without introducing unconfigured network latency.
- **Language-Aware Hygiene Tokenizer.** Comment extraction uses language-specific lexical tokenizers for Rust, Swift, and Python, correctly handling block comments, doc comments, and nested delimiters without false positives from string literals.
- **Relational Impact Graph Traversal.** Impact analysis walks the relational graph in SQLite (`traces_to`, `verifies`, `mitigates`, `governed_by`, reverse dependencies) combined with module registry path mappings and regex scanning of modified files for entity citation tokens.
- **Release Documentation & Carry-Over Model (AD-8).** Sprint close generates `rtm.md` (mapping requirements to stories, gate runs, and evidence paths) and `anomalies.md` (grouping open deferred work by safety risk with recorded rationales). Incomplete stories carry over to the next sprint via assignment records (`sprint_assignments`) without mutating story IDs or filenames.

## UX & Interaction Patterns

- **Compact Single-Line Gate Receipts.** Passing gate runs print a concise receipt: `[PASS] <gate> | <summary> | <sha> | <duration> | evidence <path>`. Failures output precise failure locations and assertion details. Infrastructure errors output `[INFRA] ... | halt and alert`.
- **Halt-and-Alert Instruction for Agents.** On infrastructure failures (`status: infra`), payloads explicitly instruct AI coding agents to halt and alert a human developer (`agent_instruction: "halt_and_alert"`), preventing infinite retry loops and speculative fixes for environment/tooling breakdowns.
- **TTY-Enforced Gate Overrides.** Transition-bound gate overrides (`--skip-gates`) are strictly disallowed in non-interactive mode. On interactive terminals, overrides require an explicit justification string and log a `DEC-` decision record. Non-interactive attempts fail closed with exit code 3.
- **Actionable Preflight Error Diagnostics.** Preflight failures provide actionable remediation messages, outputting the exact git command needed to restore compliance (e.g. rebasing against the integration branch or stashing out-of-scope edits).
- **Deterministic Exit Codes (AD-13).** Standard exit codes apply across all Epic 3 operations: `0` for success / all gates pass; `1` for logical gate failures, hygiene findings, or unverified stories; `2` for usage errors, unknown adapters, or dependency cycles; `3` for policy refusals (unjustified gate skips, dirty preflight, blocked sprint close); `4` for infrastructure failures; `5` for concurrency or version conflicts.

## Cross-Story Dependencies

- Story 3.1 (Gate Runner) provides external execution, environment passing, and process termination used by 3.2 (result taxonomy), 3.3 (dependency runner), 3.4 (ratchet execution), 3.5 (evidence generation), 3.6 (transition gates), and 3.12 (SOUP audit commands).
- Story 3.2 (Result Contract & Taxonomy) defines the status and failure schema consumed by 3.3 (aggregate exit codes), 3.4 (ratchet comparisons), 3.5 (evidence records), 3.6 (transition blocking), and 3.13 (review audit summaries).
- Story 3.3 (Gate Dependencies & `--all`) establishes topological sorting and dependency skip propagation used by 3.6 (transition-bound gate sets) and 3.13 (sprint review gate runs).
- Story 3.4 (Ratchets & Baselines) supplies metric regression checks recorded in 3.5 (evidence bundles) and snapshotted in 3.13 (sprint close baselines).
- Story 3.5 (Evidence Bundles) writes committed evidence files and hydrates `gate_runs`, which are required by 3.6 (transition verification), 3.10 (impact overlap), 3.12 (SOUP audit evidence), and 3.13 (traceability matrix generation and epic review gate checks).
- Story 3.6 (Transition-Bound Gates) integrates the gate runner (3.1–3.5) into Epic 2's state machine hooks (`pre_transition`) and implements built-in verifiers (`qdev-scope`, `qdev-deps`), blocking transitions to `review`.
- Story 3.7 (Git Preflight Guard) validates working tree and integration branch hygiene, and is triggered by 3.8 (`qdev hook pre-push`).
- Story 3.8 (Git Hook Shims) manages hook installation and dispatches `qdev hook`, invoking 3.7 (preflight), 3.9 (hygiene linting), and 3.11 (prepare-commit-msg).
- Story 3.9 (Hygiene Linter) provides comment inspection executed during 3.8 (`pre-commit`) and manual verification.
- Story 3.10 (Impact Analysis) queries the relational graph and module registry to calculate change blast radius across requirements, stories, and gates.
- Story 3.11 (Commit Messages) uses active lease information and scratchpad entries from Epic 2 to populate commit templates in 3.8 (`prepare-commit-msg`).
- Story 3.12 (SOUP & SBOM) uses 3.1's gate runner and 3.5's evidence pipeline to audit external dependencies, feeding into 3.13 (sprint reviews).
- Story 3.13 (Epic & Sprint Reviews, Sprint Close) aggregates gate evidence (3.5), requirement traces, deferred work (Epic 2), ratchets (3.4), and SOUP audits (3.12) to produce `rtm.md` and `anomalies.md`, and finalizes sprint state.
- Upstream dependencies on Epics 1 & 2:
  - Epic 1: CLI JSON envelope, exit codes, and interactivity handling (1.1); configuration loader for `[[gates]]`, `[git]`, `[hygiene]`, `[commit_messages]`, and `[soup]` (1.2); SQLite store and hydration sweep for `gates`, `gate_runs`, and baselines (1.6, 1.7); atomic write path (1.8); relational graph for `traces_to`, `verifies`, `mitigates` (1.10).
  - Epic 2: State machine lifecycle hooks (`pre_transition`, `post_transition`) for 3.6; story leases (2.3) for 3.7 preflight and 3.11 commit messages; module registry (2.13) for 3.6 scope/deps checks, 3.7 preflight, and 3.10 impact analysis; deferred work risk levels (2.8) for 3.13 sprint close checks; sprint assignments (2.9) for 3.13 carry-over.
- Downstream dependencies for Epic 4:
  - Epic 4's context projection (`qdev context`) includes bound gates and hygiene directives from 3.6 and 3.9; review context includes diff summaries and gate receipts from 3.2 and 3.5; attributed rejections (4.2) cite gate failures and constraint IDs produced by 3.2 and 3.6.
