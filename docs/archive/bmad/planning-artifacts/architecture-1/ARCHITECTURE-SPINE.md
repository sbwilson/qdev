---
title: "qdev Architecture Spine"
status: final
created: 2026-09-05
updated: 2026-09-06
---

# qdev Architecture Spine

## Paradigm
**Offline-First Local Relational CLI Tool.** `qdev` is a single-binary, cross-platform Rust executable that orchestrates hierarchical project management and verification gates. It acts as a fast relational SQLite index over structured, human-readable Markdown source files that live in Git.

This spine is the authoritative statement of architectural decisions. `docs/architecture.md` elaborates on it; where the two differ, this file wins and the other must be corrected.

## Architectural Decisions (ADs)

### AD-1: Strict Two-Crate Workspace
* **Rule**: The workspace is split into `qdev-cli` (clap arguments, stdin/stdout, text and JSON formatting, interactive prompts) and `qdev-core` (entity model, hydration, state machine, gates, Git inspection). `qdev-core` never writes to stdout or reads stdin.
* **Binds**: Project layout and dependency tree.
* **Prevents**: CLI formatting and I/O side effects from leaking into testable, headless core routines.

### AD-2: Synchronous SQLite via `rusqlite`
* **Rule**: Database interaction uses synchronous `rusqlite` with the bundled SQLite. No async runtimes (`tokio`) or ORMs in the core.
* **Binds**: Database access layer.
* **Prevents**: Binary bloat and complexity from async overhead for short-lived CLI commands.

### AD-3: Markdown Source of Truth, SQLite as Rebuildable Cache
* **Rule**: The source of truth is one Markdown file with YAML frontmatter per entity, committed to Git. SQLite is exclusively a local, gitignored, rebuildable index. Writes hit the file first, then update the cache. Deleting the cache loses nothing.
* **Binds**: Storage architecture, Git strategy, and every read and write path.
* **Prevents**: Merge-conflict-prone monolithic ledgers, binary diffs, and lock-in.

### AD-4: WAL Concurrency & Atomic Transactions
* **Rule**: SQLite connections use WAL mode with `busy_timeout = 5000ms`. All cache mutations are short atomic transactions. No process holds a write lock while waiting on external I/O or user input. File writes use write-temp-then-rename under an advisory lock on `.qdev/cache/write.lock`.
* **Binds**: Connection initialisation, transaction lifecycle, file write path.
* **Prevents**: Corruption when an agent and a human run `qdev` simultaneously.

### AD-5: Sub-process Gates with a Result Contract
* **Rule**: Gates are external executables configured in `qdev.toml` and typically stored in `.qdev/gates/`. The binary ships no project-specific verifiers. A gate communicates via exit code and, optionally, a JSON result document written to stdout or to the path in `$QDEV_RESULT_FILE`. Without a result document, qdev falls back to exit code plus a bounded tail of stderr. Output adapters for common runners are optional and selected by configuration.
* **Binds**: Gate execution model and the gate result schema.
* **Prevents**: Hardcoding Rust, Swift, or medical-specific checks into a generic binary.

### AD-6: Directory Metadata Sweep Hydration
* **Rule**: On every CLI boot, qdev sweeps the `mtime` and size of all entity files and re-parses only those whose metadata changed since the last sync, confirming with a content hash before upserting. Any write performed by qdev itself invalidates that entity's cache row explicitly. No background daemons and no lazy hydration.
* **Binds**: Boot sequence and file system layer.
* **Prevents**: Slow boots, stale reads after `git checkout`, and `mtime` illusion bugs on coarse filesystems.

