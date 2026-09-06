---
title: "qdev PRD"
status: final
created: 2026-09-05
updated: 2026-09-06
---

# qdev PRD

## 1. Vision & Purpose
To eliminate markdown context sprawl and AI hallucination in complex software development by replacing flat planning files with a relational, SQLite-indexed workflow engine over Git-native Markdown. `qdev` synthesises BMAD's hierarchical decomposition with Shape Up's negative constraints (appetites, rabbit holes, no-gos), keeping AI coding agents and human developers aligned, gated, and traceable for high-integrity software such as medical devices.

`qdev` is not itself SaMD. It is a development tool whose outputs are designed to support, not guarantee, IEC 62304 and ISO 14971 processes.

## 2. Users & Personas

| Persona | Needs |
| --- | --- |
| **Lead developer / architect** | Plan epics and stories, define constraints and ADRs, review agent output, close sprints |
| **AI coding agent** (Claude Code, Cursor, Gemini) | Receive a small, exact context payload; know the boundaries; report progress; be told precisely why something was rejected |
| **Compliance / release manager** | Traceability from requirement to story to gate evidence; residual anomaly list; SOUP inventory |
| **CI pipeline** | Deterministic JSON, stable exit codes, no prompts |

## 3. Success Metrics

| Metric | Target |
| --- | --- |
| Median tokens in a `develop` context payload | ≤ 1,200 |
| Median tokens in a `review` context payload | ≤ 2,500 |
| Cache hydration on boot, 1,000 entities, one changed | ≤ 30 ms |
| Citation rot after 10 sprints | 0 renamed IDs |
| Gate pass receipt size | ≤ 2 lines |
| Commands that can block on stdin in non-interactive mode | 0 |

## 4. Functional Requirements

### 4.1 Relational Graph Engine (FR-100s)
- **FR-101 Markdown source of truth, SQLite index**: One Markdown file with YAML frontmatter per entity, committed to Git; a local gitignored SQLite cache that can be deleted and rebuilt at any time.
- **FR-102 Relationship hydration**: Cross-entity relationships (`depends_on`, `extends`, `supersedes`, `traces_to`, `closes_dw`, `verifies`, `mitigates`) are resolved on hydration, validated, and queryable in both directions.
- **FR-103 Universal JSON output**: Every command supports `--json` with a versioned schema, a stable error envelope, and documented exit codes.
- **FR-104 Entity catalog**: PRD, Requirement, Epic, Story, ADR, Hazard, Sprint, Release, Deferred Work, Decision, Constraint, Scratchpad entry, Gate, Gate run, SOUP dependency.
- **FR-105 Validation**: `qdev validate` reports dangling relations, dependency cycles, ID collisions, schema violations, orphan deferred work, and missing risk rationale.

### 4.2 Shape Up Guardrail System (FR-200s)
- **FR-201 Negative constraint entities**: Appetite, target modules, rabbit holes, and no-gos are first-class entities with IDs, attached to epics or stories, inherited downward, and injected into every context payload.
- **FR-202 Story state machine**: `draft → ready → in-progress → review → done`, with terminal `superseded` and `abandoned`. `blocked` is computed from unmet dependencies. Forward transitions are validated by lifecycle hooks.
- **FR-203 Justified backward transitions**: A story may move backward (for example `review → in-progress`, or `in-progress → ready` for a re-spec pivot) only with a justification, which is appended to the scratchpad and the decision ledger.
- **FR-204 Story leases**: A developer or agent claims a story before mutating it. Mutations outside the leased subtree or across team ownership require an explicit, logged override.
- **FR-205 Module registry**: Modules are declared with path globs and layering so boundaries are enforceable.

### 4.3 Context & Code Hygiene (FR-300s)
- **FR-301 Context projection**: `qdev context <id>` produces a token-budgeted payload for a given phase, containing the story, inherited constraints, relevant ADR and requirement excerpts, a scratchpad summary, and hygiene directives.
- **FR-302 Scratchpads**: Append-only, committed, per-story ledgers stored separately from the story spec and returned only on request.
- **FR-303 Citation hygiene**: Agents are instructed to use compact citations (`// [E12S4]`, `// [AD-43]`); a linter reports narrative memoir comments with file, line, and rule.
- **FR-304 Attributed rejections**: Every rejection cites the ID and text of the violated constraint, gate, or policy.

