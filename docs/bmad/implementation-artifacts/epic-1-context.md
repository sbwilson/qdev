# Epic 1 Context: The Relational Storage & CLI Substrate

<!-- Generated from planning artifacts. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Establish the relational storage and CLI substrate that serves as qdev's foundation: a high-performance, offline-first developer tool where Git-tracked Markdown files with YAML frontmatter are the single source of truth and a local SQLite cache provides instant relational indexing. This epic delivers the two-crate architecture (`qdev-cli` and `qdev-core`), dual configuration loading, strict entity frontmatter schemas with collision-resistant IDs, a sub-30 ms incremental metadata sweep hydration engine, an advisory-locked atomic write path that preserves frontmatter comments, a deterministic JSON query engine, bidirectional relationship tracking with cycle detection, and workspace validation. Completing this epic guarantees all downstream workflow engines, verification gates, and AI agent skills interact with a robust, cross-platform, non-interactive CLI and storage layer.

## Stories

- Story 1.1: Workspace Scaffolding, JSON Envelope & Exit Codes
- Story 1.2: Dual Configuration Loader
- Story 1.3: `qdev init`
- Story 1.4: Entity File Format, Schemas & Attribution
- Story 1.5: Identifier Grammar & Allocation
- Story 1.6: SQLite Cache Schema & Migrations
- Story 1.7: Incremental Hydration Sweep
- Story 1.8: Write Path: Atomic Files, Locking, Frontmatter Patching
- Story 1.9: `get` / `list` Query Engine
- Story 1.10: Relations, DAG & Computed Blocked
- Story 1.11: `qdev validate`
- Story 1.12: `qdev sync` & Cache Diagnostics
- Story 1.13: `qdev schema`

## Requirements & Constraints

- **Single Source of Truth & Rebuildable Index**: Every entity is stored in Git as an individual Markdown file with YAML frontmatter. The local SQLite database serves strictly as a rebuildable index; wiping `.qdev/cache/` must incur zero data loss and regenerate an identical database.
- **Universal JSON & Strict Exit Codes**: Every command must support `--json`. Success payloads carry `schema_version`. Errors are emitted with a uniform structure (`{"error": {"code", "message", "details"}}`) to stdout in JSON mode. Process exit codes are standardized across all commands: `0` (success), `1` (logical failure / validation findings), `2` (usage error), `3` (refused by policy or needs confirmation), `4` (infrastructure failure / timeout / corruption), and `5` (concurrency conflict / lock timeout / version mismatch).
- **Boot Latency & Performance**: The incremental hydration sweep on command boot must complete in ≤ 30 ms for a 1,000-entity repository with one modified file on standard hardware.
- **Hydration Resilience & Error Containment**: Branch switching, checkouts, and external file changes must be detected and synced automatically. Files with merge conflict markers (`<<<<<<<`) produce a file-specific `merge_conflict` validation finding without halting hydration of valid files. Schema-invalid files generate `schema_violation` findings while retaining their previous cache rows marked as stale.
- **Comment-Preserving Frontmatter Mutations**: Programmatic updates to entity frontmatter must apply line-based patches that preserve existing comments, key ordering, and blank lines byte-for-byte outside the modified fields.
- **Audited Attribution & Optimistic Concurrency**: Every entity update records author type (`human` or `agent`) and author ID (`developer_id`), incrementing the entity `version`. Mutations support optimistic locking via `--if-version <N>`, failing with exit code 5 on mismatches.
- **Isolated Query Projections**: Query commands return only the requested entity and immediate metadata by default. Extended relations, constraints, and scratchpad ledgers are omitted unless explicitly requested via expansion flags.
- **Dual Configuration Model**: Project-wide policies in committed `qdev.toml` merge with gitignored developer preferences in `.qdev.local.toml`. Local scalar keys override project keys individually; array collections (`[[modules]]`, `[[gates]]`) override the entire list rather than merging element-wise. Developer identity falls back to `git config user.email` if missing.
- **Workspace Integrity Validation**: The validation engine identifies dangling relations, circular dependencies (`depends_on` cycles), duplicate planning IDs across branches, merge conflicts, schema violations, orphan deferred work, and missing risk rationale for safety mitigations.
- **Zero External Runtimes & Zero Telemetry**: The CLI operates as a standalone binary with no runtime dependencies on Python, Node, or JVM, and performs zero unsolicited network calls.

## Technical Decisions

