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
| `qdev init [--non-interactive --name --developer --team ...]` | Scaffold config, directories, cache, hooks (`--yes` accepted, no effect) |
| `qdev doctor` | Environment, cache, validation findings, gates, skills, MCP, hooks (a report — findings never change its exit code) |
| `qdev validate [--changed] [--fix-ids] [--yes]` | Dangling relations, cycles, ID collisions, schema, orphan DW, missing rationale |
| `qdev sync [--rebuild]` | Force hydration or rebuild the cache |
| `qdev schema <entity-kind>` | Print JSON Schema for an entity's frontmatter shape (`story`, `epic`, `dw`, ...) |
| `qdev schema payload <name>` | Print JSON Schema for a command's output payload (`story`, `error`, `validate`, `fix_ids`, `list`, `sync`, `doctor`; `context`/`next`/`gate_run` land with their commands) |
| `qdev config show` | Effective merged configuration |
| `qdev next [--sprint N] [--owner me]` | Deterministically select the next unblocked story |
| `qdev context <id> --phase P [--budget N] [--stats]` | Token-budgeted projection for an agent phase |
| `qdev graph --dot [--epic E12]` | Dependency DAG (`--dot` is required; no other output format yet) |
| `qdev impact <id>` | Affected stories, modules, requirements, and gates to re-run |

### Entities

| Command | Purpose |
| --- | --- |
| `qdev get <kind> <id> [--expand relations,constraints,scratch]` | One entity; isolated by default |
| `qdev list <kind> [--epic --status --owner --sprint --module]` | Filtered list |
| `qdev create story E12 --title ... --appetite small --module bridge [--author-type agent --author-id bot]` | Allocates the next ID |
| `qdev create epic|adr|requirement|hazard|prd ...` | Same pattern |
| `qdev update <kind> <id> --field value [--if-version N]` | Field-level mutation |
| `qdev update <kind> <id> --section "Acceptance Criteria" --file ac.md` | Body section replacement |
| `qdev constraint add E12S4 --kind no_go -- "Do not touch frame buffers"` | Allocates `E12S4/NG-n` |
| `qdev relate E12S4 depends_on E12S3 [--if-version N]` / `qdev unrelate ... [--if-version N]` | Manage relations |

`qdev create story` and `qdev update` share one write path: it takes the advisory lock on
`<cache_dir>/write.lock` with a 5-second timeout (`lock_timeout`, exit 5), writes atomically via
temp-file-then-rename, and upserts the cache row and marks it dirty, so the entity is queryable
without waiting for the next sweep. `create story` additionally validates the frontmatter it
generates against the story schema *before* writing — an `--appetite` or `--safety-class` outside
its enum (`tiny|small|medium|deep`, `ClassA|ClassB|ClassC`) is a logical failure (exit 1) naming
the field, and no file is created — refuses a create whose target path is already occupied
(`file_exists`, exit 5, checked while holding the lock), and records the resolved author in both
`created_by` and `updated_by`.

Run outside an initialized workspace, `create story` writes only the story file: it creates no
cache and takes no lock, so no `.qdev/` directory is left behind.

#### How qdev finds your entity

**A file is named for the entity it holds.** One rule answers "which file is entity `X`?" for
reads and writes alike: `X` lives under its kind's directory (`docs/specs/stories/`,
`docs/specs/adrs/`, `docs/state/dw/`, … per `[storage]`) in a file named `X.md`, `X-<slug>.md` or
`X_<slug>.md` — the id and the `.md` extension are both matched case-insensitively, so `e1s1.md`
and `E1S1.MD` hold `E1S1` too, on every filesystem (`X.md` stays the only spelling qdev itself
writes). The rule decides this, never the filesystem: resolution lists the directory and judges
each real name, so the answer is the same on a case-sensitive host and a case-insensitive one.
Reads answer from the frontmatter `id`; writes answer from the file name, which
under this convention is the same file — so `qdev get`, `qdev update` and `qdev relate` never
disagree about which file an entity is, and `create story` still works with no cache present.

Consequences worth knowing:

- Two files matching one id is a usage error (exit 2) naming both, never a guess — and both
  named files exist, because a match is a directory entry rather than a spelling the OS accepted.
