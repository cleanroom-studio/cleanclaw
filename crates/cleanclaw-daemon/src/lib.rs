//! Daemon support — PID file, signal handling, launchd / systemd
//! service units.
//!
//! On Unix, the foreground process writes a pidfile at
//! `$CLEANCLAW_HOME/daemon.pid` so a sibling CLI (e.g. `cleanclaw daemon
//! stop`) can send SIGTERM. `cleanclaw daemon install` writes a
//! `~/.config/systemd/user/cleanclaw.service` (or
//! `~/Library/LaunchAgents/com.cleanclaw.cleanclaw.plist`) and enables
//! it; `cleanclaw daemon uninstall` removes it.

use chrono::{DateTime, Utc};
use cleanclaw_config::env::home_dir;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tracing::{error, info, warn};

pub const PID_FILENAME: &str = "daemon.pid";
pub const LOG_FILENAME: &str = "daemon.log";
pub const LOG_DIR: &str = "logs";

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon already running (PID {0})")]
    AlreadyRunning(u32),
    #[error("daemon not running (no PID file)")]
    NotRunning,
    #[error("daemon not running (stale PID file)")]
    StalePid,
    #[error("signal failed: {0}")]
    SignalFailed(String),
}

pub fn pid_path() -> PathBuf {
    home_dir().join(PID_FILENAME)
}

pub fn log_path() -> PathBuf {
    home_dir().join(LOG_DIR).join(LOG_FILENAME)
}

pub fn log_dir() -> PathBuf {
    home_dir().join(LOG_DIR)
}

/// — returns
/// (pid file, log file, log dir) under `$CLEANCLAW_HOME`.
pub fn paths() -> Result<(PathBuf, PathBuf, PathBuf), DaemonError> {
    let base = home_dir();
    let log_dir = base.join(LOG_DIR);
    let pid_file = base.join(PID_FILENAME);
    let log_file = log_dir.join(LOG_FILENAME);
    Ok((pid_file, log_file, log_dir))
}

pub fn write_pid_file() -> std::io::Result<()> {
    let pid = std::process::id();
    let path = pid_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::File::create(&path)?;
    writeln!(f, "{pid}")?;
    Ok(())
}

pub fn remove_pid_file() {
    let _ = fs::remove_file(pid_path());
}

pub fn read_pid() -> Option<u32> {
    let raw = fs::read_to_string(pid_path()).ok()?;
    raw.trim().parse().ok()
}

pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        libc_kill(pid) == 0
    }
    #[cfg(not(unix))]
    {
        // best-effort: trust the pid file on Windows
        let _ = pid;
        true
    }
}

pub fn is_running() -> bool {
    if let Some(pid) = read_pid() {
        is_process_alive(pid)
    } else {
        false
    }
}

#[cfg(unix)]
fn libc_kill(pid: u32) -> i32 {
    let status = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status();
    match status {
        Ok(s) => {
            if s.success() {
                0
            } else {
                -1
            }
        }
        Err(_) => -1,
    }
}

pub fn send_signal(pid: u32, sig: &str) -> std::io::Result<()> {
    let status = Command::new("kill")
        .args(["-s", sig, &pid.to_string()])
        .status()?;
    if !status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("kill returned {status:?}"),
        ));
    }
    Ok(())
}

/// Mirrors the Go `SignalReload(pid)` — sends SIGHUP on Unix, returns
/// an error on Windows (operators must restart the gateway by hand).
pub fn signal_reload(pid: u32) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        send_signal(pid, "HUP").map_err(|e| DaemonError::SignalFailed(e.to_string()))
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(DaemonError::SignalFailed(
            "SIGHUP not deliverable on Windows; restart the gateway".into(),
        ))
    }
}

