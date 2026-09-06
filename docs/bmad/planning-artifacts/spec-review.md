---
title: "qdev Specification Review"
status: applied
created: 2026-09-06
applied: 2026-09-06
applied_note: >
  All recommendations were applied on 2026-09-06 to the Architecture Spine (AD-1 … AD-14),
  PRD, the five design guides, the README, and a full re-cut of epics.md (49 stories).
  docs/initial_planning.html was left unmodified as the historical record.
reviewed_inputs:
  - docs/architecture.md
  - docs/cli-reference.md
  - docs/compliance-and-safety.md
  - docs/governance-and-teams.md
  - docs/roadmap.md
  - docs/initial_planning.html
  - docs/bmad/planning-artifacts/prd-1/prd.md
  - docs/bmad/planning-artifacts/architecture-1/ARCHITECTURE-SPINE.md
  - docs/bmad/planning-artifacts/epics.md
  - docs/bmad/implementation-artifacts/sprint-status.yaml
---

# qdev Specification Review

## 1. Verdict

The core thesis is strong and well argued: the planning graph is relational, flat markdown fails at scale, and negative constraints belong in the prompt. The problem is that the repository currently holds **two specifications, not one**.

- The **design spec** (`docs/*.md`, split from `docs/initial_planning.html`) is the richer document: full data model, CLI grammar, lifecycle, compliance mapping, seven-phase roadmap.
- The **BMAD artifacts** (`prd.md`, `ARCHITECTURE-SPINE.md`, `epics.md`) are the bootstrap plan: BMAD is being used to build qdev, and qdev is not required to import or continue BMAD projects. Even so, the epics disagree with the design spec on the data model, IDs, state machine, file format, CLI grammar, and scope, and they introduce invariants that undermine promises the design spec makes.

This matters because the epics are what BMAD will implement. Wherever they diverge from the design docs, the built tool will follow the epics. One document must be declared authoritative and the other brought into line before any story is implemented. Section 6 proposes an order of work.

---

## 2. Divergence Matrix

| Topic | Design spec (`docs/*.md`) | BMAD artifacts (`docs/bmad/`) |
| --- | --- | --- |
| Story IDs | `S5E2S4`; "immutable semantic IDs" | `story-1`, `epic-1`, `Story 5`; Story 4.1 also uses `S1E2` |
| Story states | `backlog, ready, in-progress, review, done, escalated` | PRD FR-202/203: `specify → develop → review → done` plus a `pivot` state. Epics: "strictly linear", no pivot state, backward transitions with justification instead |
| Source file format | Markdown with YAML frontmatter | AD-3: "JSON/YAML"; AD-6: "all JSON files"; Epic 1 invariant: "signatures to the JSON files"; Story 1.5: hash in YAML header of Markdown |
| Cache path | `.qubric/qdev.db` | `.qdev/qdev.sqlite`; gates in `.qdev/gates/` |
| Gate model | Inline `command` in `[[gates]]`; `gate_type` enum; Red-fixture, Dual-limb, PHI-leak verifiers **built into the binary** (roadmap Phase 6) | AD-5: gates are external scripts only; nothing project-specific in the binary |
| Hydration budget | ~25 ms (architecture), <30 ms (roadmap) | <5 ms (Story 1.2), "sub-millisecond boot" (Epic 1 invariant) |
| Hydration mechanism | `mtime` + SHA-256 (roadmap), `mtime` + content hash (architecture) | `mtime` + size (epics) |
| CLI grammar | `qdev <verb> <noun>` throughout | `qdev mutate`, `qdev transition`, `qdev chore`, `qdev sprint finalize`, `qdev install-git-hooks`, `qdev generate-skills` |
| Persona for `init` | Non-interactive scaffold | Interactive prompt wizard (Story 1.1) |
| Implementation plan | Seven sequential phases | Four epics, eighteen stories, materially different scope |

### 2.1 FR coverage map is optimistic

`epics.md` claims full FR coverage. Checking each claim against the actual stories:

| FR | Mapped to | Reality |
| --- | --- | --- |
| FR-601 Artifact catalog | Epic 1 | No story implements it |
| FR-403 Git integration | Epic 3 | Story 3.3 is the commit-message hook only; the preflight guard (dirty tree, remote sync) has no story |
| FR-402 Epic & sprint reviews | Epic 3 | Story 3.4 is a sprint snapshot only; no epic review, no deferred-work rollover |
| FR-302 Relational scratchpads | Epic 1 | Referenced by other stories but no story creates the scratchpad, its storage, or its CLI |

