use anyhow::Result;
use tracing::info;

use crate::cli::Cli;

pub async fn run(_cli: &Cli) -> Result<()> {
    info!("daemon: not implemented yet (Phase 2 wires the poll loop)");
    Ok(())
}
