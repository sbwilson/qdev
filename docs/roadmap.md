# qdev v1 Scope & Execution Roadmap

> What v1 delivers, what is deferred, how the four epics sequence the work, and the edge cases the implementation must handle. The story-level plan is [epics.md](bmad/planning-artifacts/epics.md).

## Table of Contents

- [v1 Scope](#roadmap-scope)
- [Execution Plan by Epic](#implementation-plan)
- [Self-Hosting Handover](#handover)
- [Edge Cases the Implementation Must Handle](#edge-cases)

---

<a id="roadmap-scope"></a>

## 1. v1 Scope

### In scope

**Substrate**
- Two-crate Rust workspace; self-contained binary for macOS, Linux, Windows.
- Markdown-with-frontmatter entities, one file each; SQLite cache with metadata-sweep hydration.
- Full entity catalog: PRD, requirement, epic, story, ADR, hazard, constraint, sprint, release, deferred work, decision, scratchpad, gate, gate run, SOUP.
- Sprint-free IDs; hash-suffixed IDs for execution-time entities; collision detection.
- Relations with validation and cycle detection; `qdev validate`; `qdev graph --dot`.
- JSON envelope, exit codes, `qdev schema`; non-interactive mode everywhere.
- Dual configuration; module registry with paths and layers.

**Workflow**
- State machine with lifecycle hooks, justified backward transitions, computed `blocked`.
- Story leases and overrides; cross-team overrides logged as decisions.
- Constraints as entities with inheritance; attributed rejections.
- Scratchpads, decision ledger, deferred work with ISO 14971 risk fields.
- Sprints as assignments; multiple active sprints; carry-over without renames.
- Chores with path allowlists.
- `qdev next` and the root pulse.

**Gates, evidence, Git**
- Sub-process gate engine: env inheritance, timeouts, platform-native kill, bounded buffers, result contract, adapters for cargo and xcodebuild, failure taxonomy.
- Ratchets with committed baselines.
- Evidence bundles; traceability matrix and residual anomaly report at sprint close.
- Built-in data-driven gates: scope, hygiene (lint-only), module dependency direction.
- Preflight guard; hook shims; opt-in commit messages; `qdev impact`.
- SOUP audit and SBOM wrappers.

**Agent integration**
- `qdev context` projection with budgets and `--stats`.
- Skills for Claude Code, Cursor, agent directories: `/qdev`, `/qdev-plan`, `/qdev-create-story`, `/qdev-develop`, `/qdev-review`.
- `qdev mcp serve`.
- `qdev doctor`.

### Deferred to v2+

- Worktree-per-story orchestration (`qdev run --parallel`) built on leases.
- Signed evidence chain; tool validation package.
- Hygiene `--fix` with diff preview.
- Token accounting per phase from agent transcripts.
- Gate packs per ecosystem distributed outside the binary.
- Retrospective ingestion and velocity metrics.
- Web dashboard, Jira/Linear sync, PDF dossier, Postgres backend.

---

<a id="implementation-plan"></a>

## 2. Execution Plan by Epic

The plan is the four epics in `epics.md`, in order. Each epic ends with a demonstrable deliverable.

| Epic | Delivers | Exit criterion |
| --- | --- | --- |
| **E1 Relational Storage & CLI Substrate** | Workspace, config, `init`, entity formats and IDs, cache and hydration, write path, `get`/`list`, relations, `validate`, `schema`, `doctor` (cache portion) | `qdev create story` → hand-edit → `qdev get story --json` round-trips with validation findings, in under 30 ms on 1,000 fixtures |
| **E2 Workflow & Governance Engine** | State machine and hooks, leases, constraints, scratchpads, decisions, deferred work, sprints, chores, module registry, `next`, pulse | Two concurrent agents in separate worktrees cannot claim the same story; a backward transition without justification is refused with a structured error |
| **E3 Gates, Evidence & Git** | Gate engine and result contract, ratchets, evidence, transition-bound gates, preflight, hooks, hygiene lint, impact, commit messages, SOUP, epic and sprint reviews | `qdev transition story X review` runs bound gates, writes evidence, and blocks on a cited constraint violation; `qdev sprint close` emits RTM and anomaly report |
| **E4 AI Context & Integration** | `context`, attributed rejections, skill generation, MCP server, the five skills, `doctor` (skills/MCP), `graph` | A full story executed end-to-end through `/qdev-develop` and `/qdev-review` in Claude Code with payloads under budget |

---

<a id="handover"></a>

## 3. Self-Hosting Handover

BMAD is used only to bootstrap qdev. It is not a supported input format and there is no importer. Once E1 and E2 are complete:

1. Run `qdev init` in this repository.
2. Re-enter the remaining E3 and E4 stories as native qdev entities (a one-time manual or scripted step).
3. Continue development under qdev, retiring `docs/bmad/` to an archive directory.

This is the first real dogfood test of the tool.

---

<a id="edge-cases"></a>

## 4. Edge Cases the Implementation Must Handle

| # | Issue | Resolution |
| --- | --- | --- |
| 1 | Multiple active sprints (feature + patch) | Sprint assignments, `--sprint`, `default_sprint` fallback (**AD-8**) |
| 2 | Frontmatter round-tripping loses comments | Line-based frontmatter patching; never re-serialise the whole YAML document |
| 3 | Concurrent agents share a working directory | Leases record the worktree; `qdev next` skips leased stories; worktree orchestration in v2 |
| 4 | Gate commands lack environment | Inherit parent env, then apply `[environment]` |
| 5 | Coarse `mtime` filesystems | Size and hash confirmation; qdev's own writes mark rows dirty (**AD-6**) |
| 6 | ID collisions across branches | Hash-suffixed execution IDs; `validate` detects planning-ID duplicates and offers guided renumber (**AD-7**) |
| 7 | Story branch always "behind" integration | Freshness measured at merge-base with a staleness limit, not raw behind-count |
| 8 | Hand edits to entity files | Allowed; validation findings, never lock-out (**NFR-202**) |
| 9 | Windows | Job objects for kill, hook shims calling the binary, OS advisory locks (**AD-14**) |
| 10 | Agent stuck on a prompt | Non-interactive mode fails closed naming the flag (**AD-12**) |
| 11 | Gate hangs or floods output | Timeout with native kill; 1 MB ring buffers; `infra` status tells agent to halt |
| 12 | Scratchpad bloats context | Separate JSONL file; projection includes a bounded summary only (**AD-11**) |
| 13 | Memoir comments misdetected | Lint-and-report in v1; the agent rewrites; `--fix` deferred |