- A file whose name does not carry its id, or that lives outside every entity directory, is
  still *read* (hydration walks the spec and state trees recursively) but cannot be written.
  `qdev validate` reports it as `entity_file_off_convention` at `warning` severity, naming the
  file and the name it should have; a warning alone keeps `validate` at exit 0. A write attempted
  on such an entity is refused with that same sentence — the file holding the id and the rename
  that would fix it — rather than a bare "Entity file not found".
- Renaming is never done behind your back. The one exception is `qdev validate --fix-ids`, which
  renames as it renumbers — a renumbered entity accepts `qdev update` immediately, with no manual
  `mv` — and reports `old_path`/`new_path` alongside `old_id`/`new_id` so the git-visible move is
  never silent. An occupied rename target is refused, not clobbered.
- The entity's *kind* comes from its frontmatter `kind:` first, then its directory, then the
  identifier grammar — the same rule hydration uses, so a write validates against the schema the
  next boot sweep will validate against.

#### Which ids are already taken

`architecture.md` §6 is authoritative for this rule; the summary here is for command reference.

One function answers that, and everything that allocates an id uses it — `qdev create story` and
`qdev validate --fix-ids` alike, so the two cannot disagree. An id is in use if it is **declared**
by the frontmatter of any file under the spec or state tree (recursively, `.md` matched
case-insensitively — exactly the set hydration reads), **carried** by any of those file names
under the convention above (a file whose frontmatter will not parse still occupies its name), or
held in the **cache** for a hydrated entity (so an id survives its file becoming unreadable).

- `qdev create story` takes one past the highest story number its epic has used, so a gap left by a
  renumber or a deleted draft rather than always continuing past the highest id.
- `create story` never returns an id another file declares or carries. In a bare directory it
  answers from the filesystem alone; there is no cache to consult.
- Two files *carrying* one id without either declaring it is an off-convention name, not a
  duplicate: `duplicate_planning_id` reports declarations only.
- `--fix-ids` refuses a duplicate whose file is not in its kind's directory, records it as skipped
  and names the expected path. Renumbering it in place would leave the entity unwritable, and
  moving your file across directories is not a decision the tool takes silently.

Attribution is resolved from one path for every command that records an author: `--author-type`
/ `--author-id` first, then `QDEV_AUTHOR_TYPE` / `QDEV_AUTHOR_ID`, then `[identity]
developer_id`, then `git config user.email`. An author type other than `human` or `agent` is a
usage error (exit 2) whatever its source, refused before anything is written.

#### Relations, and what `relate`/`unrelate` refuse

The relation name is an enum, exactly like `--author-type`: `depends_on`, `extends`,
`supersedes`, `traces_to`, `verifies`, `mitigates`, `closes_dw`, `governed_by` (architecture.md
§8 is authoritative). Anything else — a typo, a hyphen for an underscore — is a usage error
(exit 2) from `relate` and `unrelate` alike, naming the valid relations, reported before the ids
are even resolved and before anything is written. `unrelate` in particular never reports a typo
as an idempotent no-op: exit 0 `changed: false` from `unrelate` means the edge you named is not
there, never that the relation name was not understood.

Two refusals that read alike are deliberately distinct, and both are `relate`'s:

- **The name is not a relation** → usage error, exit 2, listing the valid names. This one is
  checked by `relate` *and* `unrelate`.
- **The name is a relation, but not for these two kinds** → `invalid_relation_kind`, exit 1,
  naming both kinds (e.g. `traces_to` from a story to an epic). `verifies` is a known relation
  with no allowed pair until gates are modeled in Epic 3, so it lands here, not above.

`relate` alone also refuses a dangling target (`dangling_relation`, exit 1) and a `depends_on`
edge that would close a cycle (`dependency_cycle`, exit 1); neither writes anything. **`unrelate`
validates the relation name and nothing else** — it is removing an edge, so a target that does
not exist or a pair that would be disallowed are not reasons to refuse.

Removing an edge that is genuinely absent, under a valid relation name, stays an idempotent
success: exit 0, `changed: false`, no write, no version bump.

#### `--if-version` on the three mutating commands

`qdev update`, `qdev relate` and `qdev unrelate` all accept `--if-version N`, with one meaning:
the source entity's version must currently be exactly `N`, or the command exits 5
`version_mismatch` (details carry `expected_version` and `current_version`) and writes nothing.