### 2.2 Roadmap v1 items with no story in the epics

Deferred work (`DW-*`) CLI · Decision ledger (`DEC-*`) · ADRs as an entity · Entity relations (`depends_on`, `extends`, `supersedes`, `traces_to`, `closes_dw`) · `qdev impact` · Hygiene linter (`qdev hygiene --check`) · Root status / "What To Do Next" · `qdev doctor` · `qdev install skills|mcp|hooks` · SOUP audit & SBOM · Residual anomaly report · Multi-sprint concurrency · Releases · Worktree isolation · Multi-perspective synthesis.

### 2.3 Epic additions absent from the design spec

Some are good and should be kept; some are defects (see §3).

| Addition | Keep? |
| --- | --- |
| `author_type` / `author_id` on every mutation | Keep |
| `pre_transition` / `post_transition` lifecycle hooks | Keep |
| Gate SIGKILL timeouts with bounded stderr ring buffer | Keep |
| Logical vs. Infrastructure failure taxonomy, halt-and-alert on infra failure | Keep |
| Attributed rejections citing constraint IDs | Keep (requires §3.8) |
| Opt-in `prepare-commit-msg` hook | Keep, low priority |
| Tamper-evident frontmatter hash | Drop (§3.3) |
| Forbidding direct edits to entity files | Drop (§3.4) |
| Hierarchical RBAC | Replace with leases (§3.5) |
| Chore hunk tracking | Replace with path allowlist (§3.6) |

---

## 3. Design Defects

### 3.1 Story IDs embed the sprint number, yet stories are declared orthogonal to sprints

`governance-and-teams.md` §10 says the functional tree (PRD → Epic → Story) persists across time and sprints are a separate temporal space. But the ID grammar is `S{sprint}E{epic}S{story}`, and `epics.sprint_id` pins each epic to exactly one sprint.

Stage 7 ("atomically rolls over open debt or incomplete stories into the next sprint") therefore requires **renaming the story**, which invalidates every `// [S5E2S4]` citation in code and every `entity_relations` row. That is the line-anchor-rot problem the tool exists to eliminate.

**Fix:** Sprint-free story IDs (e.g. `E12S4`, or epic slug + seq). Model sprint membership as a `sprint_assignments(story_id, sprint_id, assigned_at, carried_from)` join table so carry-over is a new row, not a rename. Drop `epics.sprint_id`. Remove `id_prefix = "S5"` from config, which is incompatible with multiple active sprints anyway.

### 3.2 Globally monotonic counters collide under Git

`AD-*`, `DW-*`, `DEC-*` are "globally monotonic". Two branches each allocate `DW-422`; the merge succeeds and the IDs are silently ambiguous. `git worktree` isolation (roadmap §21.3) makes this routine, not rare.

**Fix:** Short content/random hash suffixes (`DW-7f3a`, as Beads does), or ULIDs, or sequential IDs plus a mandatory collision check in `qdev doctor`/`qdev validate` with a guided renumber that rewrites citations. Prefer the first; it needs no coordination.

### 3.3 The tamper-evident frontmatter hash should be dropped

Story 1.5 injects `qdev_integrity_hash` into frontmatter on every mutation and halts hydration on mismatch.

- Every edit churns the hash line, so **disjoint edits on two branches always conflict**.
- It provides no security: the algorithm ships in an open-source binary and anyone can recompute it.
- It contradicts "Zero Lock-in", "clean reviewable PR diffs", and "human-readable Git source of truth".
- Halting hydration on mismatch locks out an innocent hand-fix and makes the entity invisible to every command.
- Git already records who changed what and when; signed commits exist if provenance must be cryptographic.

**Fix:** Remove. Record `author_type`/`author_id` in scratchpad append entries and optionally in commit trailers. Keep the conflict-marker detection from Story 1.5; it is genuinely useful.

### 3.4 Forbidding direct edits is unenforceable

Epic 1's "Write-Through Mutator" invariant forbids humans and agents from editing entity files directly. Advisory file locks do not stop VS Code, and the "source of truth is human-readable Markdown" promise is meaningless if humans may not touch it.

**Fix:** The CLI is the *convenient* path, not the *only* path. Validate on hydration (schema, enums, relation targets) and surface violations via `qdev validate`. Keep field-level mutations (`qdev update story X --status ready`) cheap for agents; block-replace and patch-file modes are more token-expensive than flag-based updates and should be secondary.

### 3.5 RBAC has no principal model

