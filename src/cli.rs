use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "rob",
    version,
    about = "Roblox Dashboard, public profiles, communities, games, charts and catalogs"
)]
#[command(
    after_help = "Offline by default. --live permits one explicitly approved bounded operation.\nUsers, communities, charts, public games, Creator Store and avatar catalog reads are anonymous and never read Firefox credentials. Dashboard operations and the existing groups list use the selected Firefox session.\nPublic discovery performs no purchase, social write, member-roster harvest, media download or creator configuration change. Avatar commands do not apply or save appearance. API-key/OAuth authentication and cookie export are not supported. Key creation is plan-only unless --apply and account confirmation are supplied."
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
    /// Public profiles/search/creations and reported community memberships.
    #[command(visible_aliases = ["profiles", "profile"])]
    Users {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Public communities (Groups API), not authenticated group administration.
    #[command(visible_alias = "community")]
    Communities {
        #[command(subcommand)]
        command: CommunityCommand,
    },
    /// Public game charts and Music Top 100; anonymous one-page reads.
    Charts {
        #[command(subcommand)]
        command: ChartCommand,
    },
    /// Public experience snapshots, Top Songs, passes and developer products.
    #[command(visible_alias = "places")]
    Games {
        #[command(subcommand)]
        command: GameCommand,
    },
    /// Discover avatar clothing/accessories/bundles; never change the account avatar.
    #[command(visible_alias = "catalog")]
    Avatar {
        #[command(subcommand)]
        command: AvatarCommand,
    },
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
    /// Compare two saved success envelopes offline. No network and no credential lookup.
    Compare {
        #[arg(long)]
        before: PathBuf,
        #[arg(long)]
        after: PathBuf,
    },
    /// Render one saved success envelope to a new Markdown or CSV file.
    Report {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_enum)]
        format: ReportFormat,
    },
    /// Report whether Firefox and geckodriver exist. Paths and secrets are not returned.
    Doctor,
    /// Describe one command's request budget without executing it.
    Plan {
        /// Inner command arguments. Prefer `rob plan -- <command>`.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Discover the supported commands and safety limits without reading cookies.
    Schema,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum ReportFormat {
    Markdown,
    Csv,
}

impl Command {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Users {
                command: UserCommand::Get(_),
            } => "users get",
            Self::Users {
                command: UserCommand::Search(_),
            } => "users search",
            Self::Users {
                command: UserCommand::Communities { .. },
            } => "users communities",
            Self::Users {
                command: UserCommand::Games { .. },
            } => "users games",
            Self::Users {
                command: UserCommand::Link(_),
            } => "users link",
            Self::Communities {
                command: CommunityCommand::Get(_),
            } => "communities get",
            Self::Communities {
                command: CommunityCommand::Search(_),
            } => "communities search",
            Self::Communities {
                command: CommunityCommand::Roles { .. },
            } => "communities roles",
            Self::Communities {
                command: CommunityCommand::Games { .. },
            } => "communities games",
            Self::Communities {
                command: CommunityCommand::Link(_),
            } => "communities link",
            Self::Charts {
                command: ChartCommand::Sorts(_),
            } => "charts sorts",
            Self::Charts {
                command: ChartCommand::Games { .. },
            } => "charts games",
            Self::Charts {
                command: ChartCommand::Music(_),
            } => "charts music",
            Self::Games {
                command: GameCommand::Get(_),
            } => "games get",
            Self::Games {
                command: GameCommand::Music { .. },
            } => "games music",
            Self::Games {
                command: GameCommand::Passes { .. },
            } => "games passes",
            Self::Games {
                command: GameCommand::Products { .. },
            } => "games products",
            Self::Games {
                command: GameCommand::Icon { .. },
            } => "games icon",
            Self::Games {
                command: GameCommand::Thumbnails { .. },
            } => "games thumbnails",
            Self::Games {
                command: GameCommand::Badges { .. },
            } => "games badges",
            Self::Avatar {
                command: AvatarCommand::Search(_),
            } => "avatar search",
            Self::Avatar {
                command: AvatarCommand::Link(_),
            } => "avatar link",
            Self::Avatar {
                command: AvatarCommand::Bundle { .. },
            } => "avatar bundle",
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
            Self::Compare { .. } => "compare",
            Self::Report { .. } => "report",
            Self::Doctor => "doctor",
            Self::Plan { .. } => "plan",
            Self::Schema => "schema",
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum UserCommand {
    /// One public profile; does not identify the logged-in browser user.
    Get(crate::social::UserTarget),
    /// One keyword-search page; search is not exact identity verification.
    Search(Box<crate::social::Search>),
    /// Reported public memberships/roles. No member rosters or notification data.
    #[command(visible_alias = "groups")]
    Communities {
        #[command(flatten)]
        target: crate::social::UserTarget,
        /// Local cap for this unpaged API response, not a server page size.
        #[arg(long, default_value="25", value_parser=clap::value_parser!(u32).range(1..=25))]
        limit: u32,
    },
    /// One page of publicly returned creations, not private projects or player stats.
    Games {
        #[command(flatten)]
        target: crate::social::UserTarget,
        #[command(flatten)]
        page: crate::social::Page,
    },
    /// Offline profile URL only; does not verify existence or open a browser.
    Link(crate::social::UserTarget),
}

#[derive(Debug, Subcommand)]
pub(crate) enum CommunityCommand {
    /// Public description, owner, counter and available public shout.
    Get(crate::social::CommunityTarget),
    /// One community search page. Select an ID explicitly before further reads.
    Search(Box<crate::social::Search>),
    /// Public roles only (includePrivate=false), never permissions/member rosters.
    Roles {
        #[command(flatten)]
        target: crate::social::CommunityTarget,
        #[arg(long, default_value="25", value_parser=clap::value_parser!(u32).range(1..=25))]
        limit: u32,
    },
    /// One page of public community creations, never creator configuration.
    Games {
        #[command(flatten)]
        target: crate::social::CommunityTarget,
        #[command(flatten)]
        page: crate::social::Page,
    },
    /// Offline community URL only; does not verify existence or open a browser.
    Link(crate::social::CommunityTarget),
}

#[derive(Debug, Subcommand)]
pub(crate) enum ChartCommand {
    /// List the public chart sort IDs, not all games in every sort.
    Sorts(crate::discovery::Chart),
    /// One chart page; default Top Playing Now. No per-game detail enrichment.
    Games {
        #[command(flatten)]
        query: crate::discovery::Chart,
        #[arg(long, default_value = "top-playing-now")]
        sort: String,
        /// Local output cap; the public endpoint controls its own page size.
        #[arg(long, default_value="10", value_parser=clap::value_parser!(u32).range(1..=30))]
        limit: u32,
    },
    /// Official Music Top 100, not Creator Store trending search or all sound effects.
    Music(crate::discovery::Page),
}

#[derive(Debug, Subcommand)]
pub(crate) enum GameCommand {
    /// Public Universe snapshot and votes (2 reads; Place resolution adds 1).
    Get(crate::discovery::Target),
    /// Reported Top Songs over 7 days, not a complete BGM scan or live playback.
    Music {
        #[command(flatten)]
        target: crate::discovery::Target,
        #[arg(long, default_value="10", value_parser=clap::value_parser!(u32).range(1..=30))]
        limit: u32,
    },
    /// One public pass page; no purchase, ownership claim or creator configuration.
    Passes {
        #[command(flatten)]
        target: crate::discovery::Target,
        #[command(flatten)]
        page: crate::discovery::Page,
    },
    /// One public developer-product page, not private sales/revenue data.
    Products {
        #[command(flatten)]
        target: crate::discovery::Target,
        #[command(flatten)]
        page: crate::discovery::Page,
    },
    /// One anonymous game-icon metadata read. Image bytes are not downloaded.
    Icon {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        universe: u64,
        #[arg(long, default_value = "150x150")]
        size: String,
        #[arg(long, default_value = "Png")]
        format: String,
    },
    /// One anonymous thumbnail-list read. Image bytes are not downloaded.
    Thumbnails {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        universe: u64,
        #[arg(long, default_value = "768x432")]
        size: String,
        #[arg(long, default_value = "Png")]
        format: String,
        #[arg(long, default_value = "1", value_parser = clap::value_parser!(u32).range(1..=10))]
        count: u32,
    },
    /// One page of public badges for one Universe. No award or icon download.
    Badges {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        universe: u64,
        #[arg(long, default_value = "10", value_parser = ["10", "25", "50", "100"])]
        limit: String,
        #[arg(long, default_value = "Asc", value_parser = ["Asc", "Desc"])]
        sort: String,
        #[arg(long)]
        cursor: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AvatarCommand {
    /// One public avatar Marketplace page, not Creator Store development assets.
    Search(Box<crate::avatar::Search>),
    /// Offline catalog/try-on-page links, with an explicit ID namespace.
    Link(crate::avatar::Target),
    /// One public bundle-details read. Item IDs stay in their reported namespace.
    Bundle {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        bundle: u64,
    },
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
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
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
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        confirm_user: Option<u64>,
        /// Explicitly permit one CSRF token-validation replay only.
        #[arg(long)]
        allow_csrf_handshake: bool,
    },
    /// List key metadata only. The Dashboard uses a read-only POST for this.
    List {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
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
    /// List the signed-in account's groups and roles.
    ///
    /// The request always sends `includeLocked=false`,
    /// `includeNotificationPreferences=false`, and `discoveryType=0`.
    /// Locked communities are not requested. Notification preferences are not
    /// requested or projected. `discoveryType` 0 is the published default
    /// discovery set, not every community type and not an admin grant.
    List,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ExperienceCommand {
    /// Read settings only after the selected owner matches the experience.
    Settings {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        universe: u64,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        group: Option<u64>,
    },
    /// List non-archived user-owned experiences unless --archived is selected.
    List {
        /// Select a group explicitly instead of the personal collection.
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
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
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        universe: u64,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=9223372036854775807))]
        group: Option<u64>,
    },
}
