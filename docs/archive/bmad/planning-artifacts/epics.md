---
stepsCompleted: ["step-01-validate-prerequisites", "step-02-design-epics", "step-03-create-stories", "step-04-spec-review-recut"]
inputDocuments:
  - docs/bmad/planning-artifacts/prd-1/prd.md
  - docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md
  - docs/bmad/planning-artifacts/spec-review.md
  - docs/architecture.md
  - docs/cli-reference.md
  - docs/compliance-and-safety.md
  - docs/governance-and-teams.md
  - docs/roadmap.md
---

# qdev - Epic Breakdown

## Overview

Complete epic and story breakdown for qdev, re-cut on 2026-09-06 from the reconciled PRD, Architecture Spine (AD-1 … AD-14), and design guides after the specification review. BMAD is being used to bootstrap qdev; after Epics 1 and 2 the remaining work is re-entered as native qdev entities (see Story 4.10).

Conventions used in this document:
- IDs in examples follow **AD-7**: `E12S4`, `AD-43`, `FR-102`, `DW-7f3a`, `E12S4/NG-1`.
- "Structured error" means the JSON envelope and exit code defined in **AD-13**.
- Every command in every story must satisfy **AD-12** (non-interactive capable) and **AD-14** (cross-platform); these are not restated per story.

## Requirements Inventory

### Functional Requirements

FR-101 Markdown source of truth, SQLite index · FR-102 Relationship hydration · FR-103 Universal JSON output · FR-104 Entity catalog · FR-105 Validation
FR-201 Negative constraint entities · FR-202 Story state machine · FR-203 Justified backward transitions · FR-204 Story leases · FR-205 Module registry
FR-301 Context projection · FR-302 Scratchpads · FR-303 Citation hygiene · FR-304 Attributed rejections
FR-401 Verification gates · FR-402 Ratchets · FR-403 Evidence bundles · FR-404 Epic and sprint reviews · FR-405 Git integration · FR-406 SOUP audit
FR-501 CLI binary · FR-502 Skills · FR-503 MCP server · FR-504 Pulse and next
FR-601 Dual configuration · FR-602 Multi-owner governance · FR-603 Sprints as assignments · FR-604 Chores

### Non-Functional Requirements

NFR-101 Cross-platform · NFR-102 Zero external runtimes · NFR-103 Concurrency · NFR-104 Non-interactive
NFR-201 Hydration resilience · NFR-202 Human editability
NFR-301 Context isolation · NFR-302 Boot latency
NFR-401 Attribution · NFR-402 Traceability · NFR-403 Zero telemetry · NFR-404 Secret and PHI hygiene

### Architectural Decisions

AD-1 Two crates · AD-2 Synchronous rusqlite · AD-3 Markdown truth, SQLite cache · AD-4 WAL and atomic writes · AD-5 Sub-process gates with result contract · AD-6 Metadata sweep hydration · AD-7 Sprint-free collision-resistant IDs · AD-8 Sprints as assignments · AD-9 Leases not RBAC · AD-10 Module registry · AD-11 Separate scratchpads · AD-12 Non-interactive capable · AD-13 JSON envelope and exit codes · AD-14 Cross-platform

### FR Coverage Map

| FR | Stories |
| --- | --- |
| FR-101 | 1.4, 1.6, 1.7, 1.8 |
| FR-102 | 1.10 |
| FR-103 | 1.1, 1.9, 1.13 |
| FR-104 | 1.4, 1.5 |
| FR-105 | 1.11 |
| FR-201 | 2.5, 4.1 |
| FR-202 | 2.1 |
| FR-203 | 2.2 |
| FR-204 | 2.3, 2.4 |
| FR-205 | 2.13 |
| FR-301 | 4.1 |
| FR-302 | 2.6 |
| FR-303 | 3.9, 4.3 |
| FR-304 | 3.2, 4.2 |
| FR-401 | 3.1, 3.2, 3.3, 3.6 |
| FR-402 | 3.4 |
| FR-403 | 3.5 |
| FR-404 | 3.13 |
| FR-405 | 3.7, 3.8, 3.10, 3.11 |
| FR-406 | 3.12 |
| FR-501 | 1.1, 1.3 |
| FR-502 | 4.4, 4.6 |
| FR-503 | 4.5 |
| FR-504 | 2.11, 2.12 |
| FR-601 | 1.2 |
| FR-602 | 2.4 |
| FR-603 | 2.9 |
| FR-604 | 2.10 |
| NFR-101/104 | 1.1 (harness), 3.1, 3.8 |
| NFR-201/202 | 1.7, 1.11 |
| NFR-301/302 | 1.7, 1.9, 4.1 |
| NFR-401 | 1.4, 1.8 |
| NFR-402 | 1.10, 3.5, 3.13 |
| NFR-403 | 1.1 |
| NFR-404 | 3.8 |

## Epic List

### Epic 1: The Relational Storage & CLI Substrate
Workspace, configuration, entity formats and IDs, SQLite cache and hydration, write path, queries, relations, validation. Exit criterion: create a story, hand-edit it, and read it back as validated JSON in under 30 ms on a 1,000-entity fixture.
**FRs covered:** FR-101, FR-102, FR-103, FR-104, FR-105, FR-501, FR-601

### Epic 2: The Workflow & Governance Engine
State machine with hooks, leases, constraints, scratchpads, decisions, deferred work, sprints as assignments, chores, module registry, `next`, and the pulse. Exit criterion: two agents in separate worktrees cannot claim the same story; an unjustified backward transition is refused with a structured error.
**FRs covered:** FR-201, FR-202, FR-203, FR-204, FR-205, FR-302, FR-504, FR-602, FR-603, FR-604

