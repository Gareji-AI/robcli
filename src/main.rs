mod api;
mod auth;
mod cli;
mod error;
mod traffic;
mod transport;

use std::io::Write;
use std::process::ExitCode;

use clap::{Parser, error::ErrorKind};
use serde_json::{Value, json};

use crate::api::{Api, ListOptions};
use crate::cli::{Cli, Command, ExperienceCommand};
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
    if matches!(cli.command, Command::Schema) {
        return Ok(success(
            command,
            "Supported read-only commands; no credentials were accessed",
            json!({
                "project": "robcli", "binary": "rob", "version": env!("CARGO_PKG_VERSION"),
                "commands": ["auth check", "whoami", "experiences list", "schema"],
                "authentication": { "browser": "firefox", "apiKey": false, "oauth": false, "cookieExport": false },
                "safety": { "offlineByDefault": true, "liveFlagRequired": true, "readOnly": true,
                    "maximumRequestsPerRun": traffic::MAX_REQUESTS, "maximumRequestsPerMinute": traffic::MAX_REQUESTS,
                    "minimumRequestIntervalMs": traffic::MIN_INTERVAL_MS, "automaticRetries": false,
                    "parallelLiveProbes": 1, "redirects": false, "environmentProxies": false },
                "scope": "Personal experiences only; no group-owned listing or write operations."
            }),
        ));
    }
    if !cli.live {
        return Err(AppError::new(
            "live_not_enabled",
            "Offline by default: no browser credentials or Roblox APIs were accessed.",
            "Use --live only for one explicitly approved, bounded read-only probe. Use rob schema for offline discovery.",
        ));
    }
    let locations = auth::Locations::discover()?;
    let mut guard = TrafficGuard::acquire(&locations.state, SystemClock)?;
    let session = auth::read_session(
        &locations.firefox,
        cli.profile.as_deref(),
        SystemClock.now_ms()? / 1000,
    )?;
    let mut api = Api::new(transport::HttpsTransport::new()?, session.cookie);
    let user = api.whoami(&mut guard)?;
    let mut data = json!({
        "browser": "firefox", "profile": session.profile, "readOnly": true,
        "credentialSaved": false, "user": user
    });
    if let Command::Experiences {
        command:
            ExperienceCommand::List {
                all,
                archived,
                limit,
            },
    } = cli.command
    {
        let collection = api.experiences(
            ListOptions {
                all,
                archived,
                limit: &limit,
            },
            &mut guard,
        )?;
        data["collection"] = serde_json::to_value(collection).map_err(|_| {
            AppError::new(
                "output_failed",
                "Cannot serialize the experience listing.",
                "Stop this probe; do not automatically rerun it.",
            )
        })?;
        return Ok(success(
            command,
            "Read the selected personal experience collection",
            data,
        ));
    }
    Ok(success(
        command,
        "Verified the signed-in Firefox Roblox account",
        data,
    ))
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
