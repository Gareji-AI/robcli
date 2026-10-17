use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "rob",
    version,
    about = "Roblox Creator Dashboard and public Creator Store CLI"
)]
#[command(
    after_help = "Offline by default. --live permits one explicitly approved bounded operation.\nCreator Store discovery is anonymous and never reads Firefox credentials. Dashboard operations use the selected Firefox session.\nAPI-key/OAuth authentication and cookie export are not supported. Key creation is plan-only unless --apply and account confirmation are supplied."
)]
pub(crate) struct Cli {
    /// Explicitly allow one bounded live probe. Never use in a test loop.
    #[arg(long, global = true)]
    pub live: bool,

    /// Dashboard only: select an existing Firefox profile by name, not a cookie value.
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
    /// Discover development assets in Creator Store, not avatar Marketplace.
    #[command(visible_aliases = ["marketplace", "creator-store"])]
    Store {
        #[command(subcommand)]
        command: StoreCommand,
    },
    /// Check the signed-in Firefox account through a read-only API.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    /// Show the signed-in Roblox account. Alias of auth check.
    Whoami,
    /// Inspect the signed-in user's groups and roles.
    Groups {
        #[command(subcommand)]
        command: GroupCommand,
    },
    /// Inspect personal or explicitly selected group-owned experiences.
    Experiences {
        #[command(subcommand)]
        command: ExperienceCommand,
    },
    /// Print Dashboard destinations without opening a browser or reading cookies.
    Dashboard {
        #[command(subcommand)]
        command: DashboardCommand,
    },
    /// Administer API-key metadata using the Dashboard session, not API-key auth.
    #[command(name = "apikey")]
    ApiKey {
        #[command(subcommand)]
        command: ApiKeyCommand,
    },
    /// Export one chart through the Dashboard's existing Download CSV action.
    Analytics {
        #[command(subcommand)]
        command: AnalyticsCommand,
    },
    /// Discover the supported commands and safety limits without reading cookies.
    Schema,
}

impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Store {
                command: StoreCommand::Search(_),
            } => "store search",
            Self::Store {
                command: StoreCommand::Get { .. },
            } => "store get",
            Self::Store {
                command: StoreCommand::Link { .. },
            } => "store link",
            Self::Auth { .. } => "auth check",
            Self::Whoami => "whoami",
            Self::Groups { .. } => "groups list",
            Self::Experiences {
                command: ExperienceCommand::List { .. },
            } => "experiences list",
            Self::Experiences {
                command: ExperienceCommand::Settings { .. },
            } => "experiences settings",
            Self::Dashboard { .. } => "dashboard links",
            Self::ApiKey {
                command: ApiKeyCommand::List { .. },
            } => "apikey list",
            Self::ApiKey {
                command: ApiKeyCommand::Create { .. },
            } => "apikey create",
            Self::Analytics { .. } => "analytics export",
            Self::Schema => "schema",
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum StoreCommand {
    /// One anonymous Creator Store search page, without detail fan-out.
    Search(Box<crate::store::Search>),
    /// Read public metadata for one Creator Store asset, never its source/media bytes.
    Get {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        asset: u64,
    },
    /// Offline Studio ID and Store/preview-page links; does not check existence.
    Link {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        asset: u64,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AnalyticsCommand {
    /// A single owned private Firefox page load and one CSV click; never an internal-query guess.
    Export {
        #[arg(long)]
        url: String,
        #[arg(long)]
        chart: String,
        #[arg(long)]
        output: PathBuf,
        /// Explicit group owner, independently verified even when Dashboard omits groupId.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        group: Option<u64>,
        /// Path to Mozilla geckodriver. Otherwise use the OS-local robcli tools directory or PATH.
        #[arg(long)]
        driver: Option<PathBuf>,
        /// Consent to normal browser page assets/XHR, outside the direct-API HTTP cap.
        #[arg(long)]
        allow_browser_page_load: bool,
        /// Open a visible private Firefox for one manual login/confirmation (5-minute limit).
        #[arg(long)]
        interactive_login: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ApiKeyCommand {
    /// Plan key issuance offline; --apply additionally requires --live and --confirm-user.
    Create {
        #[arg(long)]
        request: PathBuf,
        #[arg(long)]
        secret_name: String,
        #[arg(long)]
        apply: bool,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        confirm_user: Option<u64>,
        /// Explicitly permit one CSRF token-validation replay only.
        #[arg(long)]
        allow_csrf_handshake: bool,
    },
    /// List key metadata only. The Dashboard uses a read-only POST for this.
    List {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        group: Option<u64>,
        #[arg(long)]
        all: bool,
        /// Permit one 403 token-validation replay, never a challenge bypass.
        #[arg(long)]
        allow_csrf_handshake: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Verify account identity; does not export or persist the session cookie.
    Check,
}

#[derive(Debug, Subcommand)]
pub(crate) enum GroupCommand {
    /// List groups and the account's roles; membership is not an admin grant.
    List,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ExperienceCommand {
    /// Read settings only after the selected owner matches the experience.
    Settings {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        universe: u64,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        group: Option<u64>,
    },
    /// List non-archived user-owned experiences unless --archived is selected.
    List {
        /// Select a group explicitly instead of the personal collection.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        group: Option<u64>,
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

#[derive(Debug, Subcommand)]
pub(crate) enum DashboardCommand {
    Links {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        universe: u64,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        group: Option<u64>,
    },
}
