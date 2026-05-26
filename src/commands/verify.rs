use anyhow::{Context, Result, anyhow, bail};
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, warn};

use crate::cli::{Cli, RunArgs};
use crate::config::Config;
use crate::db::{Db, State};
use crate::paths;

pub async fn run(cli: &Cli, args: RunArgs) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    verify_one(&cfg, args.run_id).await
}

/// Verify SHA256SUMS.part for each per-HOST artifact subdir under the run's
/// workdir. Each subdir is required to contain a SHA256SUMS.part manifest;
/// every other file in the subdir must appear in it with a matching SHA-256.
pub async fn verify_one(cfg: &Config, run_id: u64) -> Result<()> {
    let db = Db::open(&paths::data_dir(cfg)?)?;
    let build = db
        .get(run_id)?
        .ok_or_else(|| anyhow!("no build row for run_id {run_id}"))?;

    match build.state {
        State::Discovered => bail!("run {run_id} not yet fetched"),
        State::Verified | State::Signed | State::Published => {
            info!(
                run_id,
                state = build.state.as_str(),
                "skipping verify (already past)"
            );
            return Ok(());
        }
        State::Failed => bail!("run {run_id} is in failed state; reset manually before retrying"),
        State::Fetched => {}
    }

    let workdir = paths::workdir_for(cfg, run_id)?;
    if !workdir.exists() {
        bail!("workdir missing: {}", workdir.display());
    }

    let mut total_files = 0u32;
    let mut artifact_dirs = 0u32;
    for entry in std::fs::read_dir(&workdir)
        .with_context(|| format!("reading workdir {}", workdir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let dir = entry.path();
        match verify_artifact_dir(&dir) {
            Ok(n) => {
                total_files += n;
                artifact_dirs += 1;
            }
            Err(e) => {
                db.set_state(run_id, State::Failed, Some(&e.to_string()))?;
                return Err(e);
            }
        }
    }

    if artifact_dirs == 0 {
        bail!(
            "no artifact subdirectories found under {}",
            workdir.display()
        );
    }

    db.set_state(run_id, State::Verified, None)?;
    info!(run_id, artifact_dirs, total_files, "verify complete");
    Ok(())
}

/// Verify all files in one per-HOST artifact subdirectory against its
/// SHA256SUMS.part manifest. Returns the number of files verified.
fn verify_artifact_dir(dir: &Path) -> Result<u32> {
    let manifest_path = dir.join("SHA256SUMS.part");
    if !manifest_path.exists() {
        bail!("missing SHA256SUMS.part in artifact dir {}", dir.display());
    }
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;

    // Map basename -> expected hex digest. We index by basename because guix
    // build.sh emits paths relative to /outdir-base (e.g. `<HOST>/file.tar.gz`),
    // while inside the GHA artifact zip the same file lives at the root.
    let mut expected: HashMap<String, String> = HashMap::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (hex, fname) = line.split_once(char::is_whitespace).ok_or_else(|| {
            anyhow!(
                "malformed manifest line {}:{}: {raw}",
                manifest_path.display(),
                lineno + 1
            )
        })?;
        let fname = fname.trim_start().trim_start_matches('*');
        let basename = Path::new(fname)
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("bad filename in {}: {fname}", manifest_path.display()))?;
        expected.insert(basename.to_string(), hex.to_lowercase());
    }

    let mut verified = 0u32;
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name == "SHA256SUMS.part" {
            continue;
        }
        let Some(want) = expected.get(name) else {
            warn!(file = %path.display(), "file not listed in SHA256SUMS.part — skipping");
            continue;
        };
        let got = paths::sha256_file(&path)?;
        if &got != want {
            bail!(
                "sha256 mismatch for {}: expected {want}, got {got}",
                path.display()
            );
        }
        verified += 1;
    }

    if verified == 0 {
        bail!(
            "no files in {} matched any SHA256SUMS.part entry",
            dir.display()
        );
    }
    Ok(verified)
}
