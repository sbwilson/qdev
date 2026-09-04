# qdev CLI Reference & Configuration Guide

> Command-line grammar, universal JSON output, interactive status experience, configuration specifications, and environment bootstrapping.

## Table of Contents

- [CLI Grammar & Structured Output Architecture](#cli-grammar)
- [The Default qdev Command Experience ("What To Do Next")](#default-cli)
- [Dual Configuration Specification (qdev.toml & .qdev.local.toml)](#configuration)
- [Installation, Bootstrapping & Environment Integration](#installation-integration)

---

<a id="cli-grammar"></a>

## 5. CLI Grammar & Structured Output Architecture

`qdev` adopts a strictly standardized command line grammar modeled after modern UNIX and distributed system tools:

> [!NOTE]
> **Universal Command Syntax**
>
> ```bash
> qdev <verb> <noun> [flags/parameters] [-- <input>]
> ```

### Command Syntax Examples

- `qdev get story S5E2S4 --json` — Retrieves structured JSON representation of a story.
- `qdev create story S5E2 --title "CoreResponse Buffer" --appetite small` — Creates a new story.
- `qdev update story S5E2S4 --status in-progress` — Transitions story status.
- `qdev gate run c-abi-round-trip --format json` — Executes a verification gate and emits structured results.
- `qdev scratch append S5E2S4 -- "Used AtomicBool instead of Mutex on frame drop latch."` — Appends to the story scratchpad.
- `qdev dw add --story S5E2S4 --module bridge --title "Refactor CBridge buffer layout"` — Registers deferred technical debt.

### Universal Structured Output (JSON Support)

Every command in `qdev` accepts a global `--json` or `--format json` flag. This allows downstream tools, shell scripts, CI pipelines, and LLM tool interpreters to consume deterministic JSON payloads:

```bash
$ qdev get story S5E2S4 --json
{
  "id": "S5E2S4",
  "epic_id": "S5E2",
  "seq": 4,
  "title": "CoreResponse Buffer Layout",
  "status": "ready",
  "appetite": "small",
  "owners": ["simon", "team:core-platform"],
  "target_modules": ["bridge", "foundation"],
  "rabbit_holes": ["Do not add status field", "len == 0 does not mean empty result"],
  "no_gos": ["Do not implement Swift decoding", "Do not touch frame buffers"],
  "gates": ["c-abi-round-trip", "worker-pool-roster"],
  "version": 1
}
```

---

<a id="default-cli"></a>

## 6. The Default qdev Command Experience ("What To Do Next")

Running `qdev` with no arguments in the terminal (or calling the `/qdev` root skill in an AI chat session) acts as the interactive project pulse. It tests the environment, reports key stats, and computes the deterministic **"What To Do Next"** guidance.

```bash
$ qdev
================================================================================
  qdev v1.0 — Qubric Development Engine & Gatekeeper
================================================================================
● Environment:
  - Working Tree: CLEAN (on develop @ 8f1b2c4)
  - Remote Sync: UP TO DATE with origin/develop
  - SQLite Cache: HEALTHY (synced 18ms ago, 184 entities)

● Active Sprint: Sprint 5 (The Rust Core Port) [Target Release: 0.1.0]
  - Owner: core-platform (Active User: simon)
  - Stories: 42 Completed / 12 In-Progress / 108 Backlog (26% velocity)
  - Open Deferred Work (DW): 8 items (0 Unacceptable risks)
  - Gates: 14/14 PASSING (Ratchets at baseline)

● What To Do Next:
  → Story S5E2S4 ("CoreResponse Buffer Layout") is UNBLOCKED and READY.
  → Action: Run `/qdev-develop S5E2S4` to begin implementation.
================================================================================
```

> [!TIP]
> **Dual Access: CLI + LLM Skills**
>
> Every command in `qdev` is executable in two formats:
>
> - **CLI Binary:** `qdev status`, `qdev gate run <id>`, `qdev dw list` (for terminal use, shell scripts, and CI/CD pipelines).
> - **LLM Skills:** `/qdev`, `/qdev-develop <id>`, `/qdev-review <id>` (for interactive execution within Claude Code, Cursor, or Gemini, returning rich formatted Markdown and context).

---

<a id="configuration"></a>

## 17. Dual Configuration Specification (qdev.toml & .qdev.local.toml)

To cleanly separate team-shared project policies from individual developer identities, `qdev` splits its configuration into two files:

### 1. Project-Wide: qdev.toml (Committed to Git)

Defines project policies, module boundaries, registered teams, gates, regulatory standards, and Git rules:

```toml
# qdev.toml — Project-Wide Configuration
[project]
name = "Qubric"
default_active_sprint = 5
id_prefix = "S5"

[teams]
core-platform = ["simon", "amelia"]
ui-shell = ["sally"]
clinical-metrology = ["murat"]
repo-tooling = ["simon"]

[git]
origin_remote = "origin"
main_branch = "develop"
branching_mode = "story-branch"
branch_template = "feature/{story_id}-{slug}"
enforce_clean_working_tree = true
require_remote_sync = true

[storage]
specs_dir = "docs/specs"
evidence_dir = "docs/state/evidence"
deferred_work_dir = "docs/state/dw"
sqlite_cache = ".qubric/qdev.db"

[regulatory]
iec62304_class = "ClassB"
require_residual_anomaly_evaluation = true
enforce_verification_independence = true

[soup]
audit_command = "cargo audit"
deny_command = "cargo deny check"
sbom_format = "cyclonedx-json"

[modules]
workspace_members = ["platform", "foundation", "features", "engine", "bridge", "qb", "RustBridge"]

[hygiene]
enforce_comment_diet = true
max_inline_comment_lines = 6
forbid_tokens = ["STORY ", "Review round", "Wave "]
citation_format = "// [{entity_id}] {summary}"

# Registered Verification Gates
[[gates]]
id = "fmt"
command = "cargo fmt --all --check"

[[gates]]
id = "lint"
command = "cargo clippy --workspace --all-targets -- -D warnings"
depends_on = ["fmt"]

[[gates]]
id = "c-abi-round-trip"
command = "cargo test -p bridge --test c_abi_round_trip"
depends_on = ["lint"]
```

### 2. Per-User Local: .qdev.local.toml (Gitignored)

Holds the local developer's identity, active roles, team memberships, and personal preferences:

```toml
# .qdev.local.toml — User Local Overrides (.gitignore)
[identity]
name = "Simon"
developer_id = "simon"
teams = ["core-platform", "repo-tooling"]
active_role = "Lead Architect"

[preferences]
color_output = true
default_format = "text"          # text | json
editor = "cursor"

[local_gates]
skip_expensive_gates_by_default = false
```

---

<a id="installation-integration"></a>

## 18. Installation, Bootstrapping & Environment Integration

`qdev` is engineered to install in seconds, require zero external runtime daemons, and integrate automatically with modern AI IDEs and CLI tools.

### 1. Binary Compilation & Installation

Built using standard Rust toolchains directly from source or workspace:

```bash
# Build and install to ~/.cargo/bin
cargo install --path tools/qdev --locked

# Or via repository bootstrap script
./scripts/install-qdev.sh
```

The compiled binary is completely self-contained with embedded SQLite (`rusqlite` bundled), requiring no external database servers or C libraries.

### 2. Project Bootstrapping (qdev init)

Initializes a project workspace in any repository:

```bash
$ qdev init
✔ Created qdev.toml (project-wide configuration)
✔ Created .qdev.local.toml (personal identity: simon)
✔ Added .qubric/qdev.db and .qdev.local.toml to .gitignore
✔ Created docs/specs/ and docs/state/ directories
✔ Initialized SQLite cache schema (version 1)
```

### 3. AI Skill & MCP Server Deployment (qdev install)

A developer should not have to manually copy prompt files across AI editors. `qdev` automates its own registration into the AI environment:

| Target Environment | Installation Command | What It Automatically Registers |
| --- | --- | --- |
| **Claude Code / Desktop** | `qdev install skills --claude` | Writes `/qdev-*` skills into `.claude/skills/` and registers the `qdev` MCP server in `claude_desktop_config.json`. |
| **Cursor / VS Code** | `qdev install skills --cursor` | Generates `.cursor/rules/qdev.mdc` and registers the `qdev` stdio MCP server in `.cursor/mcp.json`. |
| **Antigravity / Gemini** | `qdev install skills --agents` | Installs skill bundles into `.agents/skills/qdev-*` with verified frontmatter manifests. |
| **Git Preflight Hooks** | `qdev install hooks` | Installs `.git/hooks/pre-commit` (runs `qdev hygiene --check`) and `.git/hooks/pre-push` (runs `qdev preflight`). |

### 4. Diagnostics & Self-Healing (qdev doctor)

Running `qdev doctor` audits the host environment: verifies Git remote status, checks SQLite schema migrations, validates skill definitions, and ensures all configured verification gate executables exist in `$PATH`:

```bash
$ qdev doctor
[✓] Rust Toolchain: rustc 1.85.0
[✓] Git Status: tracking origin/develop (clean working tree)
[✓] SQLite Cache: .qubric/qdev.db (valid schema v1, 184 entities indexed)
[✓] AI Skills: 5 skills registered in .agents/skills/ and .claude/skills/
[✓] MCP Server: stdio transport verified
[✓] Verification Gates: 14 gate commands found in PATH
All systems nominal. Ready for development.
```

---

