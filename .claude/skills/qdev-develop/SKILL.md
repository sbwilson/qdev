---
name: qdev-develop
description: "Implement a story: run preflight, claim lease, fetch develop context, update scratchpad, run gates, and transition to review"
version: "0.1.0"
qdev_version: "0.1.0"
model_hint: "claude-3-7-sonnet"
model: "claude-3-7-sonnet"
---

# Qdev Develop Skill

You are implementing a story in `qdev`.

## Invariant Rules
1. **Always use JSON output**: Run all commands with `--json`.
2. **Never assemble or inspect context from raw files directly**: Always run `qdev context <story-id> --phase develop --json`.
3. **Never modify raw spec/status files directly**: Record all progress through `qdev scratch append` and `qdev transition`.
4. **Never bypass preflight, lease claims, or transition-bound gates**: Preflight and claim must precede edits, and gates must pass before requesting review.
5. **Always quote cited IDs on refusals**: When reporting a refusal or failure to the human, always quote cited IDs (`constraint_id`, `gate_id`, `policy`, `blocking_ids`, or `holder`).

## Workflow: Story Implementation
1. Run preflight to verify clean tree, working tree scope, and branch freshness:
   ```bash
   qdev preflight --story <story-id> --json
   ```
2. Claim story lease:
   ```bash
   qdev claim <story-id> --json
   ```
3. Fetch scoped development context:
   ```bash
   qdev context <story-id> --phase develop --json
   ```
4. Implement within scope and append ongoing progress, notes, tradeoffs, or decisions to scratchpad:
   ```bash
   qdev scratch append <story-id> "<message>" --kind note --json
   ```
5. Check comment hygiene:
   ```bash
   qdev hygiene check --diff --json
   ```
6. Run verification gates bound to review transition:
   ```bash
   qdev gate run --for-transition review --story <story-id> --json
   ```
7. When all gates pass and implementation is complete, transition to review:
   ```bash
   qdev transition story <story-id> review --json
   ```
8. Handle Refusals:
   On any refusal or error from preflight, claim, gates, or transition, quote cited IDs (`constraint_id`, `gate_id`, `policy`, `blocking_ids`, `holder`).

## Available CLI Commands

Always invoke commands with `--json` when reading or modifying state:

- `qdev status`: Show pulse and status
- `qdev config`: Inspect or manage configuration
  - `qdev config show`: Show effective merged configuration
- `qdev init`: Scaffold config, directories, cache, and gitignore
  - `--name`: Project name
  - `--developer`: Developer identifier
  - `--team`: Team name(s)
- `qdev create`: Create planning and execution entities
  - `qdev create story`: Create a new story
    - `--title`: Story title
    - `--appetite`: Story appetite
    - `--module`: Target module(s)
    - `--owner`: Story owner(s)
- `qdev schema`: Print JSON Schema for an entity kind, or for a command's output payload
- `qdev update`: Update planning and execution entities
  - `--status`: Update entity status
- `qdev transition`: Transition a story across lifecycle states
  - `--skip-gates`: Skip transition-bound gates
  - `--justification`: Justification for transition or skipping gates
- `qdev get`: Fetch a single entity's isolated projection
  - `--expand`: Additional sections to include
- `qdev list`: List and filter entities of one kind
  - `--epic`: Filter by owning epic id
  - `--status`: Filter by status
  - `--owner`: Filter by owner
  - `--module`: Filter by target module id
- `qdev relate`: Add a relation from a source entity to a target entity
  - `--if-version`: Expected source entity version
- `qdev unrelate`: Remove a relation from a source entity to a target entity
  - `--if-version`: Expected source entity version
- `qdev constraint`: Declare or remove negative and rabbit-hole constraints
  - `qdev constraint add`: Add a negative or rabbit-hole constraint to an entity
    - `--kind`: Constraint kind: no_go, rabbit_hole, or appetite
    - `--if-version`: Expected existing entity version
  - `qdev constraint remove`: Remove a constraint from an entity
    - `--justification`: Justification for removing constraint
    - `--if-version`: Expected existing entity version
- `qdev graph`: Render the story dependency graph
  - `--dot`: Emit Graphviz DOT
  - `--json`: Emit structured JSON envelope
  - `--epic <ID>`: Filter to one epic's stories
  - `--sprint <ID>`: Filter to one sprint's stories
  - `--highlight-critical-path`: Highlight critical path in output
