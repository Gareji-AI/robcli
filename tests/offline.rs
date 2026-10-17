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
fn group_listing_is_available_but_offline_without_explicit_permission() {
    let root = tempfile::tempdir().unwrap();
    let output = run(root.path(), &["groups", "list"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "live_not_enabled");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn public_discovery_commands_aliases_and_schema_work_offline_without_firefox() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        &["charts", "sorts"][..],
        &["charts", "games"],
        &["charts", "music"],
        &["games", "get", "--universe", "77"],
        &["places", "get", "--place", "88"],
        &["games", "music", "--universe", "77"],
        &["games", "passes", "--universe", "77"],
        &["games", "products", "--place", "88"],
    ] {
        let out = run(root.path(), args);
        assert_eq!(
            json(&out)["error"]["code"],
            "live_not_enabled",
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    let schema = json(&run(root.path(), &["schema"]));
    assert_eq!(
        schema["data"]["publicDiscovery"]["placeAndUniverseIdsAreDistinct"],
        true
    );
    assert_eq!(schema["data"]["publicDiscovery"]["salesRevenue"], false);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn discovery_bad_input_and_profile_are_rejected_before_live_state_or_credentials() {
    let root = tempfile::tempdir().unwrap();
    for (args, code) in [
        (
            &["charts", "games", "--profile", "fake", "--live"][..],
            "discovery_anonymous_only",
        ),
        (
            &["charts", "games", "--sort", "bad/route", "--live"],
            "discovery_query_invalid",
        ),
        (
            &["charts", "sorts", "--country", "bad", "--live"],
            "discovery_query_invalid",
        ),
        (
            &[
                "games",
                "get",
                "--universe",
                "77",
                "--place",
                "88",
                "--live",
            ],
            "invalid_arguments",
        ),
        (
            &["games", "music", "--place", "0", "--live"],
            "invalid_arguments",
        ),
        (
            &[
                "games",
                "passes",
                "--universe",
                "77",
                "--limit",
                "31",
                "--live",
            ],
            "invalid_arguments",
        ),
    ] {
        let out = run(root.path(), args);
        assert_eq!(json(&out)["error"]["code"], code, "{args:?}");
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn all_discovery_reads_obey_shared_budget_without_browser_paths() {
    let root = tempfile::tempdir().unwrap();
    let state = state_root(root.path());
    std::fs::create_dir_all(&state).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::fs::write(
        state.join("traffic.json"),
        serde_json::to_vec(&serde_json::json!({"version":1,"starts":[now,now,now,now,now]}))
            .unwrap(),
    )
    .unwrap();
    for args in [
        &["charts", "games", "--live"][..],
        &["charts", "music", "--live"],
        &["games", "get", "--place", "88", "--live"],
        &["games", "music", "--universe", "77", "--live"],
        &["games", "passes", "--universe", "77", "--live"],
        &["games", "products", "--universe", "77", "--live"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_rob"))
            .args(args)
            .env_remove("APPDATA")
            .env("LOCALAPPDATA", root.path())
            .env("HOME", root.path())
            .env("XDG_STATE_HOME", root.path())
            .output()
            .unwrap();
        assert_eq!(
            json(&out)["error"]["code"],
            "traffic_budget_exhausted",
            "{args:?}"
        );
        assert!(!state.join("live.lock").exists());
    }
}

#[test]
fn creator_store_search_is_offline_by_default_and_marketplace_alias_is_discoverable() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        &[
            "marketplace",
            "search",
            "--type",
            "audio",
            "--audio-type",
            "music",
            "--query",
            "ambient",
        ],
    );
    assert_eq!(json(&output)["error"]["code"], "live_not_enabled");
    let links = run(root.path(), &["store", "link", "--asset", "77"]);
    assert!(links.status.success());
    assert_eq!(
        json(&links)["data"]["storeUrl"],
        "https://create.roblox.com/store/asset/77"
    );
    assert_eq!(json(&links)["data"]["usagePermissionVerified"], false);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn avatar_catalog_is_offline_by_default_and_links_keep_asset_bundle_namespaces_separate() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        &["avatar", "search", "--type", "jacket", "--query", "black"],
    );
    assert_eq!(json(&output)["error"]["code"], "live_not_enabled");
    let asset = run(root.path(), &["catalog", "link", "--asset", "77"]);
    let bundle = run(root.path(), &["avatar", "link", "--bundle", "77"]);
    assert!(asset.status.success());
    assert!(bundle.status.success());
    assert_eq!(
        json(&asset)["data"]["catalogUrl"],
        "https://www.roblox.com/catalog/77"
    );
    assert_eq!(
        json(&bundle)["data"]["catalogUrl"],
        "https://www.roblox.com/bundles/77"
    );
    assert_eq!(
        json(&bundle)["data"]["appearanceHint"]["canUseAsSingleAssetId"],
        false
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn avatar_profile_price_and_ambiguous_id_inputs_stop_before_credentials() {
    let root = tempfile::tempdir().unwrap();
    let profile = run(
        root.path(),
        &[
            "avatar",
            "search",
            "--query",
            "black",
            "--profile",
            "fake",
            "--live",
        ],
    );
    assert_eq!(json(&profile)["error"]["code"], "avatar_anonymous_only");
    let price = run(
        root.path(),
        &[
            "avatar",
            "search",
            "--query",
            "black",
            "--min-price",
            "100",
            "--max-price",
            "10",
            "--live",
        ],
    );
    assert_eq!(json(&price)["error"]["code"], "avatar_query_invalid");
    let ambiguous = run(
        root.path(),
        &[
            "avatar", "link", "--asset", "77", "--bundle", "77", "--live",
        ],
    );
    assert_eq!(json(&ambiguous)["error"]["code"], "invalid_arguments");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn avatar_catalog_shares_budget_without_firefox_paths_or_identity_requests() {
    let root = tempfile::tempdir().unwrap();
    let state = state_root(root.path());
    std::fs::create_dir_all(&state).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::fs::write(
        state.join("traffic.json"),
        serde_json::to_vec(&serde_json::json!({"version":1,"starts":[now,now,now,now,now]}))
            .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rob"))
        .args(["avatar", "search", "--query", "black", "--live"])
        .env_remove("APPDATA")
        .env("LOCALAPPDATA", root.path())
        .env("HOME", root.path())
        .env("XDG_STATE_HOME", root.path())
        .output()
        .unwrap();
    assert_eq!(json(&output)["error"]["code"], "traffic_budget_exhausted");
    assert!(!state.join("live.lock").exists());
}

#[test]
fn store_invalid_audio_filters_and_profile_selection_are_rejected_before_credentials() {
    let root = tempfile::tempdir().unwrap();
    let invalid = run(
        root.path(),
        &[
            "store",
            "search",
            "--type",
            "model",
            "--query",
            "tree",
            "--audio-type",
            "music",
            "--live",
        ],
    );
    assert_eq!(json(&invalid)["error"]["code"], "store_search_invalid");
    let profile = run(
        root.path(),
        &[
            "store",
            "get",
            "--asset",
            "77",
            "--profile",
            "not-a-browser",
            "--live",
        ],
    );
    assert_eq!(json(&profile)["error"]["code"], "store_anonymous_only");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn public_store_uses_the_shared_budget_without_a_firefox_directory_or_appdata() {
    let root = tempfile::tempdir().unwrap();
    let state = state_root(root.path());
    std::fs::create_dir_all(&state).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    std::fs::write(
        state.join("traffic.json"),
        serde_json::to_vec(&serde_json::json!({ "version": 1, "starts": [now,now,now,now,now] }))
            .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rob"))
        .args([
            "store", "search", "--type", "audio", "--query", "ambient", "--live",
        ])
        .env_remove("APPDATA")
        .env("LOCALAPPDATA", root.path())
        .env("HOME", root.path())
        .env("XDG_STATE_HOME", root.path())
        .output()
        .unwrap();
    assert_eq!(json(&output)["error"]["code"], "traffic_budget_exhausted");
    assert!(!state.join("live.lock").exists());
}

#[test]
fn key_creation_defaults_to_an_offline_plan_without_secret_or_browser_access() {
    let root = tempfile::tempdir().unwrap();
    let expiry = chrono::DateTime::<chrono::Utc>::from_timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            + 86400,
        0,
    )
    .unwrap()
    .to_rfc3339();
    let request = root.path().join("key-plan.json");
    std::fs::write(&request, serde_json::to_vec(&serde_json::json!({
        "name": "analytics-read", "expirationTime": expiry, "allowedCidrs": ["203.0.113.7/32"],
        "scopes": [{ "scopeType": "universe-analytics", "targetParts": ["777"], "operations": ["universe.analytics:read"] }]
    })).unwrap()).unwrap();
    let output = run(
        root.path(),
        &[
            "apikey",
            "create",
            "--request",
            request.to_str().unwrap(),
            "--secret-name",
            "analytics-read",
        ],
    );
    assert!(output.status.success());
    assert_eq!(json(&output)["data"]["dryRun"], true);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn analytics_csv_export_is_offline_and_does_not_start_a_browser_by_default() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        &[
            "analytics",
            "export",
            "--url",
            "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42",
            "--chart",
            "Day 1 retention",
            "--output",
            root.path().join("retention.csv").to_str().unwrap(),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(json(&output)["error"]["code"], "live_not_enabled");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn browser_page_loading_needs_separate_consent_before_any_credential_access() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        &[
            "analytics",
            "export",
            "--url",
            "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?annotation=Benchmark",
            "--chart",
            "Day 1 retention",
            "--output",
            root.path().join("out.csv").to_str().unwrap(),
            "--live",
        ],
    );
    assert_eq!(
        json(&output)["error"]["code"],
        "browser_navigation_not_allowed"
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn interactive_login_still_requires_live_before_reading_credentials_or_opening_firefox() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        &[
            "analytics",
            "export",
            "--url",
            "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention",
            "--chart",
            "Day 1 retention",
            "--output",
            root.path().join("out.csv").to_str().unwrap(),
            "--interactive-login",
            "--group",
            "42",
        ],
    );
    assert_eq!(json(&output)["error"]["code"], "live_not_enabled");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn contradictory_analytics_owner_is_rejected_before_browser_credentials() {
    let root = tempfile::tempdir().unwrap();
    let output = run(
        root.path(),
        &[
            "analytics",
            "export",
            "--url",
            "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42",
            "--chart",
            "Day 1 retention",
            "--output",
            root.path().join("out.csv").to_str().unwrap(),
            "--group",
            "43",
            "--live",
        ],
    );
    assert_eq!(json(&output)["error"]["code"], "analytics_owner_conflict");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn settings_group_switch_and_dashboard_links_have_explicit_targets() {
    let root = tempfile::tempdir().unwrap();
    let settings = run(
        root.path(),
        &[
            "experiences",
            "settings",
            "--universe",
            "77",
            "--group",
            "42",
        ],
    );
    assert_eq!(json(&settings)["error"]["code"], "live_not_enabled");
    let group = run(root.path(), &["experiences", "list", "--group", "42"]);
    assert_eq!(json(&group)["error"]["code"], "live_not_enabled");
    let links = run(
        root.path(),
        &["dashboard", "links", "--universe", "77", "--group", "42"],
    );
    assert!(links.status.success());
    assert_eq!(
        json(&links)["data"]["settingsUrl"],
        "https://create.roblox.com/dashboard/creations/experiences/77/configure?groupId=42"
    );
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
