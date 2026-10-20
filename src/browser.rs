use std::fs::{File, OpenOptions};
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
#[cfg(windows)]
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use reqwest::{Method, Url};
use serde_json::{Value, json};

use crate::auth::SessionCookie;
use crate::error::AppError;
use crate::traffic::{Clock, TrafficGuard};

const MAX_CSV: u64 = 20 * 1024 * 1024;

pub(crate) struct ExportOptions {
    pub url: Url,
    pub chart: String,
    output: PathBuf,
    interactive_login: bool,
    universe: u64,
    group: Option<u64>,
}

fn signed_resource_id(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|id| (1..=i64::MAX as u64).contains(&id))
}

fn browser_error(code: &'static str, message: &'static str) -> AppError {
    AppError::new(
        code,
        message,
        "Stop; no page reload, second export click, query-API fallback, or authentication bypass is attempted.",
    )
}

impl ExportOptions {
    pub fn validate(url: &str, chart: &str, output: &Path) -> Result<Self, AppError> {
        let url = Url::parse(url).map_err(|_| {
            browser_error(
                "analytics_url_invalid",
                "Provide the exact analytics-page URL from Creator Dashboard.",
            )
        })?;
        let parts: Vec<_> = url.path_segments().into_iter().flatten().collect();
        let path_valid = parts.len() == 6
            && parts[..3] == ["dashboard", "creations", "experiences"]
            && signed_resource_id(parts[3])
            && parts[4] == "analytics"
            && [
                "retention",
                "engagement",
                "acquisition",
                "demographics",
                "economy",
                "funnels",
                "explore",
            ]
            .contains(&parts[5]);
        let query: Vec<_> = url.query_pairs().collect();
        if url.scheme() != "https"
            || url.host_str() != Some("create.roblox.com")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || !path_valid
            || query.len() > 2
            || query.iter().filter(|(key, _)| key == "groupId").count() > 1
            || query.iter().filter(|(key, _)| key == "annotation").count() > 1
            || query.iter().any(|(key, value)| match key.as_ref() {
                "groupId" => !signed_resource_id(value),
                "annotation" => value != "Benchmark",
                _ => true,
            })
            || chart.is_empty()
            || chart.len() > 200
            || chart.chars().any(char::is_control)
        {
            return Err(browser_error(
                "analytics_url_invalid",
                "Only a supported Creator Dashboard analytics URL, optional numeric groupId and annotation=Benchmark are accepted.",
            ));
        }
        if !output
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("csv"))
            || output.exists()
        {
            return Err(browser_error(
                "csv_output_unavailable",
                "Choose a new .csv output file; existing files are never overwritten.",
            ));
        }
        let universe = parts[3].parse().unwrap();
        let group = query
            .iter()
            .find(|(key, _)| key == "groupId")
            .map(|(_, value)| value.parse().unwrap());
        Ok(Self {
            url,
            chart: chart.to_owned(),
            output: output.to_path_buf(),
            interactive_login: false,
            universe,
            group,
        })
    }

    pub fn with_group(mut self, group: Option<u64>) -> Result<Self, AppError> {
        if group.is_some_and(|id| id == 0 || id > i64::MAX as u64)
            || (group.is_some() && self.group.is_some() && group != self.group)
        {
            return Err(browser_error(
                "analytics_owner_conflict",
                "The explicit group and URL group hint disagree. No credentials were read.",
            ));
        }
        self.group = group.or(self.group);
        Ok(self)
    }

    pub fn with_interactive_login(mut self, enabled: bool) -> Self {
        self.interactive_login = enabled;
        self
    }
}

pub(crate) fn resolve_driver(explicit: Option<&Path>, state: &Path) -> Result<PathBuf, AppError> {
    if !cfg!(windows) {
        return Err(browser_error(
            "browser_platform_unsupported",
            "Owned private-browser export currently supports Windows only.",
        ));
    }
    let mut candidates = Vec::new();
    if let Some(path) = explicit {
        candidates.push(path.to_path_buf());
    } else {
        if let Some(parent) = state.parent() {
            candidates.push(parent.join("tools/geckodriver.exe"));
        }
        if let Some(paths) = std::env::var_os("PATH") {
            candidates
                .extend(std::env::split_paths(&paths).map(|path| path.join("geckodriver.exe")));
        }
    }
    candidates.into_iter().find(|path| path.is_file()).and_then(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(|| browser_error("geckodriver_missing", "Install Mozilla geckodriver or provide its path with --driver. No browser or credential lookup was started."))
}

trait ChartUi {
    fn execute(&mut self, phase: &str, chart: &str) -> Result<Value, AppError>;
    fn collect_csv(&mut self) -> Result<Vec<u8>, AppError>;
}

fn auth_stop(value: &Value, phase: &str) -> AppError {
    // Page/driver values are untrusted. Project fixed categories only: never
    // return the raw URL, query, DOM text, cookie, iframe source, or driver body.
    let project = |field: &str, allowed: &[&str]| {
        let value = value[field].as_str().unwrap_or("unclassified");
        if allowed.contains(&value) {
            value.to_owned()
        } else {
            "unclassified".to_owned()
        }
    };
    let mut error = browser_error(
        "browser_auth_required",
        "The browser stopped on an authentication or navigation signal. This does not by itself prove a 2FA or CAPTCHA challenge.",
    );
    error.details = Some(json!({ "browserAuth": {
        "signal": project("signal", &["login_route", "challenge_route", "unexpected_origin", "challenge_frame", "verification_dialog"]),
        "originCategory": project("originCategory", &["creator_dashboard", "roblox_web", "roblox_auth", "other"]),
        "routeCategory": project("routeCategory", &["login", "challenge", "experience_analytics", "dashboard_other", "landing", "other"]),
        "phase": if ["target-probe", "probe", "menu", "menu-probe", "download"].contains(&phase) { phase } else { "unclassified" },
        "classificationOnly": true, "automaticRetry": false
    } }));
    error
}

fn report(
    ui: &mut impl ChartUi,
    phase: &str,
    chart: &str,
    seconds: u64,
) -> Result<String, AppError> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let value = ui.execute(phase, chart)?;
        match value["state"].as_str() {
            Some("waiting") if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(250))
            }
            Some("ready" | "clicked") => return Ok(value["state"].as_str().unwrap().to_owned()),
            Some("no_data") => {
                return Err(browser_error(
                    "analytics_no_data",
                    "The selected chart reports no data for its current period.",
                ));
            }
            Some("disabled") => {
                return Err(browser_error(
                    "csv_export_disabled",
                    "Download CSV is disabled; it was not clicked.",
                ));
            }
            Some("auth_required") => {
                return Err(auth_stop(&value, phase));
            }
            Some("wrong_target") => {
                return Err(browser_error(
                    "browser_target_mismatch",
                    "The browser is not on the explicitly selected experience, analytics tab and group. No chart was read or clicked.",
                ));
            }
            _ => {
                return Err(browser_error(
                    "chart_control_unavailable",
                    "The chart or its unique CSV action is unavailable. UI selectors are not guessed further.",
                ));
            }
        }
    }
}