- `qdev validate`: Surface cached and freshly computed integrity findings
  - `--changed`: Validate changed files only
  - `--fix-ids`: Offer guided renumber for duplicate planning ids
- `qdev sync`: Run or rebuild the incremental hydration sweep
  - `--rebuild`: Purge and rebuild cache from files
- `qdev doctor`: Report structured cache and workspace diagnostics
  - `--fix`: Automatically fix remediable issues
- `qdev claim`: Claim a story lease
- `qdev release`: Release a story lease
- `qdev scratch`: Manage story scratchpads
  - `qdev scratch append`: Append an entry to a story's scratchpad
    - `--kind`: Scratchpad entry kind
  - `qdev scratch read`: Read a story's scratchpad entries
    - `--summary`: Summary view
    - `--budget`: Token budget
- `qdev decision`: Log or query decisions
  - `qdev decision log`: Log a decision against a subject entity
    - `--type`: Decision type
    - `--title`: Decision title
- `qdev dw`: Manage deferred work debt and residual anomalies
  - `qdev dw add`: Add a new deferred work record
    - `--title`: Deferred work title
    - `--risk`: Safety risk level
  - `qdev dw list`: List deferred work records
    - `--status`: Filter by status
    - `--risk`: Filter by risk
  - `qdev dw close`: Close a deferred work record
    - `--status`: Target close status
    - `--resolution`: Resolution description or story ID
- `qdev sprint`: Manage sprints and story assignments
  - `qdev sprint open`: Open a new sprint
    - `--title`: Sprint title
  - `qdev sprint assign`: Assign stories to a sprint
    - `--sprint`: Sprint identifier override
  - `qdev sprint close`: Close an active sprint with optional carry-over
    - `--status`: Terminal status
    - `--carry-over`: Target sprint for carry-over stories
- `qdev review`: Produce epic and sprint release reviews
  - `qdev review epic`: Audit story evidence and open debt for an epic
  - `qdev review sprint`: Run sprint_close-bound gates and emit release reports
- `qdev chore`: Start and commit path-allowlisted chores
  - `qdev chore start`: Start a chore
    - `--description`: Chore description
  - `qdev chore commit`: Commit a chore
    - `--message`: Commit message
  - `qdev chore list`: List this workspace's chore records
  - `qdev chore close`: Close a chore
  - `qdev chore abort`: Abort a chore
- `qdev next`: Deterministically select the next eligible story
- `qdev context`: Project a token-budgeted, phase-specific agent context for a story
  - `--phase`: Context phase (specify, develop, review)
  - `--budget`: Token budget
  - `--stats`: Include token statistics
  - `--format`: Output format (md)
- `qdev gate`: Run verification gates
  - `qdev gate run`: Execute verification gates
    - `--all`: Run all gates
    - `--for-transition`: Run gates bound to transition
    - `--story`: Story identifier to evaluate
  - `qdev gate list`: List configured gates and their status
  - `qdev gate baseline`: Inspect or record gate baselines for ratchet verification
- `qdev soup`: Audit third-party dependencies and capture SBOM evidence
  - `qdev soup audit`: Run configured dependency audit and deny checks
  - `qdev soup sbom`: Generate and attach an SBOM to a release
- `qdev preflight`: Enforce working tree scope, branch freshness, and validation health
  - `--story`: Story ID to evaluate
- `qdev install`: Install developer tools and integrations (e.g. Git hooks)
  - `qdev install hooks`: Install git hook shims
  - `qdev install skills`: Install editor and agent skill bundles
    - `--claude`: Install Claude Code skills
    - `--cursor`: Install Cursor rule
    - `--agents`: Install Agent skills
  - `qdev install mcp`: Install MCP server configuration
    - `--claude`: Install Claude Code MCP configuration
    - `--cursor`: Install Cursor MCP configuration
- `qdev hook`: Execute a Git hook
- `qdev hygiene`: Inspect source code comment hygiene
  - `qdev hygiene check`: Check comment hygiene rules
    - `--diff`: Lint only changed files in diff
- `qdev impact`: Analyze change impact across modules, active stories, relations, and gates
- `qdev mcp`: Model Context Protocol (MCP) server commands
  - `qdev mcp serve`: Start stdio-based MCP JSON-RPC 2.0 server

