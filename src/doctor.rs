use std::path::PathBuf;

use serde_json::{Value, json};

/// Existence checks only. Paths and environment values are never returned.
pub(crate) fn any_file(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| path.is_file())
}

pub(crate) fn report(firefox_present: bool, driver_present: bool) -> Value {
    let csv = if cfg!(windows) {
        "windows-only"
    } else {
        "unsupported"
    };
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "cliVersion": env!("CARGO_PKG_VERSION"),
        "firefoxExecutablePresent": firefox_present,
        "geckodriverPresent": driver_present,
        "firefoxLaunched": false,
        "geckodriverLaunched": false,
        "cookieDatabaseOpened": false,
        "profileContentsRead": false,
        "trafficStateOpened": false,
        "secretsRead": false,
        "homeDirectoryScanned": false,
        "csvBrowserAdapter": csv,
        "publicReadsNeedFirefox": false,
        "ciPortabilityIsNotDeviceVerification": true,
        "pathsReturned": false,
        "environmentValuesReturned": false
    })
}

pub(crate) fn local_presence() -> (bool, bool) {
    let driver_name = if cfg!(windows) {
        "geckodriver.exe"
    } else {
        "geckodriver"
    };
    (
        any_file(&firefox_candidates()),
        any_file(&driver_candidates(driver_name)) || path_has(driver_name),
    )
}

fn path_has(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn firefox_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        if let Some(root) = env_path("PROGRAMFILES") {
            paths.push(root.join("Mozilla Firefox/firefox.exe"));
        }
        if let Some(root) = env_path("PROGRAMFILES(X86)") {
            paths.push(root.join("Mozilla Firefox/firefox.exe"));
        }
        if let Some(root) = env_path("LOCALAPPDATA") {
            paths.push(root.join("Mozilla Firefox/firefox.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from(
            "/Applications/Firefox.app/Contents/MacOS/firefox",
        ));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        paths.push(PathBuf::from("/usr/bin/firefox"));
        paths.push(PathBuf::from("/usr/lib/firefox/firefox"));
    }
    paths
}

fn driver_candidates(name: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    if let Some(root) = env_path("LOCALAPPDATA") {
        paths.push(root.join("robcli/tools").join(name));
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = env_path("HOME") {
        paths.push(
            home.join("Library/Application Support/robcli/tools")
                .join(name),
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let root = env_path("XDG_STATE_HOME")
            .or_else(|| env_path("HOME").map(|home| home.join(".local/state")));
        if let Some(root) = root {
            paths.push(root.join("robcli/tools").join(name));
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_is_existence_only_and_report_has_no_path() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("firefox.exe");
        let present = root.path().join("geckodriver.exe");
        std::fs::write(&present, b"not-read").unwrap();
        assert!(!any_file(&[missing]));
        assert!(any_file(&[present]));
        let report = report(false, true);
        let text = report.to_string();
        assert_eq!(report["firefoxLaunched"], false);
        assert_eq!(report["cookieDatabaseOpened"], false);
        assert_eq!(report["trafficStateOpened"], false);
        assert_eq!(report["publicReadsNeedFirefox"], false);
        assert_eq!(report["pathsReturned"], false);
        assert!(!text.contains("Users"));
        assert!(!text.contains("not-read"));
    }
}
