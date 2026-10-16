use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub(crate) const MAX_REQUESTS: usize = 5;
pub(crate) const MIN_INTERVAL_MS: u64 = 2000;
pub(crate) const WINDOW_MS: u64 = 60000;

pub(crate) trait Clock {
    fn now_ms(&self) -> Result<u64, AppError>;
    fn sleep(&self, milliseconds: u64);
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> Result<u64, AppError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| u64::try_from(value.as_millis()).ok())
            .ok_or_else(invalid_state)
    }

    fn sleep(&self, milliseconds: u64) {
        std::thread::sleep(Duration::from_millis(milliseconds));
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u8,
    starts: Vec<u64>,
}

struct Lock {
    file: Option<File>,
    path: PathBuf,
}

impl Drop for Lock {
    fn drop(&mut self) {
        drop(self.file.take());
        // Never remove a lock we did not successfully acquire. A failed removal
        // leaves the next invocation blocked rather than triggering a takeover.
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) struct TrafficGuard<C: Clock> {
    _lock: Lock,
    root: PathBuf,
    starts: Vec<u64>,
    requests: usize,
    clock: C,
}

fn invalid_state() -> AppError {
    AppError::new(
        "traffic_state_invalid",
        "Traffic state is unreadable, invalid, or the clock moved backward.",
        "Stop. Do not delete state, switch its directory, or automatically remove a lock to continue testing.",
    )
}

fn exhausted(retry_after_seconds: Option<u64>) -> AppError {
    let mut error = AppError::new(
        "traffic_budget_exhausted",
        "The bounded live-probe request budget is exhausted; this operation is incomplete.",
        "Stop this probe. Do not rerun in a loop, rotate accounts, or increase the limits to complete it.",
    );
    error.retry_after_seconds = retry_after_seconds;
    error
}

impl<C: Clock> TrafficGuard<C> {
    pub fn acquire(root: &Path, clock: C) -> Result<Self, AppError> {
        std::fs::create_dir_all(root).map_err(|_| invalid_state())?;
        let path = root.join("live.lock");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path).map_err(|_| {
            AppError::new(
                "live_probe_busy",
                "A live-probe lock exists or cannot be acquired.",
                "Only one probe may run. Do not automatically take over or remove a residual lock.",
            )
        })?;
        let mut lock = Lock {
            file: Some(file),
            path,
        };
        writeln!(lock.file.as_mut().unwrap(), "{}", std::process::id())
            .map_err(|_| invalid_state())?;

        let now = clock.now_ms()?;
        let state_path = root.join("traffic.json");
        let mut starts = match File::open(&state_path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(8193)
                    .read_to_end(&mut bytes)
                    .map_err(|_| invalid_state())?;
                if bytes.len() > 8192 {
                    return Err(invalid_state());
                }
                let state: State = serde_json::from_slice(&bytes).map_err(|_| invalid_state())?;
                if state.version != 1
                    || state.starts.len() > MAX_REQUESTS
                    || state.starts.iter().any(|&start| start == 0 || start > now)
                    || state.starts.windows(2).any(|pair| pair[0] > pair[1])
                {
                    return Err(invalid_state());
                }
                state.starts
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err(invalid_state()),
        };
        starts.retain(|start| now - start < WINDOW_MS);
        if starts.len() >= MAX_REQUESTS {
            return Err(exhausted(Some(
                (WINDOW_MS - (now - starts[0])).div_ceil(1000),
            )));
        }
        Ok(Self {
            _lock: lock,
            root: root.to_path_buf(),
            starts,
            requests: 0,
            clock,
        })
    }

