use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::info;

use crate::config::LinuxSigningConfig;
use crate::sign::append_extension;

/// Produce a detached, ASCII-armored GPG signature for `tarball`. The
/// sidecar is written to `<tarball>.asc`. Idempotent: a pre-existing
/// sidecar is overwritten so re-runs are safe.
///
/// Relies on `gpg-agent` to supply the passphrase; the daemon is meant to
/// run interactively under launchd / a maintainer login session where the
/// agent is reachable. If `--batch` finds no passphrase cached and the key
/// is passphrase-protected, gpg will refuse and we return an error rather
/// than blocking on a TTY.
pub fn detach_sign(tarball: &Path, cfg: &LinuxSigningConfig) -> Result<PathBuf> {
    if !tarball.is_file() {
        bail!("not a file: {}", tarball.display());
    }
    let asc = append_extension(tarball, "asc");
    if asc.exists() {
        std::fs::remove_file(&asc)
            .with_context(|| format!("removing stale sidecar {}", asc.display()))?;
    }

    info!(
        file = %tarball.display(),
        key = cfg.gpg_key_id.as_str(),
        "gpg --detach-sign"
    );

    let status = Command::new("gpg")
        .arg("--batch")
        .arg("--yes")
        .arg("--detach-sign")
        .arg("--armor")
        .arg("--local-user")
        .arg(&cfg.gpg_key_id)
        .arg("--output")
        .arg(&asc)
        .arg(tarball)
        .status()
        .with_context(|| format!("running gpg for {}", tarball.display()))?;

    if !status.success() {
        bail!(
            "gpg exited {} for {} (passphrase cached in gpg-agent?)",
            status.code().unwrap_or(-1),
            tarball.display()
        );
    }
    Ok(asc)
}