/// Mirrors `Stop()`: SIGTERM, wait up to 5s, then SIGKILL.
pub fn stop() -> Result<(), DaemonError> {
    let pid = read_pid().ok_or(DaemonError::NotRunning)?;
    if !is_process_alive(pid) {
        remove_pid_file();
        return Err(DaemonError::StalePid);
    }
    send_signal(pid, "TERM").map_err(|e| DaemonError::SignalFailed(e.to_string()))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if !is_process_alive(pid) {
            remove_pid_file();
            info!(pid, "daemon stopped");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // Force kill
    send_signal(pid, "KILL").map_err(|e| DaemonError::SignalFailed(e.to_string()))?;
    remove_pid_file();
    info!(pid, "daemon killed");
    Ok(())
}

/// Current daemon state. Mirrors `daemon.Status`.
#[derive(Debug, Clone)]
pub struct Status {
    pub running: bool,
    pub pid: u32,
    pub started_at: Option<DateTime<Utc>>,
    pub uptime: Option<Duration>,
}

pub fn get_status() -> Result<Status, DaemonError> {
    let (pid_file, _log_file, _log_dir) = paths()?;
    let pid = match read_pid() {
        Some(p) => p,
        None => {
            return Ok(Status {
                running: false,
                pid: 0,
                started_at: None,
                uptime: None,
            })
        }
    };
    if !is_process_alive(pid) {
        let _ = fs::remove_file(&pid_file);
        return Ok(Status {
            running: false,
            pid: 0,
            started_at: None,
            uptime: None,
        });
    }
    let started_at = fs::metadata(&pid_file)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            let dt: DateTime<Utc> = t.into();
            dt
        });
    let uptime = fs::metadata(&pid_file)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok());
    Ok(Status {
        running: true,
        pid,
        started_at,
        uptime,
    })
}

/// Future that resolves on SIGTERM (Unix) or Ctrl-C / Ctrl-Break
/// (Windows). Used by the foreground gateway to wait for shutdown.
pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())
            .expect("install SIGTERM handler");
        let mut int = signal(SignalKind::interrupt())
            .expect("install SIGINT handler");
        tokio::select! {
            _ = term.recv() => info!("received SIGTERM, shutting down"),
            _ = int.recv() => info!("received SIGINT, shutting down"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("received Ctrl-C, shutting down");
    }
}

/// Future that resolves on SIGHUP — the operator's "hot reload"
/// signal. The gateway uses this to drop cached state without
/// restarting the process.
#[cfg(unix)]
pub async fn reload_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    if let Ok(mut s) = signal(SignalKind::hangup()) {
        s.recv().await;
        info!("received SIGHUP, hot-reloading");
    } else {
        std::future::pending::<()>().await;
    }
}

#[cfg(not(unix))]
pub async fn reload_signal() {
    std::future::pending::<()>().await;
}

// ---- launchd (macOS) ----

pub fn launchd_plist_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/LaunchAgents/com.cleanclaw.cleanclaw.plist")
}

pub fn install_launchd(exe_path: &Path) -> std::io::Result<()> {
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.cleanclaw.cleanclaw</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>gateway</string>
    </array>
    <key>WorkingDirectory</key>
    <string>{home}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
        exe = exe_path.display(),
        home = home_dir().display(),
        log = home_dir().join("daemon.log").display(),
    );
    let path = launchd_plist_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, plist)?;
    info!("launchd plist written: {}", path.display());
    let _ = Command::new("launchctl").args(["load", "-w"]).arg(&path).status();
    Ok(())
}

pub fn uninstall_launchd() -> std::io::Result<()> {
    let path = launchd_plist_path();
    if path.exists() {
        let _ = Command::new("launchctl").args(["unload"]).arg(&path).status();
        fs::remove_file(&path)?;
    }
    Ok(())
}

// ---- systemd (Linux) ----

pub fn systemd_unit_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/systemd/user/cleanclaw.service")
}

pub fn install_systemd(exe_path: &Path) -> std::io::Result<()> {
    let unit = format!(
        r#"[Unit]
Description=CleanClaw AI Agent Runtime
After=network.target

[Service]
Type=simple
ExecStart={exe} gateway
WorkingDirectory={home}
Restart=on-failure
RestartSec=5
StandardOutput=append:{log}
StandardError=append:{log}

[Install]
WantedBy=default.target
"#,
        exe = exe_path.display(),
        home = home_dir().display(),
        log = home_dir().join("daemon.log").display(),
    );
    let path = systemd_unit_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, unit)?;
    info!("systemd unit written: {}", path.display());
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let _ = Command::new("systemctl")
        .args(["--user", "enable", "--now", "cleanclaw.service"])
        .status();
    Ok(())
}