    /// Mutable ownership serializes every request. Persist the reservation before
    /// sending so failed attempts and interrupted requests also consume budget.
    pub fn reserve(&mut self) -> Result<(), AppError> {
        let mut now = self.clock.now_ms()?;
        if self.starts.last().is_some_and(|start| *start > now) {
            return Err(invalid_state());
        }
        self.starts.retain(|start| now - start < WINDOW_MS);
        if self.requests >= MAX_REQUESTS || self.starts.len() >= MAX_REQUESTS {
            return Err(exhausted(None));
        }
        if let Some(&last) = self.starts.last() {
            let deadline = last
                .checked_add(MIN_INTERVAL_MS)
                .ok_or_else(invalid_state)?;
            if now < deadline {
                self.clock.sleep(deadline - now);
                now = self.clock.now_ms()?;
                if now < deadline {
                    return Err(invalid_state());
                }
            }
        }
        self.starts.retain(|start| now - start < WINDOW_MS);
        self.starts.push(now);
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|_| invalid_state())?;
        serde_json::to_writer(
            &mut temporary,
            &State {
                version: 1,
                starts: self.starts.clone(),
            },
        )
        .map_err(|_| invalid_state())?;
        temporary.flush().map_err(|_| invalid_state())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| invalid_state())?;
        temporary
            .persist(self.root.join("traffic.json"))
            .map_err(|_| invalid_state())?;
        self.requests += 1;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[derive(Clone)]
    pub(crate) struct FakeClock(pub Rc<Cell<u64>>);

    impl Default for FakeClock {
        fn default() -> Self {
            Self(Rc::new(Cell::new(100000)))
        }
    }

    impl Clock for FakeClock {
        fn now_ms(&self) -> Result<u64, AppError> {
            Ok(self.0.get())
        }
        fn sleep(&self, milliseconds: u64) {
            self.0.set(self.0.get() + milliseconds);
        }
    }

    #[test]
    fn spaces_requests_and_enforces_per_run_and_cross_run_budget() {
        let root = tempfile::tempdir().unwrap();
        let clock = FakeClock::default();
        let mut guard = TrafficGuard::acquire(root.path(), clock.clone()).unwrap();
        for _ in 0..MAX_REQUESTS {
            guard.reserve().unwrap();
        }
        assert_eq!(clock.0.get(), 108000);
        assert_eq!(
            guard.reserve().unwrap_err().code,
            "traffic_budget_exhausted"
        );
        let state: State =
            serde_json::from_slice(&std::fs::read(root.path().join("traffic.json")).unwrap())
                .unwrap();
        assert!(
            state
                .starts
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= MIN_INTERVAL_MS)
        );
        drop(guard);
        assert!(
            matches!(TrafficGuard::acquire(root.path(), clock.clone()), Err(error) if error.code == "traffic_budget_exhausted")
        );
        clock.0.set(161000);
        assert!(TrafficGuard::acquire(root.path(), clock).is_ok());
    }

    #[test]
    fn cannot_reset_per_run_limit_by_waiting_out_the_window() {
        let root = tempfile::tempdir().unwrap();
        let clock = FakeClock::default();
        let mut guard = TrafficGuard::acquire(root.path(), clock.clone()).unwrap();
        for _ in 0..MAX_REQUESTS {
            guard.reserve().unwrap();
            clock.0.set(clock.0.get() + WINDOW_MS);
        }
        assert_eq!(
            guard.reserve().unwrap_err().code,
            "traffic_budget_exhausted"
        );
    }

    #[test]
    fn lock_excludes_other_processes_and_is_not_taken_over() {
        let root = tempfile::tempdir().unwrap();
        let guard = TrafficGuard::acquire(root.path(), FakeClock::default()).unwrap();
        let before = std::fs::read(root.path().join("live.lock")).unwrap();
        assert!(
            matches!(TrafficGuard::acquire(root.path(), FakeClock::default()), Err(error) if error.code == "live_probe_busy")
        );
        assert_eq!(
            std::fs::read(root.path().join("live.lock")).unwrap(),
            before
        );
        drop(guard);
        assert!(!root.path().join("live.lock").exists());
    }

    #[test]
    fn corrupt_future_and_oversized_state_fail_without_reset() {
        for bytes in [
            b"not-json".to_vec(),
            br#"{"version":1,"starts":[200000]}"#.to_vec(),
            vec![b' '; 8193],
        ] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("traffic.json"), &bytes).unwrap();
            assert!(
                matches!(TrafficGuard::acquire(root.path(), FakeClock::default()), Err(error) if error.code == "traffic_state_invalid")
            );
            assert_eq!(
                std::fs::read(root.path().join("traffic.json")).unwrap(),
                bytes
            );
            assert!(!root.path().join("live.lock").exists());
        }
    }

    #[test]
    fn backward_clock_stops_before_new_reservation() {
        let root = tempfile::tempdir().unwrap();
        let clock = FakeClock::default();
        let mut guard = TrafficGuard::acquire(root.path(), clock.clone()).unwrap();
        guard.reserve().unwrap();
        clock.0.set(99999);
        assert_eq!(guard.reserve().unwrap_err().code, "traffic_state_invalid");
    }

    #[test]
    fn separate_invocations_share_minimum_spacing() {
        let root = tempfile::tempdir().unwrap();
        let clock = FakeClock::default();
        {
            let mut guard = TrafficGuard::acquire(root.path(), clock.clone()).unwrap();
            guard.reserve().unwrap();
        }
        let mut guard = TrafficGuard::acquire(root.path(), clock.clone()).unwrap();
        guard.reserve().unwrap();
        assert_eq!(clock.0.get(), 102000);
    }
}
