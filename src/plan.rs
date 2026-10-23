use serde_json::{Value, json};

use crate::cli::{
    AnalyticsCommand, ApiKeyCommand, AvatarCommand, ChartCommand, Cli, Command, CommunityCommand,
    ExperienceCommand, GameCommand, StoreCommand, UserCommand,
};
use crate::error::AppError;
use crate::traffic::{Clock, MAX_REQUESTS, SystemClock};

struct Budget {
    auth: &'static str,
    attempts: usize,
    place_extra: usize,
    routes: Vec<&'static str>,
    browser_pages: usize,
    csv_clicks: usize,
}

pub(crate) fn describe(cli: &Cli) -> Result<Value, AppError> {
    validate(cli)?;
    let budget = budget(cli)?;
    Ok(json!({
        "command": cli.command.label(),
        "authentication": budget.auth,
        "maximumDirectApiAttempts": budget.attempts,
        "placeResolutionExtraAttempts": budget.place_extra,
        "sharedRunCap": MAX_REQUESTS,
        "routes": budget.routes,
        "browserPageLoads": budget.browser_pages,
        "csvClicks": budget.csv_clicks,
        "browserAssetsOutsideDirectCap": budget.browser_pages > 0,
        "requiredGates": required_gates(cli),
        "liveFlagPresent": cli.live,
        "applyPresent": apply_present(cli),
        "executes": false,
        "trafficLockCreated": false,
        "remainingBudgetGuaranteed": false,
        "accountPermissionsVerified": false,
        "notes": [
            "This is a request plan, not an execution.",
            "Direct API attempts share the five-request run cap and two-second spacing.",
            "Normal browser asset and XHR traffic is outside that cap.",
            "The plan does not prove the current account's permissions or the remaining traffic budget."
        ]
    }))
}

fn validate(cli: &Cli) -> Result<(), AppError> {
    if cli.profile.is_some()
        && matches!(
            cli.command,
            Command::Users { .. }
                | Command::Communities { .. }
                | Command::Charts { .. }
                | Command::Games { .. }
                | Command::Avatar { .. }
                | Command::Store { .. }
        )
    {
        return Err(AppError::new(
            "plan_anonymous_only",
            "This public command does not accept a Firefox profile.",
            "Omit --profile. The plan was not executed.",
        ));
    }
    match &cli.command {
        Command::Users {
            command: UserCommand::Search(query),
        }
        | Command::Communities {
            command: CommunityCommand::Search(query),
        } => query.validate(),
        Command::Users {
            command: UserCommand::Get(target) | UserCommand::Communities { target, .. },
        } => crate::social::validate_target(target.user),
        Command::Users {
            command: UserCommand::Games { target, page },
        } => {
            crate::social::validate_target(target.user)?;
            page.validate()
        }
        Command::Communities {
            command: CommunityCommand::Get(target) | CommunityCommand::Roles { target, .. },
        } => crate::social::validate_target(target.community),
        Command::Communities {
            command: CommunityCommand::Games { target, page },
        } => {
            crate::social::validate_target(target.community)?;
            page.validate()
        }
        Command::Users {
            command: UserCommand::Link(target),
        } => crate::social::validate_target(target.user),
        Command::Communities {
            command: CommunityCommand::Link(target),
        } => crate::social::validate_target(target.community),
        Command::Charts {
            command: ChartCommand::Sorts(chart),
        } => {
            chart.url(None)?;
            Ok(())
        }
        Command::Charts {
            command: ChartCommand::Games { query, sort, .. },
        } => {
            query.url(Some(sort.as_str()))?;
            Ok(())
        }
        Command::Charts {
            command: ChartCommand::Music(page),
        } => page.validate(),
        Command::Games {
            command: GameCommand::Get(target) | GameCommand::Music { target, .. },
        } => target.validate(),
        Command::Games {
            command: GameCommand::Passes { target, page } | GameCommand::Products { target, page },
        } => {
            target.validate()?;
            page.validate()
        }
        Command::Games {
            command:
                GameCommand::Icon {
                    universe,
                    size,
                    format,
                },
        } => {
            crate::discovery::icon_url(*universe, size, format)?;
            Ok(())
        }
        Command::Games {
            command:
                GameCommand::Thumbnails {
                    universe,
                    size,
                    format,
                    count,
                },
        } => {
            crate::discovery::thumbnail_url(*universe, size, format, *count)?;
            Ok(())
        }
        Command::Games {
            command:
                GameCommand::Badges {
                    universe,
                    limit,
                    sort,
                    cursor,
                },
        } => {
            crate::discovery::badges_url(*universe, limit, sort, cursor.as_deref())?;
            Ok(())
        }
        Command::Avatar {
            command: AvatarCommand::Search(query),
        } => {
            query.url()?;
            Ok(())
        }
        Command::Avatar {
            command: AvatarCommand::Link(target),
        } => {
            target.selection()?;
            Ok(())
        }
        Command::Avatar {
            command: AvatarCommand::Bundle { bundle },
        } => {
            if *bundle == 0 || *bundle > i64::MAX as u64 {
                return Err(AppError::new(
                    "avatar_query_invalid",
                    "The avatar catalog query or typed target is invalid.",
                    "Use one signed 64-bit Bundle ID. No credentials were accessed.",
                ));
            }
            Ok(())
        }
        Command::Store {
            command: StoreCommand::Search(query),
        } => {
            query.url()?;
            Ok(())
        }
        Command::Store {
            command: StoreCommand::Get { asset } | StoreCommand::Link { asset },
        } => {
            crate::store::links(*asset)?;
            Ok(())
        }
        Command::Analytics {
            command:
                AnalyticsCommand::Export {
                    url,
                    chart,
                    output,
                    group,
                    ..
                },
        } => {
            crate::browser::ExportOptions::validate(url, chart, output)?.with_group(*group)?;
            Ok(())
        }
        Command::ApiKey {
            command:
                ApiKeyCommand::Create {
                    request,
                    secret_name,
                    apply,
                    confirm_user,
                    ..
                },
        } => {
            crate::keys::SecretFile::validate_name(secret_name)?;
            let _ = crate::keys::KeySpec::load(request, SystemClock.now_ms()? / 1000)?;
            if *apply && confirm_user.is_none() {
                return Err(AppError::new(
                    "account_confirmation_required",
                    "Key issuance requires an explicit --confirm-user ID.",
                    "Review the offline plan and identify the intended account first. No credential lookup or request was made.",
                ));
            }
            Ok(())
        }
        Command::Plan { .. }
        | Command::Doctor
        | Command::Compare { .. }
        | Command::Report { .. }
        | Command::Schema => Err(AppError::new(
            "plan_target_unsupported",
            "Plan does not wrap doctor, compare, report, schema, or another plan.",
            "Run those offline commands directly. No request was sent.",
        )),
        Command::Auth { .. }
        | Command::Whoami
        | Command::Groups { .. }
        | Command::Experiences { .. }
        | Command::Dashboard { .. }
        | Command::ApiKey {
            command: ApiKeyCommand::List { .. },
        } => Ok(()),
    }
}

