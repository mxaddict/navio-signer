use anyhow::Result;
use clap::Parser;

mod cli;
mod commands;
mod config;
mod db;
mod github;
mod lockfile;
mod logging;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    logging::init(cli.log_level.as_deref());

    match cli.command.clone() {
        Command::Daemon => commands::daemon::run(&cli).await,
        Command::Poll => commands::poll::run(&cli).await,
        Command::Fetch(args) => commands::fetch::run(&cli, args).await,
        Command::Verify(args) => commands::verify::run(&cli, args).await,
        Command::Sign(args) => commands::sign::run(&cli, args).await,
        Command::Publish(args) => commands::publish::run(&cli, args).await,
        Command::Status => commands::status::run(&cli).await,
    }
}