fn export_chart<C: Clock>(
    ui: &mut impl ChartUi,
    chart: &str,
    guard: &mut TrafficGuard<C>,
) -> Result<Vec<u8>, AppError> {
    if report(ui, "probe", chart, 15)? != "ready" {
        return Err(browser_error(
            "chart_control_unavailable",
            "The chart was not ready.",
        ));
    }
    if report(ui, "menu", chart, 0)? != "clicked" {
        return Err(browser_error(
            "chart_control_unavailable",
            "The chart menu did not open.",
        ));
    }
    if report(ui, "menu-probe", chart, 3)? != "ready" {
        return Err(browser_error(
            "chart_control_unavailable",
            "The CSV action was not ready.",
        ));
    }
    guard.reserve()?;
    if report(ui, "download", chart, 0)? != "clicked" {
        return Err(browser_error(
            "chart_control_unavailable",
            "The CSV action was not clicked.",
        ));
    }
    let bytes = ui.collect_csv()?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| browser_error("csv_invalid", "The download was not UTF-8 CSV text."))?;
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if bytes.is_empty() || bytes.len() as u64 > MAX_CSV || trimmed.starts_with(['<', '{', '[']) {
        return Err(browser_error(
            "csv_invalid",
            "The download was empty, oversized, or not CSV data.",
        ));
    }
    Ok(bytes)
}

struct CsvTarget {
    file: File,
    path: PathBuf,
    complete: bool,
}
impl CsvTarget {
    fn reserve(path: &Path) -> Result<Self, AppError> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|_| {
            browser_error(
                "csv_output_unavailable",
                "Cannot reserve a new CSV file. No browser was launched.",
            )
        })?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            complete: false,
        })
    }
}
impl Drop for CsvTarget {
    fn drop(&mut self) {
        if !self.complete {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn export<C: Clock>(
    options: &ExportOptions,
    cookie: SessionCookie,
    driver_path: &Path,
    expected_user: u64,
    guard: &mut TrafficGuard<C>,
) -> Result<Value, AppError> {
    let mut target = CsvTarget::reserve(&options.output)?;
    let downloads = tempfile::tempdir().map_err(|_| {
        browser_error(
            "browser_storage_failed",
            "Cannot create an owned temporary download directory.",
        )
    })?;
    let mut ui = LocalDriver::launch(driver_path, downloads.path(), options)?;
    if !options.interactive_login {
        guard.reserve()?;
        ui.navigate("https://www.roblox.com/robots.txt")?;
        ui.set_session_cookie(&cookie)?;
    }
    guard.reserve()?;
    ui.navigate(options.url.as_str())?;
    if options.interactive_login {
        manual_login_gate(options, expected_user)?;
    }
    report(&mut ui, "target-probe", &options.chart, 0)?;
    let browser_cookie = if options.interactive_login {
        ui.owned_session_cookie()?
    } else {
        cookie
    };
    let mut api = crate::api::Api::new(crate::transport::HttpsTransport::new()?, browser_cookie);
    if options.interactive_login {
        api.confirm_account(expected_user, guard)?;
    }
    api.verify_experience_owner(options.universe, expected_user, options.group, guard)?;
    drop(api);
    let bytes = export_chart(&mut ui, &options.chart, guard)?;
    target
        .file
        .write_all(&bytes)
        .and_then(|()| target.file.flush())
        .and_then(|()| target.file.sync_all())
        .map_err(|_| {
            browser_error(
                "csv_write_failed",
                "The CSV could not be saved. No second download was attempted.",
            )
        })?;
    target.complete = true;
    Ok(
        json!({ "chart": options.chart, "sourceUrl": options.url.as_str(), "output": options.output.to_string_lossy(), "bytes": bytes.len(),
        "universeId": options.universe, "groupId": options.group, "ownerVerified": true,
        "interactiveLogin": options.interactive_login, "accountReverified": options.interactive_login,
        "filters": if options.interactive_login { "Filters of the manually confirmed page; not extracted automatically." } else { "Fresh Dashboard page defaults; old browser local-storage filters are not copied." },
        "browserTraffic": "One analytics page load and one export click; normal page assets/XHR are not covered by the direct-API request cap." }),
    )
}

fn manual_login_gate(options: &ExportOptions, expected_user: u64) -> Result<(), AppError> {
    writeln!(std::io::stderr(), "ROB_MANUAL_LOGIN: A new private Firefox is open. Log in as Roblox user {expected_user} in that browser only. Never type a password or verification code here.\nOpen this exact target in the SAME window after login if necessary:\n{}\nThen type continue and Enter here, or cancel. Limit: 5 minutes. No browser control or authentication polling occurs while waiting.", options.url.as_str())
        .map_err(|_| browser_error("manual_login_unavailable", "Cannot display the manual-login instructions."))?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut input = zeroize::Zeroizing::new(String::new());
        let accepted = std::io::stdin()
            .lock()
            .take(64)
            .read_line(&mut input)
            .is_ok_and(|count| count > 0 && input.trim() == "continue");
        let _ = sender.send(accepted);
    });
    match receiver.recv_timeout(Duration::from_secs(300)) {
        Ok(true) => Ok(()),
        Ok(false) => Err(browser_error(
            "manual_login_cancelled",
            "Manual login was cancelled or its input was closed. The owned browser will close.",
        )),
        Err(_) => Err(browser_error(
            "manual_login_timeout",
            "Manual login confirmation did not arrive within 5 minutes. The owned browser will close.",
        )),
    }
}

fn control_failure(action: &str, phase: &str, status: Option<u16>, kind: &str) -> AppError {
    let select = |value: &str, allowed: &[&str]| {
        if allowed.contains(&value) {
            value.to_owned()
        } else {
            "unclassified".to_owned()
        }
    };
    let mut error = browser_error(
        "browser_control_failed",
        "The owned browser action failed. Only fixed protocol categories are reported; no raw driver message is printed.",
    );
    error.details = Some(json!({ "browserControl": {
        "action": select(action, &["navigate", "read_document_url", "set_session_cookie", "read_session_cookie", "execute_script", "set_timeouts", "switch_context"]),
        "phase": select(phase, &["none", "private_attestation", "target-probe", "probe", "menu", "menu-probe", "download"]),
        "driverHttpStatus": status,
        "webdriverError": select(kind, &["no such cookie", "no such window", "invalid session id", "javascript error", "invalid argument", "script timeout", "timeout", "unexpected alert open", "unsupported operation", "unknown command", "unknown error", "transport_failed", "response_read_failed", "invalid_response", "body_too_large"])
    } }));
    error
}

fn decode_driver_reply(
    status: u16,
    bytes: &[u8],
    action: &str,
    phase: &str,
) -> Result<Value, AppError> {
    if bytes.len() > 32768 {
        return Err(control_failure(
            action,
            phase,
            Some(status),
            "body_too_large",
        ));
    }
    if !(200..300).contains(&status) {
        #[derive(serde::Deserialize)]
        struct Reply {
            value: ErrorValue,
        }
        #[derive(serde::Deserialize)]
        struct ErrorValue {
            error: String,
        }
        // message/stacktrace/data are deliberately ignored, not logged or copied.
        let reply = serde_json::from_slice::<Reply>(bytes).ok();
        let kind = reply
            .as_ref()
            .map(|reply| reply.value.error.as_str())
            .unwrap_or("invalid_response");
        return Err(control_failure(action, phase, Some(status), kind));
    }
    let mut value: Value = serde_json::from_slice(bytes)
        .map_err(|_| control_failure(action, phase, Some(status), "invalid_response"))?;
    value
        .get_mut("value")
        .map(Value::take)
        .ok_or_else(|| control_failure(action, phase, Some(status), "invalid_response"))
}

fn parse_driver_port(line: &str) -> Option<u16> {
    let rest = line.split("Listening on ").nth(1)?;
    let token = rest.split_whitespace().next()?.trim_end_matches('/');
    if !token.contains("127.0.0.1:") {
        return None;
    }
    let port = token
        .rsplit(':')
        .next()?
        .trim_matches(|c: char| !c.is_ascii_digit());
    let port = port.parse().ok()?;
    (port > 0).then_some(port)
}

#[cfg(windows)]
fn read_driver_port(stderr: std::process::ChildStderr) -> Result<u16, AppError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stderr);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if let Some(port) = parse_driver_port(&line) {
                let _ = sender.send(port);
            }
            line.clear();
        }
    });
    receiver.recv_timeout(Duration::from_secs(5)).map_err(|_| {
        browser_error(
            "driver_start_failed",
            "The owned driver did not publish a loopback port.",
        )
    })
}

