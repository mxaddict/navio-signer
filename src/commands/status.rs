use anyhow::Result;
use tracing::info;

use crate::cli::Cli;

pub async fn run(_cli: &Cli) -> Result<()> {
    info!("status: not implemented yet (Phase 2 wires up DB read)");
    Ok(())
}