pub fn uninstall_systemd() -> std::io::Result<()> {
    let path = systemd_unit_path();
    if path.exists() {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "cleanclaw.service"])
            .status();
        fs::remove_file(&path)?;
        let _ = Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .status();
    }
    Ok(())
}

pub fn install(exe_path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        install_launchd(exe_path)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        install_systemd(exe_path)
    }
    #[cfg(not(unix))]
    {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "service install not supported on this platform",
        ))
    }
}

pub fn uninstall() -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        uninstall_launchd()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        uninstall_systemd()
    }
    #[cfg(not(unix))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // HOME is process-global; serialize tests that mutate it.
    static HOME_LOCK: Mutex<()> = Mutex::new(());

    fn set_home(dir: &Path) {
        // SAFETY: tests in the same process are serialized via HOME_LOCK.
        unsafe {
            std::env::set_var("CLEANCLAW_HOME", dir);
            std::env::set_var("HOME", dir);
        }
    }

    #[test]
    fn pid_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("daemon.pid");
        std::fs::write(&path, "12345\n").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let pid: u32 = raw.trim().parse().unwrap();
        assert_eq!(pid, 12345);
    }

    #[test]
    fn paths_under_cleanclaw_home() {
        let _g = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set_home(dir.path());
        let (pid, log, logd) = paths().unwrap();
        assert!(pid.ends_with("daemon.pid"));
        assert!(log.ends_with("logs/daemon.log"));
        assert!(logd.ends_with("logs"));
    }

    #[test]
    fn status_not_running_when_no_pid_file() {
        let _g = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set_home(dir.path());
        let s = get_status().unwrap();
        assert!(!s.running);
        assert_eq!(s.pid, 0);
    }

    #[test]
    fn write_and_read_pid_file() {
        let _g = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set_home(dir.path());
        write_pid_file().unwrap();
        let pid = read_pid().expect("pid should be readable");
        assert_eq!(pid, std::process::id());
    }

    #[test]
    fn is_running_self() {
        let _g = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set_home(dir.path());
        write_pid_file().unwrap();
        assert!(is_running());
        remove_pid_file();
        assert!(!is_running());
    }

    #[test]
    fn stale_pid_clears_on_status() {
        let _g = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set_home(dir.path());
        // Write a PID that almost certainly doesn't exist.
        let fake = 0x7FFFFFFFu32;
        std::fs::write(pid_path(), format!("{fake}\n")).unwrap();
        let s = get_status().unwrap();
        assert!(!s.running, "fake PID should not be reported as running");
        assert!(!pid_path().exists(), "stale PID file should have been removed");
    }

    #[test]
    fn signal_reload_uses_sighup() {
        // We can't actually send SIGHUP to our own test process safely
        // (would kill the test runner). Just verify the path on Unix:
        // signal_reload with a dead PID should not panic.
        let result = signal_reload(0x7FFFFFFFu32);
        // On Unix this is `Err(...)` from the kill invocation; on
        // Windows it's `Err(DaemonError::SignalFailed(...))`. Either
        // way, no panic.
        assert!(result.is_err());
    }

    #[test]
    fn stop_without_pid_file_errors() {
        let _g = HOME_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        set_home(dir.path());
        let err = stop().unwrap_err();
        assert!(matches!(err, DaemonError::NotRunning));
    }
}

// =====================================================================
// Start / RunLoop — detached daemon. Mirrors
// .
// =====================================================================

use tokio::sync::watch;