fn ensure_listener_owned(port: u16, expected_pid: u32) -> Result<(), AppError> {
    match listener_owned_by(port, expected_pid) {
        Ok(true) => Ok(()),
        _ => Err(browser_error(
            "driver_endpoint_unverified",
            "The loopback driver port is not owned by the spawned driver. No session cookie was sent.",
        )),
    }
}

fn listener_owned_by(port: u16, expected_pid: u32) -> Result<bool, AppError> {
    let mut pids = listener_pids(port)?;
    pids.sort_unstable();
    pids.dedup();
    Ok(pids == [expected_pid])
}

fn deliver_owned_cookie(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    port: u16,
    expected_pid: u32,
    cookie: &SessionCookie,
) -> Result<Value, AppError> {
    ensure_listener_owned(port, expected_pid)?;
    let body = json!({ "cookie": { "name": ".ROBLOSECURITY", "value": cookie.browser_value(), "domain": ".roblox.com", "path": "/", "secure": true, "httpOnly": true, "sameSite": "Lax" } });
    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .map_err(|_| control_failure("set_session_cookie", "none", None, "transport_failed"))?;
    let status = response.status().as_u16();
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    response.take(32769).read_to_end(&mut bytes).map_err(|_| {
        control_failure(
            "set_session_cookie",
            "none",
            Some(status),
            "response_read_failed",
        )
    })?;
    if bytes.len() > 32768 {
        return Err(control_failure(
            "set_session_cookie",
            "none",
            Some(status),
            "body_too_large",
        ));
    }
    decode_driver_reply(status, &bytes, "set_session_cookie", "none")
}

#[cfg(windows)]
fn listener_pids(port: u16) -> Result<Vec<u32>, AppError> {
    use std::mem::size_of;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        TCP_TABLE_OWNER_PID_LISTENER,
    };
    let mut size = 0u32;
    let _status = unsafe {
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            2,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if size == 0 {
        return Err(browser_error(
            "driver_endpoint_unverified",
            "Cannot prove which process owns the loopback driver port.",
        ));
    }
    let mut buffer = vec![0u32; usize::try_from(size).unwrap_or(0).div_ceil(4)];
    if buffer.is_empty() {
        return Err(browser_error(
            "driver_endpoint_unverified",
            "Cannot prove which process owns the loopback driver port.",
        ));
    }
    let status = unsafe {
        GetExtendedTcpTable(
            buffer.as_mut_ptr().cast(),
            &mut size,
            0,
            2,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status != 0 {
        return Err(browser_error(
            "driver_endpoint_unverified",
            "Cannot prove which process owns the loopback driver port.",
        ));
    }
    let table = buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
    // SAFETY: GetExtendedTcpTable wrote an owner-PID listener table into `buffer`.
    // The header is one u32 count followed by that many row records.
    let count = unsafe { (*table).dwNumEntries } as usize;
    let rows_at = unsafe { std::ptr::addr_of!((*table).table).cast::<MIB_TCPROW_OWNER_PID>() };
    let bytes = usize::try_from(size).unwrap_or(0);
    if count.saturating_mul(size_of::<MIB_TCPROW_OWNER_PID>()) + size_of::<u32>() > bytes {
        return Err(browser_error(
            "driver_endpoint_unverified",
            "Cannot prove which process owns the loopback driver port.",
        ));
    }
    let rows = unsafe { std::slice::from_raw_parts(rows_at, count) };
    let loopback = u32::from_ne_bytes([127, 0, 0, 1]);
    let mut pids = Vec::new();
    for row in rows {
        let local_port = u16::from_be(row.dwLocalPort as u16);
        if local_port == port && (row.dwLocalAddr == loopback || row.dwLocalAddr == 0) {
            pids.push(row.dwOwningPid);
        }
    }
    Ok(pids)
}

#[cfg(target_os = "linux")]
fn listener_pids(port: u16) -> Result<Vec<u32>, AppError> {
    let mut inodes = Vec::new();
    for name in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let text = std::fs::read_to_string(name).map_err(|_| {
            browser_error(
                "driver_endpoint_unverified",
                "Cannot prove which process owns the loopback driver port.",
            )
        })?;
        for line in text.lines().skip(1) {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            let local = fields[1];
            let state = fields[3];
            if state != "0A" {
                continue;
            }
            let Some((addr, hex_port)) = local.split_once(':') else {
                continue;
            };
            let Ok(parsed) = u16::from_str_radix(hex_port, 16) else {
                continue;
            };
            if parsed != port {
                continue;
            }
            let loopback = matches!(
                addr,
                "0100007F"
                    | "0000000000000000000000000100007F"
                    | "0000000000000000FFFF00000100007F"
            );
            let any = matches!(addr, "00000000" | "00000000000000000000000000000000");
            if !loopback && !any {
                continue;
            }
            let inode = fields[9];
            if inode.is_empty() || inode == "0" {
                return Err(browser_error(
                    "driver_endpoint_unverified",
                    "Cannot prove which process owns the loopback driver port.",
                ));
            }
            inodes.push(inode.to_owned());
        }
    }
    let mut pids = Vec::new();
    for inode in inodes {
        pids.push(pid_for_socket_inode(&inode)?);
    }
    Ok(pids)
}

#[cfg(target_os = "linux")]
fn pid_for_socket_inode(inode: &str) -> Result<u32, AppError> {
    let needle = format!("socket:[{inode}]");
    let entries = std::fs::read_dir("/proc").map_err(|_| {
        browser_error(
            "driver_endpoint_unverified",
            "Cannot prove which process owns the loopback driver port.",
        )
    })?;
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if std::fs::read_link(fd.path()).is_ok_and(|target| target.to_string_lossy() == needle)
            {
                return Ok(pid);
            }
        }
    }
    Err(browser_error(
        "driver_endpoint_unverified",
        "Cannot prove which process owns the loopback driver port.",
    ))
}

