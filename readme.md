# qdev

> **Relational Workflow Engine, Gatekeeper, and Context Optimizer for High-Integrity Software Development**

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org)
[![Architecture](https://img.shields.io/badge/architecture-BMAD%20%2B%20Shape%20Up-blue.svg)](docs/architecture.md)
[![Compliance](https://img.shields.io/badge/compliance-IEC%2062304%20%2F%20ISO%2014971-red.svg)](docs/compliance-and-safety.md)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](#license)

---

`qdev` is an offline-first, single-binary Rust development engine and gatekeeper designed to orchestrate complex software projects and AI pair-programming workflows. 

By synthesizing **BMAD hierarchical decomposition** (PRDs → Epics → Stories → ADRs) with **Basecamp Shape Up boundary guardrails** (Appetite, Module Boundaries, Rabbit Holes, No-Gos), `qdev` solves the chronic operational failure modes of AI-assisted engineering: markdown context sprawl, syntax drift, line-anchor rot, and LLM development memoir pollution.

Specifications live as clean, reviewable Markdown with YAML frontmatter in Git, while a high-speed local SQLite database auto-hydrates on startup (~25ms) to serve as a fast query cache and relational dependency graph.

---

## ⚡ At a Glance

```bash
$ qdev
================================================================================
  qdev v1.0 — Development Engine & Gatekeeper
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

---

## 🛑 The Problem: The Markdown Context Wall

In multi-sprint, complex engineering projects (such as [Qubric](docs/architecture.md#problem-statement), an offline-first medical bronchoscopy platform), flat markdown workflows hit severe limits as projects scale to hundreds of stories:

| Failure Mode | Operational Reality | How `qdev` Solves It |
| --- | --- | --- |
| **Context Sprawl** | `AGENTS.md` and story files balloon to 6,000+ words, forcing agents to burn 20,000+ tokens just to extract 3 acceptance criteria. | **Relational Projections**: Entity queries ingest only the exact story, relevant ADRs, and bounded invariants (~400–1,200 tokens). |
| **Syntax Drift** | Flat lists like `deferred-work.md` drift into hundreds of mixed checkboxes and bullets, breaking automated drainage loops. | **Strongly Typed Schema**: Discrete entities (`DW-*`, `DEC-*`, `AD-*`) stored in SQLite with strict enums and foreign keys. |
| **Line-Anchor Rot** | Code and specs cite file line numbers (e.g. `file.md:2340-2368`). Edits upstream rot 80%+ of anchors silently. | **Immutable Semantic IDs**: Everything is indexed via stable semantic symbols (`S5E2S4`, `AD-43`, `DEC-104`). |
| **Memoir Pollution** | LLMs inject multi-paragraph historical review essays directly into production source code comments. | **Code Hygiene Filter**: Replaces memoirs with compact tags (`// [S5E2S4] ref AD-43`) and relocates history into the story scratchpad. |

---

## 💡 Core Concepts

```
                  ┌──────────────────────────────────────────────┐
                  │                 PRD (v0.1.0)                 │
                  └──────────────────────┬───────────────────────┘
                                         │
                   ┌─────────────────────┴─────────────────────┐
                   ▼                                           ▼
         ┌───────────────────┐                       ┌───────────────────┐
         │  Epic 1: Ingest   │                       │   Epic 2: Core    │
         └─────────┬─────────┘                       └─────────┬─────────┘
                   │                                           │
         ┌─────────┴─────────┐                       ┌─────────┴─────────┐
         ▼                   ▼                       ▼                   ▼
    ┌─────────┐         ┌─────────┐             ┌─────────┐         ┌─────────┐
    │ Story 1 │◄───────-┤ Story 2 │             │ Story 3 │         │ Story 4 │
    └────┬────┘ depends_on└─────────┘             └─────────┘         └────┬────┘
         │                                                                 │
         │ traces_to                                              closes_dw│
         ▼                                                                 ▼
    ┌─────────┐                                                       ┌─────────┐
    │  ADR-43 │                                                       │  DW-421 │
    └─────────┘                                                       └─────────┘
```

### 1. BMAD Tree (Positive Structure)
Tells agents **where they are**:
- **PRDs:** High-level problem definition, clinical rationale, target release.
- **Epics:** Architectural domains, phases, and major capabilities.
- **Stories:** Atomic units of work with functional acceptance criteria.
- **ADRs:** Permanent, binding architectural decisions outliving single stories.

### 2. Shape Up Guardrails (Negative Boundaries)
Tells agents **where they are forbidden to go**:
- **Appetite:** Budget clamp (`tiny`, `small`, `medium`, `deep`).
- **Target Modules:** File and subsystem boundary clamps (e.g. `["bridge", "foundation"]`).
- **Rabbit Holes:** Known architectural traps to actively avoid.
- **No-Gos:** Explicitly forbidden scope to suppress LLM hallucinations and speculative refactors.

### 3. Hydrated Projection Model
- **Git Source of Truth:** Markdown files with YAML frontmatter under `docs/specs/` and `docs/state/`.
- **High-Speed Cache:** Local, gitignored SQLite database (`.qubric/qdev.db`) auto-hydrated on startup in ~25ms.
- **Zero Lock-in:** Delete the database at any time; it fully regenerates from Git markdown.

---

## 🚀 Key Features

### 🤖 Three-Phase Model Tiering
Matches the right AI model capability to each stage of development to maximize quality and minimize cost:
- **`/qdev-create-story` (High Reasoning):** Uses deep-reasoning models (Sonnet / Gemini Thinking) to decompose epics into bounded stories with Rabbit Holes and No-Gos (~800 tokens).
- **`/qdev-develop` (Fast TDD Coding):** Uses fast code models (Sonnet / Flash / Cursor) to implement tests and code within strict module boundaries (~400 tokens).
- **`/qdev-review` (Adversarial Auditor):** Uses premier models (Opus / Pro) to audit git diffs against acceptance criteria, No-Gos, and comment hygiene rules (~1,200 tokens).

### 🧹 Code Hygiene & Distillation
Lints code comments to eliminate narrative memoirs. Replaces verbose commentary with compact bracket tags (`// [S5E2S4]`, `// [DEC-104]`, `// [AD-43]`), while automatically preserving forensic reasoning in the story’s dedicated SQLite scratchpad.

### 🚦 Zero-Noise Verification Gates
Wraps test runners and linters to strip compiler warnings and cargo noise, compressing outputs by ~95%:
- **On Pass:** Emits a single receipt: `[PASS] gate:c-abi-round-trip | 18 tests passed | duration: 1.2s`
- **On Failure:** Emits only the failing assertion snippet, saving thousands of diagnostic tokens.

### 🏥 SaMD & IEC 62304 / ISO 14971 Ready
Engineered for regulated medical software:
- **SOUP Audits:** Automated dependency auditing (`cargo audit`, `cargo deny`) and CycloneDX SBOM export.
- **Residual Anomalies:** Deferred work (`DW-*`) tagged with ISO 14971 safety risk levels and clinical rationales.
- **Verification Independence:** Strict segregation between developer and reviewer phases with cryptographically verifiable evidence receipts.

### 🛡️ Cross-Team Governance & Git Preflight
- **Multi-Ownership:** Assign stories and epics to multiple people or teams (`["simon", "team:core-platform"]`).
- **Boundary Interlocks:** Halts execution and requires explicit justification if an agent attempts to modify entities owned outside its assigned team.
- **Git Preflight Guard:** Prevents story start if uncommitted changes exist or if the branch is behind the remote tracking branch.

---

## 📦 Quick Start

### Installation

```bash
# Install via Cargo
cargo install --path tools/qdev --locked

# Or run the bootstrap installer
./scripts/install-qdev.sh
```

### Project Setup

```bash
# 1. Initialize qdev in your repository
$ qdev init
✔ Created qdev.toml (project-wide configuration)
✔ Created .qdev.local.toml (personal identity: simon)
✔ Added .qubric/qdev.db and .qdev.local.toml to .gitignore
✔ Created docs/specs/ and docs/state/ directories
✔ Initialized SQLite cache schema (version 1)

# 2. Automatically register AI skills & MCP servers
$ qdev install skills --claude   # Claude Code / Desktop
$ qdev install skills --cursor   # Cursor IDE rules & MCP
$ qdev install skills --agents   # Antigravity / Gemini skills
$ qdev install hooks             # Git pre-commit & pre-push hooks

# 3. Verify host environment
$ qdev doctor
[✓] Rust Toolchain: rustc 1.85.0
[✓] Git Status: tracking origin/develop (clean working tree)
[✓] SQLite Cache: .qubric/qdev.db (valid schema v1, 184 entities indexed)
[✓] AI Skills: Registered in .agents/skills/ and .claude/skills/
[✓] MCP Server: stdio transport verified
All systems nominal. Ready for development.
```

---

## 💻 CLI Grammar & Usage

`qdev` adheres to a clean `<verb> <noun>` grammar with universal `--json` support:

```bash
# Querying Entities
qdev get story S5E2S4 --json
qdev get epic S5E2
qdev list stories --epic S5E2 --status ready

# Lifecycle & Scratchpad
qdev update story S5E2S4 --status in-progress
qdev scratch append S5E2S4 -- "Using AtomicBool on frame drop latch."
qdev scratch read S5E2S4

# Deferred Technical Debt & Decisions
qdev dw add --story S5E2S4 --module bridge --title "Refactor CBridge buffer layout"
qdev dw list --module bridge
qdev decision log --story S5E2S4 --type human_ruling --topic "Buffer Sizing" --ruling "Fixed 4MB pool"

# Verification Gates & Impact Analysis
qdev gate run c-abi-round-trip --format json
qdev gate run --all
qdev impact S5E2S4               # Computes regression blast radius
```

---

## ⚙️ Configuration

`qdev` uses a dual-configuration pattern:

- **`qdev.toml` (Committed to Git):** Project policies, module boundaries, teams, gate definitions, and regulatory parameters.
- **`.qdev.local.toml` (Gitignored):** Individual developer identity, active team memberships, and local preferences.

```toml
# Excerpt from qdev.toml
[project]
name = "Qubric"
default_active_sprint = 5
id_prefix = "S5"

[teams]
core-platform = ["simon", "amelia"]
ui-shell = ["sally"]

[modules]
workspace_members = ["bridge", "foundation", "engine", "features"]

[hygiene]
enforce_comment_diet = true
citation_format = "// [{entity_id}] {summary}"

[[gates]]
id = "c-abi-round-trip"
command = "cargo test -p bridge --test c_abi_round_trip"
```

---

## 📚 Documentation

Detailed specifications and architecture guides are available in the [`docs/`](docs/README.md) directory:

| Guide | Description |
| --- | --- |
| 📖 **[Architecture & Data Model](docs/architecture.md)** | Full BMAD + Shape Up synthesis, lifecycle stages, indexing grammar, and complete SQLite relational schema. |
| 💻 **[CLI Reference & Configuration](docs/cli-reference.md)** | CLI grammar, universal JSON output, default interactive experience, and dual configuration specs. |
| 🏥 **[Compliance, Gates & Code Hygiene](docs/compliance-and-safety.md)** | IEC 62304 / ISO 14971 mapping, SOUP auditing, noise-stripping gate runner, and comment distillation rules. |
| 👥 **[Governance & Release Management](docs/governance-and-teams.md)** | Multi-owner teams, cross-team boundary interlocks, and orthogonal SemVer release baselines. |
| 🗺️ **[v1 Implementation Scope & Roadmap](docs/roadmap.md)** | 7-phase step-by-step implementation plan, MVP scope boundaries, and edge-case solutions. |
| 📜 **[Original Planning Document](docs/initial_planning.html)** | The original interactive HTML architecture specification. |

---

## 🗺️ Roadmap & Milestones

The v1 core implementation is executed in seven sequential, self-verifying phases:

- [ ] **Phase 1: CLI Scaffolding, Installation & Configuration Engine** (`qdev <verb> <noun>`, `qdev init`, dual config)
- [ ] **Phase 2: Storage Abstraction & SQLite Auto-Hydration Engine** (`rusqlite`, sub-30ms cache sync)
- [ ] **Phase 3: Entity Grammar (`S3E1S4`) & Relational Graph Operations** (DAG traversal, DW & Decision tables)
- [ ] **Phase 4: Git Preflight & Cross-Team Governance Guard** (clean tree enforcement, team interlocks)
- [ ] **Phase 5: Code Hygiene & Comment Distillation Filter** (memoir linter, scratchpad relocation)
- [ ] **Phase 6: Generic Gate & Ratchet Runner** (output noise compression, SaMD gate verifiers)
- [ ] **Phase 7: AI Skills, MCP Server & Project Pulse Integration** (`/qdev-*` skills, `qdev doctor`)

See [`docs/roadmap.md`](docs/roadmap.md) for detailed deliverables per phase.

---

## 📄 License

Dual-licensed under either:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
