use anyhow::{Context, Result, anyhow, bail};
use std::path::Path;
use tracing::{info, warn};

use crate::cli::{Cli, RunArgs};
use crate::config::Config;
use crate::db::{Db, State};
use crate::paths;
use crate::sign::{Platform, is_release_archive, linux};

pub async fn run(cli: &Cli, args: RunArgs) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    sign_one(&cfg, args.run_id).await
}

/// Sign all per-HOST release archives for a verified run. Linux tarballs
/// get a detached GPG signature (Phase 5). Windows/macOS signers land in
/// Phases 6 and 7; until then they're skipped with a warning and the
/// build's state is held at `verified` so the publisher won't pick it up.
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
    let mut deferred_mingw = 0u32;
    let mut deferred_darwin = 0u32;
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
            &mut deferred_mingw,
            &mut deferred_darwin,
            &mut unknown,
        ) {
            Ok(()) => {}
            Err(e) => {
                db.set_state(run_id, State::Failed, Some(&e.to_string()))?;
                return Err(e);
            }
        }
    }

    let deferred_total = deferred_mingw + deferred_darwin;
    if deferred_total > 0 {
        warn!(
            signed_linux,
            deferred_mingw,
            deferred_darwin,
            unknown,
            "sign partial: non-linux signers not implemented yet (phases 6 + 7); leaving state=verified"
        );
    } else {
        db.set_state(run_id, State::Signed, None)?;
        info!(signed_linux, unknown, "sign complete");
    }
    Ok(())
}

fn sign_one_dir(
    dir: &Path,
    cfg: &Config,
    signed_linux: &mut u32,
    deferred_mingw: &mut u32,
    deferred_darwin: &mut u32,
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
                warn!(file = %path.display(), "TODO: phase 6 — windows osslsigncode not implemented");
                *deferred_mingw += 1;
            }
            Platform::Darwin => {
                warn!(file = %path.display(), "TODO: phase 7 — darwin codesign + notarytool not implemented");
                *deferred_darwin += 1;
            }
            Platform::Unknown => {
                warn!(file = %path.display(), "unknown platform; skipping");
                *unknown += 1;
            }
        }
    }
    Ok(())
}
