# Epic 2 Context: The Workflow & Governance Engine

<!-- Compiled from planning artifacts. Edit freely. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Epic 2 delivers the workflow, governance, and operational guardrails for qdev: a formal story state machine with synchronous lifecycle hooks, justified backward transitions, worktree-aware story leases that prevent duplicate multi-agent work, scope and cross-team mutation overrides, first-class negative constraint entities that inherit downward, append-only committed scratchpads, a queryable decision ledger, discrete deferred work records with safety risk tracking, sprint assignment scheduling that avoids story renames, path-allowlisted chores, a declared module registry, the deterministic `qdev next` orchestrator selector, and the root workspace pulse dashboard. It matters because high-integrity development across parallel human and AI workers requires strict operational boundaries, transparent audit trails, and automatic coordination before verification gates and AI context projections can safely operate. The epic is complete when two agents in separate worktrees cannot claim the same story, and an unjustified backward transition is refused with a structured error.

## Stories

- Story 2.1: State Machine & Lifecycle Hooks
- Story 2.2: Justified Backward Transitions
- Story 2.3: Story Leases
- Story 2.4: Scope Enforcement & Cross-Team Overrides
- Story 2.5: Constraints as Entities
- Story 2.6: Scratchpads
- Story 2.7: Decision Ledger
- Story 2.8: Deferred Work
- Story 2.9: Sprints as Assignments
- Story 2.10: Chores With Path Allowlists
- Story 2.11: `qdev next`
- Story 2.12: The Root Pulse
- Story 2.13: Module Registry

## Requirements & Constraints

- Stories progress through a fixed lifecycle: `draft → ready → in-progress → review → done`, plus terminal `superseded` and `abandoned` reachable from any non-terminal state with justification.
- Moving `draft → ready` requires at least one acceptance criterion section, a declared appetite, and at least one target module. Moving `ready → in-progress` is refused while `blocked` is true, naming the unmet dependency story IDs.
- Backward transitions (e.g. `review → in-progress`, or `in-progress → ready`) strictly require a justification flag; omitting it fails closed with exit code 3 (`needs_justification`). Backward transitions never release an active lease and preserve all recorded evidence.
- Multi-agent concurrency is governed by leases rather than access control: claiming a story records a lease visible across all Git worktrees, blocking duplicate claims (exit code 5 on collision). Mutating an entity outside the held lease requires an explicit override and justification.
- Ownership is advisory and attributed to users or teams. Mutations across team boundaries require explicit confirmation on interactive terminals, or fail closed in non-interactive mode unless passed with override and justification flags.
- Negative constraints (appetites, rabbit holes, no-gos) are distinct entities with sequential scoped identifiers that inherit from epics down to stories and validate on hydration. Modifying or removing a constraint after a story leaves `draft` requires justification.
- Scratchpads are separate, append-only, committed JSONL ledgers per story; scratchpad operations never mutate story specification files. Appending requires an active lease or override; reading supports token-bounded summaries.
- Decisions (`DEC-` with 4+ hex suffix) record rulings, assumptions, review rejections, pivots, and overrides against subject entities in committed markdown files and SQLite cache rows.
- Deferred work (`DW-` with 4+ hex suffix) records technical debt and residual anomalies with module tags, resolution status, and safety risk levels (`negligible`, `acceptable_with_mitigation`, `unacceptable`). Non-negligible risks strictly require recorded rationale. Reaching `done` on a story closes target deferred work linked by `closes_dw`.
- Sprints schedule stories via assignment records rather than owning them; sprint carry-over preserves story IDs and links carry-over history. A story may belong to only one active sprint at a time.
- Chores provide a fast-track for trivial changes strictly constrained to declared path globs, subject to comment hygiene linting, without altering story state or conflicting with held leases.
- `qdev next` provides deterministic, reproducible selection of the next unblocked story across active sprints based on lease status, ownership match, epic phase, and story sequence.
- The root command (`qdev` with no arguments) provides an instant workspace pulse dashboard covering Git status, cache health, active lease, sprint progress, open deferred work, gate status, and the next recommended action in ≤ 100 ms plus Git status time.
- All commands support `--json`, versioned schemas, stable exit codes (0: success, 1: logical failure, 2: usage, 3: policy refusal, 4: infrastructure, 5: conflict), and complete without stdin in non-interactive environments.

## Technical Decisions

