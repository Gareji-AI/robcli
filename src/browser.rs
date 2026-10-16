use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
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
            && parts[3].parse::<u64>().is_ok_and(|id| id > 0)
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
            || query.len() > 1
            || query.iter().any(|(key, value)| {
                key != "groupId" || !value.parse::<u64>().is_ok_and(|id| id > 0)
            })
            || chart.is_empty()
            || chart.len() > 200
            || chart.chars().any(char::is_control)
        {
            return Err(browser_error(
                "analytics_url_invalid",
                "Only an exact supported Creator Dashboard analytics URL and optional numeric groupId are accepted.",
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
        Ok(Self {
            url,
            chart: chart.to_owned(),
            output: output.to_path_buf(),
        })
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
                return Err(browser_error(
                    "browser_auth_required",
                    "The browser requires login or an additional challenge.",
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
    cookie: &SessionCookie,
    driver_path: &Path,
    guard: &mut TrafficGuard<C>,
) -> Result<Value, AppError> {
    let mut target = CsvTarget::reserve(&options.output)?;
    let downloads = tempfile::tempdir().map_err(|_| {
        browser_error(
            "browser_storage_failed",
            "Cannot create an owned temporary download directory.",
        )
    })?;
    let mut ui = LocalDriver::launch(driver_path, downloads.path())?;
    guard.reserve()?;
    ui.navigate("https://www.roblox.com/robots.txt")?;
    ui.call(Method::POST, "cookie", Some(json!({ "cookie": { "name": ".ROBLOSECURITY", "value": cookie.browser_value(), "domain": ".roblox.com", "path": "/", "secure": true, "httpOnly": true, "sameSite": "Lax" } })))?;
    guard.reserve()?;
    ui.navigate(options.url.as_str())?;
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
        "filters": "Fresh Dashboard page defaults; old browser local-storage filters are not copied.",
        "browserTraffic": "One analytics page load and one export click; normal page assets/XHR are not covered by the direct-API request cap." }),
    )
}

struct LocalDriver {
    _process: OwnedDriver,
    client: reqwest::blocking::Client,
    base: String,
    session: String,
    downloads: PathBuf,
}

impl LocalDriver {
    fn launch(binary: &Path, downloads: &Path) -> Result<Self, AppError> {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| {
                browser_error(
                    "driver_start_failed",
                    "Cannot reserve a loopback driver port.",
                )
            })?;
            listener
                .local_addr()
                .map_err(|_| {
                    browser_error(
                        "driver_start_failed",
                        "Cannot inspect the owned loopback port.",
                    )
                })?
                .port()
        };
        let mut process = OwnedDriver::spawn(binary, port)?;
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
        let capabilities = private_capabilities(downloads);
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
            downloads: downloads.to_path_buf(),
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

    fn call(&mut self, method: Method, path: &str, body: Option<Value>) -> Result<Value, AppError> {
        let mut request = self.client.request(
            method,
            format!("{}/session/{}/{}", self.base, self.session, path),
        );
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(|_| {
            browser_error(
                "browser_control_failed",
                "The private browser action failed or timed out.",
            )
        })?;
        if !response.status().is_success() {
            return Err(browser_error(
                "browser_control_failed",
                "The private browser refused the action. Raw driver messages are not printed.",
            ));
        }
        let value: Value = response.json().map_err(|_| {
            browser_error(
                "browser_control_failed",
                "The private browser returned an invalid response.",
            )
        })?;
        Ok(value["value"].clone())
    }

    fn navigate(&mut self, url: &str) -> Result<(), AppError> {
        self.call(Method::POST, "url", Some(json!({ "url": url })))?;
        Ok(())
    }
}

fn private_capabilities(downloads: &Path) -> Value {
    json!({ "capabilities": { "alwaysMatch": { "browserName": "firefox", "moz:firefoxOptions": {
        "args": ["--headless", "--private-window"], "prefs": {
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
            Some(json!({ "script": CHART_SCRIPT, "args": [chart, phase] })),
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
    #[cfg(windows)]
    job: Option<winjob::Job>,
}

impl OwnedDriver {
    fn spawn(binary: &Path, port: u16) -> Result<Self, AppError> {
        #[cfg(not(windows))]
        {
            let _ = (binary, port);
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
                    &port.to_string(),
                    "--log",
                    "fatal",
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
                .stderr(Stdio::null())
                .spawn()
                .map_err(|_| {
                    browser_error(
                        "driver_start_failed",
                        "Cannot launch the local Mozilla geckodriver.",
                    )
                })?;
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
                job: Some(job),
            })
        }
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
const [chart, phase] = arguments;
const norm = s => (s || '').replace(/\s+/g, ' ').trim();
const visible = e => !!e && e.getBoundingClientRect().width > 0 && e.getBoundingClientRect().height > 0 && getComputedStyle(e).visibility !== 'hidden';
if (location.origin !== 'https://create.roblox.com' || /login|captcha|challenge/i.test(location.pathname)) return {state:'auth_required'};
if ([...document.querySelectorAll('iframe')].some(e => visible(e) && /captcha|challenge|arkoselabs/i.test(e.src || '')) || [...document.querySelectorAll('[role="dialog"]')].some(e => visible(e) && /two.step|captcha|security verification|verification required|認証/i.test(e.textContent || ''))) return {state:'auth_required'};
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
        let caps = private_capabilities(root.path());
        let options = &caps["capabilities"]["alwaysMatch"]["moz:firefoxOptions"];
        assert_eq!(options["prefs"]["browser.privatebrowsing.autostart"], true);
        assert!(options.get("profile").is_none());
    }
}
