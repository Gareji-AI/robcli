use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "rob",
    version,
    about = "Read-only Roblox Creator Dashboard CLI"
)]
#[command(
    after_help = "Offline by default. --live permits one bounded read-only probe.\nNo API-key, OAuth, cookie-export, or write commands are supported."
)]
pub(crate) struct Cli {
    /// Explicitly allow one bounded live probe. Never use in a test loop.
    #[arg(long, global = true)]
    pub live: bool,

    /// Select an existing Firefox profile by its name (not a cookie value).
    #[arg(long, global = true)]
    pub profile: Option<String>,

    /// Structured output; JSON is also the default.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Check the signed-in Firefox account through a read-only API.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Show the signed-in Roblox account. Alias of auth check.
    Whoami,
    /// Inspect user-owned experiences. Group-owned experiences are not included.
    Experiences {
        #[command(subcommand)]
        command: ExperienceCommand,
    },
    /// Discover the supported commands and safety limits without reading cookies.
    Schema,
}

impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Auth { .. } => "auth check",
            Self::Whoami => "whoami",
            Self::Experiences { .. } => "experiences list",
            Self::Schema => "schema",
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Verify account identity; does not export or persist the session cookie.
    Check,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ExperienceCommand {
    /// List non-archived user-owned experiences unless --archived is selected.
    List {
        /// Follow pagination, stopping at the shared development traffic budget.
        #[arg(long)]
        all: bool,

        /// Select archived experiences instead of non-archived experiences.
        #[arg(long)]
        archived: bool,

        /// Results per page. Not the maximum request count.
        #[arg(long, default_value = "100", value_parser = ["10", "25", "50", "100"])]
        limit: String,
    },
}
