# qdev Architecture & Data Model Specification

> Technical specification of the BMAD + Shape Up hybrid methodology, entity model, storage architecture, and lifecycle. The authoritative list of architectural decisions is the [Architecture Spine](bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md); this document elaborates on it.

## Table of Contents

- [Problem Statement: The Markdown Context Wall](#problem-statement)
- [Prior Art & Landscape Analysis](#prior-art)
- [The Core Synthesis: BMAD Tree + Shape Up](#methodology)
- [End-to-End Lifecycle](#end-to-end-lifecycle)
- [Entity Catalog](#entity-catalog)
- [Identifier Grammar](#identifiers)
- [Story State Machine](#state-machine)
- [Relations](#relations)
- [On-Disk Layout](#layout)
- [SQLite Cache Schema](#schema)
- [Storage Architecture & Hydration](#storage-architecture)
- [Context Projection & Model Tiering](#projection)

---

<a id="problem-statement"></a>

## 1. Problem Statement: The Markdown Context Wall

During the multi-sprint development of **Qubric**, a native, offline-first medical bronchoscopy platform, the project adopted the BMAD method to govern planning and code generation. As Sprint 5 expanded to 15 epics and over 160 stories, the markdown-driven approach encountered severe operational failure modes:

- **Context sprawl.** `AGENTS.md` grew from 386 to 6,144 words in ten days because story context duplicated the architecture spine. Agents loaded 20,000+ tokens to extract three acceptance criteria.
- **Syntax drift.** `deferred-work.md` accumulated 216 checkboxes, 259 bullets, and freeform text simultaneously, blinding automated drainage scripts.
- **Line-anchor rot.** Code and specs cited `deferred-work.md:2340-2368`; 17 of 21 anchors rotted silently.
- **Silent YAML comment dropping.** A re-serialising script erased header comments from `sprint-status.yaml`.

> [!NOTE]
> **The architectural insight.** Requirements, epics, stories, ADRs, deferred work, and gates form a strongly typed directed graph. Storing a graph in dozens of flat files causes consistency breakdown and token waste. The graph belongs in a relational index; the human-readable record belongs in Git. qdev keeps both, with Git as the source of truth.

---

<a id="prior-art"></a>

## 2. Prior Art & Landscape Analysis

| Category | Representative Tools | Capabilities | Gap |
| --- | --- | --- | --- |
| Agent task graphs | Beads (`bd`), `synapse`, `tacks` | Local SQLite/Dolt DAGs, hash IDs, machine-readable | No PRD/epic hierarchy, ADR binding, or regulatory traceability |
| Versioned SQL | Dolt | Git semantics on a database | External daemon; not a methodology tool |
| Requirements-as-code | StrictDoc, Sphinx-Needs | Traceability matrices, IEC 62304 | Static documentation; no agent execution loop or gating |
| AI spec toolkits | Spec Kit, ChatPRD | Linear prompt pipelines | Too flat for multi-epic systems and module boundaries |

qdev borrows hash IDs from Beads, traceability from requirements-as-code tools, and the planning tree from BMAD, and adds Shape Up boundaries and an execution gate engine.

---

<a id="methodology"></a>

## 3. The Core Synthesis: BMAD Tree + Shape Up

### BMAD hierarchical tree (positive structure)
Tells the agent **where it is**: PRD → Requirements → Epics → Stories, with ADRs and Hazards as binding cross-cutting records.

### Shape Up guardrails (negative boundaries)
Tells the agent **where it may not go**. Each is a first-class constraint entity with an ID:

- **Appetite**: `tiny | small | medium | deep`.
- **Target modules**: module IDs from the registry, which resolve to path globs.
- **Rabbit holes** (`RH-n`): known traps to avoid.
- **No-gos** (`NG-n`): explicitly forbidden scope.

Constraints attach to an epic or story and are inherited downward. A rejection always cites the constraint ID and text.

> [!WARNING]
> **Why negative constraints stop hallucinations.** Given only positive acceptance criteria, agents import forbidden libraries, refactor adjacent subsystems, and build speculative features. Injecting rabbit holes and no-gos with IDs suppresses scope creep and makes violations diagnosable.

---

<a id="end-to-end-lifecycle"></a>

## 4. End-to-End Lifecycle

| Stage | Command(s) | Outcome |
| --- | --- | --- |
| 1. Initialise | `qdev init` | Config, directories, cache, hooks |
| 2. Ideate & architect | `/qdev-plan` skill, `qdev create prd|adr|requirement` | PRD, requirements, ADRs, hazards using Structured Multi-Perspective Synthesis |
| 3. Decompose | `qdev create epic|story`, `qdev graph --dot` | Epics, stories, constraints, dependency DAG, parallel tracks |
| 4. Execute a story | `qdev claim` → `/qdev-develop` → `/qdev-review` | Code, scratchpad, gate evidence, done |
| 5. Review an epic | `qdev review epic E12` | Epic report; unclassified debt blocked |
| 6. Review a sprint | `qdev review sprint 5` | Regression gates, residual anomalies, traceability matrix |
| 7. Close a sprint | `qdev sprint close 5 --carry-over 6` | Baseline snapshot; open work assigned to next sprint without renaming |

### Structured Multi-Perspective Synthesis
qdev replaces multi-persona role-play with a single high-reasoning prompt that must answer four headings: **Product & domain value**, **Architectural constraints**, **Safety & risk profile**, **Implementation directives**. The template is shipped in the planning skill and is configurable.

---

<a id="entity-catalog"></a>

## 5. Entity Catalog

| Entity | ID | Lives in | Notes |
| --- | --- | --- | --- |
| PRD | `PRD-n` | `docs/specs/prd/` | Vision, personas, metrics; owns requirements |
| Requirement | `FR-n`, `NFR-n` | `docs/specs/requirements/` | Traceability root; `traces_to` target |
| Epic | `En` | `docs/specs/epics/` | Owners, phase, constraints |
| Story | `EnSm` | `docs/specs/stories/` | Owners, appetite, target modules, ACs, constraints |
| ADR | `AD-n` | `docs/specs/adrs/` | Context, alternatives, decision, binds, prevents |
| Hazard | `HAZ-n` | `docs/specs/hazards/` | ISO 14971 hazard, cause, control; `mitigates` source |
| Constraint | `EnSm/NG-k`, `En/RH-k` | Frontmatter of owning entity | Kind, text; inherited downward |
| Sprint | `sprint-n` | `docs/state/sprints/` | Status, owners, release, assignments with carry-over |
| Release | `x.y.z` | `docs/state/releases/` | Base version, status, baseline snapshot |
| Deferred work | `DW-hhhh` | `docs/state/dw/` | Module, risk, rationale, gate |
| Decision | `DEC-hhhh` | `docs/state/decisions/` | Subject entity, type, ruling |
| Scratchpad entry | append-only | `docs/state/scratch/<story>.jsonl` | Author, kind, text |
| Gate | slug | `qdev.toml` + `.qdev/gates/` | Command, timeout, kind, bound transitions |
| Gate run / evidence | `<sha>-<gate>` | `docs/state/evidence/<story>/` | Status, summary, metric, duration |
| SOUP dependency | `name@version` | `docs/state/soup/` | Licence, CVE status, release evaluated |
| Lease | local | `.qdev/leases/` | Holder, worktree, story, started; gitignored |

---

<a id="identifiers"></a>

## 6. Identifier Grammar

Per **AD-7**, IDs never encode a sprint and are immutable once committed.

| Kind | Grammar | Allocation |
| --- | --- | --- |
| Epic | `E{n}` | Sequential, planning time |
| Story | `E{n}S{m}` | Sequential within epic, planning time |
| ADR / Requirement / Hazard / PRD | `AD-{n}`, `FR-{n}`, `NFR-{n}`, `HAZ-{n}`, `PRD-{n}` | Sequential, planning time |
| Constraint | `{owner}/NG-{k}`, `{owner}/RH-{k}` | Sequential within owner |
| Deferred work / Decision | `DW-{hex4+}`, `DEC-{hex4+}` | Random; length grows on collision |
| Citation | `[E12S4]`, `[AD-43]`, `[DW-7f3a]`, `[E12S4/NG-2]` | Language-appropriate comment prefix |

Sequential planning IDs are allocated by `qdev create`, which scans existing files. `qdev validate` reports duplicates arising from concurrent planning on separate branches and offers a guided renumber that rewrites citations, which is the only sanctioned rename. Sub-stories are expressed with the `extends` relation, not with suffixes.

---

<a id="state-machine"></a>

## 7. Story State Machine

```
draft ──► ready ──► in-progress ──► review ──► done
             ▲            │            │
             └────────────┴────────────┘   backward: --justification required
terminal: superseded, abandoned (from any state, --justification required)
computed: blocked = any depends_on target not done
```

- **draft**: being specified. `/qdev-create-story` produces a draft; it becomes `ready` when acceptance criteria and constraints validate.
- **Forward transitions** fire `pre_transition` hooks (gates, lease check, dependency check) then `post_transition` hooks (evidence write, scratchpad note).
- **Backward transitions** implement pivots and review rejections. They require `--justification`, which is appended to the scratchpad and logged as a `DEC-` record of type `pivot` or `review_rejection`.
- **superseded** is set automatically on the target of a `supersedes` relation when the source reaches `ready`.

Epics derive status from their stories: `planning`, `active`, `done`. Sprints: `planning | active | completed | paused | abandoned`.

---

<a id="relations"></a>

## 8. Relations

| Relation | Source → Target | Semantics |
| --- | --- | --- |
| `depends_on` | Story → Story | Target must be `done` before source may enter `in-progress`. Cycles are validation errors |
| `extends` | Story → Story | Source refines target; used instead of sub-story suffixes |
| `supersedes` | Story/Epic → Story/Epic | Target becomes `superseded` |
| `traces_to` | Story → Requirement | Traceability |
| `verifies` | Gate → Requirement | Traceability from evidence to requirement |
| `mitigates` | Story/Gate → Hazard | Risk control linkage |
| `closes_dw` | Story → Deferred work | DW closes when story reaches `done` |
| `governed_by` | Story/Epic → ADR | ADR excerpts included in projection |

Relations are declared in the source entity's frontmatter and validated on hydration; a dangling target is a validation error, not a parse failure.

---

<a id="layout"></a>

## 9. On-Disk Layout

```
qdev.toml                     committed project config
.qdev.local.toml              gitignored identity and preferences
.qdev/
  gates/                      committed gate scripts
  cache/                      gitignored: cache.sqlite, write.lock
  leases/                     gitignored: one file per active lease
docs/specs/
  prd/PRD-1.md
  requirements/FR-101.md
  epics/E12.md
  stories/E12S4.md
  adrs/AD-43.md
  hazards/HAZ-14.md
docs/state/
  sprints/sprint-5.md
  releases/0.1.0.md
  dw/DW-7f3a.md
  decisions/DEC-2b91.md
  scratch/E12S4.jsonl
  evidence/E12S4/8f1b2c4-c-abi-round-trip.json
  baselines/<branch>/<gate>.json
  soup/rusqlite@0.31.0.md
```

### Story file example

```markdown
---
id: E12S4
title: CoreResponse Buffer Layout
status: ready
owners: ["simon", "team:core-platform"]
appetite: small
safety_class: ClassB
target_modules: ["bridge", "foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: Do not implement Swift decoding
  - id: RH-1
    kind: rabbit_hole
    text: len == 0 does not mean empty result
relations:
  depends_on: ["E12S3"]
  traces_to: ["FR-102"]
  governed_by: ["AD-43"]
gates: ["c-abi-round-trip"]
version: 3
created_by: { type: human, id: simon }
updated_by: { type: agent, id: claude-code }
---

## Acceptance Criteria
...
```

Frontmatter edits by qdev are line-based patches that preserve comments and ordering.

---

<a id="schema"></a>

## 10. SQLite Cache Schema

The cache mirrors the files; every table row carries `source_path` and `content_hash`. Abbreviated:

```sql
CREATE TABLE entities (           -- common index for every entity kind
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT, status TEXT,
    owners TEXT, source_path TEXT NOT NULL, content_hash TEXT NOT NULL,
    version INTEGER, created_by_type TEXT, created_by_id TEXT,
    updated_by_type TEXT, updated_by_id TEXT, updated_at TEXT
);

CREATE TABLE stories (
    id TEXT PRIMARY KEY REFERENCES entities(id),
    epic_id TEXT NOT NULL, seq INTEGER NOT NULL,
    appetite TEXT CHECK(appetite IN ('tiny','small','medium','deep')),
    safety_class TEXT CHECK(safety_class IN ('ClassA','ClassB','ClassC')),
    target_modules TEXT              -- JSON array of module ids
);

CREATE TABLE constraints (
    id TEXT PRIMARY KEY,             -- 'E12S4/NG-1'
    owner_id TEXT NOT NULL, kind TEXT CHECK(kind IN ('no_go','rabbit_hole','appetite')),
    text TEXT NOT NULL
);

CREATE TABLE relations (
    source_id TEXT NOT NULL, relation TEXT NOT NULL, target_id TEXT NOT NULL,
    PRIMARY KEY (source_id, relation, target_id)
);

CREATE TABLE sprints (
    id INTEGER PRIMARY KEY, title TEXT, release_version TEXT,
    status TEXT CHECK(status IN ('planning','active','completed','paused','abandoned')),
    owners TEXT, started_at TEXT, completed_at TEXT
);

CREATE TABLE sprint_assignments (
    sprint_id INTEGER REFERENCES sprints(id), story_id TEXT NOT NULL,
    assigned_at TEXT NOT NULL, carried_from INTEGER,
    PRIMARY KEY (sprint_id, story_id)
);

CREATE TABLE decisions (
    id TEXT PRIMARY KEY, subject_id TEXT NOT NULL,   -- story, epic, or ADR
    decision_type TEXT CHECK(decision_type IN
      ('human_ruling','agent_assumption','cross_team_override','pivot','review_rejection','lease_override')),
    topic TEXT, context TEXT, ruling TEXT,
    author_type TEXT, author_id TEXT, created_at TEXT
);

CREATE TABLE deferred_work (
    id TEXT PRIMARY KEY, origin_story_id TEXT, target_module TEXT NOT NULL,
    status TEXT CHECK(status IN ('open','done','wont_fix')),
    safety_risk TEXT CHECK(safety_risk IN ('negligible','acceptable_with_mitigation','unacceptable')),
    rationale TEXT, gate TEXT, resolution TEXT
);

CREATE TABLE scratchpad_entries (
    story_id TEXT NOT NULL, seq INTEGER NOT NULL, at TEXT NOT NULL,
    author_type TEXT, author_id TEXT, kind TEXT, text TEXT,
    PRIMARY KEY (story_id, seq)
);

CREATE TABLE gates (
    id TEXT PRIMARY KEY, command TEXT NOT NULL, kind TEXT CHECK(kind IN ('check','ratchet')),
    timeout_ms INTEGER, output_adapter TEXT, on_transition TEXT,   -- JSON array
    depends_on TEXT, metric TEXT, direction TEXT
);

CREATE TABLE gate_runs (
    id TEXT PRIMARY KEY,             -- '<sha>-<gate>'
    story_id TEXT, gate_id TEXT NOT NULL, commit_sha TEXT NOT NULL,
    status TEXT CHECK(status IN ('pass','fail','infra')),
    exit_code INTEGER, duration_ms INTEGER, metric_value REAL,
    summary TEXT, evidence_path TEXT NOT NULL, output_hash TEXT,
    run_by_type TEXT, run_by_id TEXT, ran_at TEXT
);

CREATE TABLE soup_dependencies (
    id TEXT PRIMARY KEY, name TEXT, version TEXT, license TEXT,
    cve_status TEXT, introduced_by_story TEXT, evaluated_for_release TEXT
);

CREATE TABLE sync_state (path TEXT PRIMARY KEY, mtime INTEGER, size INTEGER, content_hash TEXT);
```

Requirements, hazards, ADRs, PRDs, and releases use the `entities` table plus kind-specific detail tables of the same shape.

---

<a id="storage-architecture"></a>

## 11. Storage Architecture & Hydration

Per **AD-3**, **AD-4**, **AD-6**:

1. **Boot sweep.** Stat every file under the configured spec and state directories. Compare `mtime` and size to `sync_state`. For changed files, hash the content; re-parse only if the hash differs.
2. **Parse and upsert** frontmatter and relations in one transaction. Dangling relations and schema violations are recorded as validation findings, not fatal errors.
3. **Conflict markers** (`<<<<<<<`) produce a `merge_conflict` finding for that file and hydration continues.
4. **Writes** take the advisory lock, write to a temp file, rename, then upsert the cache row and mark it dirty so the next sweep cannot skip it on a coarse-`mtime` filesystem.
5. **Optimistic concurrency.** Every entity carries `version`; `qdev update --if-version N` fails with exit code 5 on mismatch.
6. **Worktrees.** The cache lives inside the worktree at `.qdev/cache/`; each worktree has its own. Leases record the worktree path so `qdev next` can avoid double assignment across worktrees.

Target: ≤ 30 ms for 1,000 entities with one change.

---

<a id="projection"></a>

## 12. Context Projection & Model Tiering

`qdev context <id> --phase <specify|develop|review> --budget <tokens>` is the single source of agent context. Skills and MCP tools call it rather than assembling context themselves.

| Phase | Includes | Typical budget |
| --- | --- | --- |
| `specify` | Epic goal and constraints, sibling story titles, governing ADR summaries, linked requirements | ~800 |
| `develop` | Story spec, inherited constraints with IDs, module paths, ADR excerpts, scratchpad summary, hygiene directive, bound gates | ~1,200 |
| `review` | Everything in `develop` plus the diff summary, gate receipts, and evidence paths | ~2,500 |

The projection is deterministic, reports its own token estimate with `--stats`, and truncates lowest-priority sections first when over budget. Model choice per phase is configuration (`[models]` in `qdev.toml`), not specification; the recommended pattern is a reasoning model for `specify`, a fast coding model for `develop`, and the strongest available model for `review`.
