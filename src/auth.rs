use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::header::HeaderValue;
use rusqlite::{Connection, OpenFlags, params};
use zeroize::Zeroizing;

use crate::error::AppError;

#[derive(Debug)]
pub(crate) struct Locations {
    pub firefox: PathBuf,
    pub state: PathBuf,
}

impl Locations {
    pub fn discover() -> Result<Self, AppError> {
        fn env_path(key: &str) -> Result<PathBuf, AppError> {
            std::env::var_os(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| AppError::new("local_paths_missing", "Cannot locate local application directories.", "Set up Firefox and the normal OS user directories; do not supply a cookie through arguments."))
        }
        #[cfg(target_os = "windows")]
        {
            Ok(Self {
                firefox: env_path("APPDATA")?.join("Mozilla/Firefox"),
                state: env_path("LOCALAPPDATA")?.join("robcli/traffic"),
            })
        }
        #[cfg(target_os = "macos")]
        {
            let home = env_path("HOME")?;
            Ok(Self {
                firefox: home.join("Library/Application Support/Firefox"),
                state: home.join("Library/Application Support/robcli/traffic"),
            })
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let home = env_path("HOME")?;
            let state = std::env::var_os("XDG_STATE_HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/state"));
            Ok(Self {
                firefox: home.join(".mozilla/firefox"),
                state: state.join("robcli/traffic"),
            })
        }
    }
}

pub(crate) struct SessionCookie(Zeroizing<String>);

impl fmt::Debug for SessionCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionCookie([REDACTED])")
    }
}

impl SessionCookie {
    /// Only the isolated browser adapter uses the raw value. Never log it or
    /// pass a real Firefox profile to WebDriver.
    pub fn browser_value(&self) -> &str {
        self.0.as_str()
    }

    fn new(value: String) -> Result<Self, AppError> {
        let secret = Zeroizing::new(value);
        if secret.is_empty()
            || secret
                .bytes()
                .any(|byte| byte < 0x20 || byte == 0x7f || byte == b';')
        {
            return Err(AppError::new(
                "session_invalid",
                "The stored session cannot be used as a cookie header.",
                "Renew the session in Firefox; do not paste it into the terminal.",
            ));
        }
        let cookie = Self(secret);
        cookie.header()?;
        Ok(cookie)
    }

    pub fn header(&self) -> Result<HeaderValue, AppError> {
        let value = Zeroizing::new(format!(".ROBLOSECURITY={}", self.0.as_str()));
        let mut header = HeaderValue::from_str(&value).map_err(|_| {
            AppError::new(
                "session_invalid",
                "The session header is invalid.",
                "Renew the session in Firefox.",
            )
        })?;
        header.set_sensitive(true);
        Ok(header)
    }

    #[cfg(test)]
    pub fn synthetic() -> Self {
        Self::new("synthetic-test-only".to_owned()).unwrap()
    }
}

#[derive(Debug)]
pub(crate) struct Session {
    pub profile: String,
    pub cookie: SessionCookie,
}

type Section = BTreeMap<String, String>;

fn ini(file: &Path) -> Result<Vec<Section>, AppError> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) if text.len() <= 65536 => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        _ => {
            return Err(AppError::new(
                "profile_config_unreadable",
                "Cannot read the Firefox profile configuration.",
                "Check the local Firefox profile settings.",
            ));
        }
    };
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with([';', '#']) {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            let mut section = Section::new();
            section.insert("section".to_owned(), name.to_owned());
            sections.push(section);
        } else if let Some((key, value)) = line.split_once('=')
            && let Some(section) = sections.last_mut()
        {
            section.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }
    Ok(sections)
}

