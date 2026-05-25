use anyhow::{Context, Result, bail};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::warn;

/// PID-based lockfile. Holding `LockFile` keeps the lock; on drop the lockfile
/// is removed. If a stale lockfile is found (PID not running) it is removed
/// before retrying.
pub struct LockFile {
    path: PathBuf,
}

impl LockFile {
    pub fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating lock dir {}", parent.display()))?;
        }

        match Self::open_exclusive(path) {
            Ok(f) => {
                write_pid(f)?;
                Ok(Self {
                    path: path.to_path_buf(),
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing_pid = std::fs::read_to_string(path).unwrap_or_default();
                let pid: i32 = existing_pid.trim().parse().unwrap_or(-1);
                if pid > 0 && pid_alive(pid) {
                    bail!(
                        "lock held by PID {pid} at {} (another navio-signer instance running?)",
                        path.display()
                    );
                }
                warn!(
                    pid,
                    path = %path.display(),
                    "removing stale lockfile"
                );
                std::fs::remove_file(path)
                    .with_context(|| format!("removing stale lockfile {}", path.display()))?;
                let f = Self::open_exclusive(path)
                    .with_context(|| format!("re-acquiring lockfile {}", path.display()))?;
                write_pid(f)?;
                Ok(Self {
                    path: path.to_path_buf(),
                })
            }
            Err(e) => Err(e).with_context(|| format!("acquiring lockfile {}", path.display())),
        }
    }

    fn open_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_pid(mut f: std::fs::File) -> Result<()> {
    writeln!(f, "{}", std::process::id())?;
    Ok(())
}

/// Cheap liveness check: `kill -0 PID` exits 0 if the process exists and we
/// have permission to signal it. Single-operator signer box → permission is
/// always present for our own PIDs.
fn pid_alive(pid: i32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
