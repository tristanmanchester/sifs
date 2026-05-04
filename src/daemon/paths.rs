use anyhow::{Context, Result};
use std::path::PathBuf;

const SOCKET_ENV: &str = "SIFS_DAEMON_SOCKET";
const RUNTIME_DIR_ENV: &str = "SIFS_DAEMON_RUNTIME_DIR";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonPaths {
    pub runtime_dir: PathBuf,
    pub socket: PathBuf,
    pub pid_file: PathBuf,
    pub log_file: PathBuf,
}

impl DaemonPaths {
    pub fn ensure_runtime_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.runtime_dir)
            .with_context(|| format!("create daemon runtime dir {}", self.runtime_dir.display()))
    }
}

pub fn default_daemon_paths() -> Result<DaemonPaths> {
    let runtime_dir = if let Ok(path) = std::env::var(RUNTIME_DIR_ENV) {
        PathBuf::from(path)
    } else if let Ok(socket) = std::env::var(SOCKET_ENV) {
        PathBuf::from(socket)
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(default_runtime_dir)
    } else {
        default_runtime_dir()
    };
    let socket = std::env::var(SOCKET_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| runtime_dir.join("sifs.sock"));
    Ok(DaemonPaths {
        pid_file: runtime_dir.join("sifs.pid"),
        log_file: runtime_dir.join("sifs.log"),
        runtime_dir,
        socket,
    })
}

fn default_runtime_dir() -> PathBuf {
    let uid = current_uid();
    std::env::temp_dir().join(format!("sifs-{uid}"))
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}
