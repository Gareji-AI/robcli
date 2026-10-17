mod api;
mod auth;
mod browser;
mod cli;
mod dashboard;
mod error;
mod keys;
mod traffic;
mod transport;

use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, error::ErrorKind};
use serde_json::{Value, json};

use crate::api::{Api, ListOptions};
use crate::cli::{
    AnalyticsCommand, ApiKeyCommand, Cli, Command, DashboardCommand, ExperienceCommand,
};
use crate::error::AppError;
use crate::traffic::{Clock, SystemClock, TrafficGuard};

fn success(command: &str, summary: &str, data: Value) -> Value {
    json!({ "ok": true, "command": command, "summary": summary, "data": data, "hints": [] })
}

fn failure(command: &str, error: &AppError) -> Value {
    json!({ "ok": false, "command": command, "summary": error.message, "error": error, "hints": [error.hint] })
}

fn run(cli: Cli) -> Result<Value, AppError> {
    let command = cli.command.label();
    let export_options = if let Command::Analytics {
        command:
            AnalyticsCommand::Export {
                url,
                chart,
                output,
                interactive_login,
                group,
                ..
            },
    } = &cli.command
    {
        Some(
            browser::ExportOptions::validate(url, chart, output)?
                .with_group(*group)?
                .with_interactive_login(*interactive_login),
        )
    } else {
        None
    };
    let prepared_key = if let Command::ApiKey {
        command:
            ApiKeyCommand::Create {
                request,
                secret_name,
                apply,
                confirm_user,
                ..
            },
    } = &cli.command
    {
        keys::SecretFile::validate_name(secret_name)?;
        let spec = keys::KeySpec::load(request, SystemClock.now_ms()? / 1000)?;
        if !apply {
            return Ok(success(
                command,
                "Offline key-creation plan; nothing was issued or saved",
                spec.plan(secret_name),
            ));
        }
        if confirm_user.is_none() {
            return Err(AppError::new(
                "account_confirmation_required",
                "Key issuance requires an explicit --confirm-user ID.",
                "Review the offline plan and identify the intended account first. No credential lookup or request was made.",
            ));
        }
        Some(spec)
    } else {
        None
    };
    if let Command::Dashboard {
        command: DashboardCommand::Links { universe, group },
    } = cli.command
    {
        return Ok(success(
            command,
            "Dashboard links only; no browser was opened",
            dashboard::links(universe, group),
        ));
    }
    if matches!(cli.command, Command::Schema) {
        return Ok(success(
            command,
            "Command schema and safety limits; no credentials were accessed",
            json!({
                "project": "robcli", "binary": "rob", "version": env!("CARGO_PKG_VERSION"),
                "commands": ["auth check", "whoami", "groups list", "experiences list", "experiences settings", "dashboard links", "analytics export", "apikey list", "apikey create", "schema"],
                "authentication": { "browser": "firefox", "apiKey": false, "oauth": false, "cookieExport": false },
                "safety": { "offlineByDefault": true, "liveFlagRequired": true, "readOnlyByDefault": true,
                    "maximumRequestsPerRun": traffic::MAX_REQUESTS, "maximumRequestsPerMinute": traffic::MAX_REQUESTS,
                    "minimumRequestIntervalMs": traffic::MIN_INTERVAL_MS, "automaticRetries": false,
                    "parallelLiveProbes": 1, "redirects": false, "environmentProxies": false,
                    "keyCreationRequires": ["--apply", "--live", "--confirm-user"],
                    "csrfReplay": "one token-validation replay only with explicit --allow-csrf-handshake; never 2FA/challenge bypass",
                    "browserExport": { "requires": ["--live", "--allow-browser-page-load"], "driver": "Mozilla geckodriver / private Firefox / Windows Job Object",
                        "limits": "one analytics page load, one menu click, one CSV click, no reload or re-click",
                        "normalBrowserRequestsAreOutsideDirectApiCap": true,
                        "interactiveLogin": { "flag": "--interactive-login", "timeoutSeconds": 300,
                            "humanAuthenticationOnly": true, "sameAccountReverified": true, "exactTargetRechecked": true,
                            "credentialsSaved": false, "existingFirefoxAttached": false } } },
                "scope": "Explicit personal/group selection. Key issuance uses session authentication and local secret files, never API-key authentication."
            }),
        ));
    }
    if !cli.live {
        return Err(AppError::new(
            "live_not_enabled",
            "Offline by default: no browser credentials or Roblox APIs were accessed.",
            "Use --live only for one explicitly approved bounded operation. Use rob schema for offline discovery.",
        ));
    }
    if let Command::Analytics {
        command:
            AnalyticsCommand::Export {
                allow_browser_page_load: false,
                ..
            },
    } = &cli.command
    {
        return Err(AppError::new(
            "browser_navigation_not_allowed",
            "CSV export requires explicit consent to normal browser page-load traffic.",
            "Review the browser-export limits and add --allow-browser-page-load only for one authorized export. No browser, credential lookup, or API request was started.",
        ));
    }
    let locations = auth::Locations::discover()?;
    let browser_driver = if let Command::Analytics {
        command: AnalyticsCommand::Export { driver, .. },
    } = &cli.command
    {
        Some(browser::resolve_driver(
            driver.as_deref(),
            &locations.state,
        )?)
    } else {
        None
    };
    let mut guard = TrafficGuard::acquire(&locations.state, SystemClock)?;
    let mut secret_file = if let Command::ApiKey {
        command: ApiKeyCommand::Create { secret_name, .. },
    } = &cli.command
    {
        let root = locations
            .state
            .parent()
            .ok_or_else(|| {
                AppError::new(
                    "secret_store_failed",
                    "Cannot locate the OS-local key directory.",
                    "Stop before submitting key creation.",
                )
            })?
            .join("keys");
        Some(keys::SecretFile::reserve(&root, secret_name)?)
    } else {
        None
    };
    let session = auth::read_session(
        &locations.firefox,
        cli.profile.as_deref(),
        SystemClock.now_ms()? / 1000,
    )?;
    let mut api = Api::new(transport::HttpsTransport::new()?, session.cookie);
    let user = api.whoami(&mut guard)?;
    let user_id = user.id;
    let mut data = json!({
        "browser": "firefox", "profile": session.profile, "readOnly": true,
        "sessionCookieSaved": false, "user": user
    });
    if let Some(options) = export_options {
        let cookie = api.into_cookie();
        data["csv"] = browser::export(
            &options,
            cookie,
            browser_driver.as_ref().unwrap(),
            user_id,
            &mut guard,
        )?;
        return Ok(success(
            command,
            "Saved one chart through Download CSV; no query API payload was guessed",
            data,
        ));
    }
    if let Command::ApiKey {
        command:
            ApiKeyCommand::Create {
                confirm_user,
                allow_csrf_handshake,
                ..
            },
    } = &cli.command
    {
        if *confirm_user != Some(user_id) {
            return Err(AppError::new(
                "account_confirmation_mismatch",
                "The Firefox session is not the explicitly confirmed account.",
                "Stop and select the intended Firefox profile. No key creation request was submitted.",
            ));
        }
        let spec = prepared_key.as_ref().unwrap();
        let sink = secret_file.as_mut().unwrap();
        let created: keys::Created = match api.key_post(
            true,
            spec.body(),
            *allow_csrf_handshake,
            &mut guard,
        ) {
            Ok(created) => created,
            Err(mut error) => {
                if error.code == "creation_outcome_unknown" {
                    sink.retain_for_reconciliation();
                    error.details = Some(
                        json!({ "secretFile": sink.path().to_string_lossy(), "outcome": "unknown" }),
                    );
                }
                return Err(error);
            }
        };
        let mut receipt = sink.persist(&created).map_err(|mut error| {
            error.details = Some(json!({ "keyId": created.id, "secretFile": sink.path().to_string_lossy(), "outcome": "created_secret_store_failed" }));
            error
        })?;
        receipt["serverExpiryConfirmed"] = spec
            .expiry_confirmed(created.expiration_time.as_deref())
            .into();
        receipt["requestedEnabled"] = spec.requested_enabled().into();
        receipt["note"] = "Verify owner, scopes, enable state, CIDRs, and expiry in the Dashboard. Missing server expiry is not automatically patched.".into();
        data["readOnly"] = false.into();
        data["key"] = receipt;
        return Ok(success(
            command,
            "Created one key and saved its secret outside Git; no raw secret is printed",
            data,
        ));
    }
    if matches!(cli.command, Command::Groups { .. }) {
        data["groups"] = serde_json::to_value(api.groups(user_id, &mut guard)?)
            .map_err(|_| AppError::network("output_failed", "Cannot serialize groups."))?;
        return Ok(success(
            command,
            "Read groups and roles; membership does not imply admin permissions",
            data,
        ));
    }
    if let Command::ApiKey {
        command:
            ApiKeyCommand::List {
                group,
                all,
                allow_csrf_handshake,
            },
    } = cli.command
    {
        data["collection"] = api.keys(group, all, allow_csrf_handshake, &mut guard)?;
        return Ok(success(
            command,
            "Read API-key metadata without secrets or previews",
            data,
        ));
    }
    if let Command::Experiences {
        command: ExperienceCommand::Settings { universe, group },
    } = cli.command
    {
        data["experience"] = api.settings(universe, user_id, group, &mut guard)?;
        return Ok(success(
            command,
            "Read the explicitly selected experience settings",
            data,
        ));
    }
    if let Command::Experiences {
        command:
            ExperienceCommand::List {
                all,
                archived,
                limit,
                group,
            },
    } = cli.command
    {
        let options = ListOptions {
            all,
            archived,
            limit: &limit,
        };
        let collection = match group {
            Some(id) => api.group_experiences(id, options, &mut guard)?,
            None => api.experiences(options, &mut guard)?,
        };
        data["collection"] = serde_json::to_value(collection).map_err(|_| {
            AppError::new(
                "output_failed",
                "Cannot serialize the experience listing.",
                "Stop this probe; do not automatically rerun it.",
            )
        })?;
        return Ok(success(
            command,
            "Read the selected personal or group experience collection",
            data,
        ));
    }
    match cli.command {
        Command::Auth { .. } | Command::Whoami => Ok(success(
            command,
            "Verified the signed-in Firefox Roblox account",
            data,
        )),
        _ => Err(AppError::new(
            "command_not_dispatched",
            "No handler completed the requested command.",
            "Stop; no success is claimed for an unhandled command.",
        )),
    }
}

fn emit(value: &Value) -> bool {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, value).is_ok() && writeln!(stdout).is_ok()
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            return if error.print().is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            };
        }
        Err(_) => {
            // Clap's raw diagnostics can echo supplied argument values. Never print them.
            let mut error = AppError::new(
                "invalid_arguments",
                "Invalid arguments.",
                "Run rob --help. API keys and raw cookies are not accepted as arguments.",
            );
            error.exit_code = 64;
            emit(&failure("parse", &error));
            return ExitCode::from(64);
        }
    };
    let command = cli.command.label();
    match run(cli) {
        Ok(output) => {
            if emit(&output) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            emit(&failure(command, &error));
            ExitCode::from(error.exit_code)
        }
    }
}
