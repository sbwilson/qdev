use clap::{Parser, Subcommand};

#[derive(Parser, Debug, Clone)]
#[command(
    name = "qdev",
    about = "Development Engine & Gatekeeper",
    version = env!("CARGO_PKG_VERSION"),
    disable_version_flag = true
)]
pub struct Cli {
    /// Print version information
    #[arg(short = 'V', long)]
    pub version: bool,

    /// Output JSON instead of text
    #[arg(long, global = true)]
    pub json: bool,

    /// Run in non-interactive mode
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Override scope lease or cross-team mutation restrictions
    #[arg(long = "override", global = true)]
    pub r#override: bool,

    /// Justification for override
    #[arg(long, global = true)]
    pub justification: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum Commands {
    /// Show pulse and status
    Status,
    /// Inspect or manage configuration
    Config(ConfigArgs),
    /// Scaffold config, directories, cache, and gitignore
    Init(InitArgs),
    /// Create planning and execution entities
    Create(CreateArgs),
    /// Print JSON Schema for an entity kind, or for a command's output payload
    /// (`qdev schema payload <name>`)
    Schema(SchemaArgs),
    /// Update planning and execution entities
    Update(UpdateArgs),
    /// Transition a story across lifecycle states
    Transition(TransitionArgs),
    /// Fetch a single entity's isolated projection
    Get(GetArgs),
    /// List and filter entities of one kind
    List(ListArgs),
    /// Add a relation from a source entity to a target entity
    Relate(RelateArgs),
    /// Remove a relation from a source entity to a target entity
    Unrelate(UnrelateArgs),
    /// Declare or remove negative and rabbit-hole constraints
    Constraint(ConstraintArgs),
    /// Render the story dependency graph
    Graph(GraphArgs),
    /// Surface cached and freshly computed integrity findings
    Validate(ValidateArgs),
    /// Run or rebuild the incremental hydration sweep
    Sync(SyncArgs),
    /// Report structured cache and workspace diagnostics
    Doctor,
    /// Claim a story lease
    Claim(ClaimArgs),
    /// Release a story lease
    Release(ReleaseArgs),
    /// Manage story scratchpads
    Scratch(ScratchArgs),
    /// Log or query decisions
    Decision(DecisionArgs),
    /// Manage deferred work debt and residual anomalies
    Dw(DwArgs),
    /// Manage sprints and story assignments
    Sprint(SprintArgs),
    /// Start and commit path-allowlisted chores
    Chore(ChoreArgs),
    /// Deterministically select the next eligible story
    Next(NextArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ChoreArgs {
    #[command(subcommand)]
    pub command: ChoreCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ChoreCommands {
    /// Record a chore with its path allowlist
    Start(ChoreStartArgs),
    /// Commit only the changes under the chore's allowlist
    Commit(ChoreCommitArgs),
    /// List this workspace's chore records and how each one ended
    List,
    /// Settle the open chore without committing it (work done another way, or dropped)
    Close(ChoreFinishArgs),
    /// Drop the open chore: the work is not happening
    Abort(ChoreFinishArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ChoreStartArgs {
    /// Short description of the chore
    pub title: String,

    /// Workspace-relative path glob the chore may touch (repeatable)
    ///
    /// Not clap-`required`, and one value per flag: an invocation with no `--paths` must reach
    /// `start_chore` to be refused there with `paths_required` (the code the spec's I/O matrix
    /// promises), and an open-ended `num_args` would swallow the positional title.
    #[arg(long = "paths", num_args = 1, action = clap::ArgAction::Append)]
    pub paths: Vec<String>,

    /// Start the chore even though this worktree holds a story lease
    #[arg(long = "alongside")]
    pub alongside: bool,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ChoreCommitArgs {
    /// Refuse (exit 3) instead of skipping any change outside the allowlist
    #[arg(long = "strict")]
    pub strict: bool,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct NextArgs {
    /// Select from this sprint's story assignments whether or not it is active;
    /// without it, candidates come from every sprint with `status: active`
    #[arg(long = "sprint")]
    pub sprint: Option<u32>,

    /// Only consider stories owned by this identity; "me" resolves to the current
    /// identity (config identity, or their teams) via the resolve_author chain
    #[arg(short = 'o', long = "owner")]
    pub owner: Option<String>,
}

/// Arguments for `qdev chore close` and `qdev chore abort`, which differ only in the outcome
/// they record.
#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ChoreFinishArgs {
    /// Why this chore is being settled without a commit
    #[arg(long = "reason")]
    pub reason: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaimArgs {
    /// Target entity kind or identifier (e.g. "story" or "E12S4")
    pub target: String,

    /// Story identifier when kind is specified (e.g. "E12S4")
    pub id: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ReleaseArgs {
    /// Target entity kind or identifier (e.g. "story" or "E12S4")
    pub target: Option<String>,

    /// Story identifier when kind is specified (e.g. "E12S4")
    pub id: Option<String>,

    /// Force release of another worker's or stale lease
    #[arg(long = "force")]
    pub force: bool,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncArgs {
    /// Drop and recreate the cache from Markdown before reporting counts
    #[arg(long = "rebuild")]
    pub rebuild: bool,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ValidateArgs {
    /// Restrict findings to those touching files changed since the merge-base with
    /// `config.git.integration_branch`
    #[arg(long = "changed")]
    pub changed: bool,

    /// Offer a guided renumber for duplicate planning ids
    #[arg(long = "fix-ids")]
    pub fix_ids: bool,

    /// Automatically confirm the `--fix-ids` renumber in non-interactive mode
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphArgs {
    /// Emit Graphviz DOT (the only supported output format for now)
    #[arg(long)]
    pub dot: bool,

    /// Filter to one epic's stories (e.g. E12)
    #[arg(long = "epic")]
    pub epic: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct RelateArgs {
    /// Source entity identifier (relations live in the source entity's frontmatter), e.g. E12S4
    pub source_id: String,

    /// Relation name (see `qdev relate --help` output below, or architecture.md §8): one of
    /// depends_on, extends, supersedes, traces_to, verifies, mitigates,
    /// closes_dw, governed_by (anything else is a usage error)
    pub relation: String,

    /// Target entity identifier, e.g. E12S1
    pub target_id: String,

    /// Optimistic concurrency control: expected existing source entity version
    #[arg(long = "if-version")]
    pub if_version: Option<u64>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct UnrelateArgs {
    /// Source entity identifier (relations live in the source entity's frontmatter), e.g. E12S4
    pub source_id: String,

    /// Relation name (see `qdev relate --help` output below, or architecture.md §8): one of
    /// depends_on, extends, supersedes, traces_to, verifies, mitigates,
    /// closes_dw, governed_by (anything else is a usage error)
    pub relation: String,

    /// Target entity identifier, e.g. E12S1
    pub target_id: String,

    /// Optimistic concurrency control: expected existing source entity version
    #[arg(long = "if-version")]
    pub if_version: Option<u64>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ConstraintArgs {
    #[command(subcommand)]
    pub command: ConstraintCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConstraintCommands {
    /// Add a negative or rabbit-hole constraint to an entity
    Add(ConstraintAddArgs),
    /// Remove a constraint from an entity
    Remove(ConstraintRemoveArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ConstraintAddArgs {
    /// Target entity identifier (e.g. E12S4 or E12)
    pub target: String,

    /// Constraint kind: 'no_go', 'rabbit_hole', or 'appetite'
    #[arg(long = "kind")]
    pub kind: String,

    /// Constraint description text
    pub text: String,

    /// Optimistic concurrency control: expected existing entity version
    #[arg(long = "if-version")]
    pub if_version: Option<u64>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ConstraintRemoveArgs {
    /// Constraint identifier to remove (e.g. E12S4/NG-1)
    pub target: String,

    /// Justification for removing a constraint from a non-draft entity
    #[arg(long = "justification")]
    pub justification: Option<String>,

    /// Optimistic concurrency control: expected existing entity version
    #[arg(long = "if-version")]
    pub if_version: Option<u64>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct GetArgs {
    /// Target entity kind or identifier (e.g. "story" or "E12S4")
    pub target: String,

    /// Target entity identifier when kind is specified (e.g. "E12S4")
    pub id: Option<String>,

    /// Additional sections to include: scratch (relations/constraints are always included and
    /// are accepted here as no-ops for forward compatibility)
    #[arg(long = "expand", value_delimiter = ',')]
    pub expand: Vec<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ListArgs {
    /// Entity kind to list (e.g. "story" or "stories")
    pub kind: String,

    /// Filter by owning epic id (e.g. E12)
    #[arg(long = "epic")]
    pub epic: Option<String>,

    /// Filter by status
    #[arg(short = 's', long = "status")]
    pub status: Option<String>,

    /// Filter by owner; "me" resolves against the active identity
    #[arg(short = 'o', long = "owner")]
    pub owner: Option<String>,

    /// Filter by target module id
    #[arg(short = 'm', long = "module")]
    pub module: Option<String>,

    /// Filter by assigned sprint number
    #[arg(long = "sprint")]
    pub sprint: Option<i64>,

    /// Filter decisions by subject entity id (e.g. E12S4)
    #[arg(long = "subject")]
    pub subject: Option<String>,

    /// Filter decisions by decision type
    #[arg(long = "type")]
    pub r#type: Option<String>,

    /// Filter deferred work by safety risk level
    #[arg(long = "risk")]
    pub risk: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateArgs {
    /// Target entity kind or identifier (e.g. "story" or "E12S4")
    pub target: String,

    /// Target entity identifier when kind is specified (e.g. "E12S4")
    pub id: Option<String>,

    /// Update entity status
    #[arg(short = 's', long = "status")]
    pub status: Option<String>,

    /// Update entity title
    #[arg(short = 't', long = "title")]
    pub title: Option<String>,

    /// Arbitrary frontmatter field update (KEY=VALUE)
    #[arg(long = "field", value_name = "KEY=VALUE")]
    pub field: Vec<String>,

    /// Markdown body section heading to replace
    #[arg(long = "section")]
    pub section: Option<String>,

    /// Path to file containing replacement section body
    #[arg(long = "file")]
    pub file: Option<String>,

    /// Optimistic concurrency control: expected existing entity version
    #[arg(long = "if-version")]
    pub if_version: Option<u64>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct TransitionArgs {
    /// Entity kind to transition (only "story" supported)
    pub kind: String,

    /// Story identifier (e.g. E12S4)
    pub id: String,

    /// Target status to transition to
    pub target_status: String,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,

    /// Optimistic concurrency control: expected existing entity version
    #[arg(long = "if-version")]
    pub if_version: Option<u64>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaArgs {
    /// Entity schema kind (e.g. story, prd, epic, dw), or the literal "payload" to print a CLI
    /// output-payload schema instead (see `name`)
    pub kind: String,

    /// Payload name, only used when `kind` is "payload" (e.g. story, error, validate)
    pub name: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct CreateArgs {
    #[command(subcommand)]
    pub command: CreateCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum CreateCommands {
    /// Create a new story
    Story(CreateStoryArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct CreateStoryArgs {
    /// Epic identifier (e.g. E12)
    pub epic: String,

    /// Story title
    #[arg(short = 't', long = "title")]
    pub title: Option<String>,

    /// Story appetite (tiny, small, medium, deep)
    #[arg(short = 'a', long = "appetite")]
    pub appetite: Option<String>,

    /// Target module(s)
    #[arg(short = 'm', long = "module")]
    pub module: Vec<String>,

    /// Story owner(s)
    #[arg(short = 'o', long = "owner")]
    pub owner: Vec<String>,

    /// Safety class (ClassA, ClassB, ClassC)
    #[arg(long = "safety-class")]
    pub safety_class: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct InitArgs {
    /// Project name
    #[arg(short = 'n', long = "name")]
    pub name: Option<String>,

    /// Developer identifier
    #[arg(short = 'd', long = "developer")]
    pub developer: Option<String>,

    /// Team name(s)
    #[arg(short = 't', long = "team", value_name = "TEAM")]
    pub team: Vec<String>,

    /// Retained for compatibility; has no effect (an older cache is migrated without confirming)
    ///
    /// The flag existed solely to confirm a cache schema migration. `init` now migrates an older
    /// cache unconditionally, as every other command's boot already did, so there is nothing
    /// left for it to confirm. It stays accepted — and deliberately unread — because turning
    /// `qdev init --yes` into a usage error would break every script and CI job that passes it.
    #[arg(short = 'y', long = "yes")]
    pub yes: bool,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ConfigCommands {
    /// Show effective merged configuration
    Show,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct ScratchArgs {
    #[command(subcommand)]
    pub command: ScratchCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum ScratchCommands {
    /// Append an entry to a story's scratchpad
    Append(ScratchAppendArgs),
    /// Read a story's scratchpad entries
    Read(ScratchReadArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ScratchAppendArgs {
    /// Target story identifier (e.g. "E12S4")
    pub story: String,

    /// Scratchpad entry text
    pub text: String,

    /// Scratchpad entry kind: 'note', 'decision', 'tradeoff', or 'transition'
    #[arg(long = "kind")]
    pub kind: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct ScratchReadArgs {
    /// Target story identifier (e.g. "E12S4")
    pub story: String,

    /// Filter to key decisions, transitions, and recent entries
    #[arg(long = "summary")]
    pub summary: bool,

    /// Number of recent entries to include in summary (default: 5)
    #[arg(long = "last", requires = "summary")]
    pub last: Option<usize>,

    /// Token budget to bound output entries
    #[arg(long = "budget")]
    pub budget: Option<usize>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct DecisionArgs {
    #[command(subcommand)]
    pub command: DecisionCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum DecisionCommands {
    /// Log a decision against a subject entity
    Log(DecisionLogArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct DecisionLogArgs {
    /// Subject entity identifier (e.g. E12S4, E12, AD-43)
    #[arg(long = "subject")]
    pub subject: String,

    /// Decision type: human_ruling, agent_assumption, cross_team_override, pivot, review_rejection, lease_override, story_abandoned, story_superseded
    #[arg(long = "type")]
    pub r#type: String,

    /// Short decision topic
    #[arg(long = "topic")]
    pub topic: String,

    /// Decision ruling or rationale
    #[arg(long = "ruling")]
    pub ruling: String,

    /// Context or background for the decision (defaults to "Decision on <subject>")
    #[arg(long = "context")]
    pub context: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct DwArgs {
    #[command(subcommand)]
    pub command: DwCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum DwCommands {
    /// Add a new deferred work record
    Add(DwAddArgs),
    /// List deferred work records
    List(DwListArgs),
    /// Close a deferred work record
    Close(DwCloseArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct DwAddArgs {
    /// Title of the deferred work
    #[arg(short = 't', long = "title")]
    pub title: String,

    /// Target module (must exist in registered config modules)
    #[arg(short = 'm', long = "module")]
    pub module: String,

    /// Safety risk level: negligible, acceptable_with_mitigation, unacceptable
    #[arg(long = "risk")]
    pub risk: String,

    /// Origin story ID (e.g. E12S4)
    #[arg(short = 's', long = "story")]
    pub story: Option<String>,

    /// Safety rationale (required for non-negligible risk)
    #[arg(long = "rationale")]
    pub rationale: Option<String>,

    /// Associated gate ID
    #[arg(long = "gate")]
    pub gate: Option<String>,

    /// Owners
    #[arg(short = 'o', long = "owner")]
    pub owners: Vec<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct DwListArgs {
    /// Filter by module ID
    #[arg(short = 'm', long = "module")]
    pub module: Option<String>,

    /// Filter by safety risk level
    #[arg(long = "risk")]
    pub risk: Option<String>,

    /// Filter by status (e.g. open, done, wont_fix)
    #[arg(short = 's', long = "status")]
    pub status: Option<String>,

    /// Filter by origin story ID (e.g. E12S4)
    #[arg(long = "story")]
    pub story: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct DwCloseArgs {
    /// Deferred work ID to close (e.g. DW-7f3a)
    pub id: String,

    /// Target close status ('done' or 'wont_fix', default: 'done')
    #[arg(short = 's', long = "status")]
    pub status: Option<String>,

    /// Resolution description or story ID
    #[arg(long = "resolution")]
    pub resolution: Option<String>,

    /// Justification (required for wont_fix if resolution is omitted)
    #[arg(long = "justification")]
    pub justification: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq)]
pub struct SprintArgs {
    #[command(subcommand)]
    pub command: SprintCommands,
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum SprintCommands {
    /// Open a new sprint
    Open(SprintOpenArgs),
    /// Assign stories to a sprint
    Assign(SprintAssignArgs),
    /// Close an active sprint with optional carry-over
    Close(SprintCloseArgs),
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct SprintOpenArgs {
    /// Sprint number or identifier (e.g. 6 or sprint-6)
    pub id: String,

    /// Sprint title
    #[arg(long = "title")]
    pub title: String,

    /// Target release version (e.g. 0.1.0)
    #[arg(long = "release")]
    pub release: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct SprintAssignArgs {
    /// Sprint number or first story
    pub first: Option<String>,

    /// Remaining stories or arguments
    pub rest: Vec<String>,

    /// Sprint number or ID override
    #[arg(long = "sprint")]
    pub sprint: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

#[derive(Parser, Debug, Clone, PartialEq, Eq, Default)]
pub struct SprintCloseArgs {
    /// Sprint number or identifier to close (e.g. 5 or sprint-5)
    pub id: Option<String>,

    /// Sprint number or ID override
    #[arg(long = "sprint")]
    pub sprint: Option<String>,

    /// Target sprint number or identifier for carry-over stories (e.g. 6 or sprint-6)
    #[arg(long = "carry-over")]
    pub carry_over: Option<String>,

    /// Attribution author type override ('human' or 'agent')
    #[arg(long = "author-type")]
    pub author_type: Option<String>,

    /// Attribution developer/agent ID override
    #[arg(long = "author-id")]
    pub author_id: Option<String>,
}

/// Helper to check whether `--json` was passed in raw command-line arguments.
pub fn is_json_requested(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|arg| arg == "--json" || arg.starts_with("--json="))
}