Story 2.2: "an AI agent assigned to Story-2 attempts to `qdev mutate epic-1` … rejected with an Authorization Error." Identity comes from a gitignored `.qdev.local.toml` that anyone can edit, and an agent process runs *as the user*. Nothing tells qdev which process is which, so "agent assigned to Story-2" is undefined.

**Fix: story leases.** `qdev claim S5E2S4` writes a lease (holder, worktree, started_at) and emits a session token via `QDEV_SESSION`. Mutations outside the leased story's subtree require `--override --justification`, logged as a `cross_team_override` decision. Leases also stop two agents claiming the same story and give worktree orchestration a primitive to build on. This delivers the intent of Story 2.2 and §9 governance without pretending to be an access-control system.

### 3.6 Chore hunk tracking cannot know intent

Story 2.5 wants the chore wrapper to detect which hunks "belong to" the README fix and exclude an accidental change to `src/main.rs`. A hunk carries no task identity; the only discriminator available is path.

**Fix:** `qdev chore "fix readme typo" --paths README.md docs/**`. Commit refuses (or excludes with a warning) any change outside the allowlist. Chores still run the hygiene gate and any gate marked `always`.

### 3.7 No module-to-path registry

Module boundaries are a headline feature. They are needed by: the preflight "uncommitted changes outside the active story scope" check, `target_modules` enforcement in `/qdev-develop`, `qdev impact`, the architectural-segregation gate (IEC 62304 §5.3.5), and hygiene scoping. Yet `[modules] workspace_members = [...]` is a bare list of names with no paths.

**Fix:**
```toml
[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]
layer = 2                      # for downward-only dependency checks
may_depend_on = ["foundation"]
```

### 3.8 Constraints are free text but rejections must cite constraint IDs

`stories.rabbit_holes TEXT` and `no_gos TEXT` versus Story 4.2's `constraint_id: "NG-42"`. Constraints need to be rows.

**Fix:** `constraints(id, owner_id, kind CHECK(kind IN ('rabbit_hole','no_go','appetite')), text, created_by)`, with IDs scoped to the owner (`S5E2S4/NG-1`). Epic-level constraints inherit downward (Story 4.1 already assumes this climb).

### 3.9 The schema is missing entities the spec depends on

| Missing | Why it matters |
| --- | --- |
| `adrs` table | ADRs are one of the four core BMAD tree nodes and are cited everywhere; only a `spec_path` convention exists |
| `requirements` table | The PRD uses `FR-xxx`/`NFR-xxx`; `traces_to` targets "PRD / Requirement" but there is nothing to point at. Without it there is no traceability matrix, which the compliance doc promises at Stage 7 |
| `hazards` table | `// [HAZ-014]` citations and the Hazard Analysis artifact have no home |
| `scratchpad_entries` table or file | FR-302 says "in the relational database"; Story 2.4 says "in the YAML file" (see §3.10) |
| `verifies` relation (Gate → Requirement) | Needed for the RTM; `gate_runs` only links to stories |
| `sprint_assignments` | See §3.1 |
| `leases` | See §3.5 |
| `decisions` scope | `decisions.story_id` only; ADRs and epics also receive rulings. Use a polymorphic `subject_id` |
| `soup_dependencies` provenance | No link to the story or commit that introduced the dependency |
| `gate_runs` evidence | No `evidence_path`, `output_hash`, or `run_by` |

### 3.10 Scratchpad placement is undecided and both candidates are wrong

- In the gitignored SQLite cache: lost on clone, invisible to teammates and auditors, contradicts AD-3.
- Inline in the story Markdown: bloats the exact file the agent loads on every `get`, recreating context sprawl.
- Story 1.3 additionally returns the full scratchpad in every `qdev get epic … --json`, contradicting NFR-301.

**Fix:** One append-only file per story under `docs/state/scratch/<id>.md` (JSONL or dated Markdown sections), committed, hydrated into a `scratchpad_entries` table, and returned only via `qdev scratch read` or `--with-scratchpad`. The projection for `/qdev-develop` includes a bounded *summary*, not the ledger.

### 3.11 The "behind remote" preflight rule is wrong for story branches

`branching_mode = "story-branch"` means a feature branch is almost always behind `origin/develop` once anything else merges. The guard as specified would block nearly every story start.

**Fix:** Check (a) whether the local integration branch is behind its remote, and (b) whether the story branch's merge-base with the integration branch is older than a configurable threshold (commits or days). Make both independently configurable.

### 3.12 Windows is claimed but the design is POSIX