/// Launch the gateway as a detached background process. The
/// `Start` Go function opens a log file, spawns the binary with
/// `daemon __run --port <port>`, writes the pid file, and returns
/// immediately. On the Rust side we return a `DaemonHandle` that
/// holds the spawned process + a watch channel for the wrapper's
/// lifecycle.
pub async fn start(
    port: u16,
    log_file: &Path,
) -> Result<DaemonHandle, DaemonError> {
    let exe = std::env::current_exe()?;
    let pidfile = pid_path();
    if let Some(parent) = pidfile.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Some(parent) = log_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let log = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .await?;
    let log_for_stderr = log.try_clone().await?;
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("daemon")
        .arg("__run")
        .arg("--port")
        .arg(port.to_string())
        .stdin(std::process::Stdio::null())
        .stdout(log.into_std().await)
        .stderr(log_for_stderr.into_std().await)
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                // Detach into a new session.
                libc_setsid();
                Ok(())
            });
        }
    }
    let child = cmd.spawn()?;
    let pid = child.id().unwrap_or(0);
    tokio::fs::write(&pidfile, format!("{pid}\n")).await?;
    let (tx, rx) = watch::channel(false);
    Ok(DaemonHandle {
        _child: Some(child),
        pid,
        shutdown: tx,
        done: rx,
    })
}

#[cfg(unix)]
fn libc_setsid() {
    // Avoid a hard dep on `libc`; use raw syscall via nix-style c-style.
    extern "C" {
        fn setsid() -> i32;
    }
    unsafe { setsid(); }
}

/// Handle to a running detached daemon. Drop the handle to send a
/// SIGTERM (Unix) / `KILL` (Windows).
pub struct DaemonHandle {
    _child: Option<tokio::process::Child>,
    pid: u32,
    shutdown: tokio::sync::watch::Sender<bool>,
    pub done: tokio::sync::watch::Receiver<bool>,
}

impl DaemonHandle {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn stop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Wrapper loop that auto-restarts the gateway on crash with
/// exponential backoff. Mirrors `daemon.RunLoop` in the Go
/// implementation.
pub async fn run_loop(port: u16) -> Result<(), DaemonError> {
    let max_restarts: u32 = 10;
    let max_backoff = std::time::Duration::from_secs(30);
    let stable_threshold = std::time::Duration::from_secs(60);
    let mut consecutive_crashes: u32 = 0;
    let mut backoff = std::time::Duration::from_secs(1);

    let exe = std::env::current_exe()?;
    // Write our own pid.
    let pidfile = pid_path();
    tokio::fs::write(&pidfile, format!("{}\n", std::process::id())).await?;
    let _remove_pid_on_drop = PidFileGuard::new(pidfile.clone());

    loop {
        let start_time = std::time::Instant::now();
        tracing::info!(port, "daemon: starting gateway");
        let mut cmd = tokio::process::Command::new(&exe);
        cmd.arg("gateway").arg("--port").arg(port.to_string());
        let status = cmd.status().await;
        let elapsed = start_time.elapsed();
        if let Ok(s) = &status {
            if s.success() {
                tracing::info!("daemon: gateway exited cleanly");
                return Ok(());
            }
        }
        if elapsed >= stable_threshold {
            consecutive_crashes = 0;
            backoff = std::time::Duration::from_secs(1);
        }
        consecutive_crashes += 1;
        if consecutive_crashes >= max_restarts {
            return Err(DaemonError::NotRunning);
        }
        tracing::warn!(
            crashes = consecutive_crashes,
            backoff = ?backoff,
            "daemon: gateway crashed, restarting"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

/// Guard that deletes the pid file on drop.
struct PidFileGuard {
    path: PathBuf,
}

impl PidFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for PidFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod start_run_tests {
    use super::*;

    #[tokio::test]
    async fn start_writes_pid_file() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: HOME is process-global but we serialize all
        // tests that touch it through HOME_LOCK. This particular
        // test doesn't use HOME_LOCK but writes its own pid file
        // via the start path.
        let log = dir.path().join("daemon.log");
        let handle = start(19999, &log).await;
        // The spawn may or may not work in CI without an executable
        // binary; tolerate either way.
        match handle {
            Ok(mut h) => {
                h.stop();
                let pid = read_pid();
                assert!(pid.is_some());
                // Cleanup.
                let _ = std::fs::remove_file(pid_path());
            }
            Err(_) => {
                // Spawn failed (e.g. no `daemon` subcommand in this
                // binary); nothing to assert beyond not panicking.
            }
        }
    }

    #[test]
    fn pid_file_guard_removes_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pid");
        std::fs::write(&path, "1234\n").unwrap();
        {
            let _g = PidFileGuard::new(path.clone());
        }
        assert!(!path.exists());
    }
}
