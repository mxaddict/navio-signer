use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use crate::cli::{Cli, RunArgs};
use crate::config::Config;
use crate::db::{Db, State};
use crate::github::GhClient;
use crate::paths;

pub async fn run(cli: &Cli, args: RunArgs) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    fetch_one(&cfg, args.run_id).await
}

/// Download all artifacts for a workflow run and extract them into the
/// per-run workdir. Idempotent for builds already in state `fetched` or
/// later: returns Ok without re-downloading.
pub async fn fetch_one(cfg: &Config, run_id: u64) -> Result<()> {
    let data_dir = paths::data_dir(cfg)?;
    let db = Db::open(&data_dir)?;

    let build = db
        .get(run_id)?
        .ok_or_else(|| anyhow::anyhow!("no build row for run_id {run_id}; run `poll` first"))?;

    if build.state != State::Discovered {
        info!(
            run_id,
            state = build.state.as_str(),
            "skipping fetch (state already past discovered)"
        );
        return Ok(());
    }

    let workdir = paths::workdir_for(cfg, run_id)?;
    if workdir.exists() {
        warn!(workdir = %workdir.display(), "removing stale workdir before fetch");
        std::fs::remove_dir_all(&workdir)
            .with_context(|| format!("clearing workdir {}", workdir.display()))?;
    }
    std::fs::create_dir_all(&workdir)
        .with_context(|| format!("creating workdir {}", workdir.display()))?;

    let client = GhClient::new(&cfg.github)?;
    info!(run_id, "listing artifacts");
    let artifacts = client.list_artifacts(run_id).await?;
    if artifacts.is_empty() {
        bail!("workflow run {run_id} has no artifacts (expired or never uploaded?)");
    }

    for artifact in &artifacts {
        if artifact.expired {
            db.set_state(
                run_id,
                State::Failed,
                Some(&format!("artifact {} expired", artifact.name)),
            )?;
            bail!("artifact {} for run {} is expired", artifact.name, run_id);
        }
        let dest = workdir.join(&artifact.name);
        info!(
            artifact = %artifact.name,
            size_bytes = artifact.size_in_bytes,
            "downloading"
        );
        let zip_bytes = client.download_artifact_zip(artifact.id).await?;
        paths::extract_zip(&zip_bytes, &dest)
            .with_context(|| format!("extracting artifact {}", artifact.name))?;
    }

    db.set_state(run_id, State::Fetched, None)?;
    info!(
        run_id,
        workdir = %workdir.display(),
        artifact_count = artifacts.len(),
        "fetch complete"
    );
    Ok(())
}