#[cfg(target_os = "macos")]
fn listener_pids(port: u16) -> Result<Vec<u32>, AppError> {
    let output = std::process::Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-Fp"])
        .output()
        .map_err(|_| {
            browser_error(
                "driver_endpoint_unverified",
                "Cannot prove which process owns the loopback driver port.",
            )
        })?;
    if !output.status.success() && output.stdout.is_empty() {
        return Ok(Vec::new());
    }
    if !output.status.success() {
        return Err(browser_error(
            "driver_endpoint_unverified",
            "Cannot prove which process owns the loopback driver port.",
        ));
    }
    let mut pids = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(pid) = line.strip_prefix('p') {
            pids.push(pid.trim().parse().map_err(|_| {
                browser_error(
                    "driver_endpoint_unverified",
                    "Cannot prove which process owns the loopback driver port.",
                )
            })?);
        }
    }
    Ok(pids)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn listener_pids(_port: u16) -> Result<Vec<u32>, AppError> {
    Err(browser_error(
        "driver_endpoint_unverified",
        "Cannot prove which process owns the loopback driver port.",
    ))
}

struct LocalDriver {
    _process: OwnedDriver,
    client: reqwest::blocking::Client,
    base: String,
    session: String,
    port: u16,
    driver_pid: u32,
    downloads: PathBuf,
    expected_url: String,
    expected_group: Option<String>,
}