fn profile(root: &Path, requested: Option<&str>) -> Result<(PathBuf, String), AppError> {
    let config = ini(&root.join("profiles.ini"))?;
    let profiles: Vec<_> = config
        .iter()
        .filter(|section| {
            section
                .get("section")
                .is_some_and(|name| name.starts_with("Profile"))
                && section.contains_key("Path")
        })
        .collect();
    let matches: Vec<_> = if let Some(name) = requested {
        profiles
            .into_iter()
            .filter(|section| section.get("Name").is_some_and(|value| value == name))
            .collect()
    } else {
        let installs = ini(&root.join("installs.ini"))?;
        let mut defaults: BTreeSet<_> = installs
            .iter()
            .filter_map(|section| section.get("Default").cloned())
            .collect();
        if defaults.is_empty() {
            defaults.extend(
                config
                    .iter()
                    .filter(|section| {
                        section
                            .get("section")
                            .is_some_and(|name| name.starts_with("Install"))
                    })
                    .filter_map(|section| section.get("Default").cloned()),
            );
        }
        if defaults.len() > 1 {
            return Err(ambiguous_profile());
        }
        profiles
            .into_iter()
            .filter(|section| match defaults.first() {
                Some(default) => section.get("Path") == Some(default),
                None => section.get("Default").is_some_and(|value| value == "1"),
            })
            .collect()
    };
    if matches.len() != 1 {
        return Err(ambiguous_profile());
    }
    let selected = matches[0];
    let directory = PathBuf::from(&selected["Path"]);
    let directory = if selected.get("IsRelative").is_some_and(|value| value == "1") {
        root.join(directory)
    } else {
        directory
    };
    if !directory.is_absolute() {
        return Err(ambiguous_profile());
    }
    let name = selected
        .get("Name")
        .cloned()
        .ok_or_else(ambiguous_profile)?;
    Ok((directory.join("cookies.sqlite"), name))
}

fn ambiguous_profile() -> AppError {
    AppError::new(
        "firefox_profile_ambiguous",
        "Cannot select exactly one Firefox profile.",
        "Use --profile with an existing Firefox profile name. No other profiles are scanned for cookies.",
    )
}