### Epic 3: Gates, Evidence & Git
Gate engine and result contract, ratchets, evidence bundles, transition-bound gates, preflight, hook shims, hygiene lint, impact analysis, commit messages, SOUP, epic and sprint reviews. Exit criterion: a transition to `review` runs bound gates, writes evidence, and blocks on a cited constraint violation; sprint close emits a traceability matrix and anomaly report.
**FRs covered:** FR-303, FR-304, FR-401 … FR-406

### Epic 4: AI Context & Integration
Context projection, attributed rejections, hygiene directives, skill generation, MCP server, the five skills, doctor, graph, and the self-hosting handover. Exit criterion: a story executed end-to-end through `/qdev-develop` and `/qdev-review` in Claude Code with payloads under budget.
**FRs covered:** FR-201, FR-301, FR-303, FR-304, FR-502, FR-503

---

## Epic 1: The Relational Storage & CLI Substrate

### Story 1.1: Workspace Scaffolding, JSON Envelope & Exit Codes

As a maintainer,
I want a two-crate workspace with a CLI shell that already speaks the final JSON envelope and exit codes,
So that every later story plugs into a stable output contract.

**Acceptance Criteria:**

**Given** a fresh checkout
**When** I run `cargo build --release`
**Then** the workspace contains `qdev-cli` and `qdev-core` per AD-1, and `qdev-core` has no dependency on `clap` or any terminal crate
**And** `qdev --version` and `qdev --version --json` succeed; the JSON payload carries `schema_version`
**And** an unknown subcommand exits 2 with the error envelope of AD-13
**And** a CI matrix builds and runs the test suite on macOS, Linux, and Windows
**And** the binary makes no network calls (verified by a test that runs every command with networking disabled)
**And** a `--non-interactive` global flag and `QDEV_NONINTERACTIVE` are parsed and exposed to core as a single `Interactivity` value.

### Story 1.2: Dual Configuration Loader

As a development team,
I want qdev to merge a committed `qdev.toml` with a gitignored `.qdev.local.toml`,
So that project policy is shared and identity stays local.

**Acceptance Criteria:**

**Given** both files exist
**When** any command boots
**Then** local keys override project keys individually, and arrays in `[[modules]]` and `[[gates]]` are never merged element-wise
**And** absence of the local file is not an error; `identity.developer_id` falls back to `git config user.email`
**And** `qdev config show --json` prints the effective configuration with each key annotated by its source file
**And** a schema violation in either file exits 2 naming the key and file
**And** `[environment]`, `[[modules]]`, `[[gates]]`, `[hygiene]`, `[git]`, `[models]`, `[commit_messages]`, `[regulatory]`, `[soup]` are parsed into typed structs even where later stories consume them.

### Story 1.3: `qdev init`

As a developer,
I want to initialise a workspace interactively or entirely from flags,
So that humans and agents can both bootstrap a repository.

**Acceptance Criteria:**

**Given** a Git repository without qdev
**When** I run `qdev init` on a TTY
**Then** I am prompted for project name, developer id, and teams, and the files and directories in the CLI reference §7 are created, with `.qdev/cache/`, `.qdev/leases/`, and `.qdev.local.toml` appended to `.gitignore`
**When** I run `qdev init --non-interactive --name X --developer y --team z`
**Then** the same result is produced with no prompt; a missing required flag exits 3 with `needs_confirmation` naming the flag
**When** I run `qdev init` in an initialised workspace with an older cache schema
**Then** ~~it reports the migration and applies it only with confirmation or `--yes`~~ **it reports the migration and applies it, confirming nothing** *(superseded 2026-09-11, human renegotiation: the gate contradicted story 1.6's unconditional boot rebuild, and it fired in the one path that could not perform the migration it refused. `--yes` stays accepted and inert. See `docs/bmad/implementation-artifacts/spec-init-cache-migration.md` and story 1.3's Spec Change Log.)*
**And** all created paths are relative to the repository root regardless of the current directory.

### Story 1.4: Entity File Format, Schemas & Attribution

As a system architect,
I want a strict, documented frontmatter schema for every entity kind,
So that files are hand-editable yet machine-validated.

**Acceptance Criteria:**

**Given** the entity catalog in `docs/architecture.md` §5
**When** the schemas are defined
**Then** JSON Schema documents exist for PRD, requirement, epic, story, ADR, hazard, sprint, release, deferred work, decision, scratchpad entry, SOUP dependency, and evidence record, and are embedded in the binary
**And** every entity schema requires `id`, `title` (where applicable), `status`, `version`, `created_by {type, id}`, `updated_by {type, id}` with `type ∈ {human, agent}`
**And** constraints and relations are expressed in the owning entity's frontmatter exactly as in the story file example in `docs/architecture.md` §9
**And** a fixture directory contains one valid and at least two invalid examples per kind, used by tests
**And** the schemas are printable with `qdev schema <kind>`.

### Story 1.5: Identifier Grammar & Allocation

As a developer,
I want IDs that never encode a sprint and cannot silently collide,
So that citations stay valid for the life of the project.

**Acceptance Criteria:**

**Given** AD-7
**When** the grammar is implemented
**Then** a parser accepts and classifies `E12`, `E12S4`, `AD-43`, `FR-102`, `NFR-3`, `HAZ-14`, `PRD-1`, `DW-7f3a`, `DEC-2b91`, `E12S4/NG-1`, `E12/RH-2` and rejects sprint-prefixed forms like `S5E2S4`
**And** `qdev create story E12` allocates the next free `E12S{m}` by scanning files, not the cache
**And** `DW-` and `DEC-` IDs are allocated with 4 random hex characters, extended to 6 and then 8 if a file with that ID already exists
**And** property tests verify parse/print round-trips for all forms
**And** the citation regex in the default hygiene configuration matches every form and nothing else.

