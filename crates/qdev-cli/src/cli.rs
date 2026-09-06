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
