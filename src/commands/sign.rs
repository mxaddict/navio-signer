use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use tracing::{info, warn};

use crate::cli::{Cli, RunArgs};
use crate::config::Config;
use crate::db::{Db, State};
use crate::paths;
use crate::sign::{Platform, darwin, is_release_archive, linux, windows};

pub async fn run(cli: &Cli, args: RunArgs) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    sign_one(&cfg, args.run_id).await
}

/// Sign all per-HOST release archives for a verified run.
///
/// - Linux tarballs: detached GPG signature alongside the tarball (`.asc`).
/// - Windows zip: every `.exe`/`.dll` inside gets an Authenticode signature
///   via osslsigncode, then the zip is repacked deterministically.
/// - Darwin tarball: every Mach-O binary gets `codesign --options runtime
///   --timestamp --force`, then the bundle is notarized via `xcrun
///   notarytool submit --wait`, and the tarball is repacked deterministically.
pub async fn sign_one(cfg: &Config, run_id: u64) -> Result<()> {
    let db = Db::open(&paths::data_dir(cfg)?)?;
    let build = db
        .get(run_id)?
        .ok_or_else(|| anyhow!("no build row for run_id {run_id}"))?;

    match build.state {
        State::Discovered | State::Fetched => {
            bail!(
                "run {run_id} not yet verified (state={})",
                build.state.as_str()
            )
        }
        State::Signed | State::Published => {
            info!(
                run_id,
                state = build.state.as_str(),
                "skipping sign (already past)"
            );
            return Ok(());
        }
        State::Failed => bail!("run {run_id} is in failed state; reset manually before retrying"),
        State::Verified => {}
    }

    let workdir = paths::workdir_for(cfg, run_id)?;
    if !workdir.exists() {
        bail!("workdir missing: {}", workdir.display());
    }

    let mut signed_linux = 0u32;
    let mut signed_mingw = 0u32;
    let mut signed_darwin = 0u32;
    let mut unknown = 0u32;

    for entry in std::fs::read_dir(&workdir)
        .with_context(|| format!("reading workdir {}", workdir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        match sign_one_dir(
            &dir,
            cfg,
            &mut signed_linux,
            &mut signed_mingw,
            &mut signed_darwin,
            &mut unknown,
        ) {
            Ok(()) => {}
            Err(e) => {
                db.set_state(run_id, State::Failed, Some(&e.to_string()))?;
                return Err(e);
            }
        }
    }

    db.set_state(run_id, State::Signed, None)?;
    info!(
        signed_linux,
        signed_mingw, signed_darwin, unknown, "sign complete"
    );
    Ok(())
}

fn sign_one_dir(
    dir: &Path,
    cfg: &Config,
    signed_linux: &mut u32,
    signed_mingw: &mut u32,
    signed_darwin: &mut u32,
    unknown: &mut u32,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_release_archive(name) {
            continue;
        }
        match Platform::from_filename(name) {
            Platform::Linux => {
                linux::detach_sign(&path, &cfg.signing.linux)?;
                *signed_linux += 1;
            }
            Platform::Mingw => {
                windows::sign_zip(&path, &cfg.signing.windows)?;
                *signed_mingw += 1;
            }
            Platform::Darwin => {
                darwin::sign_tarball(&path, &cfg.signing.macos)?;
                *signed_darwin += 1;
            }
            Platform::Unknown => {
                warn!(file = %path.display(), "unknown platform; skipping");
                *unknown += 1;
            }
        }
    }
    Ok(())
}
