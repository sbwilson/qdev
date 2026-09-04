# qdev Architecture & Data Model Specification

> In-depth technical specification of the BMAD + Shape Up hybrid methodology, relational data model, projection storage architecture, and three-phase lifecycle.

## Table of Contents

- [Problem Statement: The Markdown Context Wall](#problem-statement)
- [Prior Art & Landscape Analysis](#prior-art)
- [The Core Synthesis: BMAD Tree + Shape Up](#methodology)
- [The End-to-End Product Development Lifecycle](#end-to-end-lifecycle)
- [Relational Documentation Artifacts Catalog](#artifacts-catalog)
- [Indexing Grammar & Data Model (Module-Agnostic)](#grammar-datamodel)
- [Storage Architecture: SQLite + Auto-Hydration](#storage-architecture)
- [The Three-Phase Lifecycle & Model Tiering](#phase-skills)

---

<a id="problem-statement"></a>

## 1. Problem Statement: The Markdown Context Wall

During the multi-sprint development of **Qubric**—a native, offline-first medical bronchoscopy platform incorporating video ingest, metrology/SLAM, and a shared core via Crux—the project adopted the BMAD method to govern planning and code generation.

As Sprint 5 expanded to 15 epics and over 160 stories, the markdown-driven approach encountered severe operational failure modes:

### ❌ Context Sprawl & Token Exhaustion

Critical files expanded unchecked. `AGENTS.md` ballooned from 386 to 6,144 words in ten days because story context duplicated the architecture spine. Agents loaded 20,000+ tokens of markdown just to extract three acceptance criteria.

### ❌ Syntax Drift & Automation Blindness

Flat files drifted across multiple human and agent formats. `deferred-work.md` accumulated 216 checkboxes, 259 bullet items, and freeform text simultaneously, blinding automated drainage scripts (`bmad-loop sweep`).

### ❌ Line-Anchor Rotting

Source code, CI scripts, and specs cited line numbers in markdown files (e.g. `deferred-work.md:2340-2368`). As earlier lines were modified, 17 of 21 anchors rotted silently, pointing to completely unrelated entries.

### ❌ Silent YAML Comment Dropping

BMAD’s python script (`sprint_plan.py`) silently erased header comments and annotations inside `sprint-status.yaml` upon re-serialization, forcing manual recovery and hand-editing.

> [!NOTE]
> **The Fundamental Architectural Insight**
>
> Software requirements, epics, stories, architectural decisions (ADRs), deferred technical debt, and test verification gates form a **strongly typed directed relational graph**. Storing a relational graph in dozens of flat markdown files inevitably causes consistency breakdown and token waste. Relational entities belong in a relational database.

---

<a id="prior-art"></a>

## 2. Prior Art & Landscape Analysis

A thorough survey of the ecosystem reveals four distinct categories of tools addressing agent coordination, none of which fully solve the problem for high-integrity medical software:

| Category | Representative Tools | Capabilities | Gaps for Qubric |
| --- | --- | --- | --- |
| **Agent Task Graphs** | `swiftj/synapse`, `Beads (bd)`, `tacks`, `taskman` | Local-first SQLite or Dolt DAGs; machine-readable JSON; agent-focused task dependency tracking. | Too low-level. Lack hierarchical understanding of PRDs, Epics, Sprints, ADR bindings, and regulatory traceability. |
| **Versioned SQL** | `Dolt` | MySQL-compatible database with Git semantics (branch, diff, merge, snapshot). | Excellent storage substrate, but requires external CGO runtime and daemon; not a full development methodology tool. |
| **Requirements-as-Code** | `StrictDoc`, `Sphinx-Needs`, `Mantra` | Structured technical specifications; bidirectional traceability matrices; IEC 62304 compliance. | Focused on static documentation and audits; lacks interactive agentic planning, model tiering, and execution gating. |
| **AI Spec Toolkits** | `GitHub Spec Kit`, `ChatPRD` | Linear prompts: Constitution → Specify → Plan → Tasks. | Too flat. Fails to model multi-epic enterprise hierarchies, module boundaries, and complex systems refactors. |

**Conclusion:** `qdev` is created to fill this gap: providing an offline-first, single-binary Rust engine that couples deep hierarchical planning with relational integrity, Shape Up boundary enforcement, and regulatory-grade verification gates.

---

<a id="methodology"></a>

## 3. The Core Synthesis: BMAD Tree + Shape Up

Qubric adopts a hybrid approach that combines the strengths of BMAD's hierarchical decomposition with Basecamp's Shape Up boundary constraints.

### BMAD Hierarchical Tree (Positive Structure)

**Tells the AI where it is:**

- **PRD:** High-level problem definition, market/clinical rationale, target release.
- **Epics:** Architectural domains, phases, and core capabilities.
- **Stories:** Atomic units of work carrying functional intent and acceptance criteria.
- **ADRs:** Binding architectural decisions outliving individual stories.

### Shape Up Guardrails (Negative Boundaries)

**Tells the AI where it is forbidden to go:**

- **Appetite:** Budget and complexity clamp (`tiny`, `small`, `medium`, `deep`).
- **Target Modules:** Subsystem and file boundary clamp (e.g. `["bridge", "foundation"]` or `["Packages/RustBridge"]`).
- **Rabbit Holes:** Known architectural and technical traps to actively avoid.
- **No-Gos:** Explicit out-of-scope capabilities forbidden in this story.

> [!WARNING]
> **Why Negative Constraints Stop Hallucinations**
>
> LLMs are predisposed to over-engineer when given open-ended requirements. When prompted only with positive acceptance criteria, agents often import forbidden libraries (e.g. `tokio` in Qubric's Rust core), refactor adjacent subsystems, or build speculative features. Explicitly injecting **Rabbit Holes** and **No-Gos** into the developer prompt suppresses scope creep and reduces wasted tokens.

---

<a id="end-to-end-lifecycle"></a>

## 4. The End-to-End Product Development Lifecycle

`qdev` models the full lifecycle of complex software development, establishing a disciplined progression from initial ideation down to atomic commits and sprint transitions.

### Stage 1: Initialization

`qdev init` configures `qdev.toml` and `.qdev.local.toml`, registers team identities and developers, initializes the local gitignored SQLite cache, and installs Git preflight hooks.

### Stage 2: Structured Ideation & Architecture

Guided synthesis of PRDs, Architecture Spines, UX models, and Risk registers. Replaces multi-agent role-playing theater with **Structured Multi-Perspective Synthesis**.

### Stage 3: Decomposition & Concurrency Planning

Refines architecture into Sprints, Epics, and Stories. Analyzes the dependency DAG to identify **Parallel Execution Tracks** (e.g., UI shell vs. core engine).

### Stage 4: Story Execution Loop

The per-story iteration: `specify` (author spec with boundaries) → `develop` (TDD + scratchpad + hygiene filter) → `review` (adversarial audit + evidence stamp).

### Stage 5: End-of-Epic Review

`qdev review epic <id>` audits all delivered stories against the epic milestone, ensures no unclassified debt remains, and verifies epic-level gates.

### Stage 6: End-of-Sprint Review

`qdev review sprint <id>` executes full regression suites, audits Known Residual Anomalies, freezes velocity metrics, and outputs compliance traceability reports.

### Stage 7: Sprint Transition & Baseline Freeze

`qdev sprint close <id> --status done|paused|abandoned` snapshots the release baseline and atomically rolls over open debt or incomplete stories into the next sprint.

### Eliminating Persona Theater (De-Noising Multi-Agent Sessions)

A common frustration with frameworks like BMAD is "persona role-play bloat"—where multiple LLM personas (e.g., Product Manager Mary, Architect Winston, UX Sally) exchange thousands of tokens in simulated polite banter ("Hi Mary! Thanks for the great PRD...").

> [!NOTE]
> **Structured Multi-Perspective Synthesis**
>
> `qdev` retains the analytical value of multiple viewpoints (Product, Architecture, Clinical/UX, Risk) without conversational role-play. Instead of conversational agents chatting back and forth, `qdev` prompts a single high-reasoning model with a **Multi-Perspective Synthesis Schema**:
>
> ```
> # Multi-Perspective Synthesis:
> [Product & Clinical Value]: Concise statement of intent and clinical workflow impact.
> [Architectural Constraints]: Invariants, module boundaries, C-ABI signatures, FFI implications.
> [Safety & Risk Profile]: ISO 14971 hazards, dual-limb controls, failure injection modes.
> [Implementation Directives]: Acceptance criteria, Shape Up Rabbit Holes, and No-Gos.
> ```
>
> This yields identical or superior analytical depth while saving up to 80% of prompt tokens and eliminating conversational noise.

---

<a id="artifacts-catalog"></a>

## 8. Relational Documentation Artifacts Catalog

To eliminate markdown sprawl while preserving deep technical context, `qdev` identifies, indexes, and relationally stores eight core documentation artifacts produced during development:

| Artifact Type | Lifecycle Stage | Content Captured & Stored in SQLite |
| --- | --- | --- |
| **Product Brief / PRFAQ** | Ideation (Stage 2) | Working Backwards customer/clinical problem statement, customer FAQ, internal developer FAQ. |
| **Architecture Invariant Spine** | Architecture (Stage 2) | The lean list of workspace-wide non-negotiables (no async runtimes, ring boundary directions, FFI bounds). |
| **Architectural Decisions (ADRs)** | Architecture & Design | Permanent decision records: context, alternatives rejected, chosen approach, binding invariants. |
| **Sprint Change Proposals** | Course Correction (Mid-Sprint) | Formalized pivot proposals (adopted from BMAD `bmad-correct-course`): trigger, issue summary, impact analysis on stories/epics, artifact conflict resolutions. |
| **UX Interaction Specifications** | Design & Specification | State machines for viewports, safety HUD layouts, Safety PIP fallbacks, degraded UI surfaces. |
| **Hazard Analysis & Risk Controls** | Regulatory / Safety | ISO 14971 hazard IDs, hazardous situations, software causes, dual-limb risk control verifications. |
| **End-of-Epic Retrospective Dossiers** | Epic Review (Stage 5) | Delivered capabilities, action items, standing technical debt assignments (e.g. creating tooling epics). |
| **Release Baselines & Traceability Matrices** | Sprint Close (Stage 7) | The locked Requirements Traceability Matrix (RTM), SOUP SBOM, and Known Residual Anomalies report. |

---

<a id="grammar-datamodel"></a>

## 11. Indexing Grammar & Data Model (Module-Agnostic)

### The `S3E1S4` Indexing Grammar

- **Story:** `S{sprint}E{epic}S{story}` (e.g. `S3E1S4`, or sub-stories `S3E1S4a`).
- **Epic:** `S{sprint}E{epic}` (e.g. `S3E1`).
- **PRD:** `S{sprint}-PRD` (e.g. `S5-PRD`).
- **ADR:** `AD-{number}` (e.g. `AD-43`, globally monotonic across the workspace).
- **Deferred Work:** `DW-{number}` (e.g. `DW-421`, globally monotonic).
- **Decision:** `DEC-{number}` (e.g. `DEC-104`, logged per story).

### Entity Relations (The Graph Engine)

| Relationship | Source → Target | Semantic Meaning |
| --- | --- | --- |
| `depends_on` | Story → Story | Execution DAG: Target must be in `done` status before source can start. |
| `extends` | Story → Story | Iterative enhancement: Source refines or adds degraded modes to target. |
| `supersedes` | Story → Story / Epic | Deprecation: Source replaces target. Target is retired and blocked from active work. |
| `traces_to` | Story → PRD / Requirement | IEC 62304 traceability link from functional requirement to code. |
| `closes_dw` | Story → Deferred Work | Discharges a technical debt item upon verified story closure. |

### SQLite Relational Schema

```sql
-- Core Releases & Sprints
CREATE TABLE releases (
    version TEXT PRIMARY KEY,        -- e.g. '0.1.0'
    target_date TEXT,
    base_version TEXT REFERENCES releases(version),
    status TEXT CHECK(status IN ('planning', 'active', 'frozen', 'shipped'))
);

CREATE TABLE sprints (
    id INTEGER PRIMARY KEY,          -- e.g. 5
    release_version TEXT REFERENCES releases(version),
    title TEXT NOT NULL,
    owners TEXT NOT NULL,            -- JSON array: ["simon", "team:core-platform"]
    status TEXT CHECK(status IN ('planning', 'active', 'completed', 'paused', 'abandoned')),
    started_at TIMESTAMP,
    completed_at TIMESTAMP
);

-- Epics and Stories
CREATE TABLE epics (
    id TEXT PRIMARY KEY,             -- e.g. 'S5E2'
    sprint_id INTEGER REFERENCES sprints(id),
    title TEXT NOT NULL,
    owners TEXT NOT NULL,            -- JSON array of owners/teams
    phase INTEGER,
    status TEXT DEFAULT 'planning'
);

CREATE TABLE stories (
    id TEXT PRIMARY KEY,             -- e.g. 'S5E2S4'
    epic_id TEXT REFERENCES epics(id),
    seq INTEGER NOT NULL,
    title TEXT NOT NULL,
    owners TEXT NOT NULL,            -- JSON array: ["simon", "amelia"]
    status TEXT CHECK(status IN ('backlog', 'ready', 'in-progress', 'review', 'done', 'escalated')),
    appetite TEXT CHECK(appetite IN ('tiny', 'small', 'medium', 'deep')),
    safety_class TEXT CHECK(safety_class IN ('ClassA', 'ClassB', 'ClassC')),
    target_modules TEXT,             -- JSON array: ["bridge", "foundation"] or ["RustBridge"]
    rabbit_holes TEXT,               -- Pitfalls to avoid
    no_gos TEXT,                     -- Strictly out of scope
    spec_path TEXT NOT NULL,
    version INTEGER DEFAULT 1
);

-- Semantic Relationships
CREATE TABLE entity_relations (
    source_id TEXT NOT NULL,
    relation_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    notes TEXT,
    PRIMARY KEY (source_id, relation_type, target_id)
);

-- Decision Ledger
CREATE TABLE decisions (
    id TEXT PRIMARY KEY,             -- e.g. 'DEC-104'
    story_id TEXT REFERENCES stories(id),
    decision_type TEXT CHECK(decision_type IN ('human_ruling', 'agent_assumption', 'cross_team_override')),
    topic TEXT NOT NULL,
    context TEXT NOT NULL,
    ruling TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Deferred Work & Anomaly Register (IEC 62304 Clause 9)
CREATE TABLE deferred_work (
    id TEXT PRIMARY KEY,             -- e.g. 'DW-421'
    origin_story_id TEXT REFERENCES stories(id),
    target_module TEXT NOT NULL,     -- Language-agnostic module / subsystem identifier
    title TEXT NOT NULL,
    status TEXT CHECK(status IN ('open', 'done', 'wont_fix')),
    safety_risk TEXT CHECK(safety_risk IN ('negligible', 'acceptable_with_mitigation', 'unacceptable')) DEFAULT 'negligible',
    clinical_rationale TEXT,         -- Required risk justification under ISO 14971
    gate TEXT,
    resolution TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- SOUP Inventory (IEC 62304 §5.3.3)
CREATE TABLE soup_dependencies (
    id TEXT PRIMARY KEY,             -- e.g. 'rusqlite-0.31.0'
    dependency_name TEXT NOT NULL,
    version TEXT NOT NULL,
    license TEXT,
    cve_status TEXT DEFAULT 'clean',
    evaluated_for_release TEXT REFERENCES releases(version)
);

-- Executable Gates & Audit Runs
CREATE TABLE gates (
    id TEXT PRIMARY KEY,             -- e.g. 'c-abi-round-trip'
    gate_type TEXT CHECK(gate_type IN ('command', 'ratchet', 'red_fixture', 'hazard_limb', 'phi_leak', 'soup_audit', 'hygiene')),
    command TEXT NOT NULL,
    depends_on TEXT                  -- JSON array of gate IDs
);

CREATE TABLE gate_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    story_id TEXT REFERENCES stories(id),
    gate_id TEXT REFERENCES gates(id),
    commit_sha TEXT NOT NULL,
    exit_code INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    summary TEXT,
    ran_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);
```

---

<a id="storage-architecture"></a>

## 12. Storage Architecture: SQLite + Auto-Hydration

`qdev` uses a **Hydrated Projection Model**. Markdown files with YAML frontmatter act as the canonical Git source of truth, while an auto-hydrated SQLite database acts as the high-speed local query cache.

### Disk Storage (Git Source of Truth)

- Single-entity markdown files with YAML frontmatter under `docs/specs/sprint-{N}/`.
- Discrete deferred work files under `docs/state/dw/`.
- Committed to Git; produces clean, human-readable PR diffs.
- Zero binary merge conflicts.

### Local SQLite Cache (.qubric/qdev.db)

- **Gitignored** local cache file.
- Abstracted behind a Rust `Store` trait (allowing future Postgres backends).
- Auto-hydrated on startup in ~25ms via file `mtime` and content hashes.
- Can be deleted at any time and regenerated without loss of data.

---

<a id="phase-skills"></a>

## 13. The Three-Phase Lifecycle & Model Tiering

| Phase Skill | Target Model | Core Responsibility | Context Ingested |
| --- | --- | --- | --- |
| **`/qdev-create-story`** | High Reasoning (Sonnet / Gemini Thinking) | Decomposes Epic into next sequential story; queries related ADRs; defines Acceptance Criteria, Rabbit Holes, and No-Gos; initializes scratchpad. | Epic goals, relevant ADRs, sibling story titles. (~800 tokens) |
| **`/qdev-develop`** | Fast Coding (Sonnet / Flash / Cursor) | Implements code using TDD; maintains intermediate scratchpad; runs internal exit gates (No-Go checks, module boundaries, code hygiene filter). | Story spec, scratchpad, binding module invariants. (~400 tokens) |
| **`/qdev-review`** | Adversarial Audit (Opus / Pro) | Audits git diff against acceptance criteria; verifies that No-Gos, Rabbit Holes, and comment hygiene were respected; verifies all gates; stamps evidence. | Story spec, git diff, scratchpad summary, gate receipts. (~1,200 tokens) |

---