pub(crate) fn read_session(
    root: &Path,
    requested: Option<&str>,
    now_seconds: u64,
) -> Result<Session, AppError> {
    let now_seconds = i64::try_from(now_seconds).map_err(|_| {
        AppError::new(
            "clock_invalid",
            "The current time is outside the supported range.",
            "Check the OS clock before live testing.",
        )
    })?;
    let (database, profile) = profile(root, requested)?;
    let read_error = || {
        AppError::new(
            "cookie_read_failed",
            "Cannot read the Firefox cookie database read-only.",
            "Check that the selected Firefox profile exists and is readable. No database copy or broader credential search is attempted.",
        )
    };
    let connection = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| read_error())?;
    connection
        .busy_timeout(Duration::from_millis(250))
        .map_err(|_| read_error())?;
    let mut query = connection.prepare(
        "SELECT DISTINCT value FROM moz_cookies WHERE name = ?1 AND host IN (?2, ?3) AND path = '/' AND originAttributes = '' AND (expiry = 0 OR expiry > ?4) LIMIT 2"
    ).map_err(|_| read_error())?;
    let rows = query
        .query_map(
            params![".ROBLOSECURITY", ".roblox.com", "roblox.com", now_seconds],
            |row| row.get::<_, String>(0),
        )
        .map_err(|_| read_error())?;
    let values = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| read_error())?;
    if values.len() != 1 {
        return Err(AppError::new(
            if values.is_empty() {
                "session_missing"
            } else {
                "session_ambiguous"
            },
            "No unique, unexpired Roblox session was found in the selected Firefox profile.",
            "Log into your own Roblox account in that Firefox profile. Container-only sessions are not selected.",
        ));
    }
    Ok(Session {
        profile,
        cookie: SessionCookie::new(values.into_iter().next().unwrap())?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("main")).unwrap();
        std::fs::write(
            root.path().join("profiles.ini"),
            "[Profile0]\nName=main\nIsRelative=1\nPath=main\nDefault=1\n",
        )
        .unwrap();
        let db = Connection::open(root.path().join("main/cookies.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE moz_cookies (name TEXT, value TEXT, host TEXT, path TEXT, originAttributes TEXT, expiry INTEGER);").unwrap();
        root
    }

    fn add(root: &Path, name: &str, host: &str, context: &str, value: &str, expiry: u64) {
        let db = Connection::open(root.join("main/cookies.sqlite")).unwrap();
        db.execute(
            "INSERT INTO moz_cookies VALUES (?1, ?2, ?3, '/', ?4, ?5)",
            params![name, value, host, context, i64::try_from(expiry).unwrap()],
        )
        .unwrap();
    }

    #[test]
    fn reads_only_selected_session_without_modifying_database() {
        let root = fixture();
        add(
            root.path(),
            ".ROBLOSECURITY",
            ".roblox.com",
            "",
            "synthetic-test-only",
            4000000000,
        );
        add(
            root.path(),
            "other",
            ".roblox.com",
            "",
            "not-selected",
            4000000000,
        );
        add(
            root.path(),
            ".ROBLOSECURITY",
            ".example.com",
            "",
            "not-selected",
            4000000000,
        );
        add(
            root.path(),
            ".ROBLOSECURITY",
            ".roblox.com",
            "^userContextId=2",
            "not-selected",
            4000000000,
        );
        add(
            root.path(),
            ".ROBLOSECURITY",
            ".roblox.com",
            "",
            "expired",
            1,
        );
        let database = root.path().join("main/cookies.sqlite");
        let before = std::fs::read(&database).unwrap();
        let session = read_session(root.path(), None, 1000).unwrap();
        assert_eq!(session.profile, "main");
        assert_eq!(
            session.cookie.header().unwrap(),
            ".ROBLOSECURITY=synthetic-test-only"
        );
        assert_eq!(std::fs::read(database).unwrap(), before);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }

    #[test]
    fn session_debug_and_header_debug_redact_values() {
        let cookie = SessionCookie::synthetic();
        assert!(!format!("{cookie:?}").contains("synthetic-test-only"));
        assert!(!format!("{:?}", cookie.header().unwrap()).contains("synthetic-test-only"));
        assert!(cookie.header().unwrap().is_sensitive());
    }

    #[test]
    fn rejects_header_injection_and_multiple_sessions() {
        assert!(SessionCookie::new("fake\r\nInjected: value".into()).is_err());
        assert!(SessionCookie::new("fake;other=secret".into()).is_err());
        let root = fixture();
        add(
            root.path(),
            ".ROBLOSECURITY",
            ".roblox.com",
            "",
            "first",
            4000000000,
        );
        add(
            root.path(),
            ".ROBLOSECURITY",
            ".roblox.com",
            "",
            "second",
            4000000000,
        );
        assert_eq!(
            read_session(root.path(), None, 1000).unwrap_err().code,
            "session_ambiguous"
        );
    }

    #[test]
    fn install_default_wins_and_ambiguous_installs_fail_closed() {
        let root = fixture();
        std::fs::write(
            root.path().join("installs.ini"),
            "[InstallA]\nDefault=main\n",
        )
        .unwrap();
        assert_eq!(profile(root.path(), None).unwrap().1, "main");
        std::fs::write(
            root.path().join("installs.ini"),
            "[InstallA]\nDefault=main\n[InstallB]\nDefault=other\n",
        )
        .unwrap();
        assert_eq!(
            profile(root.path(), None).unwrap_err().code,
            "firefox_profile_ambiguous"
        );
        assert!(profile(root.path(), Some("main")).is_ok());
    }

    #[test]
    fn does_not_create_missing_cookie_database() {
        let root = fixture();
        let database = root.path().join("main/cookies.sqlite");
        std::fs::remove_file(&database).unwrap();
        assert_eq!(
            read_session(root.path(), None, 1000).unwrap_err().code,
            "cookie_read_failed"
        );
        assert!(!database.exists());
    }
}
