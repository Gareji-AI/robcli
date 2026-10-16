use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rob"))
        .args(args)
        // Even a regression must not discover the developer's real browser.
        .env("APPDATA", root)
        .env("LOCALAPPDATA", root)
        .env("HOME", root)
        .env("XDG_STATE_HOME", root)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn help_version_and_schema_do_not_read_browser_or_create_state() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &["--help"][..],
        &["--version"],
        &["experiences", "list", "--help"],
        &["schema", "--json"],
    ] {
        let output = run(root.path(), args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let schema = json(&run(root.path(), &["schema"]));
    assert_eq!(schema["data"]["project"], "robcli");
    assert_eq!(schema["data"]["safety"]["offlineByDefault"], true);
    assert_eq!(schema["data"]["safety"]["maximumRequestsPerRun"], 5);
    assert_eq!(schema["data"]["safety"]["minimumRequestIntervalMs"], 2000);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn all_account_commands_are_offline_unless_live_is_explicit() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &["whoami"][..],
        &["auth", "check"],
        &["experiences", "list", "--all"],
    ] {
        let output = run(root.path(), args);
        assert_eq!(output.status.code(), Some(2));
        assert_eq!(json(&output)["error"]["code"], "live_not_enabled");
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn unsupported_writes_keys_and_cookie_arguments_do_not_echo_values() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &["publish", "--live"][..],
        &["--api-key", "synthetic-do-not-echo", "whoami"],
        &["--cookie", "synthetic-do-not-echo", "whoami"],
        &["experiences", "list", "--limit", "synthetic-do-not-echo"],
    ] {
        let output = run(root.path(), args);
        assert_eq!(output.status.code(), Some(64));
        assert_eq!(json(&output)["error"]["code"], "invalid_arguments");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-do-not-echo"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-do-not-echo"));
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn live_with_a_fake_profile_root_stops_before_transport() {
    let root = tempfile::tempdir().unwrap();
    let output = run(root.path(), &["whoami", "--live"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "firefox_profile_ambiguous");
    for entry in walk(root.path()) {
        assert_ne!(entry.file_name().unwrap(), "traffic.json");
        assert_ne!(entry.file_name().unwrap(), "live.lock");
    }
}

fn state_root(root: &Path) -> std::path::PathBuf {
    #[cfg(target_os = "windows")]
    {
        root.join("robcli/traffic")
    }
    #[cfg(target_os = "macos")]
    {
        root.join("Library/Application Support/robcli/traffic")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        root.join("robcli/traffic")
    }
}

#[test]
fn another_process_cannot_replace_an_existing_live_lock() {
    let root = tempfile::tempdir().unwrap();
    let state = state_root(root.path());
    std::fs::create_dir_all(&state).unwrap();
    let lock_path = state.join("live.lock");
    std::fs::write(&lock_path, "existing-owner-sentinel").unwrap();
    let held = std::fs::File::open(&lock_path).unwrap();
    let output = run(root.path(), &["whoami", "--live"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "live_probe_busy");
    assert_eq!(
        std::fs::read_to_string(&lock_path).unwrap(),
        "existing-owner-sentinel"
    );
    assert!(!state.join("traffic.json").exists());
    drop(held);
}

#[test]
fn a_new_process_cannot_bypass_the_shared_minute_budget() {
    let root = tempfile::tempdir().unwrap();
    let state = state_root(root.path());
    std::fs::create_dir_all(&state).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let original = serde_json::to_vec(
        &serde_json::json!({ "version": 1, "starts": [now, now, now, now, now] }),
    )
    .unwrap();
    std::fs::write(state.join("traffic.json"), &original).unwrap();
    let output = run(root.path(), &["whoami", "--live"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "traffic_budget_exhausted");
    assert_eq!(std::fs::read(state.join("traffic.json")).unwrap(), original);
    assert!(!state.join("live.lock").exists());
}

#[test]
fn environment_api_keys_and_raw_cookie_values_are_not_authentication_sources() {
    let root = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rob"))
        .args(["whoami", "--live"])
        .env("APPDATA", root.path())
        .env("LOCALAPPDATA", root.path())
        .env("HOME", root.path())
        .env("XDG_STATE_HOME", root.path())
        .env("RBXOC_API_KEY", "synthetic-do-not-echo")
        .env("RBX_API_KEY", "synthetic-do-not-echo")
        .env("RBX_COOKIE", "synthetic-do-not-echo")
        .env("ROBLOSECURITY", "synthetic-do-not-echo")
        .output()
        .unwrap();
    assert_eq!(json(&output)["error"]["code"], "firefox_profile_ambiguous");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("synthetic-do-not-echo"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("synthetic-do-not-echo"));
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut output = Vec::new();
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            output.extend(walk(&path));
        }
        output.push(path);
    }
    output
}