### Story 1.6: SQLite Cache Schema & Migrations

As a system architect,
I want the cache schema created and versioned from the entity schemas,
So that the index always mirrors the files.

**Acceptance Criteria:**

**Given** an empty `.qdev/cache/`
**When** any command boots
**Then** `cache.sqlite` is created with the tables in `docs/architecture.md` §10, WAL mode, and `busy_timeout = 5000` per AD-4
**And** a `schema_version` pragma is recorded; a mismatch triggers a rebuild from files, never an in-place migration of cache data
**And** deleting the directory and re-running any command yields an identical cache (verified by a table-by-table comparison test)
**And** the `Store` trait in `qdev-core` exposes every read and write used by later stories, with the SQLite implementation as the only backend.

### Story 1.7: Incremental Hydration Sweep

As a developer or agent,
I want the cache to catch up with local file edits on every command in under 30 ms,
So that queries always reflect the working tree, including uncommitted edits and branch switches.

**Acceptance Criteria:**

**Given** 1,000 entity fixtures with one modified
**When** a command boots
**Then** the sweep compares `mtime` and size against `sync_state`, hashes only changed candidates, and re-parses only files whose hash changed, completing in ≤ 30 ms on the CI reference machine (benchmark test)
**And** after `git checkout` of another branch, every changed file is re-parsed and removed files are purged from the cache
**And** a file containing `<<<<<<<` produces a `merge_conflict` validation finding for that file only; hydration of other files continues
**And** a schema-invalid file produces a `schema_violation` finding and its previous cache row is retained and flagged stale
**And** a row explicitly marked dirty by a qdev write is always re-parsed regardless of metadata.

### Story 1.8: Write Path: Atomic Files, Locking, Frontmatter Patching

As an agent,
I want every qdev mutation to be atomic, attributed, and comment-preserving,
So that concurrent processes and hand edits coexist safely.

**Acceptance Criteria:**

**Given** two processes updating different entities simultaneously
**When** both call `qdev update`
**Then** each acquires the advisory lock on `.qdev/cache/write.lock` with a 5 s timeout, writes a temp file, renames it, upserts the cache row, and marks it dirty; a lock timeout exits 5
**And** frontmatter edits are applied as line-based patches: comments, key order, and blank lines outside the edited keys survive byte-for-byte (golden tests)
**And** `updated_by` is set from the active identity and author type, and `version` increments
**And** `--if-version N` refuses with exit 5 when the stored version differs
**And** `--section "<heading>" --file body.md` replaces exactly one Markdown section of the body and nothing else.

### Story 1.9: `get` / `list` Query Engine

As an agent,
I want isolated, deterministic JSON for any entity and filtered lists,
So that I can load exactly what I need.

**Acceptance Criteria:**

**Given** a hydrated cache
**When** I run `qdev get story E12S4 --json`
**Then** the payload matches the example in the CLI reference §3, including inherited constraints with `inherited_from`, direct relations, and computed `blocked`, but no scratchpad content and no bodies of related entities
**And** `--expand relations,constraints,scratch` adds the named sections only
**And** `qdev get AD-43`, `qdev get FR-102`, `qdev get E12S4/NG-1` resolve by universal reference without a kind
**And** `qdev list stories --epic E12 --status ready --owner me --module bridge --sprint 5` applies every filter and orders by ID
**And** text output is a compact human table; JSON output is byte-identical across runs for the same inputs.

### Story 1.10: Relations, DAG & Computed Blocked

As a planner,
I want relations validated and traversable in both directions,
So that dependency order, traceability, and impact are queryable.

**Acceptance Criteria:**

**Given** relations declared in frontmatter
**When** hydration runs
**Then** every relation in `docs/architecture.md` §8 is stored in the `relations` table with source and target kinds checked against the allowed pairs
**And** a dangling target produces a `dangling_relation` finding rather than dropping the relation
**And** a `depends_on` cycle produces a `dependency_cycle` finding naming the cycle
**And** `qdev relate E12S4 depends_on E12S3` and `qdev unrelate` edit the source file via the write path
**And** `blocked` is true for a story when any `depends_on` target is not `done`
**And** `qdev graph --dot [--epic E12]` emits Graphviz with stories coloured by status and edges labelled by relation.

### Story 1.11: `qdev validate`

As a compliance manager,
I want a single command that reports every integrity problem in the workspace,
So that hand edits and merges are safe.

**Acceptance Criteria:**

**Given** a workspace with seeded defects
**When** I run `qdev validate --json`
**Then** it reports findings with `code`, `severity`, `path`, and `message` for: schema violations, dangling relations, dependency cycles, planning-ID duplicates, orphan deferred work (origin story missing), `acceptable_with_mitigation` or `unacceptable` DW without rationale, merge conflicts, stories with `target_modules` not in the registry
**And** exit code is 1 when any `error`-severity finding exists, 0 otherwise
**And** `--changed` limits the run to files changed since the merge-base with the integration branch
**And** `--fix-ids` offers a guided renumber for duplicate planning IDs that rewrites the file, all relations, and citations in configured source paths, and refuses in non-interactive mode without `--yes`.

### Story 1.12: `qdev sync` & Cache Diagnostics

As a developer,
I want to force or rebuild hydration and see cache health,
So that I can recover from any cache state.

**Acceptance Criteria:**

