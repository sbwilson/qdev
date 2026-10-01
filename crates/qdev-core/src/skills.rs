//! Assistant skill and editor rule generation and installation per Story 4.4.
//!
//! Centralizes CLI commands and options into a `CommandCatalog`, generates assistant
//! skills for Claude Code and Agent frameworks, generates Cursor rules, stamps version
//! metadata, installs them into workspace target locations, and inspects installed skills.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{ModelsConfig, SynthesisConfig};
use crate::errors::QdevError;
use crate::schema::extract_frontmatter;
use crate::write::write_file_atomic;

/// The five core skills managed by qdev.
pub const CORE_SKILL_NAMES: &[&str] = &[
    "qdev",
    "qdev-plan",
    "qdev-create-story",
    "qdev-develop",
    "qdev-review",
];

/// The canonical set of 11 managed skill and rule file paths (relative to workspace root).
pub const MANAGED_SKILL_PATHS: &[&str] = &[
    ".claude/skills/qdev/SKILL.md",
    ".claude/skills/qdev-plan/SKILL.md",
    ".claude/skills/qdev-create-story/SKILL.md",
    ".claude/skills/qdev-develop/SKILL.md",
    ".claude/skills/qdev-review/SKILL.md",
    ".cursor/rules/qdev.mdc",
    ".agents/skills/qdev/SKILL.md",
    ".agents/skills/qdev-plan/SKILL.md",
    ".agents/skills/qdev-create-story/SKILL.md",
    ".agents/skills/qdev-develop/SKILL.md",
    ".agents/skills/qdev-review/SKILL.md",
];

/// Definition of an argument or option in the command catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionDefinition {
    pub name: &'static str,
    pub summary: &'static str,
}

/// Definition of a subcommand in the command catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubcommandDefinition {
    pub name: &'static str,
    pub summary: &'static str,
    pub options: &'static [OptionDefinition],
}

/// Definition of a top-level CLI command in the command catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandDefinition {
    pub name: &'static str,
    pub summary: &'static str,
    pub subcommands: &'static [SubcommandDefinition],
    pub options: &'static [OptionDefinition],
}

/// Centralized catalog of CLI commands, options, and summaries.
pub struct CommandCatalog;

impl CommandCatalog {
    /// Returns all core CLI commands mirrored from the clap CLI definitions.
    pub fn all() -> &'static [CommandDefinition] {
        ALL_COMMANDS
    }

    /// Finds a command by name.
    pub fn find(name: &str) -> Option<&'static CommandDefinition> {
        Self::all().iter().find(|cmd| cmd.name == name)
    }
}

