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

Sequential planning IDs are allocated by `qdev create`, which takes the lowest number the workspace's [in-use id set](#identity-in-use) leaves free. `qdev validate` reports duplicates arising from concurrent planning on separate branches and offers a guided renumber that rewrites citations and renames the file to carry the new id, which is the only sanctioned rename. Sub-stories are expressed with the `extends` relation, not with suffixes.

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

### Where the layout is configured

The three directories above are `[storage]` keys, and `qdev init` resolves them through the same
configuration loader every other command uses — so what `init` scaffolds, stamps, gitignores and
reports is the layout the next command will read, and an unparseable or invalid configuration file
is the same exit-2 refusal from `init` as from everything else.

`specs_dir` and `state_dir` may be set **only** in the committed `qdev.toml`: they select where
committed content lives, so the layout is a project decision, and either key in
`.qdev.local.toml` is a schema error (exit 2, naming the key and the file). `cache_dir` is the
exception — the cache is a machine-local, rebuildable artifact, so a developer may relocate it in
`.qdev.local.toml`. One consequence is deliberate and worth stating, because `.gitignore` is
committed: `init` writes ignore entries for the project cache directory **and** for any locally
configured one, so a relocated cache is never untracked merely by luck while the committed file
stays meaningful to everyone else. The directory hosting `gates/` and `leases/` follows the
*project* cache directory's parent, so committed gate scripts do not move with a developer's
local override.

### Identity: a file is named for the entity it holds

One rule answers "which file is entity `X`?" for every reader and every writer. `X` lives in its
kind's directory above, in a file named `X.md`, `X-<slug>.md` or `X_<slug>.md`. Hydration answers
from the frontmatter `id` and records the file's path; the write path answers from the file name,
which under this convention is the same file — so it needs no cache dependency and `qdev create
story` still works outside an initialised workspace. Frontmatter `id` remains the entity's
identity; the file name is how a writer finds it.

The convention is enforced rather than assumed: `qdev validate` reports
`entity_file_off_convention` (`warning`) for a hydrated entity whose file name does not carry its
id or that lives outside every entity directory — readable, since hydration walks both trees
recursively, but not writable. Two files matching one id is a usage error naming both. The only
sanctioned rename is `qdev validate --fix-ids`, which renames as it renumbers so the renumbered
entity is immediately writable, and reports the move in its payload.

<a id="identity-in-use"></a>

### In use: one rule for which ids are taken