### 4.4 Gates, Evidence & Git (FR-400s)
- **FR-401 Verification gates**: Configurable external gates bound to state transitions, with timeouts, a result contract, and a logical-versus-infrastructure failure taxonomy.
- **FR-402 Ratchets**: Gates that report a metric with a direction and a per-branch baseline that must not regress.
- **FR-403 Evidence bundles**: Each gate run writes a committed JSON evidence record keyed by story, gate, and commit.
- **FR-404 Epic and sprint reviews**: End-of-epic and end-of-sprint commands that audit stories, deferred work, and gates, emit a traceability matrix and residual anomaly report, and carry open work into the next sprint without renaming it.
- **FR-405 Git integration**: Preflight guard, hook shims, impact analysis, opt-in commit message generation.
- **FR-406 SOUP audit**: Wrap configured dependency audit and SBOM commands and record results per release.

### 4.5 Dual Interface (FR-500s)
- **FR-501 CLI binary**: UNIX-style CLI for humans, scripts, and CI.
- **FR-502 Skills**: Generated slash-command skills for Claude Code, Cursor, and Gemini-style agent directories.
- **FR-503 MCP server**: `qdev mcp serve` exposing the core operations as tools over stdio.
- **FR-504 Pulse and next**: A root status command and `qdev next` that deterministically select the next unblocked story.

### 4.6 Configuration & Governance (FR-600s)
- **FR-601 Dual configuration**: Committed `qdev.toml` and gitignored `.qdev.local.toml`.
- **FR-602 Multi-owner governance**: Owners are persons or teams; cross-team edits require a logged override.
- **FR-603 Sprints as assignments**: Sprint membership is recorded on the sprint, not in the story ID; multiple sprints may be active.
- **FR-604 Chores**: A fast-track for trivial changes constrained by a declared path allowlist, still subject to hygiene gates.

## 5. Non-Functional Requirements

### 5.1 Architecture & Deployment (NFR-100s)
- **NFR-101 Cross-platform**: macOS, Linux, Windows; self-contained binary; no signals, shell scripts, or POSIX-only locking in required paths.
- **NFR-102 Zero external runtimes**: No Python, Node, or JVM required to operate.
- **NFR-103 Concurrency**: Safe concurrent reads and writes from multiple `qdev` processes and worktrees.
- **NFR-104 Non-interactive**: Every command completes without stdin when requested or when stdin is not a TTY.

### 5.2 Resilience & State Sync (NFR-200s)
- **NFR-201 Hydration resilience**: Branch switches and pulls rebuild the cache correctly; Git conflict markers are reported as conflicts, not parse errors.
- **NFR-202 Human editability**: Entity files may be edited by hand; validation reports problems rather than locking entities out.

### 5.3 Token Efficiency & Latency (NFR-300s)
- **NFR-301 Context isolation**: Queries return only the requested entity and declared neighbours unless expansion is requested.
- **NFR-302 Boot latency**: ≤ 30 ms hydration for 1,000 entities with one change.

### 5.4 Integrity & Compliance Support (NFR-400s)
- **NFR-401 Attribution**: Every mutation records author type (human or agent) and author id.
- **NFR-402 Traceability**: Requirement → story → gate run → evidence is queryable and exportable.
- **NFR-403 Zero telemetry**: No network calls except those the user configures (remote Git, audit commands).
- **NFR-404 Secret and PHI hygiene**: Scratchpads and evidence are scanned by a configurable pattern gate before commit when enabled.

## 6. Out of Scope for v1
- Web dashboard, hosted database, Jira/Linear sync, PDF dossier generation.
- Importing or continuing existing BMAD projects. BMAD is used only to bootstrap qdev's own development.
- Automatic rewriting of source comments (lint-and-report only in v1).
- Cryptographic signing of evidence and formal tool validation (v2).

## 7. Risks
- **Over-claiming compliance**: mitigated by "designed to support" language and the requirements/evidence model.
- **Agent bypass of the CLI**: mitigated by validation on hydration and leases rather than unenforceable prohibitions.
- **ID collisions across branches**: mitigated by hash-suffixed execution-time IDs and `qdev validate`.
- **Token savings unproven**: mitigated by measuring payload sizes in `qdev context --stats`.