**Given** any cache state
**When** I run `qdev sync`
**Then** it performs the sweep and prints counts of parsed, unchanged, purged, and findings
**And** `qdev sync --rebuild` deletes and recreates the cache
**And** `qdev doctor` (cache section) reports schema version, entity count, last sync age, and finding counts; later stories extend `doctor` with their own sections through a registry in core.

### Story 1.13: `qdev schema`

As a CI author,
I want to print the JSON Schema for any payload,
So that consumers can validate qdev output.

**Acceptance Criteria:**

**Given** the binary
**When** I run `qdev schema story|context|gate_run|error|next|validate`
**Then** a JSON Schema is printed that validates the corresponding live output (round-trip test for every payload type)
**And** every payload includes `schema_version`, and the schema documents the semver policy in its description.

---

## Epic 2: The Workflow & Governance Engine

### Story 2.1: State Machine & Lifecycle Hooks

As a system architect,
I want stories to move through `draft → ready → in-progress → review → done` with synchronous hooks,
So that gates and leases can intercept transitions without modifying core.

**Acceptance Criteria:**

**Given** a story in `in-progress`
**When** I run `qdev transition story E12S4 review`
**Then** core verifies the move is a legal forward edge, runs registered `pre_transition` hooks in order, aborts with the first hook's structured error if any refuses, writes the new status through the write path, then runs `post_transition` hooks
**And** `draft → ready` refuses (exit 1) unless the story has at least one acceptance criterion section, an appetite, and at least one target module
**And** `ready → in-progress` refuses (exit 3) while `blocked` is true, citing the blocking story IDs
**And** `superseded` and `abandoned` are reachable from any non-terminal state with `--justification` only
**And** when a story reaches `done`, `closes_dw` targets are set to `done` with the story as resolution.

### Story 2.2: Justified Backward Transitions

As a product manager,
I want backward moves to require a recorded justification,
So that pivots and review rejections leave an audit trail without breaking the graph.

**Acceptance Criteria:**

**Given** a story in `review`
**When** I run `qdev transition story E12S4 in-progress` without `--justification`
**Then** it exits 3 with `needs_justification`
**When** I supply `--justification "Failed AC-3"`
**Then** the transition succeeds, a scratchpad entry of kind `transition` is appended, and a `DEC-` record of type `review_rejection` (or `pivot` for `in-progress → ready`) is created with the justification and author
**And** a backward move releases no lease and preserves evidence records.

### Story 2.3: Story Leases

As an agent,
I want to claim a story before working on it,
So that two agents never work the same story and my scope is explicit.

**Acceptance Criteria:**

**Given** no lease on E12S4
**When** I run `qdev claim story E12S4`
**Then** a lease file is written under `.qdev/leases/` and mirrored into the shared `.git` directory so other worktrees can see it, containing holder, author type, worktree path, branch, and start time, and `QDEV_SESSION` is printed for export
**When** another process runs `qdev claim story E12S4`
**Then** it exits 5 naming the current holder and worktree
**And** `qdev release` ends the lease; `transition … done` releases automatically; `qdev release E12S4 --force --justification` breaks a stale lease and logs a `lease_override` decision
**And** `qdev doctor` lists leases older than a configurable age.

### Story 2.4: Scope Enforcement & Cross-Team Overrides

As a lead,
I want mutations outside the leased story or across team ownership to require a logged override,
So that an agent cannot quietly rewrite the epic it is working under.

**Acceptance Criteria:**

**Given** a lease on E12S4 held by `sally` (team ui-shell)
**When** a mutating command targets E12 (owned by team core-platform)
**Then** it is classified as both out-of-lease and cross-team, and on a TTY offers the three options in `docs/governance-and-teams.md` §3
**And** in non-interactive mode it exits 3 with `needs_confirmation` naming `--override --justification`
**When** `--override --justification "…"` is supplied
**Then** the mutation proceeds and a `DEC-` record of type `cross_team_override` (or `lease_override`) is created
**And** decisions, scratchpad entries on the leased story, and deferred work originating from it never require an override.

### Story 2.5: Constraints as Entities

As a planner,
I want appetites, rabbit holes, and no-gos to have IDs and inherit downward,
So that rejections can cite them exactly.

**Acceptance Criteria:**

**Given** an epic E12 with `E12/RH-1` and a story E12S4 with `E12S4/NG-1`
**When** I run `qdev get story E12S4 --json`
**Then** `constraints` contains both, the epic's marked `inherited_from: "E12"`
**And** `qdev constraint add E12S4 --kind no_go -- "text"` allocates the next `NG-n` within the owner and writes it via the write path
**And** `qdev constraint remove E12S4/NG-1` requires `--justification` once the story has left `draft`
**And** the `constraints` table is populated on hydration and `qdev get E12S4/NG-1` resolves.

### Story 2.6: Scratchpads

As an agent,
I want an append-only, committed ledger per story that is never loaded by default,
So that reasoning is preserved without bloating the story spec.

**Acceptance Criteria:**

**Given** AD-11
**When** I run `qdev scratch append E12S4 --kind tradeoff -- "AtomicBool over Mutex on frame drop latch"`
**Then** a JSONL line with seq, timestamp, author type, author id, kind, and text is appended to `docs/state/scratch/E12S4.jsonl` under the write lock and hydrated into `scratchpad_entries`
**And** `qdev scratch read E12S4` prints entries; `--summary` prints the last N entries plus all entries of kind `decision` and `transition`, bounded by `--budget`
**And** the story spec file is never modified by scratchpad operations
**And** appending requires a lease on E12S4 or `--override`.

### Story 2.7: Decision Ledger

As a lead,
I want rulings, assumptions, and overrides recorded against any subject entity,
So that the "why" is queryable and citable.

