use anyhow::Result;
use tracing::{debug, info};

use crate::cli::Cli;
use crate::config::Config;
use crate::db::Db;
use crate::github::GhClient;

/// Page size for the workflow runs query. 50 covers ~50 push events; at our
/// poll cadence the active window is well below that.
const POLL_PAGE_SIZE: u8 = 50;

pub async fn run(cli: &Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    poll_once(&cfg).await
}

/// Reusable polling step: query GitHub for recent successful workflow runs,
/// insert any that match a configured ref into the DB as `discovered`.
pub async fn poll_once(cfg: &Config) -> Result<()> {
    let client = GhClient::new(&cfg.github)?;
    let data_dir = Db::data_dir_for(cfg.paths.data_dir.as_deref())?;
    let db = Db::open(&data_dir)?;

    info!(
        repo = client.repo.as_str(),
        workflow = client.workflow.as_str(),
        "polling for new workflow runs"
    );

    let runs = client.list_recent_successful_runs(POLL_PAGE_SIZE).await?;
    debug!(count = runs.len(), "fetched runs");

    let mut inserted = 0u32;
    let mut skipped_seen = 0u32;
    let mut skipped_ref = 0u32;

    for run in runs {
        let run_id = run.id.0;
        let Some(ref_name) = client.ref_for(&run.head_branch) else {
            skipped_ref += 1;
            continue;
        };
        if db.contains_run(run_id)? {
            skipped_seen += 1;
            continue;
        }
        db.insert_discovered(run_id, &run.head_sha, &ref_name)?;
        inserted += 1;
        info!(
            run_id,
            head_sha = %short_sha(&run.head_sha),
            ref_name = ref_name.as_str(),
            "discovered new build"
        );
    }

    info!(inserted, skipped_seen, skipped_ref, "poll complete");
    Ok(())
}

fn short_sha(sha: &str) -> &str {
    if sha.len() > 12 { &sha[..12] } else { sha }
}