NFR-101 requires Windows. Story 3.2 uses SIGKILL; hooks are shell scripts in `.git/hooks`; file locking semantics differ; `mtime` resolution differs. "Statically linked" is also not achievable on macOS (libSystem must be dynamically linked); say "self-contained, no external runtime".

**Fix:** Use a cross-platform kill (`Child::kill` plus job objects on Windows). Install hooks as a two-line shim that calls `qdev hook <name>` so logic lives in the binary. Use OS-appropriate advisory locks via a crate such as `fd-lock`.

### 3.13 Interactive prompts block agents

Story 1.1's `qdev init` wizard and the §9 three-option cross-team interlock both prompt on stdin. Agents and CI need every prompt to have a flag equivalent and a `--non-interactive` / `QDEV_NONINTERACTIVE=1` mode that fails closed with a structured error.

### 3.14 Gate output compression assumes cargo

The ~95 % compression and "extracts only the failing assertion" presuppose parsing `cargo test` output. A language-agnostic tool cannot ship parsers for every runner.

**Fix:** Per AD-5, define a **gate result contract**: a gate may write `{ "status": "pass|fail|infra", "summary": "...", "failures": [...] }` to stdout or to `$QDEV_RESULT_FILE`. If it does not, qdev falls back to exit code plus the last N lines of stderr. Ship parsers for cargo and swift/xcodebuild as optional built-in adapters, selected by `output_format` in the gate config.

"Ratchet" appears repeatedly ("Ratchets at baseline") and is never defined. Specify it: a gate that reports a numeric metric, a direction (`must_not_increase`), and a per-branch baseline stored under `docs/state/baselines/`.

### 3.15 Automatic relocation of memoir comments is risky

Phase 5 and the Compliance doc promise to *automatically* move forensic comments into the scratchpad. Distinguishing a memoir from a legitimate design comment is a judgment call; a false positive silently deletes documentation from source.

**Fix:** v1 = lint and report (`qdev hygiene --check` fails the gate and cites file:line and the matched rule); the agent performs the fix. v2 = `--fix` with a diff preview. Comment syntax must be per-language (`//`, `///`, `#`, `/* */`, `"""`), and `citation_format` must not hardcode `//`. Consider tree-sitter for comment extraction rather than regex.

### 3.16 Compliance claims outrun the design

- The README promises "cryptographically verifiable evidence receipts". Nothing in any document signs anything.
- Using a different model for review is not "verification independence" in the IEC 62304 sense.
- Badging the project "IEC 62304 / ISO 14971 ready" invites scrutiny the current design cannot survive.
- A tool used to generate design-history evidence in a regulated process may itself require **tool validation**. This is nowhere in the docs and belongs on the roadmap.

**Fix:** Soften README language to "designed to support" until the requirements model (§3.9), evidence bundles (§5.1) and a signing story exist. Add a "qdev tool validation" item to v2.

### 3.17 Smaller items

- `escalated` state appears once (architecture schema) and nowhere else.
- `supersedes` "retires" the target but no retired/superseded status exists.
- Sub-story IDs (`S3E1S4a`) break `stories.seq INTEGER`.
- `stories.version INTEGER` is a good optimistic-concurrency hook (`--if-version 3`) that nothing uses. Make it explicit.
- `gates.command NOT NULL` contradicts AD-5's external scripts.
- `entity_relations` has no FK or type validation; relation targets must be validated at hydration.
- No dependency-cycle detection is specified for `depends_on`.
- No error output format, exit-code convention, or JSON schema versioning is defined, despite "universal `--json`" being a headline.
- No test strategy for qdev itself (golden-file hydration tests, property tests for the ID grammar, JSON snapshot tests).
- The PRD has no personas, success metrics (e.g. median tokens per story context), risks, out-of-scope list, security stance (secrets or PHI in scratchpads), or telemetry stance (should be explicitly zero-telemetry given offline-first).
- Model tiering names specific vendors' models in the spec; this belongs in config.
- "Red-fixture", "Dual-limb", "PHI leak" are Qubric-specific gate names used without definition in a tool positioned as generic.
- Story sizes in `epics.md` vary from a single prompt string (4.3) to init + schema + migration (1.1). Eighteen stories is too few for the roadmap's v1 scope; expect 40–60 once re-cut.

---

## 4. Things That Are Right and Should Survive Reconciliation

