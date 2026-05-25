use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Clone, Parser)]
#[command(
    name = "navio-signer",
    version,
    about = "Sign and publish navio-core guix builds"
)]
pub struct Cli {
    /// Path to config.toml. Falls back to $XDG_CONFIG_HOME/navio-signer/config.toml.
    #[arg(long, env = "NAVIO_SIGNER_CONFIG", global = true)]
    pub config: Option<PathBuf>,

    /// Log level filter (overrides RUST_LOG). e.g. info, debug, navio_signer=trace.
    #[arg(long, env = "NAVIO_SIGNER_LOG", global = true)]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run the full daemon loop: poll -> fetch -> verify -> sign -> publish.
    Daemon,
    /// One-shot: find new guix runs and record them in the state DB.
    Poll,
    /// Download artifacts for a run.
    Fetch(RunArgs),
    /// Verify SHA256 of fetched artifacts against the manifest.
    Verify(RunArgs),
    /// Codesign artifacts (linux GPG, mingw Authenticode, darwin codesign+notarize).
    Sign(RunArgs),
    /// Upload signed artifacts to a GH release.
    Publish(RunArgs),
    /// Dump current state from the DB.
    Status,
}

#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// GitHub Actions workflow_run ID.
    pub run_id: u64,
}
