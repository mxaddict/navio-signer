// Daemon alias: invokes navio-signer with the `daemon` subcommand hardcoded.
// Lets launchd target `navio-signerd` directly without needing an argv tweak.

use anyhow::Result;
use std::process::Command;

fn main() -> Result<()> {
    let exe = std::env::current_exe()?;
    let bin_dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent dir"))?;
    let signer = bin_dir.join("navio-signer");

    let args: Vec<String> = std::env::args().skip(1).collect();
    let status = Command::new(&signer).arg("daemon").args(&args).status()?;

    std::process::exit(status.code().unwrap_or(1));
}
