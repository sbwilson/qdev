# qdev CLI Reference & Configuration Guide

> Command grammar, JSON envelope and exit codes, the default pulse command, configuration files, and environment integration.

## Table of Contents

- [Command Grammar](#cli-grammar)
- [Command Catalog](#catalog)
- [JSON Envelope, Errors & Exit Codes](#json)
- [Non-Interactive Mode](#non-interactive)
- [The Default Command ("What To Do Next")](#default-cli)
- [Configuration](#configuration)
- [Installation & Environment Integration](#installation-integration)

---

<a id="cli-grammar"></a>

## 1. Command Grammar

```
qdev <verb> <noun> <id> [flags]            generic entity operations
qdev <noun> <verb> [args] [flags]          domain operations scoped to a noun
qdev <command> [flags]                     workspace-level commands
```

- **Generic verbs** `get`, `list`, `create`, `update`, `delete` prefix the noun: `qdev get story E12S4`.
- **Domain operations** are subcommands of their noun: `qdev gate run c-abi-round-trip`, `qdev scratch append E12S4`, `qdev sprint close 5`.
- **Workspace commands** stand alone: `qdev init`, `qdev doctor`, `qdev validate`, `qdev next`, `qdev context`.
- Free text is passed after `--` or via `--file`/stdin.

Universal reference resolution: any command that takes an ID accepts any entity ID (`E12S4`, `AD-43`, `FR-102`, `DW-7f3a`, `E12S4/NG-1`).

---

<a id="catalog"></a>

## 2. Command Catalog

### Workspace

| Command | Purpose |
| --- | --- |
| `qdev` | Pulse: environment, active sprints, what to do next |
| `qdev init [--non-interactive --name --developer --team ...]` | Scaffold config, directories, cache, hooks |
| `qdev doctor [--fix]` | Environment, cache, gates, skills, MCP, hooks |
| `qdev validate [--fix-ids]` | Dangling relations, cycles, ID collisions, schema, orphan DW, missing rationale |
| `qdev sync [--rebuild]` | Force hydration or rebuild the cache |
| `qdev schema <payload>` | Print JSON Schema for a payload (`story`, `context`, `gate_run`, `error`, ...) |
| `qdev config show` | Effective merged configuration |
| `qdev next [--sprint N] [--owner me]` | Deterministically select the next unblocked story |
| `qdev context <id> --phase P [--budget N] [--stats]` | Token-budgeted projection for an agent phase |
| `qdev graph [--dot] [--epic E12]` | Dependency DAG |
| `qdev impact <id>` | Affected stories, modules, requirements, and gates to re-run |

### Entities

| Command | Purpose |
| --- | --- |
| `qdev get <kind> <id> [--expand relations,constraints,scratch]` | One entity; isolated by default |
| `qdev list <kind> [--epic --status --owner --sprint --module]` | Filtered list |
| `qdev create story E12 --title ... --appetite small --module bridge` | Allocates the next ID |
| `qdev create epic|adr|requirement|hazard|prd ...` | Same pattern |
| `qdev update <kind> <id> --field value [--if-version N]` | Field-level mutation |
| `qdev update <kind> <id> --section "Acceptance Criteria" --file ac.md` | Body section replacement |
| `qdev constraint add E12S4 --kind no_go -- "Do not touch frame buffers"` | Allocates `E12S4/NG-n` |
| `qdev relate E12S4 depends_on E12S3` / `qdev unrelate ...` | Manage relations |

### Workflow

| Command | Purpose |
| --- | --- |
| `qdev claim story E12S4` / `qdev release [E12S4]` | Take or release a lease; prints `QDEV_SESSION` |
| `qdev transition story E12S4 <state> [--justification ...]` | State change; backward requires justification |
| `qdev scratch append E12S4 [--kind note|decision|tradeoff] -- "..."` | Append ledger entry |
| `qdev scratch read E12S4 [--summary]` | Read ledger |
| `qdev decision log --subject E12S4 --type human_ruling --topic ... --ruling ...` | Record a decision |
| `qdev dw add --story E12S4 --module bridge --risk negligible --title ...` | Register deferred work |
| `qdev dw list [--module --risk --status]` / `qdev dw close DW-7f3a --resolution ...` | Manage deferred work |
| `qdev chore start "fix readme typo" --paths README.md docs/**` / `qdev chore commit` | Fast-track with path allowlist |
| `qdev sprint open 6 --title ... --release 0.1.0` | Create sprint |
| `qdev sprint assign 6 E12S4 ...` | Assign stories |
| `qdev sprint close 5 --status completed --carry-over 6` | Baseline and carry open work forward |
| `qdev review epic E12` / `qdev review sprint 5` | Reports: stories, DW, gates, traceability, anomalies |

### Gates, hygiene, Git

| Command | Purpose |
| --- | --- |
| `qdev gate run <id> [--story E12S4]` / `qdev gate run --all|--for-transition review` | Execute with evidence |
| `qdev gate list` / `qdev gate baseline <id> --set` | Inspect; set ratchet baseline |
| `qdev hygiene check [--diff] [--paths ...]` | Comment lint, report only |
| `qdev preflight [--story E12S4]` | Clean tree within scope, integration branch freshness |
| `qdev soup audit [--release 0.1.0]` / `qdev soup sbom` | Wrap configured audit and SBOM commands |
| `qdev hook <name>` | Entry point invoked by Git hook shims |

### Integration

| Command | Purpose |
| --- | --- |
| `qdev install skills --claude|--cursor|--agents` | Generate and write skills |
| `qdev install mcp --claude|--cursor` | Register the MCP server |
| `qdev install hooks` | Write hook shims |
| `qdev mcp serve` | Stdio MCP server exposing core operations |

---

<a id="json"></a>

## 3. JSON Envelope, Errors & Exit Codes

Every command accepts `--json`. Success payloads are the data object plus `schema_version`:

```json
{
  "schema_version": "1",
  "id": "E12S4",
  "epic_id": "E12",
  "title": "CoreResponse Buffer Layout",
  "status": "ready",
  "blocked": false,
  "appetite": "small",
  "owners": ["simon", "team:core-platform"],
  "target_modules": ["bridge", "foundation"],
  "constraints": [
    {"id": "E12S4/NG-1", "kind": "no_go", "text": "Do not implement Swift decoding"},
    {"id": "E12/RH-2", "kind": "rabbit_hole", "text": "len == 0 does not mean empty result", "inherited_from": "E12"}
  ],
  "relations": {"depends_on": ["E12S3"], "traces_to": ["FR-102"], "governed_by": ["AD-43"]},
  "gates": ["c-abi-round-trip"],
  "version": 3
}
```

Errors go to stdout in JSON mode, stderr in text mode:

```json
{
  "schema_version": "1",
  "error": {
    "code": "constraint_violation",
    "message": "Change touches crates/video/ which is outside target_modules",
    "details": {"constraint_id": "E12S4/NG-2", "text": "Do not touch frame buffers", "paths": ["crates/video/frame.rs"]}
  }
}
```

| Exit | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Logical failure: gate failed, validation findings, hygiene violations |
| 2 | Usage error |
| 3 | Refused by policy: preflight, lease, governance, missing justification |
| 4 | Infrastructure failure: timeout, missing tool, cache corruption |
| 5 | Conflict: version mismatch, lease held by another holder, lock timeout |

Schema changes follow semver on `schema_version`; additive fields do not bump the major.

---

<a id="non-interactive"></a>

## 4. Non-Interactive Mode

Per **AD-12**, with `--non-interactive`, `QDEV_NONINTERACTIVE=1`, or a non-TTY stdin, qdev never prompts. A command that would have prompted exits 3 with an error naming the flag:

```json
{"error": {"code": "needs_confirmation", "message": "Cross-team override requires --override --justification", "details": {"entity": "E12", "owners": ["team:core-platform"], "user": "sally"}}}
```

---

<a id="default-cli"></a>

## 5. The Default Command ("What To Do Next")

```
$ qdev
qdev 1.0 — Development Engine & Gatekeeper
Environment
  Working tree   clean (feature/E12S4-buffer @ 8f1b2c4)
  Integration    develop is up to date with origin/develop
  Cache          healthy (synced 18 ms ago, 184 entities, 0 findings)
  Lease          E12S4 held by simon since 09:41 (this worktree)

Sprint 5 — The Rust Core Port  [release 0.1.0]
  Stories        42 done / 12 in progress / 3 blocked / 108 backlog
  Deferred work  8 open (0 unacceptable)
  Gates          14/14 passing, ratchets at baseline

Next
  E12S4 "CoreResponse Buffer Layout" is ready and unblocked.
  Run: /qdev-develop E12S4   (or: qdev context E12S4 --phase develop)
```

`qdev next --json` returns the same selection for orchestrators. Ordering: stories in active sprints → not blocked → not leased → owner matches current user or team → epic phase → story seq. Ties are broken by ID, so the result is deterministic.

---

<a id="configuration"></a>

## 6. Configuration

### `qdev.toml` (committed)

```toml
[project]
name = "Qubric"
default_sprint = 5                       # fallback when --sprint is omitted

[teams]
core-platform = ["simon", "amelia"]
ui-shell = ["sally"]

[git]
remote = "origin"
integration_branch = "develop"
branching_mode = "story-branch"          # story-branch | trunk
branch_template = "feature/{story_id}-{slug}"
require_clean_tree_in_scope = true
max_integration_staleness_commits = 20   # story branch merge-base freshness

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"

[[modules]]
id = "foundation"
paths = ["crates/foundation/**"]
layer = 0

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]
layer = 2
may_depend_on = ["foundation", "engine"]

[[modules]]
id = "RustBridge"
paths = ["Packages/RustBridge/**"]
layer = 3
may_depend_on = ["bridge"]

[hygiene]
enabled = true
max_inline_comment_lines = 6
forbid_patterns = ["(?i)^\\s*(//|#)\\s*(STORY|Review round|Wave)\\b"]
citation_pattern = "\\[(E\\d+S\\d+|AD-\\d+|DW-[0-9a-f]+|DEC-[0-9a-f]+|HAZ-\\d+|E\\d+(S\\d+)?/(NG|RH)-\\d+)\\]"
languages = ["rust", "swift", "python"]  # comment syntaxes to parse

[regulatory]
iec62304_class = "ClassB"
require_rationale_for = ["acceptable_with_mitigation", "unacceptable"]

[soup]
audit_command = "cargo audit --json"
deny_command = "cargo deny check"
sbom_command = "cargo cyclonedx --format json"

[models]                                  # advisory; consumed by skills
specify = "reasoning"
develop = "fast-coding"
review = "strongest"

[commit_messages]
enabled = false                           # opt-in prepare-commit-msg
format = "conventional"

[environment]
CARGO_TARGET_DIR = "target/qdev"

[[gates]]
id = "fmt"
command = "cargo fmt --all --check"
timeout_ms = 60000

[[gates]]
id = "lint"
command = "cargo clippy --workspace --all-targets -- -D warnings"
depends_on = ["fmt"]
output_adapter = "cargo"
timeout_ms = 600000

[[gates]]
id = "c-abi-round-trip"
command = ".qdev/gates/c-abi-round-trip.sh"
depends_on = ["lint"]
on_transition = ["review"]
verifies = ["FR-102"]
timeout_ms = 300000

[[gates]]
id = "warning-count"
kind = "ratchet"
command = ".qdev/gates/warning-count.sh"
metric = "warnings"
direction = "must_not_increase"
```

### `.qdev.local.toml` (gitignored)

```toml
[identity]
developer_id = "simon"
teams = ["core-platform"]

[preferences]
color = true
default_format = "text"
editor = "cursor"

[gates]
skip = ["warning-count"]                  # local skips are recorded in evidence as skipped
```

Local values override project values key by key. Absence of the local file is not an error; identity falls back to `git config user.email`.

---

<a id="installation-integration"></a>

## 7. Installation & Environment Integration

```bash
cargo install --path tools/qdev --locked
```

The binary is self-contained with bundled SQLite. No external runtime is required.

### `qdev init`

```
$ qdev init --non-interactive --name Qubric --developer simon --team core-platform
✔ qdev.toml
✔ .qdev.local.toml (gitignored)
✔ .qdev/cache/ (gitignored), .qdev/gates/
✔ docs/specs/{prd,requirements,epics,stories,adrs,hazards}
✔ docs/state/{sprints,releases,dw,decisions,scratch,evidence,baselines,soup}
✔ cache schema v1
```

Re-running `init` in an initialised workspace checks the cache schema version and migrates with confirmation (or `--yes`).

### `qdev install`

| Target | Command | Writes |
| --- | --- | --- |
| Claude Code | `qdev install skills --claude` | `.claude/skills/qdev-*/SKILL.md` |
| Cursor | `qdev install skills --cursor` | `.cursor/rules/qdev.mdc` |
| Agent directories | `qdev install skills --agents` | `.agents/skills/qdev-*` |
| MCP | `qdev install mcp --claude|--cursor` | Stdio server entry pointing at `qdev mcp serve` |
| Git hooks | `qdev install hooks` | Shims for `pre-commit`, `pre-push`, `prepare-commit-msg` that call `qdev hook <name>` |

Skills are generated from the same command catalog as the CLI, so they cannot drift from the binary.

### `qdev mcp serve`

Stdio MCP server exposing: `get_entity`, `list_entities`, `context`, `next`, `claim`, `transition`, `scratch_append`, `scratch_read`, `dw_add`, `decision_log`, `gate_run`, `validate`. Each tool returns the same JSON payload as the CLI.

### `qdev doctor`

```
$ qdev doctor
[✓] Git: develop tracks origin/develop; clean tree
[✓] Cache: .qdev/cache/cache.sqlite schema v1, 184 entities, 0 validation findings
[✓] Modules: 7 declared, all path globs match at least one file
[✓] Gates: 14 configured, all executables found, 1 skipped locally
[✓] Hooks: 3 shims installed and current
[✓] Skills: 4 installed for Claude Code, up to date with binary 1.0.0
[✓] MCP: registered in .claude settings
[!] Leases: E12S7 held by amelia in /work/qubric-b, 3 days old
```