**Acceptance Criteria:**

**Given** a subject `E12S4`, `E12`, or `AD-43`
**When** I run `qdev decision log --subject E12S4 --type human_ruling --topic "Buffer sizing" --ruling "Fixed 4 MB pool"`
**Then** a `DEC-hhhh` file is created under `docs/state/decisions/` with context, ruling, author, and type from the allowed set in the schema
**And** `qdev list decisions --subject E12S4 --type cross_team_override` filters correctly
**And** system-generated decisions (pivot, review rejection, overrides) use the same path and schema.

### Story 2.8: Deferred Work

As a compliance manager,
I want deferred work as discrete records with module, risk level, and rationale,
So that the residual anomaly list is always accurate.

**Acceptance Criteria:**

**Given** a lease on E12S4
**When** I run `qdev dw add --story E12S4 --module bridge --risk acceptable_with_mitigation --title "Zero-copy bypass" --rationale "…"`
**Then** a `DW-hhhh` file is written under `docs/state/dw/`; omitting `--rationale` for a non-negligible risk exits 1 citing `regulatory.require_rationale_for`
**And** `--module` must exist in the registry
**And** `qdev dw list --module bridge --risk unacceptable --status open` filters; `qdev dw close DW-7f3a --resolution "…"` and `--status wont_fix --justification` update it
**And** a story's `closes_dw` relation closes the DW on `done` (Story 2.1).

### Story 2.9: Sprints as Assignments

As a release manager,
I want sprints to schedule stories without owning them,
So that carry-over never renames anything and patch sprints can run in parallel.

**Acceptance Criteria:**

**Given** AD-8
**When** I run `qdev sprint open 6 --title "…" --release 0.1.0` and `qdev sprint assign 6 E12S4 E11S9`
**Then** `docs/state/sprints/sprint-6.md` records assignments with `assigned_at`, and `sprint_assignments` is hydrated
**And** a story may be assigned to only one `active` sprint at a time; violations are validation errors
**And** `qdev list stories --sprint 6` and `qdev get sprint 6 --json` show assignments and per-status counts
**And** commands accepting `--sprint` fall back to `default_sprint`, then to the single active sprint, else exit 2 asking for `--sprint`
**And** `qdev sprint close` with `--carry-over N` creates assignments in N with `carried_from` for every non-done story and never changes a story ID (full behaviour in Story 3.13).

### Story 2.10: Chores With Path Allowlists

As a developer,
I want a fast-track for trivial changes constrained to declared paths,
So that unverified changes cannot ride along.

**Acceptance Criteria:**

**Given** `qdev chore start "fix readme typo" --paths README.md "docs/**"`
**When** I modify `README.md` and `src/main.rs` and run `qdev chore commit`
**Then** it stages and commits only changes under the allowlist, lists the excluded paths, and exits 0
**And** `--strict` makes any out-of-allowlist change a refusal (exit 3)
**And** the hygiene gate runs on the staged diff before commit
**And** the chore is recorded as a `DEC-` of type `human_ruling` topic `chore` with the paths, and no story state is touched
**And** a chore cannot start while the current worktree holds a story lease unless `--alongside` is passed.

### Story 2.11: `qdev next`

As an orchestrator,
I want a deterministic selection of the next story to work,
So that unattended loops can run.

**Acceptance Criteria:**

**Given** stories across active sprints
**When** I run `qdev next --json [--sprint N] [--owner me]`
**Then** the selection follows the ordering in the CLI reference §5 exactly, skips leased and blocked stories, and returns the story payload plus `reason` fields explaining each filter applied
**And** identical inputs produce identical output (property test over shuffled fixture order)
**And** when nothing is eligible it exits 0 with `next: null` and a list of the nearest blockers.

### Story 2.12: The Root Pulse

As a developer,
I want `qdev` with no arguments to show environment, sprint, and next action,
So that I can orient in one command.

**Acceptance Criteria:**

**Given** an initialised workspace
**When** I run `qdev`
**Then** the output matches the layout in the CLI reference §5: working tree and integration status, cache health and findings, current lease, per-sprint counts including blocked, open DW with unacceptable count, gate summary if evidence exists, and the `next` selection with both the skill and CLI invocation
**And** `qdev --json` returns the same data structured
**And** the command completes within 100 ms plus Git status time on the reference fixture.

### Story 2.13: Module Registry

As an architect,
I want modules declared with paths and layers,
So that boundaries, impact, and preflight have something to enforce.

**Acceptance Criteria:**

**Given** `[[modules]]` entries per AD-10
**When** configuration loads
**Then** each module has a unique `id`, at least one glob in `paths`, optional `layer` and `may_depend_on`, and `qdev doctor` warns when a glob matches no files
**And** a path resolver maps any repository path to zero or more module IDs
**And** `target_modules` on stories are validated against the registry (Story 1.11)
**And** `qdev config show --json` exposes the resolved registry for gates via `QDEV_MODULE_PATHS`.

---

## Epic 3: Gates, Evidence & Git

### Story 3.1: Gate Runner: Sub-process, Environment, Timeouts

As a compliance manager,
I want gates run as external processes with inherited environment and strict timeouts,
So that project-specific checks stay out of the binary and never hang an agent.

**Acceptance Criteria:**

**Given** a `[[gates]]` entry
**When** I run `qdev gate run <id>`
**Then** the command executes with the parent environment plus `[environment]`, plus `QDEV_STORY`, `QDEV_GATE`, `QDEV_COMMIT`, `QDEV_RESULT_FILE`, `QDEV_MODULE_PATHS`
**And** stdout and stderr are captured into 1 MB ring buffers each
**And** on `timeout_ms` the process tree is terminated using process groups on Unix and job objects on Windows (integration test on all three platforms)
**And** a missing executable is reported as `infra` with the resolved path
**And** local `[gates] skip` entries are honoured and recorded as `skipped_locally` in the receipt.