### AD-7: Sprint-Free, Collision-Resistant Identifiers
* **Rule**: Planning entities use sequential human-readable IDs that never encode a sprint: epics `E{n}`, stories `E{n}S{m}`, ADRs `AD-{n}`, requirements `FR-{n}` / `NFR-{n}`, hazards `HAZ-{n}`, PRDs `PRD-{n}`. Execution-time entities created concurrently by agents use short random hash suffixes: deferred work `DW-{4+ hex}`, decisions `DEC-{4+ hex}`. IDs are immutable once committed. `qdev validate` detects collisions.
* **Binds**: ID grammar, file naming, citation format.
* **Prevents**: Renaming on sprint carry-over and silent ID collisions across branches and worktrees.

### AD-8: Sprints Are Assignments, Not Ownership
* **Rule**: Epics and stories belong to the functional tree only. Sprint membership is a many-to-one assignment recorded in the sprint file, with carry-over history. Multiple sprints may be active.
* **Binds**: Sprint model, carry-over, release baselining.
* **Prevents**: Story renames and epic pinning when work spans sprints.

### AD-9: Story Leases Instead of Access Control
* **Rule**: Scope is enforced by leases. `qdev claim story <id>` records a lease (holder, worktree, time) and issues a session token via `QDEV_SESSION`. Mutations outside the leased story's subtree, or to entities owned by another team, require `--override --justification`, which is logged as a decision. There is no role-based access control; identity is advisory and audit-oriented.
* **Binds**: Governance, multi-agent concurrency, worktree orchestration.
* **Prevents**: Two agents claiming one story, silent cross-team edits, and a spoofable RBAC that provides false assurance.

### AD-10: Module Registry With Paths
* **Rule**: Every module referenced by `target_modules`, impact analysis, preflight scope, or segregation gates is declared in `qdev.toml` as `[[modules]]` with an `id`, `paths` globs, an optional `layer`, and `may_depend_on`.
* **Binds**: Boundary enforcement, impact analysis, preflight.
* **Prevents**: Module names with no enforceable meaning.

### AD-11: Scratchpads Are Separate, Append-Only, Committed Files
* **Rule**: Each story's scratchpad is a JSONL file under `docs/state/scratch/<story-id>.jsonl`, committed to Git, hydrated into the cache, and returned only on explicit request. Story spec files never contain scratchpad content. Projections include a bounded summary, not the ledger.
* **Binds**: Scratchpad storage and every projection.
* **Prevents**: Story files bloating and scratchpads being lost on clone.

### AD-12: Every Command Is Non-Interactive Capable
* **Rule**: Every prompt has a flag equivalent. With `--non-interactive` or `QDEV_NONINTERACTIVE=1`, or when stdin is not a TTY, qdev never prompts; it fails closed with a structured error naming the flag that would have answered the prompt.
* **Binds**: CLI surface, `init`, overrides, confirmations.
* **Prevents**: Agents and CI hanging on stdin.

### AD-13: Stable JSON Envelope and Exit Codes
* **Rule**: Every command supports `--json`. Success payloads carry `schema_version`. Errors are emitted as `{"error": {"code", "message", "details"}}` on stdout with a non-zero exit. Exit codes: `0` success, `1` logical failure, `2` usage error, `3` refused by policy, `4` infrastructure failure, `5` conflict. `qdev schema` prints the JSON Schema for any payload.
* **Binds**: All output paths, skills, MCP tools, CI consumers.
* **Prevents**: Agents scraping text and CI misreading failures.

### AD-14: Cross-Platform by Construction
* **Rule**: macOS, Linux, and Windows are first-class. Process termination uses platform APIs, not signals. Git hooks are two-line shims that invoke `qdev hook <name>`; hook logic lives in the binary. File locks use OS-appropriate advisory locking. The binary is self-contained with no external runtime; it is not claimed to be statically linked on macOS.
* **Binds**: Gate runner, hooks, locking, release packaging.
* **Prevents**: POSIX assumptions that break NFR-101.

## Deferred
* **State machine implementation**: formal crate versus `match` statements.
* **Signed evidence**: minisign or sigstore signing of evidence bundles, once bundles exist.
* **Tool validation package**: self-test and validation report for regulated users.