impl LocalDriver {
    fn launch(binary: &Path, downloads: &Path, options: &ExportOptions) -> Result<Self, AppError> {
        let mut process = OwnedDriver::spawn(binary)?;
        let port = process.port;
        let driver_pid = process.pid();
        ensure_listener_owned(port, driver_pid)?;
        let base = format!("http://127.0.0.1:{port}");
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(25))
            .build()
            .map_err(|_| {
                browser_error(
                    "driver_start_failed",
                    "Cannot initialize the loopback control client.",
                )
            })?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if !process.alive() {
                return Err(browser_error(
                    "driver_start_failed",
                    "The owned driver exited before creating a session.",
                ));
            }
            let ready = client
                .get(format!("{base}/status"))
                .timeout(Duration::from_millis(300))
                .send()
                .ok()
                .filter(|reply| reply.status().is_success())
                .and_then(|reply| reply.json::<Value>().ok())
                .is_some_and(|value| value["value"]["ready"] == true);
            if ready {
                ensure_listener_owned(port, driver_pid)?;
                break;
            }
            if Instant::now() >= deadline {
                return Err(browser_error(
                    "driver_start_failed",
                    "The owned driver did not become ready.",
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        ensure_listener_owned(port, driver_pid)?;
        let capabilities = private_capabilities(downloads, options.interactive_login);
        let response = client
            .post(format!("{base}/session"))
            .json(&capabilities)
            .send()
            .map_err(|_| {
                browser_error(
                    "driver_session_failed",
                    "Cannot start the isolated private Firefox session.",
                )
            })?;
        if !response.status().is_success() {
            return Err(browser_error(
                "driver_session_failed",
                "The driver refused a private browser session.",
            ));
        }
        let value: Value = response.json().map_err(|_| {
            browser_error(
                "driver_session_failed",
                "The driver returned an invalid session response.",
            )
        })?;
        let session = value["value"]["sessionId"]
            .as_str()
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            .ok_or_else(|| {
                browser_error(
                    "driver_session_failed",
                    "The driver session identifier is invalid.",
                )
            })?
            .to_owned();
        let mut driver = Self {
            _process: process,
            client,
            base,
            session,
            port,
            driver_pid,
            downloads: downloads.to_path_buf(),
            expected_url: options.url.to_string(),
            expected_group: options.group.map(|id| id.to_string()),
        };
        driver.call(
            Method::POST,
            "timeouts",
            Some(json!({ "script": 5000, "pageLoad": 20000, "implicit": 0 })),
        )?;
        driver.call(
            Method::POST,
            "moz/context",
            Some(json!({ "context": "chrome" })),
        )?;
        ensure_listener_owned(port, driver_pid)?;
        let private = driver.call(Method::POST, "execute/sync", Some(json!({
            "script": "return ChromeUtils.importESModule('resource://gre/modules/PrivateBrowsingUtils.sys.mjs').PrivateBrowsingUtils.isWindowPrivate(window);",
            "args": []
        })))?;
        if private != true {
            return Err(browser_error(
                "private_browser_unverified",
                "Private browsing could not be verified. No session cookie was injected.",
            ));
        }
        driver.call(
            Method::POST,
            "moz/context",
            Some(json!({ "context": "content" })),
        )?;
        Ok(driver)
    }

    fn set_session_cookie(&mut self, cookie: &SessionCookie) -> Result<Value, AppError> {
        let endpoint = format!("{}/session/{}/cookie", self.base, self.session);
        deliver_owned_cookie(&self.client, &endpoint, self.port, self.driver_pid, cookie)
    }

    fn call(&mut self, method: Method, path: &str, body: Option<Value>) -> Result<Value, AppError> {
        let action = match (method.clone(), path) {
            (Method::GET, "url") => "read_document_url",
            (_, "url") => "navigate",
            (Method::GET, "cookie") => "read_session_cookie",
            (_, "cookie") => "set_session_cookie",
            (_, "cookie/.ROBLOSECURITY") => "read_session_cookie",
            (_, "execute/sync") => "execute_script",
            (_, "timeouts") => "set_timeouts",
            (_, "moz/context") => "switch_context",
            _ => "unclassified",
        };
        let phase = if path == "execute/sync" {
            body.as_ref()
                .and_then(|body| body["args"][1].as_str())
                .unwrap_or("private_attestation")
                .to_owned()
        } else {
            "none".to_owned()
        };
        let mut request = self.client.request(
            method,
            format!("{}/session/{}/{}", self.base, self.session, path),
        );
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .map_err(|_| control_failure(action, &phase, None, "transport_failed"))?;
        let status = response.status().as_u16();
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        response
            .take(32769)
            .read_to_end(&mut bytes)
            .map_err(|_| control_failure(action, &phase, Some(status), "response_read_failed"))?;
        decode_driver_reply(status, &bytes, action, &phase)
    }

    fn owned_session_cookie(&mut self) -> Result<SessionCookie, AppError> {
        ensure_listener_owned(self.port, self.driver_pid)?;
        self.call(
            Method::POST,
            "moz/context",
            Some(json!({ "context": "content" })),
        )?;
        let current = self.call(Method::GET, "url", None)?;
        if current.as_str() != Some(self.expected_url.as_str()) {
            return Err(browser_error(
                "session_cookie_context_unverified",
                "The owned browser document did not match the approved analytics URL. No cookie list was read.",
            ));
        }
        ensure_listener_owned(self.port, self.driver_pid)?;
        let cookies = self.call(Method::GET, "cookie", None)?;
        let Some(cookies) = cookies.as_array() else {
            return Err(browser_error(
                "session_cookie_context_unverified",
                "The owned browser did not return a session cookie list.",
            ));
        };
        let mut value = None;
        let mut matches = 0u8;
        for cookie in cookies {
            let domain = cookie["domain"].as_str().unwrap_or("");
            if cookie["name"] == ".ROBLOSECURITY"
                && (domain == ".roblox.com" || domain == "roblox.com")
            {
                matches = matches.saturating_add(1);
                if let Some(text) = cookie["value"].as_str() {
                    value = Some(text.to_owned());
                }
            }
        }
        if matches == 1
            && let Some(value) = value
        {
            return SessionCookie::from_owned_browser(value);
        }
        Err(browser_error(
            "manual_login_session_missing",
            "The owned browser did not have exactly one expected session cookie after manual confirmation.",
        ))
    }

    fn navigate(&mut self, url: &str) -> Result<(), AppError> {
        ensure_listener_owned(self.port, self.driver_pid)?;
        self.call(Method::POST, "url", Some(json!({ "url": url })))?;
        Ok(())
    }
}

fn private_capabilities(downloads: &Path, interactive: bool) -> Value {
    json!({ "capabilities": { "alwaysMatch": { "browserName": "firefox", "moz:firefoxOptions": {
        "args": if interactive { vec!["--private-window"] } else { vec!["--headless", "--private-window"] }, "prefs": {
            "browser.privatebrowsing.autostart": true, "browser.sessionstore.resume_from_crash": false,
            "browser.sessionstore.max_tabs_undo": 0, "browser.sessionstore.max_windows_undo": 0,
            "browser.download.folderList": 2, "browser.download.dir": downloads.to_string_lossy(),
            "browser.download.useDownloadDir": true, "browser.download.always_ask_before_handling_new_types": false,
            "browser.helperApps.neverAsk.saveToDisk": "text/csv,application/csv,application/octet-stream,text/plain",
            "browser.download.manager.showWhenStarting": false,
            "toolkit.telemetry.enabled": false, "datareporting.healthreport.uploadEnabled": false
            , "network.proxy.type": 0
        }
    } } } })
}

impl ChartUi for LocalDriver {
    fn execute(&mut self, phase: &str, chart: &str) -> Result<Value, AppError> {
        self.call(
            Method::POST,
            "execute/sync",
            Some(json!({ "script": CHART_SCRIPT, "args": [chart, phase, self.expected_url, self.expected_group] })),
        )
    }

    fn collect_csv(&mut self) -> Result<Vec<u8>, AppError> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let mut files = Vec::new();
            let mut partial = false;
            let mut total = 0u64;
            for entry in std::fs::read_dir(&self.downloads).map_err(|_| {
                browser_error(
                    "csv_download_failed",
                    "Cannot inspect the owned download directory.",
                )
            })? {
                let entry = entry.map_err(|_| {
                    browser_error("csv_download_failed", "Cannot inspect a download entry.")
                })?;
                let metadata = entry.metadata().map_err(|_| {
                    browser_error("csv_download_failed", "Cannot inspect download size.")
                })?;
                total = total.saturating_add(metadata.len());
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "part") {
                    partial = true;
                }
                if metadata.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("csv"))
                {
                    files.push(path);
                }
            }
            if total > MAX_CSV {
                return Err(browser_error(
                    "csv_too_large",
                    "The browser download exceeded the 20 MiB limit.",
                ));
            }
            if files.len() > 1 {
                return Err(browser_error(
                    "csv_download_ambiguous",
                    "More than one CSV was downloaded. No file was selected automatically.",
                ));
            }
            if !partial && files.len() == 1 {
                let path = std::fs::canonicalize(&files[0]).map_err(|_| {
                    browser_error("csv_download_failed", "Cannot resolve the downloaded file.")
                })?;
                let root = std::fs::canonicalize(&self.downloads).map_err(|_| {
                    browser_error(
                        "csv_download_failed",
                        "Cannot resolve the owned download directory.",
                    )
                })?;
                if path.parent() != Some(root.as_path()) {
                    return Err(browser_error(
                        "csv_download_failed",
                        "The download escaped its owned directory.",
                    ));
                }
                let mut bytes = Vec::new();
                File::open(path)
                    .map_err(|_| {
                        browser_error("csv_download_failed", "Cannot read the downloaded CSV.")
                    })?
                    .take(MAX_CSV + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| {
                        browser_error("csv_download_failed", "Cannot finish reading the CSV.")
                    })?;
                return Ok(bytes);
            }
            if Instant::now() >= deadline {
                return Err(browser_error(
                    "csv_download_timeout",
                    "No completed CSV appeared. The export was not clicked again.",
                ));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

impl Drop for LocalDriver {
    fn drop(&mut self) {
        let _ = self
            .client
            .delete(format!("{}/session/{}", self.base, self.session))
            .timeout(Duration::from_secs(2))
            .send();
        // OwnedDriver's Job Object closes next and kills any remaining descendants.
    }
}

struct OwnedDriver {
    child: Child,
    port: u16,
    #[cfg(windows)]
    job: Option<winjob::Job>,
}

impl OwnedDriver {
    fn spawn(binary: &Path) -> Result<Self, AppError> {
        #[cfg(not(windows))]
        {
            let _ = binary;
            Err(browser_error(
                "browser_platform_unsupported",
                "Owned browser export is currently Windows-only.",
            ))
        }
        #[cfg(windows)]
        {
            let mut child = Command::new(binary)
                .args([
                    "--host",
                    "127.0.0.1",
                    "--port",
                    "0",
                    "--log",
                    "info",
                    "--allow-system-access",
                ])
                .env_remove("MOZ_LOG")
                .env_remove("MOZ_LOG_FILE")
                .env_remove("MOZ_LOG_MODULES")
                .env_remove("NSPR_LOG_MODULES")
                .env_remove("NSPR_LOG_FILE")
                .env_remove("HTTP_PROXY")
                .env_remove("HTTPS_PROXY")
                .env_remove("ALL_PROXY")
                .env_remove("http_proxy")
                .env_remove("https_proxy")
                .env_remove("all_proxy")
                .env("MOZ_CRASHREPORTER_DISABLE", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|_| {
                    browser_error(
                        "driver_start_failed",
                        "Cannot launch the local Mozilla geckodriver.",
                    )
                })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                browser_error(
                    "driver_start_failed",
                    "Cannot read the owned driver startup stream.",
                )
            })?;
            let port = match read_driver_port(stderr) {
                Ok(port) => port,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            };
            let job = match winjob::Job::attach(&child) {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
            };
            Ok(Self {
                child,
                port,
                job: Some(job),
            })
        }
    }
    fn pid(&self) -> u32 {
        self.child.id()
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for OwnedDriver {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            drop(self.job.take());
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(windows)]
mod winjob {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::*;

    pub(super) struct Job(HANDLE);
    impl Job {
        pub fn attach(child: &Child) -> Result<Self, AppError> {
            // SAFETY: create an unnamed job owned by this process. The handle is
            // closed exactly once; only our freshly launched driver is assigned.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(browser_error(
                    "browser_ownership_failed",
                    "Cannot create a kill-on-close browser Job Object.",
                ));
            }
            let job = Self(handle);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: limits has the exact documented layout and lifetime for
            // this call. The child handle belongs to the newly spawned driver.
            let ok = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                ) != 0
                    && AssignProcessToJobObject(handle, child.as_raw_handle()) != 0
            };
            if !ok {
                return Err(browser_error(
                    "browser_ownership_failed",
                    "Cannot safely own the browser process tree; no session was created.",
                ));
            }
            Ok(job)
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

