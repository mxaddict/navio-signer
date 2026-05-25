use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cli::Cli;
use crate::config::Config;
use crate::db::{Build, Db};

pub async fn run(cli: &Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    let data_dir = Db::data_dir_for(cfg.paths.data_dir.as_deref())?;
    let db = Db::open(&data_dir)?;
    let rows = db.list_all()?;
    print_table(&rows);
    Ok(())
}

fn print_table(rows: &[Build]) {
    if rows.is_empty() {
        println!("(no builds tracked yet — run `navio-signer poll`)");
        return;
    }
    println!(
        "{:>12}  {:<24}  {:<14}  {:<11}  {:<10}  RELEASE",
        "RUN_ID", "REF", "SHA", "STATE", "AGE"
    );
    let now = now_unix();
    for b in rows {
        let age = humanize_age(now.saturating_sub(b.updated_at));
        let release = b
            .release_tag
            .as_deref()
            .map(|t| t.to_string())
            .unwrap_or_else(|| "-".into());
        println!(
            "{:>12}  {:<24}  {:<14}  {:<11}  {:<10}  {}",
            b.run_id,
            truncate(&b.ref_name, 24),
            short_sha(&b.head_sha),
            b.state.as_str(),
            age,
            release
        );
    }
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(12).collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn humanize_age(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else {
        format!("{}d", s / 86_400)
    }
}
