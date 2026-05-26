use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use std::collections::HashMap;
use tracing::{info, warn};

use crate::cli::{Cli, RunArgs};
use crate::config::Config;
use crate::db::{Db, State};
use crate::github::GhClient;
use crate::paths;
use crate::publish::{
    ReleaseMeta, collect_release_assets, derive_release_meta, parse_changelog_section,
    write_combined_sha256sums,
};
use crate::sign::linux;

pub async fn run(cli: &Cli, args: RunArgs) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    publish_one(&cfg, args.run_id).await
}

/// Publish all signed assets for a run to a GH release on the source repo.
///
/// State gate: requires `Signed`; idempotent if already `Published`.
/// On success: state -> Published, release_id/release_tag persisted,
/// workdir removed.
pub async fn publish_one(cfg: &Config, run_id: u64) -> Result<()> {
    let db = Db::open(&paths::data_dir(cfg)?)?;
    let build = db
        .get(run_id)?
        .ok_or_else(|| anyhow!("no build row for run_id {run_id}"))?;

    match build.state {
        State::Discovered | State::Fetched | State::Verified => bail!(
            "run {run_id} not yet signed (state={})",
            build.state.as_str()
        ),
        State::Published => {
            info!(run_id, "skipping publish (already published)");
            return Ok(());
        }
        State::Failed => bail!("run {run_id} is in failed state; reset manually before retrying"),
        State::Signed => {}
    }

    let workdir = paths::workdir_for(cfg, run_id)?;
    if !workdir.exists() {
        bail!("workdir missing: {}", workdir.display());
    }

    let meta = derive_release_meta(&build.ref_name, &build.head_sha);
    info!(
        run_id,
        tag = meta.tag.as_str(),
        prerelease = meta.prerelease,
        "starting publish"
    );

    let mut assets = collect_release_assets(&workdir)?;
    if assets.is_empty() {
        bail!("no release assets found under {}", workdir.display());
    }

    // Top-level SHA256SUMS over the per-HOST assets, then GPG-sign it.
    let sums_path = workdir.join("SHA256SUMS");
    write_combined_sha256sums(&assets, &sums_path)
        .with_context(|| format!("writing {}", sums_path.display()))?;
    let asc_path = linux::detach_sign(&sums_path, &cfg.signing.linux)
        .context("GPG-signing combined SHA256SUMS")?;
    assets.push(sums_path);
    assets.push(asc_path);
    assets.sort();

    let client = GhClient::new(&cfg.github)?;
    let body = fetch_release_body(&client, &meta, &build.ref_name).await;

    let release = ensure_release(
        &client,
        &meta,
        &build.head_sha,
        body.as_deref().unwrap_or(""),
    )
    .await
    .context("creating or updating release")?;

    // Build name -> asset_id map for clobber.
    let mut existing: HashMap<String, u64> = HashMap::new();
    for asset in &release.assets {
        existing.insert(asset.name.clone(), asset.id.0);
    }

    for path in &assets {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(&asset_id) = existing.get(name) {
            info!(name, "deleting existing asset (clobber)");
            client.delete_asset(asset_id).await?;
        }
        let bytes =
            std::fs::read(path).with_context(|| format!("reading asset {}", path.display()))?;
        let len = bytes.len();
        info!(name, size = len, "uploading asset");
        client
            .upload_asset(release.id.0, name, Bytes::from(bytes))
            .await?;
    }

    db.set_release(run_id, Some(release.id.0 as i64), Some(meta.tag.as_str()))?;
    db.set_state(run_id, State::Published, None)?;
    info!(
        run_id,
        tag = meta.tag.as_str(),
        asset_count = assets.len(),
        "publish complete"
    );

    // Cleanup workdir; per-run state lives in sqlite from here on.
    if let Err(e) = std::fs::remove_dir_all(&workdir) {
        warn!(workdir = %workdir.display(), error = %e, "failed to remove workdir");
    }

    Ok(())
}

async fn ensure_release(
    client: &GhClient,
    meta: &ReleaseMeta,
    target_commitish: &str,
    body: &str,
) -> Result<octocrab::models::repos::Release> {
    if let Some(existing) = client.get_release_by_tag(&meta.tag).await? {
        info!(
            tag = meta.tag.as_str(),
            id = existing.id.0,
            "uploading to existing release"
        );
        return Ok(existing);
    }
    info!(tag = meta.tag.as_str(), "creating release");
    client
        .create_release(
            &meta.tag,
            target_commitish,
            &meta.title,
            body,
            meta.prerelease,
        )
        .await
}

/// Best-effort release-body fetch from the source repo's CHANGELOG.md.
/// Returns None on any miss; the caller publishes with an empty body in
/// that case (operators can fill it in manually).
async fn fetch_release_body(
    client: &GhClient,
    meta: &ReleaseMeta,
    ref_name: &str,
) -> Option<String> {
    let lookup_ref = if let Some(tag) = ref_name.strip_prefix("refs/tags/") {
        tag.to_string()
    } else {
        "master".to_string()
    };
    match client.fetch_file("CHANGELOG.md", &lookup_ref).await {
        Ok(Some(text)) => parse_changelog_section(&text, &meta.changelog_section),
        Ok(None) => {
            warn!("CHANGELOG.md not found at {lookup_ref}; using empty body");
            None
        }
        Err(e) => {
            warn!(error = %e, "failed to fetch CHANGELOG.md; using empty body");
            None
        }
    }
}
