//! Managed temporary directories for workspace tests.
//!
//! Directories live under `<target>/tmp/komga-rust-test/p<pid>-<rand>` rather than the system temp
//! directory so `cargo clean` reclaims them, antivirus exclusions on the workspace cover them, and
//! Windows transient file locks (e.g. background SQLite pool closes or antivirus scans) are retried
//! on drop instead of silently leaking directories.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};
use std::thread;
use std::time::Duration;

static SWEEP_ONCE: Once = Once::new();
static ATEXIT_ONCE: Once = Once::new();
static DEFERRED_REMOVALS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Base directory holding all managed test directories and caches (`<target>/tmp/komga-rust-test`).
#[must_use]
pub fn root() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("komga-test-support lives under <workspace>/crates/")
                .join("target")
        },
        PathBuf::from,
    );
    target.join("tmp").join("komga-rust-test")
}

/// A temporary directory under [`root`] that retries removal on drop and sweeps orphaned
/// directories from exited test processes.
#[derive(Debug)]
pub struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// Creates a new temporary directory under [`root`].
    ///
    /// # Errors
    /// Returns any filesystem error creating [`root`] or the unique directory inside it.
    pub fn new() -> io::Result<Self> {
        let base = root();
        fs::create_dir_all(&base)?;
        SWEEP_ONCE.call_once(|| sweep_orphans(&base));
        let prefix = format!("p{}-", std::process::id());
        let dir = tempfile::Builder::new().prefix(&prefix).tempdir_in(&base)?;
        Ok(Self { path: dir.keep() })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if remove_dir_with_retries(&self.path, 5).is_err() {
            // On a single-threaded `#[tokio::test]` runtime, `sqlx::Pool::drop` schedules
            // connection closes onto the very runtime thread blocked inside `Drop`, so retries here
            // cannot let them run. Deferring cleanup to `atexit` removes the directory right after
            // the runtime shuts down and releases every handle.
            defer_removal(self.path.clone());
        }
    }
}

/// Convenience drop-in replacement for `tempfile::tempdir()`.
///
/// # Errors
/// As [`TestDir::new`].
pub fn tempdir() -> io::Result<TestDir> {
    TestDir::new()
}

fn defer_removal(path: PathBuf) {
    ATEXIT_ONCE.call_once(|| {
        unsafe extern "C" {
            fn atexit(cb: extern "C" fn()) -> std::ffi::c_int;
        }
        // SAFETY: `cleanup_deferred` is a plain `extern "C" fn()` with no unwinds.
        unsafe {
            let _ = atexit(cleanup_deferred);
        }
    });
    if let Ok(mut pending) = DEFERRED_REMOVALS.lock() {
        pending.push(path);
    }
}

extern "C" fn cleanup_deferred() {
    let paths = DEFERRED_REMOVALS
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default();
    for path in paths {
        if let Err(error) = remove_dir_with_retries(&path, 10) {
            eprintln!(
                "komga-test-support: failed to remove {}: {error}",
                path.display()
            );
        }
    }
}

/// Retries `remove_dir_all` briefly to wait out async handle closes (such as `sqlx-sqlite`
/// connection worker threads) and transient Windows filesystem filter locks.
fn remove_dir_with_retries(path: &Path, max_retries: u32) -> io::Result<()> {
    let mut delay = Duration::from_millis(2);
    let mut attempts = 0;
    loop {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) if attempts >= max_retries => return Err(error),
            Err(_) => {
                attempts += 1;
                thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(40));
            }
        }
    }
}

/// Removes directories left behind by terminated test processes (for example after a forced kill
/// or a drop-time lock). Entries without a `p<pid>-` prefix (such as `templates`) are untouched.
fn sweep_orphans(base: &Path) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    let current_pid = std::process::id();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid) = parse_pid_prefix(name) else {
            continue;
        };
        if pid != current_pid && !is_process_alive(pid) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn parse_pid_prefix(name: &str) -> Option<u32> {
    let rest = name.strip_prefix('p')?;
    let (digits, _) = rest.split_once('-')?;
    digits.parse().ok()
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    const STILL_ACTIVE: u32 = 259;

    // SAFETY: `OpenProcess`, `GetExitCodeProcess`, and `CloseHandle` are Win32 APIs called with
    // valid arguments and the returned handle is closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &raw mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return true;
    };
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    // SAFETY: `kill(pid, 0)` performs existence/permission check without sending a signal.
    let result = unsafe { kill(pid, 0) };
    if result == 0 {
        return true;
    }
    // EPERM (process exists but owned by another user) means alive; ESRCH means gone.
    io::Error::last_os_error().raw_os_error() != Some(3)
}

#[cfg(not(any(windows, unix)))]
fn is_process_alive(_pid: u32) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_under_target_tmp_and_cleans_up_on_drop() {
        let dir = tempdir().expect("create TestDir");
        let path = dir.path().to_path_buf();
        assert!(path.starts_with(root()), "{}", path.display());
        assert!(path.is_dir());
        fs::write(path.join("probe.txt"), b"ok").expect("write probe");
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn parse_pid_prefix_ignores_non_pid_entries() {
        assert_eq!(parse_pid_prefix("p12345-abc"), Some(12345));
        assert_eq!(parse_pid_prefix("templates"), None);
        assert_eq!(parse_pid_prefix("pabc-123"), None);
    }

    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }
}