### Story 3.2: Gate Result Contract & Failure Taxonomy

As an agent,
I want gate outcomes classified as pass, fail, or infrastructure failure with precise failures,
So that I fix the right thing or stop.

**Acceptance Criteria:**

**Given** a gate that writes the result JSON in `docs/compliance-and-safety.md` §2 to stdout or `$QDEV_RESULT_FILE`
**When** it exits
**Then** qdev validates the document against the schema, and uses its `status`, `summary`, `failures`, `metric`, `constraint_ids`
**And** without a result document qdev derives `pass`/`fail` from the exit code and a summary from the last 40 stderr lines
**And** timeout, kill, missing executable, or an invalid result document when `output_adapter = "json"` is declared yield `status: infra` and the payload includes `agent_instruction: "halt_and_alert"`
**And** receipts are printed exactly as in §2 (single line on pass)
**And** `output_adapter = "cargo" | "xcodebuild"` extracts failing test locations and assertion text from raw output (fixture tests per adapter); unknown adapters exit 2.

### Story 3.3: Gate Dependencies & `--all`

As a developer,
I want gates ordered by dependency and runnable as a set,
So that expensive gates do not run when cheap ones fail.

**Acceptance Criteria:**

**Given** gates with `depends_on`
**When** I run `qdev gate run --all` or `--for-transition review`
**Then** gates run in topological order, a failed dependency marks dependents `skipped` with the reason, and a cycle in `depends_on` is a configuration error (exit 2)
**And** the aggregate exit code is 1 if any gate failed, 4 if any was `infra` and none failed, else 0
**And** `qdev gate list --json` shows each gate's kind, transitions, dependencies, and last known status from evidence.

### Story 3.4: Ratchets & Baselines

As a lead,
I want metric gates that must not regress against a committed baseline,
So that quality only moves one way.

**Acceptance Criteria:**

**Given** a gate with `kind = "ratchet"`, `metric`, and `direction`
**When** it runs and returns a numeric `metric`
**Then** qdev loads `docs/state/baselines/<integration-branch>/<gate>.json`, compares by direction, and reports `fail` with the delta on regression, `pass` otherwise; with no baseline it reports `pass` with `baseline: null` and a warning
**And** `qdev gate baseline <id> --set` writes the current value with author, commit, and timestamp through the write path
**And** sprint close records all current ratchet values in the release snapshot.

### Story 3.5: Evidence Bundles

As a compliance manager,
I want every gate run recorded as a committed JSON file,
So that verification records exist per commit.

**Acceptance Criteria:**

**Given** any gate run
**When** it completes
**Then** `docs/state/evidence/<story-or-_workspace>/<sha>-<gate>.json` is written matching the schema in `docs/compliance-and-safety.md` §4, including `output_sha256`, `run_by`, `verifies` copied from the gate config, and `skipped_locally`
**And** the record is hydrated into `gate_runs`
**And** `qdev get story E12S4 --expand evidence` lists the latest run per gate
**And** evidence files are never modified after creation; a rerun creates a new file for a new commit or a `-2` suffix for the same commit.

### Story 3.6: Transition-Bound Gates

As a compliance manager,
I want gates to block state transitions,
So that a story cannot reach review or done without passing them.

**Acceptance Criteria:**

**Given** gates with `on_transition = ["review"]`
**When** `qdev transition story E12S4 review` runs
**Then** a `pre_transition` hook runs those gates via Story 3.3 with `QDEV_STORY=E12S4`; any `fail` blocks the transition with the gate payload as the error; any `infra` blocks with `halt_and_alert`
**And** the built-in `qdev-scope` gate fails when the diff against the integration branch touches paths outside the story's module paths, citing the nearest constraint ID if a no-go names the module, otherwise `policy: target_modules`
**And** the built-in `qdev-deps` gate fails when a module imports from a module not in its `may_depend_on` or from a higher layer (Rust `use` and Swift `import` resolvers in v1)
**And** `--skip-gates --justification` is available only to humans on a TTY and logs a `DEC-` of type `human_ruling` topic `gate_skip`.

### Story 3.7: Git Preflight Guard

As a lead,
I want story work to start only from a clean, fresh state,
So that merge collisions and out-of-scope edits are caught early.

**Acceptance Criteria:**

**Given** `[git]` configuration
**When** I run `qdev preflight [--story E12S4]`
**Then** uncommitted changes outside the leased story's module paths (or chore paths) cause a refusal (exit 3) listing the paths
**And** in `story-branch` mode it checks that the local integration branch is not behind its remote and that the story branch's merge-base is within `max_integration_staleness_commits`, and a story branch being behind the integration branch is not itself an error
**And** in `trunk` mode it checks the current branch against its remote
**And** blocking validation findings cause a refusal
**And** the message names the exact Git command to fix the state.

### Story 3.8: Git Hook Shims & `qdev hook`

As a developer,
I want hooks installed as tiny shims that call the binary,
So that hook logic is cross-platform and versioned with qdev.

**Acceptance Criteria:**

**Given** `qdev install hooks`
**When** it runs
**Then** `pre-commit`, `pre-push`, and `prepare-commit-msg` shims are written as in `docs/compliance-and-safety.md` §6, existing non-qdev hooks are preserved by chaining, and the shims work under Git for Windows
**And** `qdev hook pre-commit` runs `hygiene check --diff`, `validate --changed`, and, when `[hygiene] secret_patterns` is configured, a pattern scan of staged scratchpad and evidence files (NFR-404)
**And** `qdev hook pre-push` runs `preflight`
**And** `qdev doctor` reports missing or outdated shims and `--fix` rewrites them.