- **Two-Crate Architecture**: Strict workspace boundary between `qdev-cli` and `qdev-core`. `qdev-cli` handles argument parsing via clap, terminal interaction, human-readable table rendering, and JSON output emission. `qdev-core` implements domain models, the `Store` trait, SQLite operations, hydration, validation, and write logic without any dependencies on clap or terminal crates, and without writing to stdout or reading stdin.
- **Synchronous SQLite (`rusqlite`)**: Database operations use synchronous `rusqlite` with bundled SQLite in `qdev-core`. No async runtimes (`tokio`) or ORMs are used in core, minimizing binary footprint and command boot overhead.
- **WAL Mode & Advisory File Locking**: SQLite connections enable WAL mode with `busy_timeout = 5000ms`. File writes coordinate multi-process concurrency using an advisory lock on `.qdev/cache/write.lock` with a 5-second timeout, followed by an atomic write-temp-file-then-rename sequence.
- **Incremental Metadata Sweep**: Hydration inspects file `mtime` and size against `sync_state`. Only modified files are hashed, and only files whose content hash changed are parsed. Cache rows touched by `qdev` writes are marked dirty to guarantee re-parsing regardless of filesystem timestamps.
- **Cache Rebuild Over Migrations**: Cache schema changes trigger an automatic clean drop-and-rebuild from source Markdown files rather than schema migrations.
- **Sprint-Independent ID Grammar**: Planning entity IDs never encode sprint numbers to eliminate citation rot across sprints.
  - Epics: `E{n}`
  - Stories: `E{n}S{m}`
  - ADRs, Requirements, Hazards, PRDs: `AD-{n}`, `FR-{n}`, `NFR-{n}`, `HAZ-{n}`, `PRD-{n}`
  - Negative constraints: `{owner}/NG-{k}` (no-go) and `{owner}/RH-{k}` (rabbit hole)
  - Execution-time entities: `DW-{hex4+}` (deferred work) and `DEC-{hex4+}` (decisions) with collision-expanding hexadecimal suffixes.
- **Relational DAG & Computed State**: Entity relations (`depends_on`, `extends`, `supersedes`, `traces_to`, `governed_by`, `mitigates`, `closes_dw`, `verifies`) are indexed in a relational table. Stories dynamically evaluate `blocked = true` when any `depends_on` target is not `done`.
- **Cross-Platform Construction**: First-class support for macOS, Linux, and Windows. Process management, path handling, and locking avoid POSIX-only APIs, signals, or external shell scripts.

## UX & Interaction Patterns

- **Non-Interactive First & Closed-Fail Policy**: Every command is fully operational non-interactively. With `--non-interactive`, `QDEV_NONINTERACTIVE=1`, or a non-TTY stdin, prompts are forbidden. Commands that require input fail closed with exit code 3 (`needs_confirmation`) and structured error details identifying the missing flag.
- **Interactive Scaffolding Wizard**: Running `qdev init` on a TTY prompts for project name, developer ID, and teams, setting up `.qdev.local.toml`, directory structures, and `.gitignore`. In non-interactive mode, all configuration is accepted via CLI flags.
- **Dual Output Formatting**: Query, sync, validation, and diagnostic commands emit compact human-readable terminal tables by default when run interactively, and switch to deterministic, byte-identical JSON when `--json` is supplied.
- **Guided ID Collision Resolution**: Validation detects duplicate planning IDs caused by parallel branches and provides an interactive `--fix-ids` workflow to renumber IDs and rewrite references across entity files and code citations. In non-interactive mode, `--fix-ids` requires `--yes` to proceed.

## Cross-Story Dependencies

- **Story 1.1 → All Stories**: Scaffolds `qdev-cli` and `qdev-core`, defining the output JSON envelope, error taxonomy, exit codes, and interactivity detection needed by every subcommand.
- **Story 1.2 → Stories 1.3, 1.6, 1.8**: Dual config parser provides storage directory paths, module registries, and developer identity fallbacks.
- **Story 1.3 → Stories 1.4, 1.6**: `qdev init` bootstraps the directory hierarchy (`docs/specs/*`, `docs/state/*`, `.qdev/cache/`) and gitignore configuration.
- **Stories 1.4 & 1.5 → Stories 1.6, 1.7, 1.8, 1.10**: Frontmatter schemas, ID classification, and allocation logic govern entity persistence, parsing, and relation extraction.
- **Story 1.6 → Stories 1.7, 1.9, 1.10, 1.12**: SQLite schema definitions and the `Store` trait establish the queryable database layer for hydration and queries.
- **Story 1.7 → Stories 1.9, 1.10, 1.11, 1.12**: Incremental hydration sweep reads Markdown files into the cache, unblocking entity queries, relation graph building, and validation.
- **Story 1.8 → Stories 1.10, 1.11**: The atomic write path and advisory locking provide mutation primitives for relation management (`qdev relate`) and ID fixes (`qdev validate --fix-ids`).
- **Story 1.9 & 1.10 → Story 1.11**: Query engine and DAG dependency cycle detection directly feed `qdev validate`.
- **Downstream Epic Dependencies**:
  - **Epic 2 (Workflow & Governance)**: Relies on Epic 1's atomic write path, frontmatter patching, `Store` trait, relation indexing, and computed `blocked` state.
  - **Epic 3 (Gates & Evidence)**: Relies on the universal JSON envelope, exit codes, and schema validation.
  - **Epic 4 (AI Integration)**: Relies on isolated entity projections, deterministic JSON queries, and schema exports.
