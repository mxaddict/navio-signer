use anyhow::Result;
use tracing::info;

use crate::cli::{Cli, RunArgs};

pub async fn run(_cli: &Cli, args: RunArgs) -> Result<()> {
    info!(run_id = args.run_id, "fetch: not implemented yet (Phase 3)");
    Ok(())
}