### Story 3.9: Hygiene Linter (Lint and Report)

As a compliance manager,
I want narrative memoir comments reported with file, line, and rule,
So that source stays readable and the agent fixes its own output.

**Acceptance Criteria:**

**Given** `[hygiene]` configuration and source in the configured languages
**When** I run `qdev hygiene check --diff`
**Then** comments are extracted with a language-aware tokenizer for Rust, Swift, and Python (line, block, and doc comments), not by regex over raw source
**And** findings are produced for blocks over `max_inline_comment_lines`, matches of `forbid_patterns`, and story banners or review-round narratives, each with `file:line`, rule id, and excerpt; exit 1 on any finding
**And** compact citations matching `citation_pattern` are never flagged
**And** no file is modified; `--fix` exits 2 with a message that automatic fixing is deferred.

### Story 3.10: Impact Analysis

As a reviewer,
I want the blast radius of a change,
So that I know which stories, requirements, and gates to re-check.

**Acceptance Criteria:**

**Given** a story ID or a set of paths
**When** I run `qdev impact E12S4` or `qdev impact --paths crates/bridge/src/lib.rs`
**Then** it resolves modules via the registry, then reports: stories with those `target_modules` in `in-progress` or `review`, requirements reached via `traces_to` and `verifies`, hazards via `mitigates`, ADRs via `governed_by`, and gates whose `verifies` overlap
**And** dependents via reverse `depends_on` and `extends` are listed with depth
**And** citations `[E…]`, `[AD-…]`, `[DW-…]` found in the changed files are included as `cited_entities`.

### Story 3.11: Opt-In Commit Messages

As an agent,
I want commit messages drafted from the leased story and recent scratchpad entries,
So that history is contextual without extra effort, only when the team opted in.

**Acceptance Criteria:**

**Given** `[commit_messages] enabled = true`
**When** `git commit` fires `prepare-commit-msg` with an empty message
**Then** `qdev hook prepare-commit-msg` writes a message in the configured format (`simple` or `conventional`) with the story ID as scope, the story title, and the last three scratchpad entries of kind `decision` or `tradeoff` as body lines
**And** it never overwrites a message the user supplied
**And** with `enabled = false` (default) the hook is a no-op.

### Story 3.12: SOUP Audit & SBOM

As a compliance manager,
I want dependency audits and an SBOM recorded per release,
So that SOUP evidence exists.

**Acceptance Criteria:**

**Given** `[soup]` commands
**When** I run `qdev soup audit --release 0.1.0`
**Then** the configured audit and deny commands run through the gate runner, their results are recorded as evidence under `_workspace`, and each reported dependency is upserted under `docs/state/soup/` with licence and CVE status where the output provides them (cargo-audit JSON parser in v1; generic pass-through otherwise)
**And** `qdev soup sbom` runs the SBOM command and stores the artefact path in the release record
**And** `qdev review sprint` includes the SOUP summary.

### Story 3.13: Epic & Sprint Reviews, Sprint Close

As a release manager,
I want end-of-epic and end-of-sprint reports and a safe close,
So that baselines, traceability, and anomalies are captured and open work carries forward.

**Acceptance Criteria:**

**Given** an epic E12
**When** I run `qdev review epic E12`
**Then** it reports story statuses, open DW originating from its stories with risk levels, gate evidence coverage per story, and unclassified debt, and exits 1 if any story is `done` without evidence for its bound gates
**Given** an active sprint 5
**When** I run `qdev review sprint 5`
**Then** it runs gates marked `on_transition = ["sprint_close"]`, and emits `rtm.md` (requirement → stories → gate runs → evidence paths) and `anomalies.md` (open DW grouped by risk with rationale) under `docs/state/releases/<version>/`
**When** I run `qdev sprint close 5 --status completed --carry-over 6`
**Then** it refuses (exit 3) while any `unacceptable` DW lacks rationale or any story in the sprint is `in-progress` with a live lease; otherwise it writes the baseline snapshot (counts, ratchet values, commit SHA, SOUP summary), sets the sprint `completed`, and assigns every non-done story to sprint 6 with `carried_from: 5`
**And** `--status paused|abandoned` skips the baseline and records a `DEC-` with the reason.

---

## Epic 4: AI Context & Integration

### Story 4.1: `qdev context` Projection

As an agent,
I want one command that produces exactly the context I need for a phase within a token budget,
So that I never load the whole repository.

**Acceptance Criteria:**

**Given** a story E12S4
**When** I run `qdev context E12S4 --phase develop --budget 1200 --json`
**Then** the payload contains, in priority order: the story spec body, constraints with IDs including inherited ones, resolved module paths, excerpts of governing ADRs (the `Rule` and `Prevents` sections), linked requirement titles, a scratchpad summary, bound gates, and the hygiene directive from `docs/compliance-and-safety.md` §1
**And** `--phase specify` and `--phase review` include the sections in `docs/architecture.md` §12
**And** when over budget, lowest-priority sections are truncated first and the payload lists what was truncated
**And** `--stats` reports the estimated token count per section using a documented estimator, and the reference fixture stays under 1,200 tokens for `develop`
**And** the output is deterministic and available as Markdown (`--format md`) for skills.

### Story 4.2: Attributed Rejections

As an agent,
I want every refusal to cite the exact constraint, gate, or policy,
So that I never guess what went wrong.

