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
    /// Fetch a single entity's isolated projection
    Get(GetArgs),
    /// List and filter entities of one kind
    List(ListArgs),
    /// Add a relation from a source entity to a target entity
    Relate(RelateArgs),
    /// Remove a relation from a source entity to a target entity
    Unrelate(UnrelateArgs),
    /// Render the story dependency graph
    Graph(GraphArgs),
    /// Surface cached and freshly computed integrity findings
    Validate(ValidateArgs),
    /// Run or rebuild the incremental hydration sweep
    Sync(SyncArgs),
    /// Report structured cache and workspace diagnostics
    Doctor,
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

    /// Relation name (e.g. depends_on, extends, supersedes, traces_to, mitigates, closes_dw,
    /// governed_by)
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

    /// Relation name (e.g. depends_on)
    pub relation: String,

    /// Target entity identifier, e.g. E12S1
    pub target_id: String,

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

    /// Automatically confirm migrations or prompts
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

/// Helper to check whether `--json` was passed in raw command-line arguments.
pub fn is_json_requested(args: &[String]) -> bool {
    args.iter()
        .skip(1)
        .take_while(|a| *a != "--")
        .any(|arg| arg == "--json" || arg.starts_with("--json="))
}