The rule above answers "which file is entity `X`?". Its completion answers "is `X` already
taken?", and **one function answers it for everything that allocates an id** — `qdev create
story` and `qdev validate --fix-ids` alike. Neither keeps a scan of its own: four components used
to answer this question and the allocator answered differently (one flat directory,
case-sensitively, filenames only), so `create story` handed out ids other files already declared.

An id is in use if it is any of three things, because each is a way a workspace already owns it:

- **declared** — the frontmatter `id` of any file under `specs_dir` or `state_dir`, recursively,
  matching the `.md` extension case-insensitively: exactly the set hydration reads, so an id qdev
  can read is an id qdev treats as taken;
- **carried** — the id a file *name* in those trees carries under the convention above. A file
  whose frontmatter will not parse declares nothing yet still occupies its name, and allocating
  that id would refuse the create with `file_exists` for an id the user never chose;
- **cached** — any id the cache holds for a hydrated entity, so an id survives its file becoming
  unreadable. The cache is a union *member*, not the source: `qdev create story` works in a bare
  directory with no cache at all, from the filesystem alone.

`qdev create story` allocates one past the highest number its epic has used — never a lower free one, because an id is a citation target and AD-7 calls them immutable once committed, so a gap is left deliberately. `qdev validate --fix-ids` takes the lowest free number instead (`next_available_id`), because a renumber must land somewhere free. Both read the same in-use set, and
refuses rather than colliding when it cannot. Two files *carrying* one id in their names without
either declaring it is an off-convention name, not a collision — `duplicate_planning_id` still
reports declarations only.

Because the whole in-use set is checked, `--fix-ids` can meet a duplicate whose file lives outside
its kind's directory. It refuses that entry, records it as skipped and names the expected path:
renumbering in place would write a correctly named file in a directory no writer resolves — a
repair it did not achieve — and moving a user's file is not a decision the tool takes silently.
The `entity_file_off_convention` warning on the same file already names the destination.

An entity's *kind* is resolved by one rule too — frontmatter `kind:`, then the directory, then
the identifier grammar, then `Story` — used by hydration and by every writer, so a write
validates against the schema the following sweep validates against.

Frontmatter edits by qdev are line-based patches that preserve comments and ordering.

---

<a id="schema"></a>

## 10. SQLite Cache Schema

The cache mirrors the files; every table row carries `source_path` and `content_hash`.

The cache schema version is the value of `CACHE_SCHEMA_VERSION` (currently 3), carried by
`PRAGMA user_version` alone — the single application-owned stamp, written only by
`stamp_cache_version`. SQLite's internal `schema_version` cookie is never read or written: it is
incremented automatically on every DDL statement, so it cannot distinguish a healthy cache from
a stale one. Cache validity is `user_version` plus a table-presence and column check. A cache
stamped older is rebuilt from the files; one stamped newer than the binary supports is refused
with `schema_version_mismatch` (exit 5), recoverable with `qdev sync --rebuild` or by deleting
the cache file. If a second version dimension is ever needed it belongs in `sync_meta` as an
ordinary row, not in a pragma.

Abbreviated:

```sql
CREATE TABLE entities (           -- common index for every entity kind
    id TEXT PRIMARY KEY, kind TEXT NOT NULL, title TEXT, status TEXT,
    owners TEXT, source_path TEXT NOT NULL, content_hash TEXT NOT NULL,
    version INTEGER, created_by_type TEXT, created_by_id TEXT,
    updated_by_type TEXT, updated_by_id TEXT, updated_at TEXT,
    stale INTEGER NOT NULL DEFAULT 0  -- last parse failed; row kept, marked stale
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

-- Hydration bookkeeping. `sync_state` above answers "has this file changed"; these three
-- answer "what did the last pass find", "what must the next pass re-read regardless of
-- mtime", and "when did we last sweep".
CREATE TABLE findings (
    path TEXT NOT NULL, code TEXT NOT NULL, severity TEXT NOT NULL,
    message TEXT, message_key TEXT NOT NULL, found_at TEXT NOT NULL,
    PRIMARY KEY (path, code, message_key)   -- message_key: several findings of one code per file
);

CREATE TABLE dirty_entities (id TEXT PRIMARY KEY, dirty_at TEXT);

CREATE TABLE sync_meta (id INTEGER PRIMARY KEY CHECK (id = 1), last_synced_at TEXT NOT NULL);
```

That is 16 tables (`ALL_TABLE_NAMES` in `store/sqlite.rs` is the authority; `inspect_cache_schema`
requires every one of them to be present). Requirements, hazards, ADRs, PRDs, and releases use the
`entities` table plus kind-specific detail tables of the same shape.

---

<a id="storage-architecture"></a>

## 11. Storage Architecture & Hydration

Per **AD-3**, **AD-4**, **AD-6**:

1. **Boot sweep.** Stat every file under the configured spec and state directories. Compare `mtime` and size to `sync_state`. For changed files, hash the content; re-parse only if the hash differs. A file whose metadata is unchanged is still re-parsed when the cache holds no row claiming it and no finding explaining why — see [the convergence invariant](#convergence-invariant), which is what that gate cannot assume on its own.
1a. **What "unchanged" is allowed to mean.** A file is skipped only when its content is known to be unchanged *and* it is still readable — the gate compares content, not proxies for content. Four rules make that true without giving up the incremental sweep, and a fifth layer in the write path backs them up:
    - `sync_state.mtime` holds a **change stamp in nanoseconds**: the later of the file's mtime and its ctime, at whatever precision the filesystem provides. mtime alone answers "was it written"; ctime also moves when its permissions change, which is how a file becomes unreadable without "changing". Whole seconds made a same-length edit landing in the same wall-clock second as the previous hydration invisible forever, which is the default shape of a machine-speed agent edit. A value written by an older binary (whole seconds) compares unequal to a nanosecond one, so every file is re-read once after the upgrade and the cache self-heals — no schema version bump and no migration.
    - A **whole-second mtime is unresolved, not unchanged.** When the stored and current mtimes are equal and carry no sub-second part, all they establish is that the file was last written somewhere inside that second, so the file is read and hashed rather than trusted; the hash then decides whether it re-parses. That is the state a coarse-granularity filesystem (HFS+, some network mounts) leaves on every write, and it is the smallest window a full-precision comparison cannot resolve — hashing exactly it costs a read for recently touched files rather than a hash of the whole workspace on every boot, which AD-6's 30 ms budget rules out.
    - **Becoming unreadable is a change.** A file that became unreadable with no change to its mtime or size used to be skipped outright, so `qdev validate` exited 0 where `sync --rebuild` exits 1. Folding ctime into the stamp catches it from the `stat` the sweep already makes: the stamp moves, the file takes the read-and-hash path, and the failed read is recorded as a `read_error`. The alternative — opening every file the gate would skip — was measured and rejected: it doubled the warm sweep (6 ms to 14 ms at N=1000) to answer a question `stat` already answers. **ctime is a Unix facility**; on Windows the stamp is mtime alone, so a permission-only change there is not seen until the file is otherwise touched.
    - A file that reads again with unchanged content has its `read_error` and its stale flag **cleared**, since a rebuild of the same tree would have neither.
    - The write path's `sync_state` delete and dirty mark is the backing layer and the reason a qdev-originated write was never the case at risk: only out-of-band edits (an agent editing Markdown, `sed`, `git checkout`) reach this gate without a dirty row.
2. **Parse and upsert** frontmatter and relations in one transaction. Schema violations are recorded as validation findings, not fatal errors, and the file keeps its previous cache row marked `stale`.
2a. **Revalidate the relation graph** once every file has been parsed — dangling targets, disallowed kind pairs and `depends_on` cycles are re-derived from the whole `entities`/`relations` tables, not just the files touched this pass, so an out-of-band edit elsewhere is caught. The three codes are cleared first, so a relation problem that no longer exists stops being reported.
3. **Conflict markers** (`<<<<<<<`) produce a `merge_conflict` finding for that file and hydration continues.
4. **Writes** take the advisory lock, write to a temp file, rename, then upsert the cache row and mark it dirty so the next sweep cannot skip it on a coarse-`mtime` filesystem.
5. **Optimistic concurrency.** Every entity carries `version`; `qdev update --if-version N` fails with exit code 5 on mismatch.
6. **Worktrees.** The cache lives inside the worktree at `.qdev/cache/`; each worktree has its own. Leases record the worktree path so `qdev next` can avoid double assignment across worktrees.

Target: ≤ 30 ms for 1,000 entities with one change.

<a id="convergence-invariant"></a>

### The convergence invariant

The incremental sweep and the full rebuild (`qdev sync --rebuild`) are two implementations of one thing. **Hydrating the same tree by either path leaves the same rows in every table and the same findings.** The sweep is an optimisation, never a different answer, and it is asserted directly as table-by-table equality rather than by checking individual symptoms. Three rules keep it true:

- **A purge deletes only what the removed file owned:** its `entities` row, its detail and constraint rows, and the relations it *declared* (`source_id`). Edges pointing **into** it are declared by other files, which are unchanged and will not be re-parsed, so deleting them would destroy state no later pass restores. They stay, and `dangling_relation` reports them — the same answer the rebuild gives by re-reading the declaring file. `relations` has no foreign key precisely so a row pointing at an absent entity is representable. A dangling edge is reported, never repaired: qdev neither invents nor deletes a relation a file declares, so `get --expand relations` and `graph --dot` show the edge, which is the truth about the workspace.
- **Every known, readable entity file either has an `entities` row claiming it or a finding explaining why it does not** — `schema_violation`, `merge_conflict` or `read_error`, the three codes that explain a missing row. A relation-graph finding does not count: those are recorded against files that parsed successfully. The sweep enforces this on the files its change gate would otherwise skip, because a cascading purge or another file taking over an id can invalidate rows without the owning file changing. Both halves are answered from the maps the sweep already loads, so this costs no extra I/O and the 30 ms budget keeps its shape.
- **An unreadable file produces a `read_error` finding from both paths.** Neither swallows it — including a file that became unreadable with *no* change to its mtime or size, which the sweep sees because its change stamp folds in ctime (see 1a above; on Windows, which has no ctime, that case waits until the file is otherwise touched).

Two deliberate exceptions, both narrow:

- **Retention.** A file that parsed before and now fails to parse or read keeps its previous rows flagged `stale`; a rebuild has no previous rows to retain. The findings agree because a stale row is treated as *absent* by everything that derives a finding: relation validation reads only live entities and only the edges a live entity declares, so an edge into a stale entity dangles (as it does on a rebuild) and a stale entity's own retained edges are invisible (as they are on a rebuild). `qdev sync --rebuild` remains the guaranteed repair for anything else.
- **Stale rows are excluded from computed checks.** `qdev validate`'s four cache-reading checks (`orphan_deferred_work`, `dw_missing_rationale`, `target_module_not_registered`, `entity_file_off_convention`) skip stale rows: qdev does not assert things about a file it could not read, the file already carries the `schema_violation`, `merge_conflict` or `read_error` finding naming the actionable problem, and a rebuild has no stale rows to derive from. Reads are untouched — `qdev get` and `qdev list` still return a stale entity with its `stale` flag set, which is what the retention exists for.

**When two files declare one id, the last in sorted path order owns the cache row.** Both paths follow it — the rebuild by walking sorted, the sweep by re-parsing both files (each is unaccounted-for once the other takes the row) and finishing with the same winner. Note `qdev validate --fix-ids` renumbers the *other* way round, keeping the id on the first sorted path, so the file it renumbers is the one holding the row.

Duplicate-id detection (`duplicate_planning_id`) covers every directory hydration reads — `specs_dir` **and** `state_dir` — so a collision among sprints, deferred work, decisions, releases or SOUP is reported like one among stories.

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