- Markdown-with-frontmatter source of truth, SQLite as a rebuildable cache (AD-3, AD-6).
- Synchronous `rusqlite`, WAL, short transactions, no async runtime (AD-2, AD-4).
- Two-crate split (AD-1).
- Sub-process gates with exit-code semantics (AD-5), once paired with a result contract.
- Negative-constraint injection and attributed rejections.
- Logical vs. infrastructure failure taxonomy with halt-and-alert.
- Lifecycle hooks as the extension point for gates.
- Author attribution on mutations.
- Zero-noise pass receipts.
- The "What To Do Next" pulse as the default command.
- Structured Multi-Perspective Synthesis replacing persona theatre.

---

## 5. Feature Recommendations

### 5.1 Add to v1

| Feature | Rationale |
| --- | --- |
| **`qdev context <id> --budget N [--phase specify\|develop\|review]`** | The token-bounded projection is the product's core value and no command produces it. Skills should call this rather than assemble context themselves; it can then be tested in isolation and its token counts measured |
| **Self-hosting handover** | Once the substrate (Epic 1) and state machine (Epic 2) land, re-enter the remaining epics as native qdev entities and finish the build under qdev. This is the first real dogfood test and does not require a generic BMAD importer |
| **`qdev mcp serve`** | Referenced by `qdev install mcp` and `qdev doctor` but never defined. For an agent-first tool the MCP surface matters more than slash skills. Define tools as thin wrappers over `qdev-core`: `get_entity`, `list_ready`, `context`, `scratch_append`, `transition`, `gate_run` |
| **Story leases** (`qdev claim / release`) | Replaces RBAC (§3.5); prerequisite for parallel agents |
| **Module registry with paths** | §3.7; prerequisite for boundaries, impact, preflight scope |
| **Committed evidence bundles** | `evidence_dir` is configured and `.evidence.json` is named in Phase 6, but no story writes it. One JSON per gate run per commit under `docs/state/evidence/<story>/` |
| **`qdev validate`** | Dangling relations, dependency cycles, orphan DW, `acceptable_with_mitigation` without rationale, ID collisions, schema violations. Also the enforcement mechanism that replaces §3.3/§3.4 |
| **Versioned JSON output + `qdev schema`** | `schema_version` in every payload; published JSON Schema; documented exit codes and error envelope |
| **`qdev next --json`** | The "What To Do Next" selection as a real command with a defined ordering (unblocked → owner match → epic phase → seq) so an unattended loop can drive it |
| **`qdev graph --dot`** | Trivial to emit; unblocks visual review now rather than in v2 |
| **Non-interactive mode** | §3.13 |
| **Requirements entity + `traces_to` + `verifies`** | Minimum data model to make the compliance claims defensible |

### 5.2 v2 and beyond

- **Worktree-per-story orchestration** (`qdev run --parallel`), built on leases.
- **Requirements Traceability Matrix export** and a **signed evidence chain** (minisign/sigstore) once requirements and evidence bundles exist.
- **qdev tool validation package**: self-test suite and validation report for regulated users.
- **Ratchet baselines per branch** with history and drift reports.
- **Token accounting per story phase**, captured from agent transcripts, to measure the savings thesis rather than assert it.
- **Gate packs** per ecosystem (rust, swift, python) distributed outside the binary.
- **Configurable model routing** per phase, replacing hardcoded model names.
- **Hygiene `--fix`** with diff preview (§3.15).
- **Retrospective ingestion and velocity metrics** at sprint close.
- Already listed: web dashboard, Jira/Linear sync, PDF dossier, Postgres backend.

---

## 6. Recommended Order of Work

1. **Reconcile into one canonical spec.** Use the design spec as the base. Fold in the epics' good additions (author attribution, lifecycle hooks, timeouts, failure taxonomy, attributed rejections). Resolve every row in §2 explicitly.
2. **Fix the data model** before anything else, since it affects every downstream story: sprint-free IDs, collision-free counters, `sprint_assignments`, `requirements`, `adrs`, `hazards`, `constraints`, scratchpad placement, leases.
3. **Drop** the frontmatter hash, the direct-edit ban, and hunk tracking. **Replace** RBAC with leases.
4. **Define the two contracts** everything else plugs into: the module registry and the gate result contract (including ratchets).
5. **Define the CLI surface** once: verb/noun grammar, JSON envelope, exit codes, non-interactive mode, and the `context` / `next` / `validate` / `mcp serve` commands.
6. **Re-cut the epics and stories** from the reconciled spec. Expect 40–60 stories; size them consistently. Plan a handover point after Epics 1 and 2 where the remaining work is re-entered as qdev entities and the build continues under qdev itself.
7. **Soften compliance language** in the README until §3.9 and §5.1 evidence bundles exist.
