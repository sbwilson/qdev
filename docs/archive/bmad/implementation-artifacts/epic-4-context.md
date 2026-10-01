# Epic 4 Context: AI Context & Integration

<!-- Compiled from planning artifacts. Edit freely. Regenerate with compile-epic-context if planning docs change. -->

## Goal

Make the tool usable by AI agents end-to-end: give every agent exactly the context it needs for a phase within a token budget, make every refusal explain itself with a citable ID, and expose all operations through generated slash-command skills and an MCP server. This is where qdev's core thesis — token-efficient, boundary-aware, traceable agent work — is actually realized. The epic's exit criterion: a story executed end-to-end through `/qdev-develop` and `/qdev-review` in Claude Code with payloads under budget.

## Stories

- Story 4.1: `qdev context` Projection
- Story 4.2: Attributed Rejections
- Story 4.3: Hygiene Directive & Citation Template
- Story 4.4: Skill Generation & Install
- Story 4.5: MCP Server
- Story 4.6: The Five Skills
- Story 4.7: Doctor: Environment, Skills, MCP, Hooks, Leases
- Story 4.8: Structured Multi-Perspective Synthesis Template
- Story 4.9: `qdev graph` Rendering Options
- Story 4.10: Self-Hosting Handover

## Requirements & Constraints

- Context payloads must be token-budgeted and phase-specific. Median targets: develop ≤ 1,200 tokens, review ≤ 2,500. Over budget, lowest-priority sections truncate first and the payload lists what was truncated.
- Payloads must include inherited negative constraints (appetites, rabbit holes, no-gos, target modules) — they are first-class entities that inherit downward from epic to story and are injected into every context.
- Queries must return only the requested entity and declared neighbours unless expansion is requested (context isolation).
- Every refusal — gate fail, scope violation, lease conflict, missing justification, cross-team edit, blocked dependency — must cite at least one of: constraint ID, gate ID, policy, blocking IDs, or lease holder, plus the human-readable rule text. A conformance test must iterate every error code in the binary and assert the attribution fields are present.
- Citation hygiene: agents must be instructed to write compact ID citations (`// [E12S4]`, `// [AD-43]`); the linter reports narrative comments by file, line, and rule — lint-and-report only in v1, no automatic rewriting.
- Output must be deterministic: stable JSON envelope with versioned schema, documented exit codes, and Markdown formatting for skill consumption. Token estimation must use a documented estimator.
- Every command must be non-interactive capable: never prompt when stdin is not a TTY or non-interactive mode is set; fail closed with a structured error naming the flag that would have answered.
- Zero telemetry: no network calls; MCP is stdio-only.

## Technical Decisions

- **Two-crate split**: `qdev-cli` owns all I/O, formatting, and prompts; `qdev-core` is headless (no stdin/stdout). MCP tools and skills are thin wrappers over core.
- **Storage**: Markdown files with YAML frontmatter are the source of truth in Git; SQLite is a rebuildable local cache. Context projection reads hydrated entities, never re-parses ad hoc.
- **Constraints are rows, not free text**: `constraints(id, owner_id, kind, text, created_by)` with owner-scoped IDs; kinds are `rabbit_hole`, `no_go`, `appetite`. Epic-level constraints inherit downward — context projection must resolve and include inherited constraints.
- **Scratchpads are separate, append-only, committed files** (JSONL per story); projections include a bounded summary, never the full ledger.
- **Error envelope**: `{"error": {"code", "message", "details"}}` with exit codes 0 success / 1 logical / 2 usage / 3 policy refusal / 4 infrastructure / 5 conflict. Attributed rejections attach their IDs in `details`.
- **MCP server** (`qdev mcp serve`): tools are thin wrappers over `qdev-core` (entity get, ready list, context, scratch append, transition, gate run), advertised with JSON Schemas generated from the CLI schema work, returning the same payloads as the CLI. Every tool runs non-interactive so refusals surface as structured errors.
- **Skills are generated, never hand-written**: produced from the same command registry that drives CLI argument parsing, stamped with the binary version, and installed per editor. Generated skills invoke the CLI with `--json` and call `qdev context` — they never assemble context from files directly.
- **Synthesis template**: a four-heading structured prompt template lives in config with a shipped default; domain-specific headings are configuration, and the prompt rejects output missing any heading. No persona role-play.
- **Doctor** is one health command covering Git state, cache, modules, gates (with executable resolution and local skips), hook shims, skill version stamps, MCP registration, and stale leases; `--fix` repairs after confirmation, `--json` gives per-check status for CI.
- **Graph**: builds on the existing DOT output; adds sprint filtering, distinct blocked stories, lease holders, and critical-path highlighting; `--json` emits nodes/edges for other renderers.
- Cross-platform by construction (macOS/Linux/Windows): process handling via platform APIs, hooks as two-line shims, OS-appropriate locking.

## UX & Interaction Patterns

- The five skills (`/qdev`, `/qdev-plan`, `/qdev-create-story`, `/qdev-develop`, `/qdev-review`) run the full loop from a chat interface: pulse rendering, planning via the synthesis template, story creation with constraints, then the develop sequence (preflight → claim → context → work → bound gates → transition to review) and the review sequence (context → audit diff against acceptance criteria and constraints → done or back to in-progress with justification).
- Refusal experience: the develop skill instructs the agent to quote the cited ID when reporting a failure to the human — the human never sees an unexplained stop.
- The review payload includes current hygiene findings for the diff so the reviewer enforces them in the same pass.
- The hygiene directive text and per-language citation templates come from configuration, so teams can adapt them without code changes.
- Model tiering hints surface in each skill's frontmatter; no vendor model names in the binary.

## Cross-Story Dependencies

- **4.1 (context)** depends on Epic 1 (entity hydration, JSON envelope/schemas, cache latency) and Epic 2 (constraints, scratchpads, leases, state machine) and Epic 3 (gate runs for bound gates).
- **4.2 (attributed rejections)** depends on the constraints entity and IDs (Epic 2), the gate result contract (Epic 3), and the JSON error envelope (Epic 1).
- **4.5 (MCP)** depends on the CLI schema work from Epic 1 for generated JSON Schemas.
- **4.6 (skills)** depends on 4.1, 4.4, plus Epic 2 (claim/transition) and Epic 3 (preflight, gates) — it is the integration point and should land late in the epic.
- **4.9 (graph)** depends on the DOT output from Epic 1.
- **4.10 (handover)** is the epic's capstone and has hard prerequisites: Epics 1 and 2 done, Epic 3 at least complete through its gates/preflight story. It runs `qdev init` on this repository, re-enters the remaining stories as native entities, archives the planning docs, and completes at least one remaining story end-to-end via the five skills — which makes it depend on 4.6.
- Within the epic, a sensible order: 4.1 and 4.2 first (they define what agents see), then 4.3, 4.4, 4.5, 4.7, 4.8 in parallel, then 4.6, 4.9, and 4.10 last.