**Acceptance Criteria:**

**Given** any refusal path (gate fail, scope violation, lease conflict, missing justification, cross-team edit, blocked dependency)
**When** it occurs
**Then** the error envelope's `details` includes at least one of `constraint_id`, `gate_id`, `policy`, `blocking_ids`, or `holder`, plus the human-readable text of the rule
**And** a conformance test iterates every error code in the binary and asserts the attribution fields are present
**And** the `develop` skill instructs the agent to quote the cited ID when reporting a failure to the human.

### Story 4.3: Hygiene Directive & Citation Template

As a compliance manager,
I want the hygiene rules injected into every agent context,
So that agents write compact citations from the start.

**Acceptance Criteria:**

**Given** `qdev context --phase develop|review`
**When** the payload is built
**Then** it includes the directive text and a per-language citation template derived from `[hygiene]` configuration
**And** the `review` payload includes the hygiene findings for the current diff so the reviewer can enforce them.

### Story 4.4: Skill Generation & Install

As a development team,
I want skills generated from the command catalog and installed per editor,
So that skills cannot drift from the binary.

**Acceptance Criteria:**

**Given** `qdev install skills --claude|--cursor|--agents`
**When** it runs
**Then** files are written to the locations in the CLI reference §7, generated from the same command registry that drives `clap`, and stamped with the binary version
**And** `qdev doctor` reports installed skills whose stamp differs from the binary
**And** generated skills invoke `qdev … --json` and `qdev context`, never assemble context from files directly.

### Story 4.5: MCP Server

As an agent host,
I want qdev's operations as MCP tools over stdio,
So that agents call them natively.

**Acceptance Criteria:**

**Given** `qdev mcp serve`
**When** a client connects
**Then** the tools listed in the CLI reference §7 are advertised with JSON Schemas generated from Story 1.13 and return the same payloads as the CLI
**And** every tool runs with `Interactivity::NonInteractive`, so refusals surface as structured errors
**And** `qdev install mcp --claude|--cursor` registers the server, and `qdev doctor` verifies the registration and a handshake.

### Story 4.6: The Five Skills

As a developer,
I want `/qdev`, `/qdev-plan`, `/qdev-create-story`, `/qdev-develop`, and `/qdev-review`,
So that the full loop runs from a chat interface.

**Acceptance Criteria:**

**Given** installed skills
**When** I run `/qdev`
**Then** it runs `qdev --json` and renders the pulse
**When** I run `/qdev-plan`
**Then** it uses the Structured Multi-Perspective Synthesis template to draft PRD, requirements, ADRs, and hazards via `qdev create`
**When** I run `/qdev-create-story E12`
**Then** it calls `qdev context E12 --phase specify`, drafts a story with acceptance criteria and constraints, creates it via `qdev create story` and `qdev constraint add`, and transitions it to `ready`
**When** I run `/qdev-develop E12S4`
**Then** it runs `qdev preflight`, `qdev claim`, `qdev context --phase develop`, works within scope appending to the scratchpad, runs `qdev gate run --for-transition review`, and transitions to `review`
**When** I run `/qdev-review E12S4`
**Then** it uses `qdev context --phase review`, audits the diff against acceptance criteria and constraints, and either transitions to `done` or back to `in-progress` with a justification
**And** the `[models]` hints are surfaced in each skill's frontmatter.

### Story 4.7: Doctor: Environment, Skills, MCP, Hooks, Leases

As a developer,
I want one health command covering everything qdev depends on,
So that setup problems are obvious.

**Acceptance Criteria:**

**Given** `qdev doctor`
**When** it runs
**Then** the output matches the CLI reference §7: Git state, cache, modules, gates with executable resolution and local skips, hook shims, skills version stamps, MCP registration, and stale leases
**And** `--fix` rewrites hook shims, regenerates outdated skills, and rebuilds a corrupt cache after confirmation or `--yes`
**And** `--json` returns per-check status for CI.

### Story 4.8: Structured Multi-Perspective Synthesis Template

As a planner,
I want a configurable four-heading synthesis template,
So that planning has multiple perspectives without persona role-play.

**Acceptance Criteria:**

**Given** the `/qdev-plan` and `/qdev-create-story` skills
**When** they prompt a model
**Then** the prompt requires the four headings in `docs/architecture.md` §4 and rejects output missing any heading
**And** the template lives in `qdev.toml` under `[synthesis]` with the shipped default, so domain-specific headings (for example clinical value) are configuration.

### Story 4.9: `qdev graph` Rendering Options

As a planner,
I want the dependency DAG filtered and annotated,
So that parallel tracks are visible.

**Acceptance Criteria:**

**Given** Story 1.10's DOT output
**When** I run `qdev graph --dot --sprint 5 --highlight-critical-path`
**Then** the graph is filtered to the sprint, blocked stories are visually distinct, leased stories show the holder, and the longest dependency chain is highlighted
**And** `--json` emits nodes and edges for other renderers.

### Story 4.10: Self-Hosting Handover

As the qdev team,
I want to finish building qdev with qdev,
So that the tool is exercised on a real project before release.

**Acceptance Criteria:**

**Given** Epics 1 and 2 are done and Epic 3 is at least at Story 3.6
**When** the handover runs
**Then** `qdev init` is executed in this repository, the remaining stories of Epics 3 and 4 are created as native entities with their constraints and relations, and `docs/bmad/` is moved to `docs/archive/bmad/`
**And** at least one remaining story is completed end-to-end via `/qdev-develop` and `/qdev-review` with evidence committed
**And** payload sizes from `qdev context --stats` for that story are recorded in the PRD success-metrics table.
