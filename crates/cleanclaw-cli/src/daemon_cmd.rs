//! `cleanclaw daemon …` — start / stop / status / install / uninstall
//! / reload / restart / logs / run.
//!
//! The CLI is a
//! thin shell wrapper around `cleanclaw-daemon`; the actual launchd
//! / systemd unit generation lives in `cleanclaw-daemon::install*`.

use clap::Subcommand;
use cleanclaw_core::Result;

#[derive(Subcommand)]
pub enum DaemonCmd {
    /// Print the daemon status (pid + running).
    Status,
    /// Start the gateway (foreground; `--detach` for background).
    Start {
        #[arg(long)]
        detach: bool,
    },
    /// Send SIGTERM to the running gateway.
    Stop,
    /// Send SIGHUP to the running gateway (hot reload).
    Reload,
    /// Stop + Start.
    Restart,
    /// Tail the daemon's log file.
    Logs {
        #[arg(long, default_value_t = 50)]
        lines: usize,
    },
    /// Install the daemon as a launchd / systemd service.
    Install,
    /// Uninstall the daemon service.
    Uninstall,
    /// Run the daemon in the foreground (internal, hidden).
    Run,
}

pub async fn run(cmd: DaemonCmd) -> Result<()> {
    match cmd {
        DaemonCmd::Status => status(),
        DaemonCmd::Start { detach } => start(detach),
        DaemonCmd::Stop => stop(),
        DaemonCmd::Reload => reload(),
        DaemonCmd::Restart => restart(),
        DaemonCmd::Logs { lines } => logs(lines),
        DaemonCmd::Install => install(),
        DaemonCmd::Uninstall => uninstall(),
        DaemonCmd::Run => run_foreground().await,
    }
}

fn status() -> Result<()> {
    match cleanclaw_daemon::get_status() {
        Ok(s) if s.running => {
            println!("running (pid={})", s.pid);
            if let Some(up) = s.uptime {
                println!("uptime:   {}s", up.as_secs());
            }
            Ok(())
        }
        _ => {
            println!("stopped");
            std::process::exit(1);
        }
    }
}

fn start(detach: bool) -> Result<()> {
    if cleanclaw_daemon::is_running() {
        return Err(cleanclaw_core::CleanClawError::Conflict(
            "daemon already running".into(),
        ));
    }
    if detach {
        // Re-exec ourselves with the hidden `daemon run` subcommand.
        let exe = std::env::current_exe()?;
        let pid = std::process::Command::new(exe)
            .args(["daemon", "run"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("spawn: {e}")))?;
        println!("started (pid={})", pid.id());
        Ok(())
    } else {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("rt: {e}")))?;
        rt.block_on(run_foreground())
    }
}

fn stop() -> Result<()> {
    let status = cleanclaw_daemon::get_status().map_err(de)?;
    if !status.running {
        return Err(cleanclaw_core::CleanClawError::NotFound(
            "no running daemon".into(),
        ));
    }
    cleanclaw_daemon::stop().map_err(de)?;
    println!("stopped");
    Ok(())
}

fn reload() -> Result<()> {
    let status = cleanclaw_daemon::get_status().map_err(de)?;
    if !status.running {
        return Err(cleanclaw_core::CleanClawError::NotFound(
            "no running daemon".into(),
        ));
    }
    cleanclaw_daemon::signal_reload(status.pid).map_err(de)?;
    println!("reload signaled");
    Ok(())
}

fn restart() -> Result<()> {
    if cleanclaw_daemon::is_running() {
        cleanclaw_daemon::stop().ok();
        for _ in 0..50 {
            if !cleanclaw_daemon::is_running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    start(true)
}

fn logs(lines: usize) -> Result<()> {
    let path = cleanclaw_daemon::log_path();
    if !path.exists() {
        println!("(no log file at {})", path.display());
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let tail: Vec<&str> = raw.lines().rev().take(lines).collect();
    for line in tail.iter().rev() {
        println!("{line}");
    }
    Ok(())
}

fn install() -> Result<()> {
    let exe = std::env::current_exe()
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("current_exe: {e}")))?;
    cleanclaw_daemon::install(&exe).map_err(ioe)?;
    println!("installed");
    Ok(())
}

fn uninstall() -> Result<()> {
    cleanclaw_daemon::uninstall().map_err(ioe)?;
    println!("uninstalled");
    Ok(())
}

fn ioe(e: std::io::Error) -> cleanclaw_core::CleanClawError {
    cleanclaw_core::CleanClawError::Internal(format!("io: {e}"))
}

fn de(e: cleanclaw_daemon::DaemonError) -> cleanclaw_core::CleanClawError {
    use cleanclaw_daemon::DaemonError::*;
    match e {
        Io(io) => cleanclaw_core::CleanClawError::Internal(format!("io: {io}")),
        AlreadyRunning(pid) => {
            cleanclaw_core::CleanClawError::Conflict(format!("already running: {pid}"))
        }
        NotRunning => cleanclaw_core::CleanClawError::NotFound("not running".into()),
        StalePid => cleanclaw_core::CleanClawError::Conflict("stale pid file".into()),
        SignalFailed(msg) => cleanclaw_core::CleanClawError::Internal(format!("signal: {msg}")),
    }
}

async fn run_foreground() -> Result<()> {
    let env = cleanclaw_config::load_env();
    let port: u16 = std::env::var("CLEANCLAW_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(18953);
    let gw = cleanclaw_gateway::Gateway::boot(env, port)
        .await
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("boot: {e}")))?;
    gw.run()
        .await
        .map_err(|e| cleanclaw_core::CleanClawError::Internal(format!("run: {e}")))?;
    Ok(())
}