const ALL_COMMANDS: &[CommandDefinition] = &[
    CommandDefinition {
        name: "status",
        summary: "Show pulse and status",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "config",
        summary: "Inspect or manage configuration",
        subcommands: &[SubcommandDefinition {
            name: "show",
            summary: "Show effective merged configuration",
            options: &[],
        }],
        options: &[],
    },
    CommandDefinition {
        name: "init",
        summary: "Scaffold config, directories, cache, and gitignore",
        subcommands: &[],
        options: &[
            OptionDefinition {
                name: "--name",
                summary: "Project name",
            },
            OptionDefinition {
                name: "--developer",
                summary: "Developer identifier",
            },
            OptionDefinition {
                name: "--team",
                summary: "Team name(s)",
            },
        ],
    },
    CommandDefinition {
        name: "create",
        summary: "Create planning and execution entities",
        subcommands: &[SubcommandDefinition {
            name: "story",
            summary: "Create a new story",
            options: &[
                OptionDefinition {
                    name: "--title",
                    summary: "Story title",
                },
                OptionDefinition {
                    name: "--appetite",
                    summary: "Story appetite",
                },
                OptionDefinition {
                    name: "--module",
                    summary: "Target module(s)",
                },
                OptionDefinition {
                    name: "--owner",
                    summary: "Story owner(s)",
                },
            ],
        }],
        options: &[],
    },
    CommandDefinition {
        name: "schema",
        summary: "Print JSON Schema for an entity kind, or for a command's output payload",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "update",
        summary: "Update planning and execution entities",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--status",
            summary: "Update entity status",
        }],
    },
    CommandDefinition {
        name: "transition",
        summary: "Transition a story across lifecycle states",
        subcommands: &[],
        options: &[
            OptionDefinition {
                name: "--skip-gates",
                summary: "Skip transition-bound gates",
            },
            OptionDefinition {
                name: "--justification",
                summary: "Justification for transition or skipping gates",
            },
        ],
    },
    CommandDefinition {
        name: "get",
        summary: "Fetch a single entity's isolated projection",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--expand",
            summary: "Additional sections to include",
        }],
    },
    CommandDefinition {
        name: "list",
        summary: "List and filter entities of one kind",
        subcommands: &[],
        options: &[
            OptionDefinition {
                name: "--epic",
                summary: "Filter by owning epic id",
            },
            OptionDefinition {
                name: "--status",
                summary: "Filter by status",
            },
            OptionDefinition {
                name: "--owner",
                summary: "Filter by owner",
            },
            OptionDefinition {
                name: "--module",
                summary: "Filter by target module id",
            },
        ],
    },
    CommandDefinition {
        name: "relate",
        summary: "Add a relation from a source entity to a target entity",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--if-version",
            summary: "Expected source entity version",
        }],
    },
    CommandDefinition {
        name: "unrelate",
        summary: "Remove a relation from a source entity to a target entity",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--if-version",
            summary: "Expected source entity version",
        }],
    },
    CommandDefinition {
        name: "constraint",
        summary: "Declare or remove negative and rabbit-hole constraints",
        subcommands: &[
            SubcommandDefinition {
                name: "add",
                summary: "Add a negative or rabbit-hole constraint to an entity",
                options: &[
                    OptionDefinition {
                        name: "--kind",
                        summary: "Constraint kind: no_go, rabbit_hole, or appetite",
                    },
                    OptionDefinition {
                        name: "--if-version",
                        summary: "Expected existing entity version",
                    },
                ],
            },
            SubcommandDefinition {
                name: "remove",
                summary: "Remove a constraint from an entity",
                options: &[
                    OptionDefinition {
                        name: "--justification",
                        summary: "Justification for removing constraint",
                    },
                    OptionDefinition {
                        name: "--if-version",
                        summary: "Expected existing entity version",
                    },
                ],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "graph",
        summary: "Render the story dependency graph",
        subcommands: &[],
        options: &[
            OptionDefinition {
                name: "--dot",
                summary: "Emit Graphviz DOT",
            },
            OptionDefinition {
                name: "--epic",
                summary: "Filter to one epic's stories",
            },
        ],
    },
    CommandDefinition {
        name: "validate",
        summary: "Surface cached and freshly computed integrity findings",
        subcommands: &[],
        options: &[
            OptionDefinition {
                name: "--changed",
                summary: "Validate changed files only",
            },
            OptionDefinition {
                name: "--fix-ids",
                summary: "Offer guided renumber for duplicate planning ids",
            },
        ],
    },
    CommandDefinition {
        name: "sync",
        summary: "Run or rebuild the incremental hydration sweep",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--rebuild",
            summary: "Purge and rebuild cache from files",
        }],
    },
    CommandDefinition {
        name: "doctor",
        summary: "Report structured cache and workspace diagnostics",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--fix",
            summary: "Automatically fix remediable issues",
        }],
    },
    CommandDefinition {
        name: "claim",
        summary: "Claim a story lease",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "release",
        summary: "Release a story lease",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "scratch",
        summary: "Manage story scratchpads",
        subcommands: &[
            SubcommandDefinition {
                name: "append",
                summary: "Append an entry to a story's scratchpad",
                options: &[
                    OptionDefinition {
                        name: "--kind",
                        summary: "Scratchpad entry kind",
                    },
                ],
            },
            SubcommandDefinition {
                name: "read",
                summary: "Read a story's scratchpad entries",
                options: &[
                    OptionDefinition {
                        name: "--summary",
                        summary: "Summary view",
                    },
                    OptionDefinition {
                        name: "--budget",
                        summary: "Token budget",
                    },
                ],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "decision",
        summary: "Log or query decisions",
        subcommands: &[SubcommandDefinition {
            name: "log",
            summary: "Log a decision against a subject entity",
            options: &[
                OptionDefinition {
                    name: "--type",
                    summary: "Decision type",
                },
                OptionDefinition {
                    name: "--title",
                    summary: "Decision title",
                },
            ],
        }],
        options: &[],
    },
    CommandDefinition {
        name: "dw",
        summary: "Manage deferred work debt and residual anomalies",
        subcommands: &[
            SubcommandDefinition {
                name: "add",
                summary: "Add a new deferred work record",
                options: &[
                    OptionDefinition {
                        name: "--title",
                        summary: "Deferred work title",
                    },
                    OptionDefinition {
                        name: "--risk",
                        summary: "Safety risk level",
                    },
                ],
            },
            SubcommandDefinition {
                name: "list",
                summary: "List deferred work records",
                options: &[
                    OptionDefinition {
                        name: "--status",
                        summary: "Filter by status",
                    },
                    OptionDefinition {
                        name: "--risk",
                        summary: "Filter by risk",
                    },
                ],
            },
            SubcommandDefinition {
                name: "close",
                summary: "Close a deferred work record",
                options: &[
                    OptionDefinition {
                        name: "--status",
                        summary: "Target close status",
                    },
                    OptionDefinition {
                        name: "--resolution",
                        summary: "Resolution description or story ID",
                    },
                ],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "sprint",
        summary: "Manage sprints and story assignments",
        subcommands: &[
            SubcommandDefinition {
                name: "open",
                summary: "Open a new sprint",
                options: &[
                    OptionDefinition {
                        name: "--title",
                        summary: "Sprint title",
                    },
                ],
            },
            SubcommandDefinition {
                name: "assign",
                summary: "Assign stories to a sprint",
                options: &[OptionDefinition {
                    name: "--sprint",
                    summary: "Sprint identifier override",
                }],
            },
            SubcommandDefinition {
                name: "close",
                summary: "Close an active sprint with optional carry-over",
                options: &[
                    OptionDefinition {
                        name: "--status",
                        summary: "Terminal status",
                    },
                    OptionDefinition {
                        name: "--carry-over",
                        summary: "Target sprint for carry-over stories",
                    },
                ],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "review",
        summary: "Produce epic and sprint release reviews",
        subcommands: &[
            SubcommandDefinition {
                name: "epic",
                summary: "Audit story evidence and open debt for an epic",
                options: &[],
            },
            SubcommandDefinition {
                name: "sprint",
                summary: "Run sprint_close-bound gates and emit release reports",
                options: &[],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "chore",
        summary: "Start and commit path-allowlisted chores",
        subcommands: &[
            SubcommandDefinition {
                name: "start",
                summary: "Start a chore",
                options: &[OptionDefinition {
                    name: "--description",
                    summary: "Chore description",
                }],
            },
            SubcommandDefinition {
                name: "commit",
                summary: "Commit a chore",
                options: &[OptionDefinition {
                    name: "--message",
                    summary: "Commit message",
                }],
            },
            SubcommandDefinition {
                name: "list",
                summary: "List this workspace's chore records",
                options: &[],
            },
            SubcommandDefinition {
                name: "close",
                summary: "Close a chore",
                options: &[],
            },
            SubcommandDefinition {
                name: "abort",
                summary: "Abort a chore",
                options: &[],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "next",
        summary: "Deterministically select the next eligible story",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "context",
        summary: "Project a token-budgeted, phase-specific agent context for a story",
        subcommands: &[],
        options: &[
            OptionDefinition {
                name: "--phase",
                summary: "Context phase (specify, develop, review)",
            },
            OptionDefinition {
                name: "--budget",
                summary: "Token budget",
            },
            OptionDefinition {
                name: "--stats",
                summary: "Include token statistics",
            },
            OptionDefinition {
                name: "--format",
                summary: "Output format (md)",
            },
        ],
    },
    CommandDefinition {
        name: "gate",
        summary: "Run verification gates",
        subcommands: &[
            SubcommandDefinition {
                name: "run",
                summary: "Execute verification gates",
                options: &[
                    OptionDefinition {
                        name: "--all",
                        summary: "Run all gates",
                    },
                    OptionDefinition {
                        name: "--for-transition",
                        summary: "Run gates bound to transition",
                    },
                    OptionDefinition {
                        name: "--story",
                        summary: "Story identifier to evaluate",
                    },
                ],
            },
            SubcommandDefinition {
                name: "list",
                summary: "List configured gates and their status",
                options: &[],
            },
            SubcommandDefinition {
                name: "baseline",
                summary: "Inspect or record gate baselines for ratchet verification",
                options: &[],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "soup",
        summary: "Audit third-party dependencies and capture SBOM evidence",
        subcommands: &[
            SubcommandDefinition {
                name: "audit",
                summary: "Run configured dependency audit and deny checks",
                options: &[],
            },
            SubcommandDefinition {
                name: "sbom",
                summary: "Generate and attach an SBOM to a release",
                options: &[],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "preflight",
        summary: "Enforce working tree scope, branch freshness, and validation health",
        subcommands: &[],
        options: &[OptionDefinition {
            name: "--story",
            summary: "Story ID to evaluate",
        }],
    },
    CommandDefinition {
        name: "install",
        summary: "Install developer tools and integrations (e.g. Git hooks)",
        subcommands: &[
            SubcommandDefinition {
                name: "hooks",
                summary: "Install git hook shims",
                options: &[],
            },
            SubcommandDefinition {
                name: "skills",
                summary: "Install editor and agent skill bundles",
                options: &[
                    OptionDefinition {
                        name: "--claude",
                        summary: "Install Claude Code skills",
                    },
                    OptionDefinition {
                        name: "--cursor",
                        summary: "Install Cursor rule",
                    },
                    OptionDefinition {
                        name: "--agents",
                        summary: "Install Agent skills",
                    },
                ],
            },
            SubcommandDefinition {
                name: "mcp",
                summary: "Install MCP server configuration",
                options: &[
                    OptionDefinition {
                        name: "--claude",
                        summary: "Install Claude Code MCP configuration",
                    },
                    OptionDefinition {
                        name: "--cursor",
                        summary: "Install Cursor MCP configuration",
                    },
                ],
            },
        ],
        options: &[],
    },
    CommandDefinition {
        name: "hook",
        summary: "Execute a Git hook",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "hygiene",
        summary: "Inspect source code comment hygiene",
        subcommands: &[SubcommandDefinition {
            name: "check",
            summary: "Check comment hygiene rules",
            options: &[OptionDefinition {
                name: "--diff",
                summary: "Lint only changed files in diff",
            }],
        }],
        options: &[],
    },
    CommandDefinition {
        name: "impact",
        summary: "Analyze change impact across modules, active stories, relations, and gates",
        subcommands: &[],
        options: &[],
    },
    CommandDefinition {
        name: "mcp",
        summary: "Model Context Protocol (MCP) server commands",
        subcommands: &[SubcommandDefinition {
            name: "serve",
            summary: "Start stdio-based MCP JSON-RPC 2.0 server",
            options: &[],
        }],
        options: &[],
    },
];

fn render_command_catalog_markdown() -> String {
    let mut out = String::new();
    out.push_str("## Available CLI Commands\n\n");
    out.push_str("Always invoke commands with `--json` when reading or modifying state:\n\n");
    for cmd in CommandCatalog::all() {
        out.push_str(&format!("- `qdev {}`: {}\n", cmd.name, cmd.summary));
        for opt in cmd.options {
            out.push_str(&format!("  - `{}`: {}\n", opt.name, opt.summary));
        }
        for sub in cmd.subcommands {
            out.push_str(&format!("  - `qdev {} {}`: {}\n", cmd.name, sub.name, sub.summary));
            for opt in sub.options {
                out.push_str(&format!("    - `{}`: {}\n", opt.name, opt.summary));
            }
        }
    }
    out
}

/// The Structured Multi-Perspective Synthesis Template embedded in planning skills.
pub const STRUCTURED_SYNTHESIS_TEMPLATE: &str = r#"### Structured Multi-Perspective Synthesis Template
Every planning proposal must provide answers under each of the required headings:

#### Product & domain value
Problem statement, user persona, measurable success metrics, appetite.

#### Architectural constraints
Boundaries, target modules, dependencies, cross-crate interfaces, invariants.

#### Safety & risk profile
ISO 14971 hazards, regulatory compliance (IEC 62304), negative constraints (no-gos and rabbit holes).

#### Implementation directives
Specific CLI commands to create stories (`qdev create story`) and establish relations (`qdev relate`).

#### Rejection Directive
Reject any model response or proposal missing any required heading and re-prompt.
"#;

fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn resolve_model_tier<'a>(skill_name: &str, models: Option<&'a ModelsConfig>) -> &'a str {
    match skill_name {
        "qdev" => models
            .and_then(|m| m.specify.as_deref())
            .unwrap_or("reasoning"),
        "qdev-plan" => models
            .and_then(|m| m.specify.as_deref())
            .unwrap_or("reasoning"),
        "qdev-create-story" => models
            .and_then(|m| m.specify.as_deref())
            .unwrap_or("reasoning"),
        "qdev-develop" => models
            .and_then(|m| m.develop.as_deref())
            .unwrap_or("fast-coding"),
        "qdev-review" => models
            .and_then(|m| m.review.as_deref())
            .unwrap_or("strongest"),
        _ => models
            .and_then(|m| m.specify.as_deref())
            .unwrap_or("reasoning"),
    }
}

/// Generates the skill content for a specific core skill with optional model hints and synthesis configuration.
pub fn generate_skill_content_configured(
    skill_name: &str,
    models: Option<&ModelsConfig>,
    synthesis: Option<&SynthesisConfig>,
) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let catalog_md = render_command_catalog_markdown();
    let model_tier = escape_yaml(resolve_model_tier(skill_name, models));

    let default_synthesis = SynthesisConfig::default();
    let effective_synthesis = synthesis.unwrap_or(&default_synthesis);
    let synthesis_template = effective_synthesis.render_template();
    let resolved_headings = effective_synthesis.resolved_headings();
    let headings_list = resolved_headings
        .iter()
        .map(|h| format!("   - {}", h))
        .collect::<Vec<_>>()
        .join("\n");

    match skill_name {
        "qdev" => format!(
            r#"---
name: qdev
description: "Inspect project pulse, discover next recommended work, and coordinate development workflow"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
---

# Qdev Assistant Skill

You are operating in a workspace governed by `qdev`.

## Invariant Rules
1. **Always use JSON output**: Every workspace state read and mutation must be executed via `qdev <subcommand> ... --json`.
2. **Never inspect or assemble raw spec files**: Always invoke `qdev context <story-id> --phase <phase> --json` (phases: `specify`, `develop`, `review`) to inspect context. Never parse, read, or construct context from markdown files in `docs/` directly.
3. **Never write or edit raw entity markdown files directly**: State mutations must be executed through `qdev create`, `qdev update`, `qdev transition`, `qdev relate`, etc.

## Primary Workflow: Pulse & Triage
1. Check project status, pulse, and sprint health:
   ```bash
   qdev --json
   ```
   (or `qdev status --json`)
   Render the pulse to the user, including:
   - Workspace status and health
   - Sprint state and active goals
   - Gate status and baseline ratchet checks
   - Active story leases and holders
   - Next recommended steps
2. Determine the next eligible story or blocker:
   ```bash
   qdev next --json
   ```
3. Run workspace diagnostics when troubleshooting:
   ```bash
   qdev doctor --json
   ```
4. If a story is selected, transition to the appropriate skill:
   - Planning: `/qdev-plan`
   - Story Creation: `/qdev-create-story`
   - Development: `/qdev-develop`
   - Code Review: `/qdev-review`

{catalog_md}
"#
        ),
        "qdev-plan" => format!(
            r#"---
name: qdev-plan
description: "Plan and manage architecture, requirements, epics, and hazards"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
---

# Qdev Planning Skill

You are creating or refining architecture, requirements, epics, and hazards in `qdev`.

## Invariant Rules
1. **Always use JSON output**: Every read and mutation must be executed via `qdev <subcommand> ... --json`.
2. **Never read or write raw entity files directly**: All operations go through `qdev create`, `qdev update`, `qdev relate`, `qdev get`, and `qdev list`.
3. **Never assemble context from raw files**: Use `qdev context <id> --phase specify --json` to inspect context.

{synthesis_template}## Workflow: Planning Entities
1. Query existing entities:
   ```bash
   qdev list epic --json
   qdev list requirement --json
   qdev list adr --json
   qdev list hazard --json
   ```
2. Inspect specific entity details:
   ```bash
   qdev get <id> --json
   ```
3. Synthesize planning proposal using the Structured Multi-Perspective Synthesis Template covering all required headings:
{headings_list}
   Strict Rejection Directive: Reject any model response or proposal missing any required heading and re-prompt.
4. Execute implementation directives:
   Create stories with `qdev create story` and establish relations with `qdev relate`:
   ```bash
   qdev create story <epic-id> --title "<title>" --module "<module>" --appetite <appetite> --json
   qdev relate <source-id> <relation> <target-id> --json
   ```
5. Establish relationships between entities:
   ```bash
   qdev relate <source-id> <relation> <target-id> --json
   ```
6. Verify workspace consistency:
   ```bash
   qdev validate --json
   ```

{catalog_md}
"#
        ),
        "qdev-create-story" => format!(
            r#"---
name: qdev-create-story
description: "Specify and create user stories, establish constraints, and transition stories to ready"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
---

# Qdev Create Story Skill

You are creating and refining user stories in `qdev`.

## Invariant Rules
1. **Always use JSON output**: Every command must use `--json`.
2. **Never assemble or inspect context from raw files directly**: Always run `qdev context <epic-id> --phase specify --json` or `qdev get <id> --json`.
3. **Never write raw story files directly**: Always use `qdev create story`.

{synthesis_template}## Workflow: Story Creation & Readiness
1. Inspect parent epic context:
   ```bash
   qdev context <epic-id> --phase specify --json
   ```
2. Synthesize story proposal, acceptance criteria, and negative constraints (no-gos and rabbit holes) using the Structured Multi-Perspective Synthesis Template covering all required headings:
{headings_list}
   Strict Rejection Directive: Reject any model response or proposal missing any required heading and re-prompt.
3. Create story with title, target modules, and appetite:
   ```bash
   qdev create story <epic-id> --title "<title>" --module "<module>" --appetite <appetite> --json
   ```
4. Declare negative or rabbit-hole constraints:
   ```bash
   qdev constraint add <story-id> --kind <no_go|rabbit_hole|appetite> "<constraint text>" --json
   ```
5. Relate dependencies:
   ```bash
   qdev relate <story-id> depends_on <dependency-id> --json
   ```
6. Once story has clear acceptance criteria and negative constraints, transition to ready:
   ```bash
   qdev transition story <story-id> ready --json
   ```

{catalog_md}
"#
        ),
        "qdev-develop" => format!(
            r#"---
name: qdev-develop
description: "Implement a story: run preflight, claim lease, fetch develop context, update scratchpad, run gates, and transition to review"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
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

{catalog_md}
"#
        ),
        "qdev-review" => format!(
            r#"---
name: qdev-review
description: "Review a story: audit review context and diff against AC and constraints, evaluate impact, and transition to done or in_progress"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
---

# Qdev Review Skill

You are reviewing story implementation in `qdev`.

## Invariant Rules
1. **Always use JSON output**: Run all commands with `--json`.
2. **Never assemble or inspect context from raw files directly**: Always run `qdev context <story-id> --phase review --json`.
3. **Never update raw status files directly**: Use `qdev transition` and `qdev scratch append`.

## Workflow: Story Review
1. Fetch scoped review context:
   ```bash
   qdev context <story-id> --phase review --json
   ```
2. Audit diff against acceptance criteria and negative constraints.
3. Analyze change impact across modules and dependencies:
   ```bash
   qdev impact <story-id> --json
   ```
4. Run all verification gates:
   ```bash
   qdev gate run --all --story <story-id> --json
   ```
5. If approved, transition to done:
   ```bash
   qdev transition story <story-id> done --json
   ```
6. If changes requested, append findings to scratchpad and transition back to in_progress with justification:
   ```bash
   qdev scratch append <story-id> "<review finding>" --kind note --json
   qdev transition story <story-id> in_progress --justification "<justification>" --json
   ```

{catalog_md}
"#
        ),
        _ => format!(
            r#"---
name: {skill_name}
description: "Qdev assistant skill"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
---

# Qdev Assistant Skill ({skill_name})

You are operating in a workspace governed by `qdev`.

## Invariant Rules
1. **Always use JSON output**: Every workspace state read and mutation must be executed via `qdev <subcommand> ... --json`.
2. **Never inspect or assemble raw spec files**: Always invoke `qdev context <story-id> --phase <phase> --json`.
3. **Never write or edit raw entity markdown files directly**: State mutations must be executed through `qdev create`, `qdev update`, `qdev transition`, `qdev relate`, etc.

{catalog_md}
"#
        ),
    }
}

/// Generates the skill content for a specific core skill with optional model hints.
pub fn generate_skill_content_with_models(
    skill_name: &str,
    models: Option<&ModelsConfig>,
) -> String {
    generate_skill_content_configured(skill_name, models, None)
}

/// Generates the skill content for a specific core skill with default model tiers.
pub fn generate_skill_content(skill_name: &str) -> String {
    generate_skill_content_configured(skill_name, None, None)
}

/// Generates the Cursor rule content for `.cursor/rules/qdev.mdc` with optional model hints and synthesis configuration.
pub fn generate_cursor_rule_content_configured(
    models: Option<&ModelsConfig>,
    synthesis: Option<&SynthesisConfig>,
) -> String {
    let version = env!("CARGO_PKG_VERSION");
    let catalog_md = render_command_catalog_markdown();
    let model_tier = escape_yaml(
        models
            .and_then(|m| m.specify.as_deref())
            .unwrap_or("reasoning"),
    );

    let default_synthesis = SynthesisConfig::default();
    let effective_synthesis = synthesis.unwrap_or(&default_synthesis);
    let synthesis_template = effective_synthesis.render_template();
    let headings_summary = effective_synthesis.resolved_headings().join(", ");

    format!(
        r#"---
description: "Qdev workspace development and workflow instructions"
globs: "*"
version: "{version}"
qdev_version: "{version}"
model_hint: "{model_tier}"
model: "{model_tier}"
---

# Qdev Workspace Rules

You are working in a codebase governed by `qdev`.

## Core Invariant Rules
- **Always use JSON output**: Every workspace state read and mutation must be executed via `qdev <subcommand> ... --json`.
- **Never assemble or inspect context from raw files directly**: Always use `qdev context <id> --phase <phase> --json` (phases: `specify`, `develop`, `review`). Never parse, inspect, or reconstruct markdown files from `docs/` directly.
- **Never write or edit raw entity markdown files directly**: Always use `qdev create`, `qdev update`, `qdev transition`, `qdev relate`, etc.

{synthesis_template}## Workflows
- `/qdev`: Pulse inspection (`qdev --json` or `qdev status --json`) and next step discovery (`qdev next --json`). Render workspace health, sprint state, gates, leases, and next steps.
- `/qdev-plan`: Manage architecture, requirements, epics, and hazards using the Structured Multi-Perspective Synthesis Template covering all required headings ({headings_summary}) and rejecting proposals missing any required heading (`qdev list`, `qdev get`, `qdev create story`, `qdev relate`).
- `/qdev-create-story`: Story specification, constraints, and transition to ready using the Structured Multi-Perspective Synthesis Template covering all required headings ({headings_summary}) and rejecting proposals missing any required heading (`qdev context <epic-id> --phase specify --json`, draft acceptance criteria and negative constraints, `qdev create story`, `qdev constraint add`, `qdev transition story <id> ready --json`).
- `/qdev-develop`: Implementation lifecycle (`qdev preflight --story <id> --json`, `qdev claim <id> --json`, `qdev context <id> --phase develop --json`, `qdev scratch append`, `qdev gate run --for-transition review`, `qdev transition story <id> review --json`, quote cited IDs on refusal).
- `/qdev-review`: Code review and audit (`qdev context <id> --phase review --json`, audit diff against AC/constraints, `qdev impact <id> --json`, `qdev gate run --all`, `qdev transition story <id> done --json` or `qdev transition story <id> in_progress --justification "<justification>" --json`).

{catalog_md}
"#
    )
}

/// Generates the Cursor rule content for `.cursor/rules/qdev.mdc` with optional model hints.
pub fn generate_cursor_rule_content_with_models(models: Option<&ModelsConfig>) -> String {
    generate_cursor_rule_content_configured(models, None)
}

/// Generates the Cursor rule content for `.cursor/rules/qdev.mdc`.
pub fn generate_cursor_rule_content() -> String {
    generate_cursor_rule_content_configured(None, None)
}

/// Generates Claude skills returning pairs of (relative_path, content) with optional model hints and synthesis configuration.
pub fn generate_claude_skills_configured(
    models: Option<&ModelsConfig>,
    synthesis: Option<&SynthesisConfig>,
) -> Vec<(PathBuf, String)> {
    CORE_SKILL_NAMES
        .iter()
        .map(|name| {
            let path = PathBuf::from(".claude/skills")
                .join(name)
                .join("SKILL.md");
            let content = generate_skill_content_configured(name, models, synthesis);
            (path, content)
        })
        .collect()
}

/// Generates Claude skills returning pairs of (relative_path, content) with optional model hints.
pub fn generate_claude_skills_with_models(models: Option<&ModelsConfig>) -> Vec<(PathBuf, String)> {
    generate_claude_skills_configured(models, None)
}

/// Generates Claude skills returning pairs of (relative_path, content).
pub fn generate_claude_skills() -> Vec<(PathBuf, String)> {
    generate_claude_skills_configured(None, None)
}

/// Generates Cursor rule returning (relative_path, content) with optional model hints and synthesis configuration.
pub fn generate_cursor_rule_configured(
    models: Option<&ModelsConfig>,
    synthesis: Option<&SynthesisConfig>,
) -> (PathBuf, String) {
    (
        PathBuf::from(".cursor/rules/qdev.mdc"),
        generate_cursor_rule_content_configured(models, synthesis),
    )
}

/// Generates Cursor rule returning (relative_path, content) with optional model hints.
pub fn generate_cursor_rule_with_models(models: Option<&ModelsConfig>) -> (PathBuf, String) {
    generate_cursor_rule_configured(models, None)
}

/// Generates Cursor rule returning (relative_path, content).
pub fn generate_cursor_rule() -> (PathBuf, String) {
    generate_cursor_rule_configured(None, None)
}

/// Generates Agent skills returning pairs of (relative_path, content) with optional model hints and synthesis configuration.
pub fn generate_agent_skills_configured(
    models: Option<&ModelsConfig>,
    synthesis: Option<&SynthesisConfig>,
) -> Vec<(PathBuf, String)> {
    CORE_SKILL_NAMES
        .iter()
        .map(|name| {
            let path = PathBuf::from(".agents/skills")
                .join(name)
                .join("SKILL.md");
            let content = generate_skill_content_configured(name, models, synthesis);
            (path, content)
        })
        .collect()
}

/// Generates Agent skills returning pairs of (relative_path, content) with optional model hints.
pub fn generate_agent_skills_with_models(models: Option<&ModelsConfig>) -> Vec<(PathBuf, String)> {
    generate_agent_skills_configured(models, None)
}

/// Generates Agent skills returning pairs of (relative_path, content).
pub fn generate_agent_skills() -> Vec<(PathBuf, String)> {
    generate_agent_skills_configured(None, None)
}

/// Options specifying target environments for `install_skills`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SkillInstallOptions {
    pub claude: bool,
    pub cursor: bool,
    pub agents: bool,
}

/// Payload returned by `install_skills`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsInstallReport {
    pub targets: Vec<String>,
    pub installed_files: Vec<String>,
    pub version: String,
}

/// Status returned by `inspect_skills`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsStatus {
    pub binary_version: String,
    pub installed_count: usize,
    pub outdated_count: usize,
    pub up_to_date: bool,
    pub outdated_skills: Vec<String>,
}

fn write_managed_file(
    workspace_root: &Path,
    rel_path: &Path,
    content: &str,
) -> Result<String, QdevError> {
    let full_path = workspace_root.join(rel_path);

    // If file already exists, check whether it is a qdev-managed file.
    // Never overwrite non-qdev skills or rules.
    if full_path.exists() {
        let existing = fs::read_to_string(&full_path).map_err(|e| {
            QdevError::conflict(
                "skill_conflict",
                format!(
                    "Existing file at '{}' is unreadable: {}; refusing to overwrite",
                    rel_path.display(),
                    e
                ),
            )
        })?;

        let is_qdev_managed = if let Ok(fm) = extract_frontmatter(&existing) {
            fm.get("qdev_version").is_some() || fm.get("version").is_some()
        } else {
            false
        };

        if !is_qdev_managed {
            return Err(QdevError::conflict(
                "skill_conflict",
                format!(
                    "Existing file at '{}' is not a qdev skill/rule; refusing to overwrite",
                    rel_path.display()
                ),
            ));
        }
    }

    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to create directory '{}': {}", parent.display(), e),
            )
        })?;
    }

    write_file_atomic(&full_path, content)?;

    Ok(rel_path.to_string_lossy().to_string())
}

/// Installs skills and rules for selected target editors and agents with optional model hints and synthesis configuration.
///
/// Reads advisory model selections from `models` and synthesis configuration from `synthesis`,
/// or falls back to loading workspace configuration from `workspace_root`, or uses defaults.
pub fn install_skills_configured(
    workspace_root: &Path,
    options: &SkillInstallOptions,
    models: Option<&ModelsConfig>,
    synthesis: Option<&SynthesisConfig>,
) -> Result<SkillsInstallReport, QdevError> {
    if !options.claude && !options.cursor && !options.agents {
        return Err(QdevError::usage_error(
            "specify at least one of --claude, --cursor, or --agents",
        ));
    }

    let loaded_config;
    let (effective_models, effective_synthesis) = match (models, synthesis) {
        (Some(m), Some(s)) => (Some(m), Some(s)),
        _ => {
            if let Ok(annotated) = crate::config::load_config(workspace_root) {
                loaded_config = Some(annotated.config);
                let cfg = loaded_config.as_ref().unwrap();
                (
                    models.or(Some(&cfg.models)),
                    synthesis.or(Some(&cfg.synthesis)),
                )
            } else {
                (models, synthesis)
            }
        }
    };

    let mut targets = Vec::new();
    let mut installed_files = Vec::new();

    if options.claude {
        targets.push("claude".to_string());
        for (rel_path, content) in generate_claude_skills_configured(effective_models, effective_synthesis) {
            let written = write_managed_file(workspace_root, &rel_path, &content)?;
            installed_files.push(written);
        }
    }

    if options.cursor {
        targets.push("cursor".to_string());
        let (rel_path, content) = generate_cursor_rule_configured(effective_models, effective_synthesis);
        let written = write_managed_file(workspace_root, &rel_path, &content)?;
        installed_files.push(written);
    }

    if options.agents {
        targets.push("agents".to_string());
        for (rel_path, content) in generate_agent_skills_configured(effective_models, effective_synthesis) {
            let written = write_managed_file(workspace_root, &rel_path, &content)?;
            installed_files.push(written);
        }
    }

    Ok(SkillsInstallReport {
        targets,
        installed_files,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Installs skills and rules for selected target editors and agents with optional model hints.
///
/// Reads advisory model selections from `models`, or falls back to loading workspace
/// configuration from `workspace_root`, or uses default model tiers.
/// Auto-loads workspace `synthesis` configuration if not explicitly supplied.
pub fn install_skills_with_models(
    workspace_root: &Path,
    options: &SkillInstallOptions,
    models: Option<&ModelsConfig>,
) -> Result<SkillsInstallReport, QdevError> {
    install_skills_configured(workspace_root, options, models, None)
}

/// Installs skills and rules for selected target editors and agents.
///
/// Auto-loads model and synthesis configuration from `workspace_root` or falls back to defaults.
pub fn install_skills(
    workspace_root: &Path,
    options: &SkillInstallOptions,
) -> Result<SkillsInstallReport, QdevError> {
    install_skills_configured(workspace_root, options, None, None)
}

/// Inspects the installation state of qdev skills and rules in the workspace.
pub fn inspect_skills(workspace_root: &Path) -> Result<SkillsStatus, QdevError> {
    let binary_version = env!("CARGO_PKG_VERSION");
    let mut installed_count = 0usize;
    let mut outdated_count = 0usize;
    let mut outdated_skills = Vec::new();

    for rel_path_str in MANAGED_SKILL_PATHS {
        let full_path = workspace_root.join(rel_path_str);
        if !full_path.is_file() {
            continue;
        }

        let content = match fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => continue, // Cannot read or foreign file -> ignore
        };

        let fm = match extract_frontmatter(&content) {
            Ok(fm) => fm,
            Err(_) => continue, // Foreign file lacking frontmatter -> ignore
        };

        let stamp = fm
            .get("qdev_version")
            .and_then(|v| v.as_str())
            .or_else(|| fm.get("version").and_then(|v| v.as_str()));

        let Some(stamp) = stamp else {
            // Foreign file lacking qdev version stamp -> ignore
            continue;
        };

        installed_count += 1;

        if stamp != binary_version {
            outdated_count += 1;
            outdated_skills.push(rel_path_str.to_string());
        }
    }

    let up_to_date = outdated_count == 0;

    Ok(SkillsStatus {
        binary_version: binary_version.to_string(),
        installed_count,
        outdated_count,
        up_to_date,
        outdated_skills,
    })
}
