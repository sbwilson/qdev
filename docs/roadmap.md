# qdev v1 Implementation Scope & Execution Roadmap

> Seven-phase milestone plan for v1 MVP core delivery, scope boundaries, and solutions for flagged edge cases.

## Table of Contents

- [Version 1 MVP Implementation Scope](#roadmap-scope)
- [Step-by-Step Implementation Roadmap (v1 Execution Plan)](#implementation-plan)
- [In-Depth Architectural Review & Flagged Areas](#architectural-review)

---

<a id="roadmap-scope"></a>

## 19. Version 1 MVP Implementation Scope

### In Scope for v1 (The Core Engine)

- Single-binary Rust CLI: `qdev` in `tools/qdev` with root status & "what to do next" guidance.
- Full support for `qdev <verb> <noun> [flags] [-- <input>]` grammar and global `--json`.
- Dual configuration engine: `qdev.toml` + `.qdev.local.toml`.
- Automated environment integration: `qdev install skills`, `qdev install mcp`, `qdev install hooks`.
- Self-diagnostic health audit command: `qdev doctor`.
- Multi-owner support (persons + teams) with Cross-Team Governance interlocks.
- Language-agnostic module system replacing hardcoded crate assumptions.
- Embedded SQLite engine with auto-hydration from markdown frontmatter.
- Full `S3E1S4` grammar parser and entity relationship join table.
- The 3 core slash-command skills (`/qdev-create-story`, `/qdev-develop`, `/qdev-review`).
- Multi-Entity compact citation system (`[DEC-xxx]`, `[S...E...S...]`, `[AD-xx]`, `[HAZ-xx]`, `[DW-xxx]`).
- Code Hygiene & Comment Distillation engine with auto-relocation to scratchpads.
- Structured Multi-Perspective Synthesis (de-noised ideation & planning).
- Relational Documentation Artifacts Catalog indexing 8 core development document types.
- End-of-Epic and End-of-Sprint review and transition commands.
- Gate execution engine with stdout compression and exit code parsing.
- Red-fixture, Dual-limb hazard, and PHI leak gate verifiers.
- SOUP audit gate integration and SBOM export.
- Automated Regression Impact Analysis CLI (`qdev impact <story_id>`).
- ISO 14971 risk fields in Deferred Work and Residual Anomaly report export.
- Git preflight guard (dirty check, remote sync check).
- Per-story scratchpad management (`qdev scratch append/read`).
- Decision ledger (`qdev decision log`).
- Discrete Deferred Work CLI (`qdev dw add/list/close`).

### Deferred to v2 (Future Capabilities)

- Embedded Axum web server and browser dashboard (`qdev serve`).
- Formal PDF regulatory submission dossier generator (v1 outputs JSON/Markdown).
- Hosted / Networked PostgreSQL backend integration.
- Bidirectional Jira / Linear cloud synchronization.
- Automated visual graph rendering of the story dependency DAG.

---

<a id="implementation-plan"></a>

## 20. Step-by-Step Implementation Roadmap (v1 Execution Plan)

To deliver the `qdev` v1 core systematically without disrupting active Qubric development, the implementation is organized into seven sequential, self-verifying milestones:

1. **Phase 1: CLI Scaffolding, Installation & Configuration Engine**

   Create `tools/qdev` in the repository. Implement the `clap` v4 CLI supporting `qdev <verb> <noun>` grammar and global `--json`. Implement `qdev init` and the configuration merger for `qdev.toml` (committed) and `.qdev.local.toml` (local user overrides). Provide `scripts/install-qdev.sh`.

   *Deliverable:* `qdev --version` and `qdev config show --json` functional.

2. **Phase 2: Storage Abstraction & SQLite Auto-Hydration Engine**

   Define the Rust `Store` trait. Implement the `rusqlite` embedded storage engine. Implement the file-scanner checking `mtime` and SHA-256 hashes of Markdown files in `docs/` against `_sync_state`. Implement bidirectional frontmatter serialization.

   *Deliverable:* `qdev sync` parses test specs into `.qubric/qdev.db` in < 30ms.

3. **Phase 3: Entity Grammar & Relational Graph Operations**

   Implement the `S3E1S4` grammar parser. Support multi-owner definitions (persons and teams) and the semantic relationship DAG (`depends_on`, `extends`, `supersedes`, `traces_to`, `closes_dw`). Implement discrete Deferred Work (`DW-*`) and Decision Ledger (`DEC-*`) operations.

   *Deliverable:* `qdev get story S5E2S4 --json` and `qdev dw list --module bridge` functional.

4. **Phase 4: Git Preflight & Cross-Team Governance Guard**

   Implement the Git working-tree inspector (verifying clean tree and sync status against `origin/develop`). Implement the Cross-Team Governance interlock prompting for justification when modifying entities owned by external teams. Implement `qdev install hooks`.

   *Deliverable:* `qdev preflight` refuses execution if local branch is behind remote.

5. **Phase 5: Code Hygiene & Comment Distillation Filter**

   Build the comment linter and distillation parser (`qdev hygiene --check`). Enforce compact multi-entity citations (`[DEC-xxx]`, `[S...E...S...]`, `[AD-xx]`). Implement automated relocation of forensic essay comments into story scratchpads.

   *Deliverable:* `qdev hygiene --check` catches forensic memoir comments in diffs.

6. **Phase 6: Generic Gate & Ratchet Runner**

   Implement the process execution wrapper with stdout/stderr noise stripping (~95% token savings). Implement the three core SaMD gate types (Red-Fixture runner, Dual-Limb Hazard verifier, PHI leak scanner) and SOUP dependency auditor. Implement `qdev impact <id>` blast-radius calculation.

   *Deliverable:* `qdev gate run c-abi-round-trip --json` executes and generates `.evidence.json`.

7. **Phase 7: LLM Skills, MCP Registration & Root Pulse Integration**

   Implement the root `qdev` status command ("What To Do Next" calculation) and `qdev doctor`. Implement `qdev install skills` and `qdev install mcp`. Author the three core slash-command skills: `/qdev-create-story`, `/qdev-develop`, and `/qdev-review`. Migrate existing Sprint 5 entities into the new schema.

   *Deliverable:* Full end-to-end story execution using `/qdev-*` in active development.

---

<a id="architectural-review"></a>

## 21. In-Depth Architectural Review & Flagged Areas

An adversarial architectural review of this specification identified four subtle edge cases that the implementation must explicitly address:

### 1. Multi-Sprint Concurrency (Active + Patch Sprints)

**Issue:** In `qdev.toml`, having a single `active_sprint = 5` fails when an emergency security patch is being developed in parallel under Sprint 54.

**Resolution:** The schema must support multiple sprints in `status = 'active'`. Commands accept an optional `--sprint <id>` parameter, falling back to `default_active_sprint` from configuration if omitted.

### 2. Markdown Frontmatter Preservation & Round-Tripping

**Issue:** Generic YAML dumpers (like PyYAML) strip blank lines and inline comments within the frontmatter block when re-serializing files.

**Resolution:** Use a formatting-preserving parser/emitter in Rust (e.g., `toml_edit` or clean line-based YAML frontmatter patching) so that manual annotations inside frontmatter survive edits.

### 3. Multi-Agent Worktree Collisions

**Issue:** If multiple autonomous agents (e.g. Subagent A and Subagent B) work on different stories concurrently, they cannot share the same checked-out working directory without git index collisions.

**Resolution:** When operating in agentic multi-track mode, `qdev` should automatically leverage **Git Worktrees** (`git worktree add`) to isolate execution branches into separate working directories.

### 4. Environment Variable Inheritance in Gate Execution

**Issue:** Shell and cargo test commands executed by the Gate Runner inside `qdev` might fail if critical environment variables (such as `CARGO_TARGET_DIR`, `DEVELOPER_DIR`, or tool paths) are not propagated.

**Resolution:** The `qdev` gate execution engine must explicitly inherit the parent shell's environment while injecting configured project variables from `[environment]` blocks in `qdev.toml`.

---