- **State Machine & Lifecycle Hooks.** Core executes transitions by checking preconditions, executing registered synchronous `pre_transition` hooks, persisting changes through the atomic write path, and invoking `post_transition` hooks. Hooks can inspect lease state, block invalid transitions, update deferred work targets, and append audit events.
- **Worktree-Visible Leases.** Leases write to a local gitignored file and are mirrored into the shared `.git` directory so separate worktrees discover active claims instantly without needing database synchronization or network coordination. A lease records holder, author type, worktree path, branch, and start timestamp, and exposes a session token. Breaking a stale lease requires a force flag and logs a lease override decision.
- **Scope & Governance Enforcement.** Mutations verify whether an active lease is held. Touching an entity outside the leased story's immediate tree (its scratchpad, constraints, or originated deferred work) or crossing configured team boundaries triggers a policy refusal (exit code 3) unless overridden with `--override --justification`. Overrides write structured `DEC-` audit entries.
- **Constraint Inheritance & Grammar.** Constraints use scoped IDs (`{owner_id}/{kind_prefix}-{n}`, such as `E12S4/NG-1` or `E12/RH-1`). Hydration indexes them in SQLite and resolves downward inheritance so story queries and context projections include both direct and inherited constraints.
- **Dedicated Append-Only Scratchpads.** Stored at `docs/state/scratch/<story-id>.jsonl` under advisory write lock. Records capture sequence, timestamp, author attribution, kind (`note`, `decision`, `tradeoff`, `transition`), and body text. Projections and read queries extract the tail plus key decisions and transitions bounded by a token budget.
- **Hex-Suffixed Entity Allocation.** Suffixes for `DEC-` and `DW-` use random 4+ hex characters that widen on collision, preventing ID collisions when concurrent agents or separate worktrees create records independently.
- **Sprint Assignment Model.** Sprint definitions live in `docs/state/sprints/sprint-{n}.md`. Sprint membership is decoupled from story identity: `sprint_assignments` tracks `story_id`, `sprint_id`, `assigned_at`, and `carried_from`. Multiple sprints may be active concurrently.
- **Path-Constrained Chores.** `qdev chore` stages and commits only paths matching declared globs. In strict mode, any unstaged or out-of-allowlist modification halts the operation. Chores log a decision record of type `human_ruling` and cannot start in a worktree holding an active story lease unless explicitly requested alongside.
- **Module Registry.** Modules configured in project settings specify unique IDs, path globs, layers, and dependency rules. A path resolver maps repository files to modules. Story `target_modules` and deferred work targets are validated against registered module IDs.
- **Deterministic Next Selection.** Selection strictly orders by: active sprint membership → not blocked → not leased → current user or team ownership match → epic phase → story sequence number, tie-broken by story ID. Returns null and nearest blockers when no work is available.

## UX & Interaction Patterns

- **Interactive Prompts with Graceful Non-Interactive Fallback.** When an override or cross-team edit is attempted on a TTY, the CLI presents an interactive choice (override with justification, adjust ownership, or abort). In non-interactive mode (`--non-interactive`, `QDEV_NONINTERACTIVE=1`, or headless pipes), the command fails closed with exit code 3, emitting a structured error that names the exact flags required.
- **Actionable Rejections.** Refused transitions or commands (e.g. blocked stories, lease conflicts, missing justifications, or un-rationalized deferred work) output clear reasons citing the exact conflicting IDs, holders, or required arguments.
- **Root Pulse Orientation.** Invoking `qdev` without arguments provides a human-readable, single-screen dashboard summarizing working tree state, integration branch sync, cache validity, active lease, sprint burn-down counts, high-risk deferred work, and the next recommended command for both humans and AI coding agents.

## Cross-Story Dependencies

- Story 2.1 (State Machine) serves as the governance core: 2.2 (backward transitions) hooks into its validation pipeline; 2.3 (leases) automatically clears leases on transition to `done`; 2.8 (deferred work) resolves linked records on story completion.
- Story 2.3 (Story Leases) provides the concurrency gate required by 2.4 (scope enforcement), 2.6 (scratchpad append authorization), 2.8 (deferred work authoring), 2.10 (chore mutual exclusion), 2.11 (filtering out claimed stories in `next`), and 2.12 (displaying current lease in pulse).
- Story 2.7 (Decision Ledger) provides the shared audit destination for 2.2 (pivot and review rejection records), 2.3 (forced lease override records), 2.4 (cross-team and out-of-scope overrides), and 2.10 (chore execution logs).
- Story 2.8 (Deferred Work) depends on 2.13 (validating target modules) and feeds 2.12 (surfacing open and unacceptable defect counts in pulse).
- Story 2.9 (Sprints as Assignments) provides active sprint scopes to 2.11 (`next` candidate selection) and sprint summary metrics to 2.12 (pulse dashboard).
- Story 2.13 (Module Registry) provides target module validation for story readiness in 2.1 and deferred work in 2.8, as well as path boundary definitions for 2.4.
- Story 2.11 (`qdev next`) synthesizes outputs from 2.1 (unblocked stories), 2.3 (unleased stories), and 2.9 (active sprint assignments) to feed 2.12 (the next action section of the root pulse).
- Upstream dependencies on Epic 1: Relies on Epic 1's CLI envelope and exit codes (1.1), configuration schema (1.2), SQLite cache and hydration sweep (1.6, 1.7), atomic write path and locking (1.8), and relational graph engine with computed blocked status (1.10).
- Downstream dependencies for Epics 3 & 4: Epic 3's verification gates attach directly to 2.1's lifecycle hooks; Epic 4's context projection and AI skills consume 2.5's constraints, 2.6's scratchpad summaries, 2.3's lease commands, and 2.11's `next` recommendation.