fn budget(cli: &Cli) -> Result<Budget, AppError> {
    let anonymous = "anonymous";
    let session = "firefox-session";
    let none = "none";
    let (auth, attempts, place_extra, routes, browser_pages, csv_clicks) = match &cli.command {
        Command::Games {
            command: GameCommand::Get(target),
        } => (
            anonymous,
            if target.place.is_some() { 3 } else { 2 },
            usize::from(target.place.is_some()),
            vec![
                "GET games.roblox.com/v1/games",
                "GET games.roblox.com/v1/games/votes",
            ],
            0,
            0,
        ),
        Command::Games {
            command: GameCommand::Music { target, .. },
        } => (
            anonymous,
            if target.place.is_some() { 2 } else { 1 },
            usize::from(target.place.is_some()),
            vec!["GET apis.roblox.com/music-discovery/v1/experience-songs"],
            0,
            0,
        ),
        Command::Games {
            command: GameCommand::Passes { target, .. } | GameCommand::Products { target, .. },
        } => (
            anonymous,
            if target.place.is_some() { 2 } else { 1 },
            usize::from(target.place.is_some()),
            vec!["GET apis.roblox.com game-pass or developer-product page"],
            0,
            0,
        ),
        Command::Games {
            command: GameCommand::Icon { .. },
        } => (
            anonymous,
            1,
            0,
            vec!["GET thumbnails.roblox.com/v1/games/icons"],
            0,
            0,
        ),
        Command::Games {
            command: GameCommand::Thumbnails { .. },
        } => (
            anonymous,
            1,
            0,
            vec!["GET thumbnails.roblox.com/v1/games/multiget/thumbnails"],
            0,
            0,
        ),
        Command::Games {
            command: GameCommand::Badges { .. },
        } => (
            anonymous,
            1,
            0,
            vec!["GET badges.roblox.com/v1/universes/{universeId}/badges"],
            0,
            0,
        ),
        Command::Charts { .. }
        | Command::Avatar {
            command: AvatarCommand::Search(_),
        }
        | Command::Store {
            command: StoreCommand::Search(_) | StoreCommand::Get { .. },
        }
        | Command::Users {
            command:
                UserCommand::Get(_)
                | UserCommand::Search(_)
                | UserCommand::Communities { .. }
                | UserCommand::Games { .. },
        }
        | Command::Communities {
            command:
                CommunityCommand::Get(_)
                | CommunityCommand::Search(_)
                | CommunityCommand::Roles { .. }
                | CommunityCommand::Games { .. },
        } => (anonymous, 1, 0, vec!["one allowlisted anonymous GET"], 0, 0),
        Command::Avatar {
            command: AvatarCommand::Bundle { .. },
        } => (
            anonymous,
            1,
            0,
            vec!["GET catalog.roblox.com/v1/bundles/{bundleId}/details"],
            0,
            0,
        ),
        Command::Auth { .. } | Command::Whoami => (
            session,
            1,
            0,
            vec!["GET users.roblox.com/v1/users/authenticated"],
            0,
            0,
        ),
        Command::Groups { .. } => (
            session,
            2,
            0,
            vec![
                "GET users.roblox.com/v1/users/authenticated",
                "GET groups.roblox.com/v2/users/{userId}/groups/roles",
            ],
            0,
            0,
        ),
        Command::Experiences {
            command: ExperienceCommand::Settings { .. },
        } => (
            session,
            3,
            0,
            vec![
                "GET users.roblox.com/v1/users/authenticated",
                "GET develop.roblox.com/v1/universes/{universeId}",
                "GET develop.roblox.com/v1/universes/{universeId}/configuration",
            ],
            0,
            0,
        ),
        Command::Experiences {
            command: ExperienceCommand::List { all, .. },
        } => (
            session,
            if *all { MAX_REQUESTS } else { 2 },
            0,
            vec!["GET develop.roblox.com universe list"],
            0,
            0,
        ),
        Command::ApiKey {
            command:
                ApiKeyCommand::List {
                    all,
                    allow_csrf_handshake,
                    ..
                },
        } => (
            session,
            // With --all the listing keeps POSTing while nextCursor is
            // nonempty, so only the shared run cap is a truthful maximum.
            if *all {
                MAX_REQUESTS
            } else {
                2 + usize::from(*allow_csrf_handshake)
            },
            0,
            vec![
                "GET users.roblox.com/v1/users/authenticated",
                "POST apis.roblox.com/cloud-authentication/v1/apiKeys",
            ],
            0,
            0,
        ),
        Command::ApiKey {
            command:
                ApiKeyCommand::Create {
                    apply,
                    allow_csrf_handshake,
                    ..
                },
        } => (
            if *apply { session } else { none },
            if *apply {
                2 + usize::from(*allow_csrf_handshake)
            } else {
                0
            },
            0,
            if *apply {
                vec![
                    "GET users.roblox.com/v1/users/authenticated",
                    "POST apis.roblox.com/cloud-authentication/v1/apiKey",
                ]
            } else {
                vec![]
            },
            0,
            0,
        ),
        Command::Analytics {
            command: AnalyticsCommand::Export {
                interactive_login, ..
            },
        } => (
            session,
            if *interactive_login { 3 } else { 2 },
            0,
            vec![
                "GET users.roblox.com/v1/users/authenticated",
                "GET develop.roblox.com/v1/universes/{universeId}",
            ],
            1,
            1,
        ),
        Command::Users {
            command: UserCommand::Link(_),
        }
        | Command::Communities {
            command: CommunityCommand::Link(_),
        }
        | Command::Avatar {
            command: AvatarCommand::Link(_),
        }
        | Command::Store {
            command: StoreCommand::Link { .. },
        }
        | Command::Dashboard { .. } => (none, 0, 0, vec![], 0, 0),
        Command::Plan { .. }
        | Command::Doctor
        | Command::Compare { .. }
        | Command::Report { .. }
        | Command::Schema => {
            return Err(AppError::new(
                "plan_target_unsupported",
                "Plan does not wrap doctor, compare, report, schema, or another plan.",
                "Run those offline commands directly. No request was sent.",
            ));
        }
    };
    Ok(Budget {
        auth,
        attempts,
        place_extra,
        routes,
        browser_pages,
        csv_clicks,
    })
}