The comparison happens before any outcome is reported, **including an outcome that would have
changed nothing** — relating an edge that is already there, or unrelating one that is not. So
exit 0 from a command given `--if-version` always means the expectation held, which is what makes
the flag usable as a compare-and-swap fence: an agent can read the version from `qdev get`, act
on it, and be refused if anything moved in between.

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
  "kind": "story",
  "epic_id": "E12",
  "title": "CoreResponse Buffer Layout",
  "status": "ready",
  "blocked": false,
  "appetite": "small",
  "safety_class": "ClassB",
  "owners": ["simon", "team:core-platform"],
  "target_modules": ["bridge", "foundation"],
  "constraints": [
    {"id": "E12S4/NG-1", "kind": "no_go", "text": "Do not implement Swift decoding"},
    {"id": "E12/RH-2", "kind": "rabbit_hole", "text": "len == 0 does not mean empty result", "inherited_from": "E12"}
  ],
  "relations": {"depends_on": ["E12S3"], "traces_to": ["FR-102"], "governed_by": ["AD-43"]},
  "version": 3,
  "stale": false
}
```

`qdev get` does not yet include a `gates` field — story-to-gate association isn't modeled until Epic 3.

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

`qdev schema payload <name>` prints the JSON Schema for one of these output-payload shapes
(currently `story`, `error`, `validate` — each schema's own `description` restates this semver
policy):

```
$ qdev schema payload story
$ qdev schema payload validate --json
```

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
specs_dir = "docs/specs"                 # project-only: committed content
state_dir = "docs/state"                 # project-only: committed content
cache_dir = ".qdev/cache"                # may be relocated in .qdev.local.toml

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

[storage]
cache_dir = "local/cache"                 # cache_dir only; see below
```

Local values override project values key by key. Absence of the local file is not an error; identity falls back to `git config user.email`.

`[storage]` is the one section whose keys are not all locally overridable. `specs_dir` and
`state_dir` select where committed content lives, so they are a project decision and may be set
only in `qdev.toml`; either key in `.qdev.local.toml` is a schema violation (exit 2, naming the
key and the file). `cache_dir` names a machine-local, rebuildable artifact, so it may be
relocated locally. Because `.gitignore` is committed, `qdev init` then writes ignore entries for
the project cache directory **and** for the locally configured one — the relocated cache is never
untracked merely by luck, and the committed file stays meaningful to everyone else.

Two consequences of `.gitignore` being committed, both deliberate:

- **Relocate your cache, then re-run `qdev init`.** No ordinary command edits `.gitignore`, so a
  `cache_dir` set *after* initialisation is live before it is ignored. `qdev init` is idempotent
  and adds the entry.
- **Entries accumulate and are never pruned.** A directory that stops being anyone's cache keeps
  its line, and developers who choose different local caches each add one. Stale entries are
  harmless — they ignore nothing — but nothing removes them.

Every `[storage]` value names a directory inside the workspace: an empty value, an absolute path,
or one containing `..` is a schema violation (exit 2) from every command. Values are normalized
once by the loader — surrounding whitespace and a trailing `/` are stripped — so `init` and every
other command resolve the same directory.

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
✔ cache schema v3
```

Re-running `init` in an initialised workspace checks the cache schema version. **A cache stamped
older is migrated, unconditionally and without asking** — dropped, recreated at the current
schema, and repopulated from the Markdown files — which is exactly what every other command's
boot already does with one. The cache is a machine-local, rebuildable index (AD-3/FR-101), so
there is nothing to confirm: no data lives only there. A cache stamped *newer* than the binary
supports is refused by `init` as by everything else — `schema_version_mismatch`, exit 5,
recoverable with `qdev sync --rebuild` or by deleting the cache file. A cache stamped current but
structurally incomplete — a missing table, a missing column — is rebuilt too: `init` classifies
the file with the same inspector the boot path uses, so it cannot call a cache healthy that the
next command would drop and rebuild.

A migration reports what it repopulated, because a rebuild that found nothing is otherwise an
exit 0 with no signal:

```
✔ cache schema migrated to v3 (184 files rehydrated)
```

Under `--json` the same numbers are `cache_migrated`, `cache_schema_version` and
`cache_files_rehydrated`. A `0` there on a workspace full of stories means the rebuild read
nothing — most likely a `[storage]` layout pointing somewhere else.

`init`'s `-y`/`--yes` flag is **retained for compatibility and has no effect.** It existed solely
to confirm that migration; it stays accepted so scripts and CI jobs that pass it keep working.
`init`'s other refusals are unchanged: in non-interactive mode a missing `--name`, `--developer`
or `--team` is still exit 3 (`needs_confirmation`) naming the flag, and `--yes` never satisfied
those.

`init` resolves `[storage]` through the same configuration loader as every other command, so the
paths above are the *configured* layout rather than fixed defaults: the directories it creates,
the cache it creates and stamps, the `.gitignore` entries it writes, and every path in
`created_files` / `created_directories` come from that one answer, and the payload also reports it
as `storage` and `qdev_dir`. An unparseable or invalid `qdev.toml` or `.qdev.local.toml` is
therefore the same exit-2 usage error from `init` as from everything else, and nothing is
scaffolded.

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
[✓] Cache: .qdev/cache/cache.sqlite schema v3, 184 entities, 0 validation findings
[✓] Modules: 7 declared, all path globs match at least one file
[✓] Gates: 14 configured, all executables found, 1 skipped locally
[✓] Hooks: 3 shims installed and current
[✓] Skills: 4 installed for Claude Code, up to date with binary 1.0.0
[✓] MCP: registered in .claude settings
[!] Leases: E12S7 held by amelia in /work/qubric-b, 3 days old
```