// Chart names are JSON arguments, never interpolated into executable script.
// This uses the visible menu reported by the user's screenshot, not an invented
// analytics-query payload. Missing/ambiguous UI is a stop, not a selector sweep.
const CHART_SCRIPT: &str = r#"
const [chart, phase, expected, expectedGroup] = arguments;
const norm = s => (s || '').replace(/\s+/g, ' ').trim();
const visible = e => !!e && e.getBoundingClientRect().width > 0 && e.getBoundingClientRect().height > 0 && getComputedStyle(e).visibility !== 'hidden';
const originCategory = location.origin === 'https://create.roblox.com' ? 'creator_dashboard' : location.origin === 'https://www.roblox.com' ? 'roblox_web' : location.origin === 'https://auth.roblox.com' ? 'roblox_auth' : 'other';
const routeCategory = /login/i.test(location.pathname) ? 'login' : /captcha|challenge/i.test(location.pathname) ? 'challenge' : /^\/dashboard\/creations\/experiences\/[1-9][0-9]*\/analytics\//.test(location.pathname) ? 'experience_analytics' : location.pathname.startsWith('/dashboard') ? 'dashboard_other' : location.pathname === '/' ? 'landing' : 'other';
const authSignal = signal => ({state:'auth_required', signal, originCategory, routeCategory});
// Preserve the conservative stop conditions; classify them rather than silently
// weakening a refusal or claiming that a signal proves a security challenge.
if (originCategory !== 'creator_dashboard') return authSignal(routeCategory === 'login' && ['roblox_web','roblox_auth'].includes(originCategory) ? 'login_route' : routeCategory === 'challenge' && ['roblox_web','roblox_auth'].includes(originCategory) ? 'challenge_route' : 'unexpected_origin');
if (routeCategory === 'login') return authSignal('login_route');
if (routeCategory === 'challenge') return authSignal('challenge_route');
if ([...document.querySelectorAll('iframe')].some(e => visible(e) && /captcha|challenge|arkoselabs/i.test(e.src || ''))) return authSignal('challenge_frame');
if ([...document.querySelectorAll('[role="dialog"]')].some(e => visible(e) && /two.step|captcha|security verification|verification required|認証/i.test(e.textContent || ''))) return authSignal('verification_dialog');
if (expected) {
  const target = new URL(expected);
  const groups = new URLSearchParams(location.search).getAll('groupId');
  const selectedGroup = expectedGroup === undefined ? target.searchParams.get('groupId') : expectedGroup;
  // groupId is a removable Dashboard context hint, not the ownership proof.
  // Before any chart action Rust verifies the selected universe's owner via API.
  if (location.pathname !== target.pathname || groups.length > 1 || (groups.length === 1 && groups[0] !== selectedGroup)) return {state:'wrong_target'};
}
if (phase === 'target-probe') return {state:'ready'};
if (phase === 'menu-probe' || phase === 'download') {
  const matches = [...document.querySelectorAll('[role="menuitem"],li,button,a')].filter(e => visible(e) && ['Download CSV','CSVをダウンロード'].includes(norm(e.textContent)));
  const items = matches.filter(e => !matches.some(other => other !== e && e.contains(other)));
  if (items.length !== 1) return {state:items.length ? 'ambiguous' : 'waiting'};
  const item = items[0];
  if (item.disabled || item.closest('[aria-disabled="true"],.Mui-disabled')) return {state:'disabled'};
  if (phase === 'download') { item.click(); return {state:'clicked'}; }
  return {state:'ready'};
}
const titles = [...document.querySelectorAll('h1,h2,h3,h4,h5,h6,[role="heading"],p,span')].filter(e => visible(e) && norm(e.textContent) === norm(chart) && ![...e.children].some(c => norm(c.textContent) === norm(chart)));
if (titles.length !== 1) return {state:titles.length ? 'ambiguous' : 'waiting'};
let candidate = null;
for (let node = titles[0].parentElement, depth = 0; node && depth < 8; node = node.parentElement, depth++) {
  const buttons = [...node.querySelectorAll('button')].filter(visible);
  let menus = buttons.filter(b => b.querySelector('[data-testid="MoreVertIcon"],[data-testid="MoreHorizIcon"]') || /more|options|actions|chart menu/i.test(b.getAttribute('aria-label') || '') || ['menu','true'].includes(b.getAttribute('aria-haspopup')));
  if (!menus.length && buttons.length <= 3 && buttons.some(b => norm(b.textContent) === 'Explore')) menus = buttons.filter(b => !norm(b.textContent) && !b.getAttribute('aria-label'));
  if (menus.length > 1) break;
  if (menus.length === 1) candidate = {node,button:menus[0]};
}
if (!candidate) return {state:'waiting'};
if (candidate.node.querySelector('[role="progressbar"],[aria-busy="true"]')) return {state:'waiting'};
if (/No data for selected period|選択した期間.*データ.*ありません/i.test(candidate.node.textContent)) return {state:'no_data'};
if (phase === 'menu') { candidate.button.click(); return {state:'clicked'}; }
return {state:'ready'};
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traffic::tests::FakeClock;

    #[test]
    fn driver_log_port_accepts_only_ipv4_loopback() {
        assert_eq!(
            parse_driver_port("169\tgeckodriver\tINFO\tListening on 127.0.0.1:4444"),
            Some(4444)
        );
        assert_eq!(parse_driver_port("Listening on 127.0.0.1:9\n"), Some(9));
        assert_eq!(parse_driver_port("Listening on 0.0.0.0:4444"), None);
        assert_eq!(parse_driver_port("Listening on 127.0.0.1:0"), None);
    }

    #[test]
    fn this_process_owns_its_loopback_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(listener_owned_by(port, std::process::id()).unwrap());
        assert!(!listener_owned_by(port, 0).unwrap());
    }

    #[test]
    fn competing_listener_receives_no_session_cookie() {
        use std::io::{Read, Write};
        use std::sync::{Arc, Mutex};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let captured = Arc::clone(&seen);
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(700) {
                if let Ok((mut socket, _)) = listener.accept() {
                    socket
                        .set_read_timeout(Some(Duration::from_millis(400)))
                        .unwrap();
                    let mut buf = [0u8; 8192];
                    let mut all = Vec::new();
                    loop {
                        match socket.read(&mut buf) {
                            Ok(0) => break,
                            Ok(count) => all.extend_from_slice(&buf[..count]),
                            Err(_) => break,
                        }
                    }
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\n{{\"value\":1}}"
                    );
                    *captured.lock().unwrap() = all;
                    return;
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(400))
            .build()
            .unwrap();
        let endpoint = format!("http://127.0.0.1:{port}/session/synthetic/cookie");
        let error = deliver_owned_cookie(
            &client,
            &endpoint,
            port,
            0,
            &crate::auth::SessionCookie::synthetic(),
        )
        .unwrap_err();
        assert_eq!(error.code, "driver_endpoint_unverified");
        assert!(
            !serde_json::to_string(&error)
                .unwrap()
                .contains("synthetic-test-only")
        );
        server.join().unwrap();
        let captured = seen.lock().unwrap();
        let body = String::from_utf8_lossy(&captured);
        assert!(body.is_empty(), "{body}");
    }

    #[test]
    fn owned_endpoint_proof_allows_a_synthetic_cookie_only_for_this_process() {
        use std::io::{Read, Write};
        use std::sync::{Arc, Mutex};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::<u8>::new()));
        let captured = Arc::clone(&seen);
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_millis(400)))
                .unwrap();
            let mut buf = [0u8; 8192];
            let mut all = Vec::new();
            loop {
                match socket.read(&mut buf) {
                    Ok(0) => break,
                    Ok(count) => all.extend_from_slice(&buf[..count]),
                    Err(_) => break,
                }
            }
            let _ = write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{{\"value\":true}}"
            );
            *captured.lock().unwrap() = all;
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let endpoint = format!("http://127.0.0.1:{port}/session/synthetic/cookie");
        let value = deliver_owned_cookie(
            &client,
            &endpoint,
            port,
            std::process::id(),
            &crate::auth::SessionCookie::synthetic(),
        )
        .unwrap();
        assert_eq!(value, json!(true));
        server.join().unwrap();
        let captured = seen.lock().unwrap();
        let body = String::from_utf8_lossy(&captured);
        assert!(body.contains("synthetic-test-only"));
        assert!(!body.contains("Cookie:"));
    }

    #[test]
    fn webdriver_failure_distinguishes_cookie_and_script_actions_without_private_messages() {
        for (kind, action, phase) in [
            ("no such cookie", "read_session_cookie", "none"),
            ("javascript error", "execute_script", "target-probe"),
            ("private-synthetic-value", "execute_script", "probe"),
        ] {
            let body = serde_json::to_vec(&json!({"value":{"error":kind,"message":"private-synthetic-value", "stacktrace":"private-synthetic-value"}})).unwrap();
            let error = decode_driver_reply(404, &body, action, phase).unwrap_err();
            let output = serde_json::to_value(error).unwrap();
            assert_eq!(output["details"]["browserControl"]["action"], action);
            assert_eq!(output["details"]["browserControl"]["phase"], phase);
            assert_eq!(
                output["details"]["browserControl"]["webdriverError"],
                if kind == "private-synthetic-value" {
                    "unclassified"
                } else {
                    kind
                }
            );
            assert!(!output.to_string().contains("private-synthetic-value"));
        }
    }

    struct NoData {
        calls: Vec<String>,
    }
    impl ChartUi for NoData {
        fn execute(&mut self, phase: &str, _: &str) -> Result<serde_json::Value, AppError> {
            self.calls.push(phase.to_owned());
            Ok(serde_json::json!({ "state": "no_data" }))
        }
        fn collect_csv(&mut self) -> Result<Vec<u8>, AppError> {
            panic!("no-data must not download")
        }
    }

    #[test]
    fn no_data_chart_stops_without_menu_click_or_download() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut ui = NoData { calls: Vec::new() };
        assert_eq!(
            export_chart(&mut ui, "Day 1 retention", &mut guard)
                .unwrap_err()
                .code,
            "analytics_no_data"
        );
        assert_eq!(ui.calls, vec!["probe"]);
    }

    struct AuthSignal {
        value: Value,
        calls: Vec<String>,
    }
    impl ChartUi for AuthSignal {
        fn execute(&mut self, phase: &str, _: &str) -> Result<Value, AppError> {
            self.calls.push(phase.to_owned());
            Ok(self.value.clone())
        }
        fn collect_csv(&mut self) -> Result<Vec<u8>, AppError> {
            panic!("an authentication signal must not download")
        }
    }

    #[test]
    fn authentication_stop_reports_only_allowlisted_diagnostics_and_does_not_retry() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut ui = AuthSignal {
            value: json!({ "state": "auth_required", "signal": "login_route",
                "originCategory": "roblox_web", "routeCategory": "login",
                "url": "https://example.com/?token=synthetic-private-value", "cookie": "synthetic-private-value" }),
            calls: Vec::new(),
        };
        let error = export_chart(&mut ui, "Day 1 retention", &mut guard).unwrap_err();
        assert_eq!(error.code, "browser_auth_required");
        let serialized = serde_json::to_value(&error).unwrap();
        assert_eq!(
            serialized["details"]["browserAuth"]["signal"],
            "login_route"
        );
        assert_eq!(
            serialized["details"]["browserAuth"]["originCategory"],
            "roblox_web"
        );
        assert_eq!(
            serialized["details"]["browserAuth"]["routeCategory"],
            "login"
        );
        assert!(!serialized.to_string().contains("synthetic-private-value"));
        assert_eq!(ui.calls, ["probe"]);
    }

    #[test]
    fn unrecognized_authentication_diagnostics_are_not_echoed() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut ui = AuthSignal {
            value: json!({ "state": "auth_required", "signal": "private-synthetic-value",
                "originCategory": "private-synthetic-value", "routeCategory": "private-synthetic-value" }),
            calls: Vec::new(),
        };
        let error = export_chart(&mut ui, "Day 1 retention", &mut guard).unwrap_err();
        let serialized = serde_json::to_value(&error).unwrap();
        let details = &serialized["details"]["browserAuth"];
        assert_eq!(details["signal"], "unclassified");
        assert_eq!(details["originCategory"], "unclassified");
        assert_eq!(details["routeCategory"], "unclassified");
        assert_eq!(details["classificationOnly"], true);
        assert_eq!(details["automaticRetry"], false);
        assert!(!serialized.to_string().contains("private-synthetic-value"));
        assert_eq!(ui.calls, ["probe"]);
    }

    #[test]
    fn interactive_browser_is_visible_private_and_never_uses_an_existing_profile() {
        let root = tempfile::tempdir().unwrap();
        let caps = private_capabilities(root.path(), true);
        let options = &caps["capabilities"]["alwaysMatch"]["moz:firefoxOptions"];
        assert_eq!(options["args"], json!(["--private-window"]));
        assert_eq!(options["prefs"]["browser.privatebrowsing.autostart"], true);
        assert!(options.get("profile").is_none());
        assert!(!options.to_string().contains("--profile"));
    }

    #[test]
    fn canonical_benchmark_url_accepts_explicit_owner_but_not_conflicting_or_arbitrary_queries() {
        let root = tempfile::tempdir().unwrap();
        let canonical = "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?annotation=Benchmark";
        let selected =
            ExportOptions::validate(canonical, "Day 1 retention", &root.path().join("out.csv"))
                .unwrap()
                .with_group(Some(42))
                .unwrap();
        assert_eq!(selected.group, Some(42));
        assert_eq!(selected.universe, 77);
        let with_hint = "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42";
        assert!(
            ExportOptions::validate(with_hint, "Day 1 retention", &root.path().join("other.csv"))
                .unwrap()
                .with_group(Some(43))
                .is_err()
        );
        assert!(
            ExportOptions::validate(
                &canonical.replace("Benchmark", "private-value"),
                "Day 1 retention",
                &root.path().join("out.csv")
            )
            .is_err()
        );
        assert!(
            ExportOptions::validate(
                &format!("{canonical}&annotation=Benchmark"),
                "Day 1 retention",
                &root.path().join("out.csv")
            )
            .is_err()
        );
        let max = i64::MAX.to_string();
        let over = (i64::MAX as u64 + 1).to_string();
        let accepted = ExportOptions::validate(
            &format!(
                "https://create.roblox.com/dashboard/creations/experiences/{max}/analytics/retention?groupId={max}"
            ),
            "Day 1 retention",
            &root.path().join("max.csv"),
        )
        .unwrap();
        assert_eq!(accepted.universe, i64::MAX as u64);
        assert_eq!(accepted.group, Some(i64::MAX as u64));
        for url in [
            format!(
                "https://create.roblox.com/dashboard/creations/experiences/{over}/analytics/retention"
            ),
            format!(
                "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId={over}"
            ),
        ] {
            let Err(error) =
                ExportOptions::validate(&url, "Day 1 retention", &root.path().join("over.csv"))
            else {
                panic!("an ID above i64::MAX must be rejected");
            };
            assert_eq!(error.code, "analytics_url_invalid");
            assert!(!serde_json::to_string(&error).unwrap().contains(&over));
        }
        assert!(accepted.with_group(Some(u64::MAX)).is_err());
    }

    struct Scripted {
        replies: std::collections::VecDeque<&'static str>,
        calls: Vec<String>,
        collected: usize,
        bytes: Vec<u8>,
    }
    impl ChartUi for Scripted {
        fn execute(&mut self, phase: &str, _: &str) -> Result<Value, AppError> {
            self.calls.push(phase.to_owned());
            Ok(json!({ "state": self.replies.pop_front().expect("unexpected UI action") }))
        }
        fn collect_csv(&mut self) -> Result<Vec<u8>, AppError> {
            self.collected += 1;
            Ok(self.bytes.clone())
        }
    }

    #[test]
    fn disabled_csv_action_is_never_clicked_or_retried() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut ui = Scripted {
            replies: ["ready", "clicked", "disabled"].into(),
            calls: Vec::new(),
            collected: 0,
            bytes: Vec::new(),
        };
        assert_eq!(
            export_chart(&mut ui, "Day 1 retention", &mut guard)
                .unwrap_err()
                .code,
            "csv_export_disabled"
        );
        assert_eq!(ui.calls, ["probe", "menu", "menu-probe"]);
        assert_eq!(ui.collected, 0);
        assert!(!root.path().join("traffic.json").exists());
    }

    #[test]
    fn one_chart_gets_one_export_click_and_one_csv() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut ui = Scripted {
            replies: ["ready", "clicked", "ready", "clicked"].into(),
            calls: Vec::new(),
            collected: 0,
            bytes: b"Date,Value\n2026-10-01,0.2\n".to_vec(),
        };
        assert_eq!(
            export_chart(&mut ui, "Day 1 retention", &mut guard).unwrap(),
            b"Date,Value\n2026-10-01,0.2\n"
        );
        assert_eq!(ui.calls, ["probe", "menu", "menu-probe", "download"]);
        assert_eq!(ui.collected, 1);
    }

    #[test]
    fn unexpected_html_download_is_not_reported_as_csv() {
        let root = tempfile::tempdir().unwrap();
        let mut guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let mut ui = Scripted {
            replies: ["ready", "clicked", "ready", "clicked"].into(),
            calls: Vec::new(),
            collected: 0,
            bytes: b"<html>login</html>".to_vec(),
        };
        assert_eq!(
            export_chart(&mut ui, "Day 1 retention", &mut guard)
                .unwrap_err()
                .code,
            "csv_invalid"
        );
        assert_eq!(ui.collected, 1);
    }

    #[test]
    fn browser_url_cannot_target_login_external_sites_or_credentials() {
        let root = tempfile::tempdir().unwrap();
        for url in [
            "https://example.com/dashboard/creations/experiences/77/analytics/retention",
            "http://create.roblox.com/dashboard/creations/experiences/77/analytics/retention",
            "https://create.roblox.com/dashboard/credentials",
            "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?token=private",
            "https://create.roblox.com/dashboard/creations/experiences/77/analytics/retention?groupId=42&groupId=43",
        ] {
            assert!(
                ExportOptions::validate(url, "Day 1 retention", &root.path().join("out.csv"))
                    .is_err()
            );
        }
        let caps = private_capabilities(root.path(), false);
        let options = &caps["capabilities"]["alwaysMatch"]["moz:firefoxOptions"];
        assert_eq!(options["prefs"]["browser.privatebrowsing.autostart"], true);
        assert!(options.get("profile").is_none());
    }
}