fn required_gates(cli: &Cli) -> Vec<&'static str> {
    match &cli.command {
        Command::Users {
            command: UserCommand::Link(_),
        }
        | Command::Communities {
            command: CommunityCommand::Link(_),
        }
        | Command::Avatar {
            command: AvatarCommand::Link(_),
        }
        | Command::Store {
            command: StoreCommand::Link { .. },
        }
        | Command::Dashboard { .. }
        | Command::ApiKey {
            command: ApiKeyCommand::Create { apply: false, .. },
        } => vec![],
        Command::ApiKey {
            command: ApiKeyCommand::Create { apply: true, .. },
        } => {
            vec!["--apply", "--live", "--confirm-user"]
        }
        Command::Analytics { .. } => vec!["--live", "--allow-browser-page-load"],
        _ => vec!["--live"],
    }
}

fn apply_present(cli: &Cli) -> bool {
    matches!(
        cli.command,
        Command::ApiKey {
            command: ApiKeyCommand::Create { apply: true, .. },
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    fn plan(args: &[&str]) -> Value {
        let cli = Cli::try_parse_from(args).unwrap();
        describe(&cli).unwrap()
    }

    #[test]
    fn direct_attempt_counts_match_the_execution_paths() {
        assert_eq!(
            plan(&["rob", "games", "get", "--universe", "77"])["maximumDirectApiAttempts"],
            2
        );
        assert_eq!(
            plan(&["rob", "games", "get", "--place", "88"])["maximumDirectApiAttempts"],
            3
        );
        assert_eq!(
            plan(&["rob", "games", "get", "--place", "88"])["placeResolutionExtraAttempts"],
            1
        );
        assert_eq!(
            plan(&["rob", "games", "passes", "--universe", "77"])["maximumDirectApiAttempts"],
            1
        );
        assert_eq!(
            plan(&["rob", "experiences", "settings", "--universe", "77"])["maximumDirectApiAttempts"],
            3
        );
        assert_eq!(
            plan(&["rob", "experiences", "list", "--all"])["maximumDirectApiAttempts"],
            5
        );
        assert_eq!(
            plan(&["rob", "experiences", "list"])["maximumDirectApiAttempts"],
            2
        );
        assert_eq!(plan(&["rob", "--live", "whoami"])["executes"], false);
        assert_eq!(
            plan(&["rob", "--live", "whoami"])["trafficLockCreated"],
            false
        );
        assert_eq!(
            plan(&["rob", "games", "icon", "--universe", "77"])["routes"][0],
            "GET thumbnails.roblox.com/v1/games/icons"
        );
        assert_eq!(
            plan(&["rob", "avatar", "bundle", "--bundle", "77"])["maximumDirectApiAttempts"],
            1
        );
    }

    #[test]
    fn apikey_list_plan_matches_the_paginated_execution_path() {
        // `apikey list` first verifies the identity, then POSTs list pages.
        // One page uses two attempts, or three with the CSRF replay gate.
        assert_eq!(
            plan(&["rob", "apikey", "list"])["maximumDirectApiAttempts"],
            2
        );
        assert_eq!(
            plan(&["rob", "apikey", "list", "--allow-csrf-handshake"])["maximumDirectApiAttempts"],
            3
        );
        // `--all` keeps POSTing while nextCursor is nonempty, so only the
        // shared run cap is a truthful maximum -- with or without the replay.
        assert_eq!(
            plan(&["rob", "apikey", "list", "--all"])["maximumDirectApiAttempts"],
            5
        );
        assert_eq!(
            plan(&["rob", "apikey", "list", "--all", "--allow-csrf-handshake"])["maximumDirectApiAttempts"],
            5
        );
    }

    #[test]
    fn key_creation_without_apply_plans_zero_attempts() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("request.json");
        let later = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86_400;
        let expiry = chrono::DateTime::from_timestamp(i64::try_from(later).unwrap(), 0)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        std::fs::write(
            &path,
            format!(
                r#"{{"name":"read","expirationTime":"{expiry}","allowedCidrs":["203.0.113.7/32"],"scopes":[{{"scopeType":"universe-analytics","targetParts":["777"],"operations":["universe.analytics:read"]}}]}}"#
            ),
        )
        .unwrap();
        let cli = Cli::try_parse_from([
            "rob",
            "apikey",
            "create",
            "--request",
            path.to_str().unwrap(),
            "--secret-name",
            "example",
        ])
        .unwrap();
        let value = describe(&cli).unwrap();
        assert_eq!(value["maximumDirectApiAttempts"], 0);
        assert_eq!(value["authentication"], "none");
        assert_eq!(value["trafficLockCreated"], false);
    }
}