`qdev doctor` is a report, never a gate: findings never change its exit code, so it exits 0 on a
workspace full of them. (It can still fail for its own reasons — an unreadable workspace, a
config that will not parse.) `qdev validate` is the command that exits 1 on an `error`-severity
finding.

Diagnostics are contributed by independent sections, reported in a fixed order — currently
`cache`, then `validation`. `qdev doctor --json` emits each as a flat object under `sections`
(see `qdev schema payload doctor`); later epics append their own.

#### The `validation` section

| Field | Meaning |
| --- | --- |
| `status` | `ok`, or `unavailable` when the validation pass could not complete (e.g. a cache too damaged to read) — in which case the count fields are `null` |
| `unavailable_reason` | The error code that stopped the pass; `null` when `status` is `ok` |
| `finding_count` | Total findings for the workspace |
| `findings_by_code` | Per-code counts, code-sorted; `{}` on a clean workspace |

This section runs the same function `qdev validate` runs, so neither command can see a check the
other cannot. Two differences in what they *report*: `qdev validate --changed` narrows its output
to findings under the changed files, and `validate`'s exit code keys on `error`-severity findings
alone, while `finding_count` here counts every severity. A bare `qdev validate` on the same
workspace reports the same total.

The set covers both the findings hydration recorded in the cache (`schema_violation`,
`read_error`, `dangling_relation`, `invalid_relation_kind`, `dependency_cycle`,
`merge_conflict`) and the five computed fresh at request time (`duplicate_planning_id`,
`orphan_deferred_work`, `dw_missing_rationale`, `target_module_not_registered`,
`entity_file_off_convention`). Computed findings are never written back to the cache: a
persisted one would outlive the defect it describes. `duplicate_planning_id` covers every
directory hydration reads — `specs_dir` and `state_dir` — and the four checks that read cached
rows treat a row flagged `stale` as *absent*, so no finding is derived from content a file no
longer has, and none from an entity a check merely refers to either: a deferred-work row whose
origin story is merge-conflicted reports `orphan_deferred_work`, exactly as a rebuild of the same
tree does. One helper on the store answers that existence question for every derivation site (see
architecture §11, "The convergence invariant"). Reads are unaffected — `qdev get` and `qdev list`
still return a stale entity with its `stale` flag set.

Because those checks re-read the workspace's spec files, `doctor` now costs a directory walk
that it did not before — noticeable only on large workspaces, where the cheap sections still
report first.

#### The `cache` section's `finding_count` is cache-native only

The `cache` section reports on the cache database itself, and its `finding_count` is a count of
rows in the `findings` table — the findings *hydration* recorded, nothing else. The computed
checks above are deliberately never written there, so they cannot appear in it. Read that field
as "how much did hydration flag", not as "is my workspace healthy"; a `finding_count` of `0` in
the `cache` section is not a clean bill of health. The `validation` section answers the health
question.
