# Epic 1 Context: The Relational Storage & CLI Substrate

<!-- Compiled from planning artifacts. Edit freely. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Epic 1 builds the foundation everything else in qdev stands on: a two-crate Rust workspace whose CLI already speaks the final JSON envelope and exit codes, a dual configuration loader, strict hand-editable Markdown entity formats with embedded JSON Schemas, a collision-resistant identifier grammar, a rebuildable SQLite cache with an incremental hydration sweep, an atomic and attributed write path, and the query, relation, and validation surfaces built on top. It matters because every later epic — workflow, gates, and AI context — assumes a stable storage contract and a stable output contract; changing either later would invalidate work already done. The epic is complete when a developer can create a story, hand-edit the file, and read it back as validated JSON in under 30 ms on a 1,000-entity fixture.

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

- Markdown with YAML frontmatter is the only source of truth, one file per entity, committed to Git. SQLite is a local, gitignored index that can be deleted and rebuilt at any time with no loss.
- Entity files are meant to be edited by hand. Bad input must produce reported findings, never a lockout and never a hard parse failure that stops the rest of the workspace from hydrating.
- Every command supports `--json`, every success payload carries a schema version, and every error is the same envelope on stdout with a documented exit code (success / logical failure / usage / refused by policy / infrastructure / conflict).
- Every command must complete without stdin when non-interactive mode is requested or stdin is not a TTY; a prompt that cannot be answered fails closed with a structured error naming the flag that would have answered it. Every prompt has a flag equivalent.
- macOS, Linux, and Windows are first-class and all three are exercised in CI. No signals, shell scripts, or POSIX-only locking on any required path; no external runtime (Python, Node, JVM) is needed to operate.
- Concurrent qdev processes across worktrees must read and write safely.
- No telemetry and no network calls beyond what the user explicitly configures; this is enforced by test.
- Every mutation records author type (human or agent) and author id, and bumps the entity version.
- Latency target: hydration of 1,000 entities with one change in ≤ 30 ms on the CI reference machine, measured by benchmark test.
- Queries return only the requested entity and its declared neighbours; expansion is opt-in. Bodies of related entities and scratchpad content are never included by default.
- JSON output must be byte-identical across runs for the same inputs.
- Traceability from requirement through story to relations must be queryable in both directions.

## Technical Decisions

- **Crate split.** `qdev-cli` owns clap, stdin/stdout, text and JSON formatting, and prompts; `qdev-core` owns the entity model, hydration, and all logic, and never touches stdout or stdin. `qdev-core` must not depend on clap or any terminal crate.
- **Database.** Synchronous `rusqlite` with bundled SQLite. No async runtime, no ORM. WAL mode, `busy_timeout = 5000`, short atomic transactions; no write lock is held across external I/O or user input.
- **Cache lifecycle.** The cache schema is versioned by pragma. A version mismatch triggers a full rebuild from files — never an in-place migration of cache data. A `Store` trait in core exposes every read and write later stories need, with SQLite as the only backend.
- **Hydration.** On every boot, sweep `mtime` and size of all entity files against recorded sync state, hash only the changed candidates, and re-parse only those whose hash changed. Rows a qdev write marked dirty are always re-parsed regardless of metadata. Removed files are purged. No daemons, no lazy hydration.
- **Degraded inputs.** Git conflict markers become a conflict finding scoped to that file; a schema-invalid file becomes a schema-violation finding with its previous cache row retained and flagged stale. Neither stops the sweep.
- **Write path.** Advisory lock on a single workspace lock file with a 5 s timeout, then write-temp-then-rename, then upsert the cache row and mark it dirty. Frontmatter edits are line-based patches: comments, key order, and blank lines outside the edited keys survive byte-for-byte. Optimistic concurrency via an expected-version flag; mismatch is a conflict. Body edits target exactly one named Markdown section.
- **Identifiers.** Planning IDs are sequential and never encode a sprint (epic, story, ADR, functional and non-functional requirement, hazard, PRD forms, plus nested constraint references). Execution-time IDs created concurrently by agents use short random hex suffixes that widen on collision. IDs are immutable once committed; allocation scans files rather than the cache; validate detects collisions. The default hygiene citation regex must match every ID form and nothing else.
- **Schemas.** JSON Schema documents for every entity kind and every payload type are embedded in the binary and printable. Each entity requires id, title where applicable, status, version, and created/updated attribution carrying both author type and id. Fixtures hold one valid and at least two invalid examples per kind, used by tests. Schemas round-trip against live output.
- **Relations.** Relations live in the owning entity's frontmatter and are stored in a relations table with source and target kinds checked against the allowed pairs. A dangling target is a finding, not a dropped relation; a dependency cycle is a finding naming the cycle. `blocked` on a story is computed from unmet dependencies, not stored.
- **Config.** A committed project file merges with a gitignored local file; local keys override project keys individually and array-of-table sections are replaced wholesale, never merged element-wise. The local file's absence is not an error, and developer identity falls back to the Git user email. Config sections later stories consume are parsed into typed structs now. Schema violations exit as usage errors naming the key and the file.
- **Doctor extensibility.** Diagnostics are registered through a registry in core so later epics add their own sections rather than editing one command.
- **Superseded decision to respect.** Cache-schema migration during init reports and applies the rebuild without confirming; the confirmation flag is accepted and inert. See `docs/bmad/implementation-artifacts/spec-init-cache-migration.md`.

## Cross-Story Dependencies

- 1.1 (envelope, exit codes, interactivity) underpins every other story in the epic; nothing else can be finished before its output contract exists.
- 1.2 feeds 1.3 (init writes the config it just learned to read) and supplies the module registry and gate config that 1.10, 1.11, and Epic 3 consume.
- 1.4 and 1.5 together define what hydration parses: 1.6 and 1.7 depend on both, and 1.13 prints the schemas 1.4 defines.
- 1.6 must land before 1.7, 1.8, 1.9, and 1.12; the `Store` trait it defines is the seam those stories write against.
- 1.8 is the only mutation path — 1.10's relate/unrelate and 1.11's id-renumber fix both go through it, as does story creation in 1.5.
- 1.9 and 1.10 both read hydrated state from 1.7; 1.9's payload includes the inherited constraints and computed `blocked` that 1.10 produces.
- 1.11 aggregates findings produced by 1.7 and 1.10 rather than re-deriving them; 1.12's doctor surfaces the same finding counts.
- Downstream: Epic 2's state machine, leases, and scratchpads build on the write path and the `Store` trait; Epic 3's gates consume the gate config parsed in 1.2 and the module registry from 1.2/1.10; Epic 4's context projection depends on 1.9's isolation guarantees and 1.4's constraint representation.
