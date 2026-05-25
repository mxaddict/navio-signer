use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use octocrab::Octocrab;
use octocrab::models::workflows::{Run, WorkflowListArtifact};
use octocrab::models::{ArtifactId, RunId};
use octocrab::params::actions::ArchiveFormat;

use crate::config::GithubConfig;

/// One configured ref selector. Specs in config look like `refs/heads/master`,
/// `refs/tags/v*`, or `refs/tags/stable` (exact).
#[derive(Debug, Clone)]
pub enum RefPattern {
    Branch(String),
    /// `refs/tags/<prefix>*` — matches any tag head_branch starting with prefix.
    TagPrefix(String),
    /// `refs/tags/<name>` — exact tag match.
    TagExact(String),
}

impl RefPattern {
    pub fn parse(spec: &str) -> Result<Self> {
        if let Some(name) = spec.strip_prefix("refs/heads/") {
            if name.is_empty() {
                bail!("empty branch in ref pattern: {spec}");
            }
            return Ok(RefPattern::Branch(name.to_string()));
        }
        if let Some(rest) = spec.strip_prefix("refs/tags/") {
            if rest.is_empty() {
                bail!("empty tag in ref pattern: {spec}");
            }
            if let Some(prefix) = rest.strip_suffix('*') {
                return Ok(RefPattern::TagPrefix(prefix.to_string()));
            }
            return Ok(RefPattern::TagExact(rest.to_string()));
        }
        bail!(
            "unsupported ref pattern: {spec} \
             (use refs/heads/<branch>, refs/tags/<name>, or refs/tags/<prefix>*)"
        )
    }

    pub fn matches(&self, head_branch: &str) -> bool {
        match self {
            RefPattern::Branch(b) => head_branch == b,
            RefPattern::TagPrefix(p) => head_branch.starts_with(p),
            RefPattern::TagExact(t) => head_branch == t,
        }
    }
}

pub struct GhClient {
    pub octo: Octocrab,
    pub owner: String,
    pub repo: String,
    pub workflow: String,
    pub refs: Vec<RefPattern>,
}

impl GhClient {
    pub fn new(cfg: &GithubConfig) -> Result<Self> {
        let octo = Octocrab::builder()
            .personal_token(cfg.token.clone())
            .build()
            .context("building octocrab client")?;
        let (owner, repo) = parse_source_repo(&cfg.source_repo)?;
        let refs = cfg
            .refs
            .iter()
            .map(|s| RefPattern::parse(s))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            octo,
            owner,
            repo,
            workflow: cfg.workflow.clone(),
            refs,
        })
    }

    /// Most recent successful push runs of the configured workflow.
    /// Filters server-side by status=success + event=push; ref filtering
    /// is done client-side via `ref_for`.
    pub async fn list_recent_successful_runs(&self, per_page: u8) -> Result<Vec<Run>> {
        let page = self
            .octo
            .workflows(self.owner.clone(), self.repo.clone())
            .list_runs(self.workflow.clone())
            .status("success")
            .event("push")
            .per_page(per_page)
            .send()
            .await
            .context("listing workflow runs")?;
        Ok(page.items)
    }

    /// List all artifacts attached to a workflow run.
    pub async fn list_artifacts(&self, run_id: u64) -> Result<Vec<WorkflowListArtifact>> {
        let etagged = self
            .octo
            .actions()
            .list_workflow_run_artifacts(self.owner.clone(), self.repo.clone(), RunId(run_id))
            .per_page(100u8)
            .send()
            .await
            .context("listing workflow run artifacts")?;
        let page = etagged
            .value
            .ok_or_else(|| anyhow!("artifacts list returned no body"))?;
        Ok(page.items)
    }

    /// Download a single artifact as a zip archive (raw bytes).
    pub async fn download_artifact_zip(&self, artifact_id: ArtifactId) -> Result<Bytes> {
        let bytes = self
            .octo
            .actions()
            .download_artifact(
                self.owner.clone(),
                self.repo.clone(),
                artifact_id,
                ArchiveFormat::Zip,
            )
            .await
            .context("downloading artifact zip")?;
        Ok(bytes)
    }

    /// If `head_branch` matches one of the configured refs, return the
    /// canonical `refs/heads/...` or `refs/tags/...` form for storage.
    pub fn ref_for(&self, head_branch: &str) -> Option<String> {
        for pat in &self.refs {
            if pat.matches(head_branch) {
                return Some(match pat {
                    RefPattern::Branch(_) => format!("refs/heads/{head_branch}"),
                    RefPattern::TagPrefix(_) | RefPattern::TagExact(_) => {
                        format!("refs/tags/{head_branch}")
                    }
                });
            }
        }
        None
    }
}

fn parse_source_repo(spec: &str) -> Result<(String, String)> {
    let (owner, repo) = spec
        .split_once('/')
        .ok_or_else(|| anyhow!("source_repo must be 'owner/name': got {spec}"))?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        bail!("invalid source_repo: {spec}");
    }
    Ok((owner.to_string(), repo.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_pattern_branch() {
        let p = RefPattern::parse("refs/heads/master").unwrap();
        assert!(p.matches("master"));
        assert!(!p.matches("main"));
        assert!(!p.matches("v1.0.0"));
    }

    #[test]
    fn ref_pattern_tag_glob() {
        let p = RefPattern::parse("refs/tags/v*").unwrap();
        assert!(p.matches("v1.0.0"));
        assert!(p.matches("v0.1.0-rc1"));
        assert!(!p.matches("master"));
        assert!(!p.matches("release-1.0"));
    }

    #[test]
    fn ref_pattern_tag_exact() {
        let p = RefPattern::parse("refs/tags/stable").unwrap();
        assert!(p.matches("stable"));
        assert!(!p.matches("stable-1"));
    }

    #[test]
    fn ref_pattern_rejects_unknown() {
        assert!(RefPattern::parse("refs/pulls/123").is_err());
        assert!(RefPattern::parse("master").is_err());
    }

    #[test]
    fn parse_source_repo_ok() {
        assert_eq!(
            parse_source_repo("nav-io/navio-core").unwrap(),
            ("nav-io".into(), "navio-core".into())
        );
    }

    #[test]
    fn parse_source_repo_rejects_bad() {
        assert!(parse_source_repo("nav-io").is_err());
        assert!(parse_source_repo("/navio").is_err());
        assert!(parse_source_repo("nav-io/").is_err());
        assert!(parse_source_repo("a/b/c").is_err());
    }
}
