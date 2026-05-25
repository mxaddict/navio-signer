use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info};

use crate::cli::Cli;
use crate::commands::poll;
use crate::config::Config;
use crate::db::Db;
use crate::lockfile::LockFile;

pub async fn run(cli: &Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    let lock_path = runtime_lock_path(&cfg)?;
    let _lock = LockFile::acquire(&lock_path)?;
    info!(lock = %lock_path.display(), "lockfile acquired");

    let interval = Duration::from_secs(cfg.daemon.poll_interval_secs);
    info!(
        interval_secs = cfg.daemon.poll_interval_secs,
        "starting daemon loop"
    );

    loop {
        match poll::poll_once(&cfg).await {
            Ok(()) => {}
            Err(e) => error!(error = %e, "poll iteration failed; continuing"),
        }
        // TODO Phase 3+: also drive fetch/verify/sign/publish for any builds
        // in their respective pending states.
        tokio::time::sleep(interval).await;
    }
}

fn runtime_lock_path(cfg: &Config) -> Result<PathBuf> {
    let xdg = xdg::BaseDirectories::with_prefix("navio-signer");
    if let Ok(dir) = xdg.get_runtime_directory() {
        return Ok(dir.join("navio-signer.lock"));
    }
    // macOS: $XDG_RUNTIME_DIR usually isn't set. Fall back to data dir.
    let data_dir = Db::data_dir_for(cfg.paths.data_dir.as_deref())?;
    Ok(data_dir.join("runtime").join("navio-signer.lock"))
}
